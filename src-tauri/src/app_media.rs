//! `appmedia://` プロトコル（#402）。アプリ形式アーティファクトの `<video>` / `<audio>` が、承認された
//! ローカルファイルと URL を `src` に直接（パス指定で）使えるようにする。
//!
//! iframe から見えるオリジンは `http://appmedia.localhost`（Windows。他の OS は `appmedia://localhost`）の
//! 1 つだけで、すべての要求を Rust が検査して返す。ルーティングは**パスだけ**で行い、host は見ない。
//!
//! - `/fs?u=worktree://…`: `media.read` のスコープに入るローカルファイル（`app_fs::resolve_scoped_path`）
//! - `/net?u=https://…`: `media.allow` のプレフィックスに入る URL（`app_net` と同じ検査つきクライアント）
//!
//! # 認可
//!
//! 要求元 webview のラベル（`appinst-<n>` / `appinst-<n>-w<k>`）から実行中のインスタンスを引き、保存済みの
//! 承認（`resolved.media`）だけを見る。アプリ以外の webview・停止中のインスタンス・権限の無いアプリは 403。
//!
//! # チャンク応答
//!
//! Tauri のプロトコル応答は本体をストリームできないので、**常に 206 のチャンク応答**にする（1 回の応答は
//! [`CHUNK_BYTES`] まで）。`<video>` は `Range: bytes=N-` で続きを取りに来るので、シークも効く。
//! Range が無い要求は `bytes=0-` として扱う。配信するのは拡張子（または上流の Content-Type）がメディアの
//! ものだけで、任意ファイルの読み出し口にはしない。

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use futures_util::future::Abortable;
use tauri::http::{header, Method, Request, Response};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder};

use crate::app_fs::Access;
use crate::app_instance::{add_log, context_for_label, InstanceContext, LABEL_PREFIX};
use crate::app_manifest::ResolvedMedia;
use crate::app_net::{AppNetRegistry, Policy, SystemLookup};

/// 1 回の応答で返す最大バイト数
pub const CHUNK_BYTES: u64 = 2 * 1024 * 1024;
/// インスタンス（ラベル）ごとの同時要求数。`net.fetch` の枠とは分ける（動画の Range 要求で
/// `net.fetch` を枯らさないため）。超えた要求は 503（`<video>` は 503 を再試行せずエラーにする）。
/// ブラウザが取り消した要求も、完了か全体タイムアウトまで枠を占有する（Tauri のプロトコル応答には
/// キャンセル通知が無い）ので、複数の要素や連続シークで埋まらない程度に大きくしてある
const MAX_CONCURRENT_PER_INSTANCE: usize = 24;
/// 上流が Range を無視（200）したときに、読み捨ててよい先頭のバイト数。チャンクごとに先頭から
/// 読み捨てるので、帯域の消費がチャンク数の 2 乗で増える。超える位置は 502 にする
const MAX_UPSTREAM_SKIP_BYTES: u64 = 8 * 1024 * 1024;
/// `u` パラメータの最大長（デコード後）
const MAX_SRC_CHARS: usize = 2048;

/// 実行中の要求の台帳。構造は `AppNetRegistry` と同じで、枠だけ別に持つ。
#[derive(Default)]
pub struct AppMediaRegistry(AppNetRegistry);

/// インスタンスの停止時（`app_instance::release`）。実行中の要求を中断し、以後は受け付けない。
pub fn on_instance_released(app: &AppHandle, label: &str) {
    app.state::<AppMediaRegistry>().0.abort_all(label, true);
}

/// ホットリロード・権限セットの入れ替え時。実行中の要求は中断する（新しい iframe が取り直す）。
pub fn on_reload(app: &AppHandle, label: &str) {
    app.state::<AppMediaRegistry>().0.abort_all(label, false);
}

// ─── ルーティング ──────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Fs(String),
    Net(String),
}

fn parse_route(uri: &str) -> Result<Route, String> {
    let url = reqwest::Url::parse(uri).map_err(|e| format!("URL を解釈できません: {}", e))?;
    let src = url
        .query_pairs()
        .find(|(k, _)| k == "u")
        .map(|(_, v)| v.into_owned())
        .ok_or_else(|| "u パラメータがありません".to_string())?;
    if src.chars().count() > MAX_SRC_CHARS {
        return Err(format!("u が長すぎます（{} 文字まで）", MAX_SRC_CHARS));
    }
    match url.path() {
        "/fs" => Ok(Route::Fs(src)),
        "/net" => Ok(Route::Net(src)),
        other => Err(format!("未知のパス '{}' です（/fs と /net だけ）", other)),
    }
}

fn is_instance_label(label: &str) -> bool {
    label.starts_with(LABEL_PREFIX)
}

// ─── Range ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeReq {
    /// `bytes=N-`
    From(u64),
    /// `bytes=N-M`（両端を含む）
    Bounded(u64, u64),
    /// `bytes=-N`（末尾 N バイト）
    Suffix(u64),
}

/// Range ヘッダの解釈。ヘッダが無い・形式が不正なものは `bytes=0-` として扱う（RFC 9110 は不正な
/// Range の無視を許す）。複数範囲は先頭の 1 つだけを返す。
fn parse_range(header: Option<&str>) -> RangeReq {
    let first = header
        .and_then(|h| h.trim().strip_prefix("bytes="))
        .and_then(|spec| spec.split(',').next())
        .map(str::trim)
        .and_then(|spec| spec.split_once('-'));
    let Some((a, b)) = first else { return RangeReq::From(0) };
    let (a, b) = (a.trim(), b.trim());
    match (a.is_empty(), b.is_empty()) {
        (true, false) => b.parse().map(RangeReq::Suffix).unwrap_or(RangeReq::From(0)),
        (false, true) => a.parse().map(RangeReq::From).unwrap_or(RangeReq::From(0)),
        (false, false) => match (a.parse(), b.parse()) {
            (Ok(s), Ok(e)) => RangeReq::Bounded(s, e),
            _ => RangeReq::From(0),
        },
        (true, true) => RangeReq::From(0),
    }
}

/// 総サイズが分かっているときに、返す範囲（両端を含む）を決める。満たせない（416）なら `None`。
fn plan_chunk(req: RangeReq, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let (start, want_end) = match req {
        RangeReq::From(s) => (s, u64::MAX),
        RangeReq::Bounded(s, e) => (s, e),
        RangeReq::Suffix(n) => {
            if n == 0 {
                return None;
            }
            (size.saturating_sub(n), u64::MAX)
        }
    };
    if start >= size || want_end < start {
        return None;
    }
    let end = want_end.min(size - 1).min(start.saturating_add(CHUNK_BYTES - 1));
    Some((start, end))
}

/// 上流へ送る Range（総サイズが分からないので、チャンク上限だけで切り詰める）。
fn upstream_range(req: RangeReq) -> String {
    match req {
        RangeReq::From(s) => format!("bytes={}-{}", s, s.saturating_add(CHUNK_BYTES - 1)),
        RangeReq::Bounded(s, e) => format!("bytes={}-{}", s, e.min(s.saturating_add(CHUNK_BYTES - 1))),
        RangeReq::Suffix(n) => format!("bytes=-{}", n.min(CHUNK_BYTES)),
    }
}

/// `bytes a-b/total`（total は `*` でもよい）。
fn parse_content_range(h: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = h.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (a, b) = range.trim().split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    let (a, b): (u64, u64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a <= b).then_some((a, b, total))
}

// ─── Content-Type ─────────────────────────────────────────────────

/// ファイル名の拡張子から、配信してよいメディアの Content-Type を引く。表に無いものは `None`。
fn media_type_for_name(name: &str) -> Option<&'static str> {
    let (_, ext) = name.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        _ => return None,
    })
}

fn is_media_content_type(ct: &str) -> bool {
    let ct = ct.trim().to_ascii_lowercase();
    ct.starts_with("video/") || ct.starts_with("audio/") || ct.starts_with("application/ogg")
}

// ─── 応答 ────────────────────────────────────────────────────────

/// 失敗の応答。`log` が真ならアプリのデバッグログにも残す。
struct Reply {
    status: u16,
    msg: String,
    /// 416 のときの総サイズ（`Content-Range: bytes */size`）
    range_total: Option<u64>,
    log: bool,
}

impl Reply {
    fn new(status: u16, msg: impl Into<String>) -> Self {
        Reply { status, msg: msg.into(), range_total: None, log: true }
    }
    fn forbidden(msg: impl Into<String>) -> Self {
        Reply::new(403, msg)
    }
    fn not_satisfiable(size: u64) -> Self {
        Reply { status: 416, msg: "範囲が不正です".to_string(), range_total: Some(size), log: false }
    }
    fn into_response(self) -> Response<Vec<u8>> {
        let mut b = Response::builder()
            .status(self.status)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store");
        if let Some(size) = self.range_total {
            b = b.header(header::CONTENT_RANGE, format!("bytes */{}", size));
        }
        b.body(self.msg.into_bytes()).unwrap_or_else(|_| Response::new(Vec::new()))
    }
}

fn plain(status: u16, msg: &str) -> Response<Vec<u8>> {
    Reply::new(status, msg).into_response()
}

/// 206 のチャンク応答。`total` が `None` のときは総サイズ不明（`*`）。HEAD は本体を空にする。
fn chunk_response(
    content_type: &str,
    start: u64,
    end: u64,
    total: Option<u64>,
    body: Vec<u8>,
    head: bool,
) -> Response<Vec<u8>> {
    let total = total.map(|t| t.to_string()).unwrap_or_else(|| "*".to_string());
    Response::builder()
        .status(206)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total))
        .header(header::CONTENT_LENGTH, (end - start + 1).to_string())
        // 承認を取り消した後に、WebView のキャッシュから再生され続けないようにする
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(if head { Vec::new() } else { body })
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

// ─── 認可 ────────────────────────────────────────────────────────

fn authorize(app: &AppHandle, label: &str) -> Result<(InstanceContext, ResolvedMedia), Response<Vec<u8>>> {
    if !is_instance_label(label) {
        log::warn!("[app-media] アプリ以外の webview からの要求を拒否: {}", label);
        return Err(plain(403, "アプリのウィンドウからだけ使えます"));
    }
    let ctx = context_for_label(app, label).map_err(|e| {
        log::warn!("[app-media] 要求を拒否: {} — {}", label, e);
        plain(403, &e)
    })?;
    let media = app
        .state::<crate::app_grant::AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .and_then(|g| g.resolved.media);
    match media {
        Some(m) => Ok((ctx, m)),
        None => {
            log_denied(app, &ctx, "?", "このアプリにはメディア再生の権限が承認されていません");
            Err(plain(403, "メディア再生の権限が承認されていません"))
        }
    }
}

/// 拒否・失敗はアプリのデバッグログへ残す。成功は残さない（Range 要求は 1 本の動画で何十回も来る）。
fn log_denied(app: &AppHandle, ctx: &InstanceContext, src: &str, error: &str) {
    let note = if ctx.reapproval_pending { "（再承認待ちのため、新しい権限はまだ使えません）" } else { "" };
    let src = crate::app_net::loggable_url(src);
    log::debug!("[app-media] 拒否 {}/{} {} — {}", ctx.worktree_id, ctx.artifact_id, src, error);
    add_log(
        app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        "warn",
        format!("media を拒否/失敗: {} — {}{}", src, error, note),
    );
}

// ─── 配信 ────────────────────────────────────────────────────────

/// プロトコルのエントリポイント。
pub fn handle(ctx: UriSchemeContext<'_, tauri::Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let label = ctx.webview_label().to_string();
    let method = request.method().clone();
    let uri = request.uri().to_string();
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    // どの経路でも必ず respond する（落とすと <video> が待ち続ける）
    tauri::async_runtime::spawn(async move {
        responder.respond(serve(app, label, method, uri, range).await);
    });
}

async fn serve(
    app: AppHandle,
    label: String,
    method: Method,
    uri: String,
    range: Option<String>,
) -> Response<Vec<u8>> {
    let head = match method {
        Method::GET => false,
        Method::HEAD => true,
        _ => return plain(405, "GET / HEAD だけです"),
    };
    let (ctx, media) = match authorize(&app, &label) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let route = match parse_route(&uri) {
        Ok(r) => r,
        Err(e) => {
            log_denied(&app, &ctx, &uri, &e);
            return plain(400, &e);
        }
    };
    let req = parse_range(range.as_deref());

    let reg = app.state::<AppMediaRegistry>();
    let (id, abort_reg) = match reg.0.begin_with_limit(&ctx.instance_label, MAX_CONCURRENT_PER_INSTANCE) {
        Ok(v) => v,
        Err(e) => {
            log::debug!("[app-media] {} — {}", ctx.instance_label, e);
            return plain(503, &e);
        }
    };
    let (src, fut) = match &route {
        Route::Fs(u) => (u.as_str(), futures_util::future::Either::Left(serve_fs(&media, u, req, head))),
        Route::Net(u) => (u.as_str(), futures_util::future::Either::Right(serve_net(&app, &media, u, req, head))),
    };
    let out = match Abortable::new(fut, abort_reg).await {
        Ok(r) => r,
        Err(_) => Err(Reply { log: false, ..Reply::new(503, "アプリの再読み込みまたは停止で中断しました") }),
    };
    reg.0.end(&ctx.instance_label, id);
    match out {
        Ok(resp) => resp,
        Err(r) => {
            if r.log {
                log_denied(&app, &ctx, src, &r.msg);
            }
            r.into_response()
        }
    }
}

async fn serve_fs(media: &ResolvedMedia, u: &str, req: RangeReq, head: bool) -> Result<Response<Vec<u8>>, Reply> {
    let fs = media
        .fs
        .clone()
        .ok_or_else(|| Reply::forbidden("ローカルファイルの再生（media.read）が承認されていません"))?;
    let u = u.to_string();
    tauri::async_runtime::spawn_blocking(move || read_local(&fs, &u, req, head))
        .await
        .map_err(|e| Reply::new(500, format!("読み出しに失敗しました: {}", e)))?
}

fn read_local(
    fs: &crate::app_manifest::ResolvedFs,
    u: &str,
    req: RangeReq,
    head: bool,
) -> Result<Response<Vec<u8>>, Reply> {
    // パストラバーサル・symlink・スコープ外の検査はここで効く
    let path = crate::app_fs::resolve_scoped_path(fs, u, Access::Read).map_err(Reply::forbidden)?;
    let content_type = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(media_type_for_name)
        .ok_or_else(|| Reply::forbidden("メディアとして配信できない拡張子です"))?;
    let meta = std::fs::metadata(&path).map_err(|e| Reply::new(404, format!("ファイルを開けません: {}", e)))?;
    if !meta.is_file() {
        return Err(Reply::new(404, "ファイルではありません"));
    }
    let size = meta.len();
    let (start, end) = plan_chunk(req, size).ok_or_else(|| Reply::not_satisfiable(size))?;
    let mut body = Vec::new();
    if !head {
        let mut f = std::fs::File::open(&path).map_err(|e| Reply::new(404, format!("ファイルを開けません: {}", e)))?;
        f.seek(SeekFrom::Start(start)).map_err(|e| Reply::new(500, e.to_string()))?;
        body = vec![0u8; (end - start + 1) as usize];
        f.read_exact(&mut body).map_err(|e| Reply::new(500, format!("読み出しに失敗しました: {}", e)))?;
    }
    Ok(chunk_response(content_type, start, end, Some(size), body, head))
}

async fn serve_net(
    app: &AppHandle,
    media: &ResolvedMedia,
    u: &str,
    req: RangeReq,
    head: bool,
) -> Result<Response<Vec<u8>>, Reply> {
    if media.allow.is_empty() {
        return Err(Reply::forbidden("URL の再生（media.allow）が承認されていません"));
    }
    let parsed = reqwest::Url::parse(u).map_err(|e| Reply::forbidden(format!("URL を解釈できません: {}", e)))?;
    let policy = Arc::new(Policy::new(&media.allow, crate::app_net::own_ports(app)));
    policy.check_url(&parsed).map_err(Reply::forbidden)?;

    let (client, denied) = crate::app_net::build_guarded_client(policy, Arc::new(SystemLookup), &parsed)
        .map_err(|e| Reply::new(500, e))?;
    let take_denied = || denied.lock().ok().and_then(|mut d| d.take());
    let fail = |e: reqwest::Error| match take_denied() {
        Some(msg) => Reply::forbidden(msg),
        None => Reply::new(502, crate::app_net::describe_error(&e)),
    };

    // 検査済みの URL へ Range 付きで取りに行く（Cookie は送らず、Set-Cookie は中継しない）
    let mut resp = client
        .get(parsed.clone())
        .header(header::RANGE, upstream_range(req))
        .send()
        .await
        .map_err(&fail)?;

    let status = resp.status().as_u16();
    let name = parsed.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    let upstream_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|ct| is_media_content_type(ct))
        .map(|ct| ct.split(';').next().unwrap_or(ct).trim().to_string());
    let from_ext = media_type_for_name(name);

    match status {
        206 => {
            let content_type = upstream_type
                .or_else(|| from_ext.map(str::to_string))
                .ok_or_else(|| Reply::forbidden("メディアとして配信できない Content-Type です"))?;
            let (start, mut end, total) = resp
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range)
                .ok_or_else(|| Reply::new(502, "上流の Content-Range を解釈できません"))?;
            // 上流がこちらの上限より大きい範囲を返しても、上限までしか読まない
            end = end.min(start.saturating_add(CHUNK_BYTES - 1));
            if head {
                return Ok(chunk_response(&content_type, start, end, total, Vec::new(), true));
            }
            let want = end - start + 1;
            let body = read_window(&mut resp, 0, want).await.map_err(&fail)?;
            if (body.len() as u64) < want {
                return Err(Reply::new(502, "上流の応答が途中で終わりました"));
            }
            Ok(chunk_response(&content_type, start, end, total, body, false))
        }
        200 => {
            // 上流が Range を無視した。総サイズが分かるときだけ、該当範囲を切り出して 206 で返す
            let content_type = upstream_type
                .or_else(|| from_ext.map(str::to_string))
                .ok_or_else(|| Reply::forbidden("メディアとして配信できない Content-Type です"))?;
            let size = resp
                .content_length()
                .ok_or_else(|| Reply::new(502, "上流が Range を無視し、総サイズも分かりません"))?;
            let (start, end) = plan_chunk(req, size).ok_or_else(|| Reply::not_satisfiable(size))?;
            if start > MAX_UPSTREAM_SKIP_BYTES {
                return Err(Reply::new(
                    502,
                    format!(
                        "上流が Range に対応していないため、先頭から {} MiB を超える位置は再生できません",
                        MAX_UPSTREAM_SKIP_BYTES / (1024 * 1024)
                    ),
                ));
            }
            if head {
                return Ok(chunk_response(&content_type, start, end, Some(size), Vec::new(), true));
            }
            let want = end - start + 1;
            let body = read_window(&mut resp, start, want).await.map_err(&fail)?;
            if (body.len() as u64) < want {
                return Err(Reply::new(502, "上流の応答が途中で終わりました"));
            }
            // resp を落として接続を切る
            Ok(chunk_response(&content_type, start, end, Some(size), body, false))
        }
        416 => {
            let total = resp
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().strip_prefix("bytes */"))
                .and_then(|t| t.trim().parse().ok());
            Err(Reply { status: 416, msg: "範囲が不正です".to_string(), range_total: total, log: false })
        }
        404 => Err(Reply::new(404, "上流が 404 を返しました")),
        other => Err(Reply::new(502, format!("上流が {} を返しました", other))),
    }
}

/// 本文の先頭 `skip` バイトを読み捨て、続く `take` バイトまでを集める。ストリームが先に終われば短く返る。
async fn read_window(resp: &mut reqwest::Response, skip: u64, take: u64) -> Result<Vec<u8>, reqwest::Error> {
    let mut to_skip = skip;
    let mut out: Vec<u8> = Vec::with_capacity(take.min(CHUNK_BYTES) as usize);
    while (out.len() as u64) < take {
        let Some(chunk) = resp.chunk().await? else { break };
        let mut data: &[u8] = &chunk;
        if to_skip > 0 {
            let n = to_skip.min(data.len() as u64) as usize;
            to_skip -= n as u64;
            data = &data[n..];
        }
        let room = (take - out.len() as u64) as usize;
        out.extend_from_slice(&data[..data.len().min(room)]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    // ── parse_range ──

    #[test]
    fn missing_or_malformed_range_is_from_zero() {
        assert_eq!(parse_range(None), RangeReq::From(0));
        assert_eq!(parse_range(Some("")), RangeReq::From(0));
        assert_eq!(parse_range(Some("items=0-1")), RangeReq::From(0));
        assert_eq!(parse_range(Some("bytes=abc-")), RangeReq::From(0));
        assert_eq!(parse_range(Some("bytes=-")), RangeReq::From(0));
        assert_eq!(parse_range(Some("bytes=1-x")), RangeReq::From(0));
    }

    #[test]
    fn parses_open_bounded_and_suffix_ranges() {
        assert_eq!(parse_range(Some("bytes=0-")), RangeReq::From(0));
        assert_eq!(parse_range(Some("bytes=1048576-")), RangeReq::From(MIB));
        assert_eq!(parse_range(Some("bytes=10-19")), RangeReq::Bounded(10, 19));
        assert_eq!(parse_range(Some("bytes=-500")), RangeReq::Suffix(500));
        assert_eq!(parse_range(Some(" bytes= 5 - 9 ")), RangeReq::Bounded(5, 9));
    }

    #[test]
    fn multi_range_uses_the_first_range_only() {
        assert_eq!(parse_range(Some("bytes=0-9, 20-29")), RangeReq::Bounded(0, 9));
        assert_eq!(parse_range(Some("bytes=100-, 0-9")), RangeReq::From(100));
    }

    // ── plan_chunk ──

    #[test]
    fn open_range_is_truncated_to_the_chunk_size() {
        let size = 10 * MIB;
        assert_eq!(plan_chunk(RangeReq::From(0), size), Some((0, CHUNK_BYTES - 1)));
        assert_eq!(plan_chunk(RangeReq::From(MIB), size), Some((MIB, MIB + CHUNK_BYTES - 1)));
        // 末尾では総サイズで切る
        assert_eq!(plan_chunk(RangeReq::From(size - 10), size), Some((size - 10, size - 1)));
    }

    #[test]
    fn bounded_range_is_clamped_to_size_and_chunk() {
        assert_eq!(plan_chunk(RangeReq::Bounded(10, 19), 100), Some((10, 19)));
        assert_eq!(plan_chunk(RangeReq::Bounded(10, 1000), 100), Some((10, 99)));
        assert_eq!(plan_chunk(RangeReq::Bounded(0, 100 * MIB), 100 * MIB), Some((0, CHUNK_BYTES - 1)));
    }

    #[test]
    fn suffix_range_starts_at_size_minus_n() {
        assert_eq!(plan_chunk(RangeReq::Suffix(10), 100), Some((90, 99)));
        // 総サイズより大きい suffix は全体
        assert_eq!(plan_chunk(RangeReq::Suffix(1000), 100), Some((0, 99)));
        assert_eq!(plan_chunk(RangeReq::Suffix(0), 100), None);
    }

    #[test]
    fn unsatisfiable_ranges_return_none() {
        // start >= size
        assert_eq!(plan_chunk(RangeReq::From(100), 100), None);
        assert_eq!(plan_chunk(RangeReq::Bounded(200, 300), 100), None);
        // end < start
        assert_eq!(plan_chunk(RangeReq::Bounded(20, 10), 100), None);
        // 空ファイルはどの範囲も満たせない
        assert_eq!(plan_chunk(RangeReq::From(0), 0), None);
        assert_eq!(plan_chunk(RangeReq::Suffix(1), 0), None);
    }

    #[test]
    fn upstream_range_is_truncated_without_knowing_the_size() {
        assert_eq!(upstream_range(RangeReq::From(0)), format!("bytes=0-{}", CHUNK_BYTES - 1));
        assert_eq!(upstream_range(RangeReq::From(100)), format!("bytes=100-{}", 100 + CHUNK_BYTES - 1));
        assert_eq!(upstream_range(RangeReq::Bounded(10, 19)), "bytes=10-19");
        assert_eq!(upstream_range(RangeReq::Bounded(0, u64::MAX)), format!("bytes=0-{}", CHUNK_BYTES - 1));
        assert_eq!(upstream_range(RangeReq::Suffix(5)), "bytes=-5");
        assert_eq!(upstream_range(RangeReq::Suffix(u64::MAX)), format!("bytes=-{}", CHUNK_BYTES));
    }

    #[test]
    fn parses_content_range() {
        assert_eq!(parse_content_range("bytes 0-99/1000"), Some((0, 99, Some(1000))));
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(parse_content_range("bytes 9-5/100"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes a-b/c"), None);
    }

    // ── ルート ──

    #[test]
    fn routes_by_path_and_ignores_host() {
        let u = "worktree%3A%2F%2Fvideos%2Fa%20b.mp4";
        let want = Route::Fs("worktree://videos/a b.mp4".to_string());
        assert_eq!(parse_route(&format!("http://appmedia.localhost/fs?u={}", u)), Ok(want));
        // Windows 以外のオリジンや、Tauri が書き換えた形でも同じ
        assert_eq!(
            parse_route("appmedia://localhost/net?u=https%3A%2F%2Fcdn.example.com%2Fv.mp4%3Fa%3D1"),
            Ok(Route::Net("https://cdn.example.com/v.mp4?a=1".to_string()))
        );
    }

    #[test]
    fn rejects_unknown_paths_and_missing_source() {
        assert!(parse_route("http://appmedia.localhost/other?u=x").is_err());
        assert!(parse_route("http://appmedia.localhost/fs").is_err());
        assert!(parse_route("http://appmedia.localhost/fs/?u=x").is_err());
        assert!(parse_route("not a url").is_err());
        let long = "a".repeat(MAX_SRC_CHARS + 1);
        assert!(parse_route(&format!("http://appmedia.localhost/fs?u={}", long)).is_err());
    }

    #[test]
    fn only_app_instance_labels_are_accepted() {
        assert!(is_instance_label("appinst-1"));
        assert!(is_instance_label("appinst-1-w2"));
        for other in ["main", "artifact-x", "sub-home", "tray-popup", "", "appinst"] {
            assert!(!is_instance_label(other), "{}", other);
        }
    }

    // ── Content-Type ──

    #[test]
    fn maps_media_extensions_and_rejects_the_rest() {
        assert_eq!(media_type_for_name("a.mp4"), Some("video/mp4"));
        assert_eq!(media_type_for_name("A.MP4"), Some("video/mp4"));
        assert_eq!(media_type_for_name("a.m4v"), Some("video/mp4"));
        assert_eq!(media_type_for_name("a.webm"), Some("video/webm"));
        assert_eq!(media_type_for_name("a.ogv"), Some("video/ogg"));
        assert_eq!(media_type_for_name("a.mov"), Some("video/quicktime"));
        assert_eq!(media_type_for_name("a.mp3"), Some("audio/mpeg"));
        assert_eq!(media_type_for_name("a.m4a"), Some("audio/mp4"));
        assert_eq!(media_type_for_name("a.ogg"), Some("audio/ogg"));
        assert_eq!(media_type_for_name("a.wav"), Some("audio/wav"));
        assert_eq!(media_type_for_name("a.flac"), Some("audio/flac"));
        for bad in ["a.txt", "a.json", "a.exe", "a", "mp4", "a.mp4.txt", ".env", ""] {
            assert_eq!(media_type_for_name(bad), None, "{}", bad);
        }
    }

    #[test]
    fn upstream_content_type_must_be_media() {
        assert!(is_media_content_type("video/mp4"));
        assert!(is_media_content_type("Audio/MPEG; charset=x"));
        assert!(is_media_content_type("application/ogg"));
        assert!(!is_media_content_type("text/html"));
        assert!(!is_media_content_type("application/octet-stream"));
        assert!(!is_media_content_type(""));
    }

    // ── 応答 ──

    fn header_of<'a>(r: &'a Response<Vec<u8>>, name: &str) -> &'a str {
        r.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
    }

    #[test]
    fn chunk_response_carries_range_headers() {
        let r = chunk_response("video/mp4", 10, 19, Some(100), vec![0; 10], false);
        assert_eq!(r.status(), 206);
        assert_eq!(header_of(&r, "content-range"), "bytes 10-19/100");
        assert_eq!(header_of(&r, "content-length"), "10");
        assert_eq!(header_of(&r, "accept-ranges"), "bytes");
        assert_eq!(header_of(&r, "content-type"), "video/mp4");
        assert_eq!(header_of(&r, "cache-control"), "no-store");
        assert_eq!(r.body().len(), 10);
    }

    #[test]
    fn head_response_has_headers_but_no_body() {
        let r = chunk_response("video/mp4", 0, 99, Some(100), vec![0; 100], true);
        assert_eq!(header_of(&r, "content-length"), "100");
        assert!(r.body().is_empty());
    }

    #[test]
    fn unknown_total_is_star() {
        let r = chunk_response("audio/mpeg", 5, 9, None, vec![0; 5], false);
        assert_eq!(header_of(&r, "content-range"), "bytes 5-9/*");
    }

    #[test]
    fn unsatisfiable_reply_reports_the_size() {
        let r = Reply::not_satisfiable(100).into_response();
        assert_eq!(r.status(), 416);
        assert_eq!(header_of(&r, "content-range"), "bytes */100");
        assert_eq!(header_of(&r, "cache-control"), "no-store");
    }

    #[test]
    fn denial_is_403_and_not_cacheable() {
        let r = Reply::forbidden("x").into_response();
        assert_eq!(r.status(), 403);
        assert_eq!(header_of(&r, "cache-control"), "no-store");
    }

    // ── 同時実行の枠 ──

    #[test]
    fn registry_limits_per_label_and_frees_slots() {
        let reg = AppMediaRegistry::default();
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_PER_INSTANCE {
            held.push(reg.0.begin_with_limit("appinst-1", MAX_CONCURRENT_PER_INSTANCE).unwrap());
        }
        // 枠が埋まると 503 になる。別のインスタンスには影響しない
        assert!(reg.0.begin_with_limit("appinst-1", MAX_CONCURRENT_PER_INSTANCE).is_err());
        assert!(reg.0.begin_with_limit("appinst-2", MAX_CONCURRENT_PER_INSTANCE).is_ok());
        // 終わった要求の枠は戻る
        let (id, _) = held.pop().unwrap();
        reg.0.end("appinst-1", id);
        assert!(reg.0.begin_with_limit("appinst-1", MAX_CONCURRENT_PER_INSTANCE).is_ok());
        // 停止したラベルは受け付けない
        reg.0.abort_all("appinst-1", true);
        assert!(reg.0.begin_with_limit("appinst-1", MAX_CONCURRENT_PER_INSTANCE).is_err());
    }

    #[test]
    fn upstream_skip_limit_is_a_few_chunks() {
        // Range を無視する上流でも、先頭の数チャンクぶんは再生でき、無限には読み捨てない
        assert!(MAX_UPSTREAM_SKIP_BYTES >= CHUNK_BYTES);
        assert!(MAX_UPSTREAM_SKIP_BYTES <= 16 * MIB);
    }

    // ── ローカルファイルの読み出し ──

    #[test]
    fn local_read_returns_the_requested_chunk() {
        use crate::app_manifest::{ResolvedFs, ResolvedFsScope};
        let dir = std::env::temp_dir().join(format!("oretachi-media-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("v")).unwrap();
        let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        std::fs::write(dir.join("v").join("a.mp4"), &data).unwrap();
        std::fs::write(dir.join("v").join("a.txt"), b"secret").unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        let fs = ResolvedFs {
            worktree_root: Some(root.to_string_lossy().into_owned()),
            artifact_root: None,
            scopes: vec![ResolvedFsScope {
                scheme: "worktree".into(),
                prefix: "v/".into(),
                path: root.join("v").to_string_lossy().into_owned(),
                write: false,
            }],
            allow_sensitive_paths: false,
        };

        let r = read_local(&fs, "worktree://v/a.mp4", RangeReq::Bounded(100, 199), false).ok().unwrap();
        assert_eq!(r.status(), 206);
        assert_eq!(header_of(&r, "content-range"), "bytes 100-199/5000");
        assert_eq!(r.body().as_slice(), &data[100..200]);

        // 末尾まで
        let r = read_local(&fs, "worktree://v/a.mp4", RangeReq::From(4990), false).ok().unwrap();
        assert_eq!(r.body().as_slice(), &data[4990..]);

        // 範囲外は 416、メディアでない拡張子・スコープ外・.. は 403
        assert_eq!(read_local(&fs, "worktree://v/a.mp4", RangeReq::From(5000), false).err().unwrap().status, 416);
        assert_eq!(read_local(&fs, "worktree://v/a.txt", RangeReq::From(0), false).err().unwrap().status, 403);
        assert_eq!(read_local(&fs, "worktree://other/a.mp4", RangeReq::From(0), false).err().unwrap().status, 403);
        assert_eq!(read_local(&fs, "worktree://v/../a.mp4", RangeReq::From(0), false).err().unwrap().status, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
