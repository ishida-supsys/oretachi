//! アプリ形式アーティファクトの WebSocket API（`net.ws`）。Rust 側のプロキシ。
//!
//! iframe の CSP（`connect-src 'none'`）は変えず、アプリの WebSocket はここを経由する。関門は 1 箇所で、
//! `app_ws_open` が **承認済みの URL プレフィックス（`AppGrantStore` の `resolved.network`）だけ**を見て通す。
//! プレフィックスは `net.fetch` と同じリスト（`network.allow`）で、`ws://` / `wss://` のものだけが効く
//! （スキームは完全一致。`https://x/` の承認は `wss://x/` に効かない）。詳細は `app_net.rs` の冒頭。
//!
//! # 検査（すべて Rust 側）
//!
//! - URL を承認済みプレフィックスと照合し、ループバック名・IP リテラル・oretachi 自身のポートを拒否する。
//! - **接続先 IP は名前解決の後・接続の後に検査する**。`connect_async` は自前で名前解決するので使わず、
//!   解決結果に禁止 IP が 1 つでも混じれば拒否 → 検査済みの IP へ自分で TCP 接続 → `peer_addr` を再検査 →
//!   （wss なら）TLS → WebSocket のハンドシェイク、の順に手で組む。
//! - システムプロキシは使わない。リダイレクト（3xx）は追わない（101 以外のハンドシェイク応答は失敗）。
//! - TLS の証明書検証は `net.fetch`（reqwest）と同じ OS の検証器（rustls-platform-verifier）。
//! - 1 メッセージの最大サイズ・インスタンスごと / 全体の同時接続数・送信キューの深さ（バックプレッシャー）に上限。
//! - `send` のたびに、保存済みの承認と接続先 URL を照合し直す（取り消し・縮小されていれば切断して拒否）。
//!
//! # 接続の寿命
//!
//! 接続 ID は**呼び出し側（iframe）が決める**（受信イベントがハンドルの登録より先に届いて取りこぼされるのを
//! 避けるため）。台帳のキーは `(インスタンスのラベル, ID)` なので、他のインスタンスの接続は操作できない。
//! インスタンスの停止・承認の取り消し・ホットリロード・権限の入れ替え（`app_net::on_*` 経由）で必ず切断する。
//! リロードでは切断するだけで、新しい iframe が張り直す。
//!
//! 受信・終了・エラーは `emit_to_instance` で `app-ws-event` として push する（aiTerminal / subWindow と同じ経路）。
//! 拒否・失敗はアプリのデバッグログとログファイルへ残す（URL のクエリは落とす）。

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::future::{AbortHandle, AbortRegistration, Abortable};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message, WebSocketConfig};
use tokio_tungstenite::WebSocketStream;

use crate::app_net::{caller_net, loggable_url, own_ports, Lookup, Policy, SystemLookup, CONNECT_TIMEOUT, MAX_URL_CHARS};

// ─── 上限 ────────────────────────────────────────────────────────────

/// 1 メッセージの最大サイズ（受信も送信も。バイト）。iframe ブリッジは base64 で運ぶので、その分の余裕も見た値
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_PER_INSTANCE: usize = 4;
const MAX_TOTAL: usize = 16;
/// 送信キューの深さ（メッセージ数）。満杯なら `send` を拒否する（アプリに待たせず、バッファを無限に伸ばさない）
const SEND_QUEUE_DEPTH: usize = 32;
const MAX_PROTOCOLS: usize = 5;
const MAX_ID_CHARS: usize = 64;
const MAX_CLOSE_REASON_BYTES: usize = 123;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 接続の確立全体（名前解決・TCP・TLS・ハンドシェイク）の上限。iframe ブリッジの長い待ち（30 秒）より短くする
const OPEN_TIMEOUT: Duration = Duration::from_secs(25);
/// 1 回の書き込みが進まないとき（相手が読まない）に諦めるまで
const WRITE_TIMEOUT: Duration = Duration::from_secs(20);
/// こちらから close を送ってから、相手の close を待つ時間
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

pub const EVENT_NAME: &str = "app-ws-event";

// ─── 接続の確立 ──────────────────────────────────────────────────────

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type BoxIo = Box<dyn Io>;
type Ws = WebSocketStream<BoxIo>;

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
        .max_write_buffer_size(MAX_MESSAGE_BYTES * 2)
}

/// `Sec-WebSocket-Protocol` に載せられるトークン（RFC 6455 / RFC 7230 の token）。
fn protocol_is_token(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 128
        && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// 検査 → 名前解決 → TCP → peer 再検査 → TLS → ハンドシェイク。コマンドとテストの両方がここを通る。
async fn connect(
    policy: &Policy,
    lookup: &dyn Lookup,
    url: &reqwest::Url,
    protocols: &[String],
) -> Result<Ws, String> {
    policy.check_ws_url(url)?;
    let Some(host) = url.host() else {
        return Err("URL にホストがありません".to_string());
    };
    let port = url.port_or_known_default().ok_or_else(|| "URL にポートがありません".to_string())?;

    // 接続先の IP を決める。名前の場合は解決結果のすべてを検査する（1 つでも禁止なら拒否）
    let ips: Vec<IpAddr> = match &host {
        url::Host::Ipv4(ip) => vec![IpAddr::V4(*ip)],
        url::Host::Ipv6(ip) => vec![IpAddr::V6(*ip)],
        url::Host::Domain(d) => {
            let ips = lookup
                .lookup(d.to_string())
                .await
                .map_err(|e| format!("名前解決に失敗しました（{}）: {}", d, e))?;
            if ips.is_empty() {
                return Err(format!("名前解決の結果が空です（{}）", d));
            }
            ips
        }
    };
    if let Some(bad) = ips.iter().find(|ip| policy.ip_blocked(**ip)) {
        return Err(format!("{} はループバック／プライベート／予約済みアドレス（{}）へ解決されるため接続できません", host, bad));
    }

    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let mut last = None;
        for ip in &ips {
            match TcpStream::connect((*ip, port)).await {
                Ok(s) => return Ok(s),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("接続先がありません")))
    })
    .await
    .map_err(|_| format!("接続がタイムアウトしました（{} 秒）", CONNECT_TIMEOUT.as_secs()))?
    .map_err(|e| format!("接続に失敗しました: {}", e))?;
    // 接続後の再検査（接続先 IP が検査したものと同じであること）
    let peer = tcp.peer_addr().map_err(|e| format!("接続先を確認できませんでした: {}", e))?;
    if policy.ip_blocked(peer.ip()) {
        return Err(format!("接続先（{}）はループバック／プライベート／予約済みアドレスです", peer.ip()));
    }
    let _ = tcp.set_nodelay(true);

    let io: BoxIo = if url.scheme() == "wss" {
        crate::app_net::ensure_crypto_provider();
        let config = <rustls::ClientConfig as rustls_platform_verifier::ConfigVerifierExt>::with_platform_verifier()
            .map_err(|e| format!("TLS の設定に失敗しました: {}", e))?;
        let server_name = match &host {
            url::Host::Ipv4(ip) => rustls::pki_types::ServerName::IpAddress(IpAddr::V4(*ip).into()),
            url::Host::Ipv6(ip) => rustls::pki_types::ServerName::IpAddress(IpAddr::V6(*ip).into()),
            url::Host::Domain(d) => rustls::pki_types::ServerName::try_from(d.to_string())
                .map_err(|_| format!("ホスト名 '{}' が TLS で使えません", d))?,
        };
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(server_name, tcp))
            .await
            .map_err(|_| "TLS のハンドシェイクがタイムアウトしました".to_string())?
            .map_err(|e| format!("TLS のハンドシェイクに失敗しました: {}", e))?;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };

    let mut target = url.clone();
    target.set_fragment(None);
    let mut request = target
        .as_str()
        .into_client_request()
        .map_err(|e| format!("WebSocket の要求を作れませんでした: {}", e))?;
    if !protocols.is_empty() {
        let value = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&protocols.join(", "))
            .map_err(|_| "サブプロトコルが不正です".to_string())?;
        request.headers_mut().insert("Sec-WebSocket-Protocol", value);
    }
    // 101 以外（3xx のリダイレクトを含む）はここで Err になる。リダイレクトは追わない
    let (ws, _response) = tokio::time::timeout(HANDSHAKE_TIMEOUT, tokio_tungstenite::client_async_with_config(request, io, Some(ws_config())))
        .await
        .map_err(|_| "WebSocket のハンドシェイクがタイムアウトしました".to_string())?
        .map_err(describe_handshake_error)?;
    Ok(ws)
}

fn describe_handshake_error(e: tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error;
    match e {
        Error::Http(resp) => format!(
            "サーバーが WebSocket への切り替えを受け付けませんでした（HTTP {}。リダイレクトは追いません）",
            resp.status().as_u16()
        ),
        other => format!("WebSocket のハンドシェイクに失敗しました: {}", other),
    }
}

// ─── 送受信 ──────────────────────────────────────────────────────────

/// アプリへ push するイベント（`app-ws-event`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WsEvent {
    /// 受信。`binary` なら `data` は base64
    Message { id: String, binary: bool, data: String },
    /// 通信の途中の失敗。直後に `Close` が来る
    Error { id: String, message: String },
    /// 終了。`clean` は close ハンドシェイクを経たか
    Close { id: String, code: Option<u16>, reason: String, clean: bool },
}

pub type Sink = Arc<dyn Fn(WsEvent) + Send + Sync>;

/// 送信キューの要素
#[derive(Debug)]
enum Out {
    Text(String),
    Binary(Vec<u8>),
    Close(Option<(u16, String)>),
}

struct Ended {
    code: Option<u16>,
    reason: String,
    clean: bool,
}

/// 接続を使い切るまで回す。戻り値は終了の理由（`Close` イベントはこの値から出す）。
async fn pump(id: &str, ws: Ws, mut rx: mpsc::Receiver<Out>, sink: &Sink) -> Ended {
    let (mut write, mut read) = ws.split();
    let mut sent_close = false;
    let mut close_deadline: Option<tokio::time::Instant> = None;
    let unclean = |reason: &str| Ended { code: Some(1006), reason: reason.to_string(), clean: false };

    loop {
        let deadline = async {
            match close_deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            out = rx.recv(), if !sent_close => {
                let msg = match out {
                    // 台帳から外された（呼び出し元が先に消えた）。黙って終わる
                    None => return unclean("接続が破棄されました"),
                    Some(Out::Text(t)) => Message::text(t),
                    Some(Out::Binary(b)) => Message::binary(b),
                    Some(Out::Close(frame)) => {
                        sent_close = true;
                        close_deadline = Some(tokio::time::Instant::now() + CLOSE_TIMEOUT);
                        Message::Close(frame.map(|(code, reason)| CloseFrame { code: CloseCode::from(code), reason: reason.into() }))
                    }
                };
                match tokio::time::timeout(WRITE_TIMEOUT, write.send(msg)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        let message = format!("送信に失敗しました: {}", e);
                        sink(WsEvent::Error { id: id.to_string(), message: message.clone() });
                        return unclean(&message);
                    }
                    Err(_) => {
                        let message = format!("送信が進みません（{} 秒）", WRITE_TIMEOUT.as_secs());
                        sink(WsEvent::Error { id: id.to_string(), message: message.clone() });
                        return unclean(&message);
                    }
                }
            }
            msg = read.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    sink(WsEvent::Message { id: id.to_string(), binary: false, data: t.as_str().to_string() });
                }
                Some(Ok(Message::Binary(b))) => {
                    sink(WsEvent::Message {
                        id: id.to_string(),
                        binary: true,
                        data: base64::engine::general_purpose::STANDARD.encode(&b),
                    });
                }
                // ping への pong は tungstenite が返す
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                Some(Ok(Message::Close(frame))) => {
                    // 相手からの close への返事（tungstenite が積んでいる）を流してから閉じる
                    let _ = tokio::time::timeout(Duration::from_secs(2), write.close()).await;
                    return match frame {
                        Some(f) => Ended { code: Some(u16::from(f.code)), reason: f.reason.as_str().to_string(), clean: true },
                        None => Ended { code: None, reason: String::new(), clean: true },
                    };
                }
                Some(Err(e)) => {
                    use tokio_tungstenite::tungstenite::Error;
                    // 相手が close を送らずに切った（正常終了の扱いにはしない）
                    let (code, message) = match &e {
                        Error::ConnectionClosed | Error::AlreadyClosed => (1006, "接続が閉じられました".to_string()),
                        Error::Capacity(_) => (1009, format!("メッセージが大きすぎます（{} バイトまで）", MAX_MESSAGE_BYTES)),
                        other => (1006, format!("受信に失敗しました: {}", other)),
                    };
                    if !matches!(e, Error::ConnectionClosed | Error::AlreadyClosed) {
                        sink(WsEvent::Error { id: id.to_string(), message: message.clone() });
                    }
                    return Ended { code: Some(code), reason: message, clean: false };
                }
                None => return unclean("接続が閉じられました"),
            },
            _ = deadline => return unclean("close の応答がありませんでした"),
        }
    }
}

// ─── 台帳 ────────────────────────────────────────────────────────────

struct Conn {
    /// 承認との再照合に使う（接続時の URL。fragment は落ちている）
    url: String,
    tx: mpsc::Sender<Out>,
    abort: AbortHandle,
    worktree_id: String,
    artifact_id: String,
    /// 開いた側のウィンドウ（常駐かサブウィンドウ）。リロードで切るのは、そのウィンドウが開いたものだけ
    window_label: String,
}

#[derive(Default)]
struct Inner {
    /// (インスタンスのラベル, 接続 ID) → 接続
    conns: HashMap<(String, String), Conn>,
    /// 停止したインスタンスのラベル（ラベルは再利用されない）
    dead: HashSet<String>,
}

#[derive(Default)]
pub struct AppWsRegistry {
    inner: Mutex<Inner>,
}

struct Slot {
    rx: mpsc::Receiver<Out>,
    abort_reg: AbortRegistration,
}

impl AppWsRegistry {
    /// 接続の枠を確保する（ハンドシェイクの前。中断はこの時点から効く）。
    fn begin(&self, label: &str, id: &str, url: &str, worktree_id: &str, artifact_id: &str, window_label: &str) -> Result<Slot, String> {
        let Ok(mut inner) = self.inner.lock() else {
            return Err("WebSocket の台帳のロックが壊れています".to_string());
        };
        if inner.dead.contains(label) {
            return Err("アプリが停止しました".to_string());
        }
        let key = (label.to_string(), id.to_string());
        if inner.conns.contains_key(&key) {
            return Err("この接続 ID はすでに使われています".to_string());
        }
        if inner.conns.keys().filter(|(l, _)| l == label).count() >= MAX_PER_INSTANCE {
            return Err(format!("このアプリが同時に開ける WebSocket は {} 本までです", MAX_PER_INSTANCE));
        }
        if inner.conns.len() >= MAX_TOTAL {
            return Err(format!("oretachi 全体で同時に開ける WebSocket は {} 本までです", MAX_TOTAL));
        }
        let (tx, rx) = mpsc::channel(SEND_QUEUE_DEPTH);
        let (abort, abort_reg) = AbortHandle::new_pair();
        inner.conns.insert(
            key,
            Conn {
                url: url.to_string(),
                tx,
                abort,
                worktree_id: worktree_id.to_string(),
                artifact_id: artifact_id.to_string(),
                window_label: window_label.to_string(),
            },
        );
        Ok(Slot { rx, abort_reg })
    }

    fn end(&self, label: &str, id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.conns.remove(&(label.to_string(), id.to_string()));
        }
    }

    fn get(&self, label: &str, id: &str) -> Option<(String, mpsc::Sender<Out>)> {
        let inner = self.inner.lock().ok()?;
        inner.conns.get(&(label.to_string(), id.to_string())).map(|c| (c.url.clone(), c.tx.clone()))
    }

    /// 1 本を切る（台帳から外して中断）。あれば true。
    fn kill(&self, label: &str, id: &str) -> bool {
        let Ok(mut inner) = self.inner.lock() else { return false };
        match inner.conns.remove(&(label.to_string(), id.to_string())) {
            Some(c) => {
                c.abort.abort();
                true
            }
            None => false,
        }
    }

    /// インスタンスの接続をすべて切る。`stopped` なら以後は開けないようにする。
    fn abort_all(&self, label: &str, stopped: bool) -> Vec<(String, String, String)> {
        let Ok(mut inner) = self.inner.lock() else { return Vec::new() };
        if stopped {
            inner.dead.insert(label.to_string());
        }
        let keys: Vec<(String, String)> = inner.conns.keys().filter(|(l, _)| l == label).cloned().collect();
        let mut out = Vec::new();
        for k in keys {
            if let Some(c) = inner.conns.remove(&k) {
                c.abort.abort();
                out.push((k.1, c.worktree_id, c.artifact_id));
            }
        }
        out
    }

    /// 1 つのウィンドウが開いた接続だけを切る（同じインスタンスの別ウィンドウのものは残す）。
    fn abort_window(&self, label: &str, window_label: &str) -> Vec<(String, String, String)> {
        let Ok(mut inner) = self.inner.lock() else { return Vec::new() };
        let keys: Vec<(String, String)> = inner
            .conns
            .iter()
            .filter(|((l, _), c)| l == label && c.window_label == window_label)
            .map(|(k, _)| k.clone())
            .collect();
        let mut out = Vec::new();
        for k in keys {
            if let Some(c) = inner.conns.remove(&k) {
                c.abort.abort();
                out.push((k.1, c.worktree_id, c.artifact_id));
            }
        }
        out
    }

    #[cfg(test)]
    fn count(&self, label: &str) -> usize {
        self.inner.lock().unwrap().conns.keys().filter(|(l, _)| l == label).count()
    }
}

/// インスタンスの停止（`app_net::on_instance_released` 経由）。接続を切り、以後は開けないようにする。
pub fn on_instance_released(app: &AppHandle, label: &str) {
    let closed = app.state::<AppWsRegistry>().abort_all(label, true);
    log_closed(app, &closed, "アプリの停止");
}

/// ホットリロード・権限セットの入れ替え（`app_net::on_reload` 経由）。切るだけで、新しい iframe が張り直す。
pub fn on_reload(app: &AppHandle, label: &str) {
    let closed = app.state::<AppWsRegistry>().abort_all(label, false);
    log_closed(app, &closed, "アプリの再読み込み");
}

fn log_closed(app: &AppHandle, closed: &[(String, String, String)], why: &str) {
    for (id, worktree_id, artifact_id) in closed {
        log::debug!("[app-ws] 切断 {}/{} {} — {}", worktree_id, artifact_id, id, why);
    }
    // ログ（アプリのデバッグログ）は 1 インスタンス 1 行にまとめる
    let mut seen = HashSet::new();
    for (_, w, a) in closed {
        if seen.insert((w.clone(), a.clone())) {
            let n = closed.iter().filter(|(_, w2, a2)| w2 == w && a2 == a).count();
            crate::app_instance::add_log(app, w, a, "info", format!("{} により WebSocket {} 本を切断しました", why, n));
        }
    }
}

// ─── Tauri コマンド（appinst-* ウィンドウからだけ） ────────────────────

fn app_sink(app: AppHandle, label: String) -> Sink {
    Arc::new(move |ev: WsEvent| {
        crate::app_subwindow::emit_to_instance(&app, &label, EVENT_NAME, ev);
    })
}

fn log_failure(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, op: &str, url: &str, error: &str) {
    let note = if ctx.reapproval_pending { "（再承認待ちのため、新しい権限はまだ使えません）" } else { "" };
    let url = if url.is_empty() { String::new() } else { format!(" {}", loggable_url(url)) };
    log::debug!("[app-ws] 拒否 {} {}/{}{} — {}", op, ctx.worktree_id, ctx.artifact_id, url, error);
    crate::app_instance::add_log(
        app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        "warn",
        format!("net.ws を拒否/失敗: {}{} — {}{}", op, url, error, note),
    );
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.chars().count() > MAX_ID_CHARS || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("接続 ID が不正です（英数字・-・_ で {} 文字まで）", MAX_ID_CHARS));
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenOpts {
    pub protocols: Vec<String>,
}

#[tauri::command]
pub async fn app_ws_open(
    window: tauri::Window,
    app: AppHandle,
    id: String,
    url: String,
    opts: Option<OpenOpts>,
) -> Result<(), String> {
    let opts = opts.unwrap_or_default();
    let (ctx, allow) = caller_net(&window, &app)?;
    let fail = |e: String| {
        log_failure(&app, &ctx, "open", &url, &e);
        e
    };
    validate_id(&id).map_err(&fail)?;
    if url.chars().count() > MAX_URL_CHARS {
        return Err(fail(format!("URL が長すぎます（{} 文字まで）", MAX_URL_CHARS)));
    }
    let parsed = reqwest::Url::parse(&url).map_err(|e| fail(format!("URL を解釈できません: {}", e)))?;
    if opts.protocols.len() > MAX_PROTOCOLS || opts.protocols.iter().any(|p| !protocol_is_token(p)) {
        return Err(fail(format!("protocols は {} 個までの英数字トークンで指定してください", MAX_PROTOCOLS)));
    }
    let policy = Arc::new(Policy::new(&allow, own_ports(&app)));
    // 枠を取る前に URL の検査を済ませる（承認外の URL が台帳の枠を使わないように）
    policy.check_ws_url(&parsed).map_err(&fail)?;

    let label = ctx.instance_label.clone();
    let reg = app.state::<AppWsRegistry>();
    let Slot { rx, abort_reg } = reg
        .begin(&label, &id, parsed.as_str(), &ctx.worktree_id, &ctx.artifact_id, window.label())
        .map_err(&fail)?;

    let (res_tx, res_rx) = oneshot::channel::<Result<(), String>>();
    let task_app = app.clone();
    let (task_label, task_id) = (label.clone(), id.clone());
    let (wt, art) = (ctx.worktree_id.clone(), ctx.artifact_id.clone());
    let protocols = opts.protocols.clone();
    let target = parsed.clone();
    tokio::spawn(async move {
        let sink = app_sink(task_app.clone(), task_label.clone());
        let fut = async {
            let connecting = tokio::time::timeout(OPEN_TIMEOUT, connect(&policy, &SystemLookup, &target, &protocols));
            let ws = match connecting.await.unwrap_or_else(|_| Err(format!("接続がタイムアウトしました（{} 秒）", OPEN_TIMEOUT.as_secs()))) {
                Ok(ws) => ws,
                Err(e) => {
                    let _ = res_tx.send(Err(e));
                    return;
                }
            };
            // 呼び出し側がもういない（コマンドの future が落とされた）なら、そのまま閉じる
            if res_tx.send(Ok(())).is_err() {
                return;
            }
            let ended = pump(&task_id, ws, rx, &sink).await;
            if !ended.clean {
                log::debug!("[app-ws] 切断 {}/{} {} — {}", wt, art, task_id, ended.reason);
                crate::app_instance::add_log(
                    &task_app,
                    &wt,
                    &art,
                    "warn",
                    format!("net.ws の接続が切れました: {} — {}", loggable_url(target.as_str()), ended.reason),
                );
            }
            sink(WsEvent::Close { id: task_id.clone(), code: ended.code, reason: ended.reason, clean: ended.clean });
        };
        // 再読み込み・停止・取り消しで中断されたときは、イベントを出さない（受け手の iframe はもういない）
        let _ = Abortable::new(fut, abort_reg).await;
        task_app.state::<AppWsRegistry>().end(&task_label, &task_id);
    });

    match res_rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            app.state::<AppWsRegistry>().end(&label, &id);
            Err(fail(e))
        }
        Err(_) => {
            app.state::<AppWsRegistry>().end(&label, &id);
            Err(fail("アプリの再読み込みまたは停止で接続を中断しました".to_string()))
        }
    }
}

#[tauri::command]
pub async fn app_ws_send(
    window: tauri::Window,
    app: AppHandle,
    id: String,
    data: String,
    binary: Option<bool>,
) -> Result<(), String> {
    let (ctx, allow) = match caller_net(&window, &app) {
        Ok(v) => v,
        Err(e) => {
            // 承認が無くなっている。残っている接続があれば、使えないまま生かさず切る
            if let Ok(ctx) = crate::app_instance::context_for_label(&app, window.label()) {
                let closed = app.state::<AppWsRegistry>().abort_all(&ctx.instance_label, false);
                log_closed(&app, &closed, "承認の取り消し");
                // iframe は生きたままなので、切れたことを知らせる（リロード・停止の切断とは違い、受け手がいる）
                let sink = app_sink(app.clone(), ctx.instance_label.clone());
                for (id, _, _) in closed {
                    sink(WsEvent::Close { id, code: Some(1008), reason: "承認が取り消された".to_string(), clean: false });
                }
            }
            return Err(e);
        }
    };
    let fail = |e: String| {
        log_failure(&app, &ctx, "send", "", &e);
        e
    };
    validate_id(&id).map_err(&fail)?;
    let reg = app.state::<AppWsRegistry>();
    let Some((url, tx)) = reg.get(&ctx.instance_label, &id) else {
        return Err(fail("この接続は存在しないか、すでに閉じられています".to_string()));
    };

    // 呼び出しのたびに、保存済みの承認と接続先を照合し直す（承認が縮小されていれば切る）
    let still_allowed = reqwest::Url::parse(&url)
        .map_err(|e| e.to_string())
        .and_then(|u| Policy::new(&allow, own_ports(&app)).check_ws_url(&u));
    if let Err(e) = still_allowed {
        if reg.kill(&ctx.instance_label, &id) {
            app_sink(app.clone(), ctx.instance_label.clone())(WsEvent::Close {
                id: id.clone(),
                code: Some(1008),
                reason: "承認が取り消された、または縮小されました".to_string(),
                clean: false,
            });
        }
        return Err(fail(format!("この接続先はもう承認されていません: {}", e)));
    }

    let out = if binary.unwrap_or(false) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.as_bytes())
            .map_err(|e| fail(format!("data が base64 として不正です: {}", e)))?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(fail(format!("メッセージが大きすぎます: {} バイト（上限 {}）", bytes.len(), MAX_MESSAGE_BYTES)));
        }
        Out::Binary(bytes)
    } else {
        if data.len() > MAX_MESSAGE_BYTES {
            return Err(fail(format!("メッセージが大きすぎます: {} バイト（上限 {}）", data.len(), MAX_MESSAGE_BYTES)));
        }
        Out::Text(data)
    };
    match tx.try_send(out) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full(_)) => Err(fail(format!(
            "送信キューが一杯です（{} 件）。相手が受け取るのを待ってから送ってください",
            SEND_QUEUE_DEPTH
        ))),
        Err(mpsc::error::TrySendError::Closed(_)) => Err(fail("接続はすでに閉じられています".to_string())),
    }
}

#[tauri::command]
pub async fn app_ws_close(
    window: tauri::Window,
    app: AppHandle,
    id: String,
    code: Option<u16>,
    reason: Option<String>,
) -> Result<(), String> {
    let (ctx, _allow) = caller_net(&window, &app)?;
    let fail = |e: String| {
        log_failure(&app, &ctx, "close", "", &e);
        e
    };
    validate_id(&id).map_err(&fail)?;
    // ブラウザの WebSocket.close と同じ: 1000 か 3000〜4999 だけ
    let frame = match code {
        None => None,
        Some(c) if c == 1000 || (3000..=4999).contains(&c) => {
            let reason = reason.unwrap_or_default();
            if reason.len() > MAX_CLOSE_REASON_BYTES {
                return Err(fail(format!("reason は {} バイトまでです", MAX_CLOSE_REASON_BYTES)));
            }
            Some((c, reason))
        }
        Some(_) => return Err(fail("code は 1000 か 3000〜4999 だけ指定できます".to_string())),
    };
    let reg = app.state::<AppWsRegistry>();
    // すでに無い接続の close は何もしない（冪等）
    let Some((_url, tx)) = reg.get(&ctx.instance_label, &id) else { return Ok(()) };
    if tx.try_send(Out::Close(frame)).is_err() {
        // キューが満杯（相手が読まない）なら、待たずに切る
        if reg.kill(&ctx.instance_label, &id) {
            app_sink(app.clone(), ctx.instance_label.clone())(WsEvent::Close {
                id,
                code: Some(1006),
                reason: "送信キューが満杯のため切断しました".to_string(),
                clean: false,
            });
        }
    }
    Ok(())
}

/// このウィンドウが開いた接続をすべて閉じる（同じインスタンスの別ウィンドウのものは残す）。ホスト
/// （`AppInstanceApp.vue`）がリロードで iframe を差し替える直前に呼ぶ（Rust 側の `on_reload` は通知の時点で
/// インスタンス全体を切るが、差し替えまでの間に旧 iframe が開いた接続を残さないため）。
/// 承認の有無は問わない（取り消し後でも閉じられる）。
#[tauri::command]
pub async fn app_ws_close_all(window: tauri::Window, app: AppHandle) -> Result<u32, String> {
    crate::app_instance::require_instance_window(&window)?;
    let ctx = crate::app_instance::context_for_label(&app, window.label())?;
    let closed = app.state::<AppWsRegistry>().abort_window(&ctx.instance_label, window.label());
    log_closed(&app, &closed, "アプリの再読み込み");
    Ok(closed.len() as u32)
}

// ─── テスト ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct NoLookup;
    impl Lookup for NoLookup {
        fn lookup(&self, _host: String) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send>> {
            Box::pin(async { Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such host")) })
        }
    }

    struct FixedLookup(Vec<IpAddr>);
    impl Lookup for FixedLookup {
        fn lookup(&self, _host: String) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send>> {
            let v = self.0.clone();
            Box::pin(async move { Ok(v) })
        }
    }

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    /// 本番相当（ループバック不可）
    fn policy(prefixes: &[&str]) -> Policy {
        let p: Vec<String> = prefixes.iter().map(|s| crate::app_net::normalize_prefix(s).unwrap()).collect();
        Policy::new(&p, vec![])
    }

    /// ループバックを許すのはテストだけ
    fn open_policy(prefixes: &[String]) -> Policy {
        Policy::new(prefixes, vec![]).with_private_allowed()
    }

    fn collector() -> (Sink, Arc<Mutex<Vec<WsEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let e2 = events.clone();
        (Arc::new(move |ev| e2.lock().unwrap().push(ev)), events)
    }

    /// 接続を 1 本受けて `handler` に渡すサーバー。ポートを返す
    async fn serve_ws<F, Fut>(handler: F) -> u16
    where
        F: Fn(WebSocketStream<TcpStream>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                tokio::spawn(async move {
                    if let Ok(ws) = tokio_tungstenite::accept_async(sock).await {
                        handler(ws).await;
                    }
                });
            }
        });
        port
    }

    /// 受けた Text / Binary をそのまま返すサーバー
    async fn echo_server() -> u16 {
        serve_ws(|mut ws| async move {
            while let Some(Ok(m)) = ws.next().await {
                match m {
                    Message::Text(_) | Message::Binary(_) => {
                        if ws.send(m).await.is_err() {
                            return;
                        }
                    }
                    // close への返事は読み続ける中で tungstenite が流す（すぐ drop すると reset になる）
                    _ => {}
                }
            }
        })
        .await
    }

    async fn wait_events(events: &Arc<Mutex<Vec<WsEvent>>>, n: usize) {
        for _ in 0..200 {
            if events.lock().unwrap().len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("events not received: {:?}", events.lock().unwrap());
    }

    // ── プレフィックス・スキーム ──

    #[test]
    fn ws_prefixes_are_scheme_exact() {
        let p = policy(&["wss://api.example.com/v1/", "https://web.example.com/"]);
        assert!(p.check_ws_url(&url("wss://api.example.com/v1/stream")).is_ok());
        assert!(p.check_ws_url(&url("wss://api.example.com:443/v1/stream")).is_ok());
        // スキーム違い・別パス・別ホスト・別ポート
        assert!(p.check_ws_url(&url("ws://api.example.com/v1/stream")).is_err());
        assert!(p.check_ws_url(&url("wss://api.example.com/v2/stream")).is_err());
        assert!(p.check_ws_url(&url("wss://other.example.com/v1/stream")).is_err());
        assert!(p.check_ws_url(&url("wss://api.example.com:8443/v1/stream")).is_err());
        // https の承認は wss に効かない。wss の承認は net.fetch（http / https）に効かない
        assert!(p.check_ws_url(&url("wss://web.example.com/")).is_err());
        assert!(p.check_url(&url("https://api.example.com/v1/x")).is_err());
        assert!(p.check_url(&url("wss://api.example.com/v1/x")).is_err());
        // http / https の URL は ws の入口では通らない
        assert!(p.check_ws_url(&url("https://web.example.com/")).is_err());
    }

    #[test]
    fn ws_check_rejects_loopback_private_and_own_port() {
        let raw = |u: &str| Policy::new(&[u.to_string()], vec![]);
        for u in ["ws://127.0.0.1:9/", "ws://[::1]:9/", "ws://10.0.0.1/", "ws://192.168.1.2/", "ws://169.254.169.254/", "ws://localhost/", "ws://a.localhost/"] {
            assert!(raw(u).check_ws_url(&url(u)).is_err(), "{}", u);
        }
        let p = Policy::new(&["wss://public.example.com:7777/".to_string()], vec![7777]);
        assert!(p.check_ws_url(&url("wss://public.example.com:7777/")).is_err());
        assert!(crate::app_net::normalize_prefix("wss://example.com/a?x=1").is_err());
        assert!(crate::app_net::normalize_prefix("ws://127.0.0.1/").is_err());
        assert!(crate::app_net::normalize_prefix("wss://user:pw@example.com/").is_err());
        assert_eq!(crate::app_net::normalize_prefix("WSS://Example.com").unwrap(), "wss://example.com/");
        assert_eq!(crate::app_net::normalize_prefix("ws://example.com:80/a").unwrap(), "ws://example.com/a");
    }

    #[test]
    fn protocol_tokens() {
        assert!(protocol_is_token("graphql-ws"));
        assert!(protocol_is_token("v1.json"));
        assert!(!protocol_is_token(""));
        assert!(!protocol_is_token("a b"));
        assert!(!protocol_is_token("a,b"));
        assert!(!protocol_is_token("a\r\nb"));
    }

    #[test]
    fn ids_are_validated() {
        assert!(validate_id("a1-B_2").is_ok());
        assert!(validate_id("").is_err());
        assert!(validate_id("a b").is_err());
        assert!(validate_id(&"x".repeat(MAX_ID_CHARS + 1)).is_err());
    }

    // ── 接続前・接続後の IP 検査 ──

    #[tokio::test]
    async fn connect_rejects_names_that_resolve_to_private_addresses() {
        let p = policy(&["wss://rebind.example.com/"]);
        for ips in [vec!["10.0.0.5"], vec!["127.0.0.1"], vec!["169.254.169.254"], vec!["93.184.216.34", "127.0.0.1"], vec!["::1"], vec!["::ffff:127.0.0.1"]] {
            let lookup = FixedLookup(ips.iter().map(|s| s.parse().unwrap()).collect());
            let e = connect(&p, &lookup, &url("wss://rebind.example.com/"), &[]).await.err().unwrap();
            assert!(e.contains("接続できません"), "{:?}: {}", ips, e);
        }
        // 解決できない・空
        assert!(connect(&p, &NoLookup, &url("wss://rebind.example.com/"), &[]).await.is_err());
        assert!(connect(&p, &FixedLookup(vec![]), &url("wss://rebind.example.com/"), &[]).await.is_err());
    }

    #[tokio::test]
    async fn connect_rejects_urls_outside_the_grant_before_any_network() {
        let p = policy(&["wss://api.example.com/v1/"]);
        // lookup は呼ばれない（呼ばれたら NoLookup の別のエラー文になる）
        let e = connect(&p, &NoLookup, &url("wss://api.example.com/v2/"), &[]).await.err().unwrap();
        assert!(e.contains("承認された URL プレフィックス"), "{}", e);
        let e = connect(&p, &NoLookup, &url("ws://127.0.0.1:1/"), &[]).await.err().unwrap();
        assert!(e.contains("承認された URL プレフィックス") || e.contains("ループバック"), "{}", e);
    }

    // ── 送受信（ループバックの実サーバー） ──

    #[tokio::test]
    async fn echo_text_and_binary_round_trip() {
        let port = echo_server().await;
        let prefix = format!("ws://127.0.0.1:{}/", port);
        let p = open_policy(&[prefix.clone()]);
        let ws = connect(&p, &NoLookup, &url(&format!("ws://127.0.0.1:{}/chat", port)), &[]).await.unwrap();

        let (tx, rx) = mpsc::channel(SEND_QUEUE_DEPTH);
        let (sink, events) = collector();
        let pump_task = tokio::spawn(async move { pump("c1", ws, rx, &sink).await });
        tx.send(Out::Text("hello".into())).await.unwrap();
        tx.send(Out::Binary(vec![1, 2, 3, 255])).await.unwrap();
        wait_events(&events, 2).await;
        {
            let ev = events.lock().unwrap();
            assert_eq!(ev[0], WsEvent::Message { id: "c1".into(), binary: false, data: "hello".into() });
            assert_eq!(
                ev[1],
                WsEvent::Message { id: "c1".into(), binary: true, data: base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3, 255]) }
            );
        }
        // こちらからの close は、相手の close を待って clean で終わる
        tx.send(Out::Close(Some((1000, "bye".into())))).await.unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(5), pump_task).await.unwrap().unwrap();
        assert!(ended.clean, "{}", ended.reason);
    }

    #[tokio::test]
    async fn server_close_is_reported_with_code_and_reason() {
        let port = serve_ws(|mut ws| async move {
            let _ = ws
                .close(Some(CloseFrame { code: CloseCode::from(4001), reason: "nope".into() }))
                .await;
            while ws.next().await.is_some() {}
        })
        .await;
        let p = open_policy(&[format!("ws://127.0.0.1:{}/", port)]);
        let ws = connect(&p, &NoLookup, &url(&format!("ws://127.0.0.1:{}/", port)), &[]).await.unwrap();
        let (_tx, rx) = mpsc::channel(SEND_QUEUE_DEPTH);
        let (sink, _events) = collector();
        let ended = tokio::time::timeout(Duration::from_secs(5), pump("c", ws, rx, &sink)).await.unwrap();
        assert_eq!((ended.code, ended.reason.as_str(), ended.clean), (Some(4001), "nope", true));
    }

    #[tokio::test]
    async fn abrupt_disconnect_is_unclean() {
        let port = serve_ws(|ws| async move { drop(ws) }).await;
        let p = open_policy(&[format!("ws://127.0.0.1:{}/", port)]);
        let ws = connect(&p, &NoLookup, &url(&format!("ws://127.0.0.1:{}/", port)), &[]).await.unwrap();
        let (_tx, rx) = mpsc::channel(SEND_QUEUE_DEPTH);
        let (sink, _events) = collector();
        let ended = tokio::time::timeout(Duration::from_secs(5), pump("c", ws, rx, &sink)).await.unwrap();
        assert!(!ended.clean);
        assert_eq!(ended.code, Some(1006));
    }

    #[tokio::test]
    async fn oversized_incoming_message_is_refused() {
        let port = serve_ws(|mut ws| async move {
            let _ = ws.send(Message::text("x".repeat(MAX_MESSAGE_BYTES + 1))).await;
            while ws.next().await.is_some() {}
        })
        .await;
        let p = open_policy(&[format!("ws://127.0.0.1:{}/", port)]);
        let ws = connect(&p, &NoLookup, &url(&format!("ws://127.0.0.1:{}/", port)), &[]).await.unwrap();
        let (_tx, rx) = mpsc::channel(SEND_QUEUE_DEPTH);
        let (sink, events) = collector();
        let ended = tokio::time::timeout(Duration::from_secs(10), pump("c", ws, rx, &sink)).await.unwrap();
        assert!(!ended.clean);
        assert_eq!(ended.code, Some(1009));
        assert!(matches!(events.lock().unwrap().first(), Some(WsEvent::Error { .. })));
    }

    // ── ハンドシェイク ──

    /// 生の HTTP 応答を返すだけのサーバー（WebSocket として受けない）
    async fn raw_http(response: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn redirect_and_non_upgrade_responses_fail_the_handshake() {
        let redirect = raw_http("HTTP/1.1 302 Found\r\nLocation: ws://127.0.0.1:1/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        let ok_page = raw_http("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi").await;
        for (port, status) in [(redirect, "302"), (ok_page, "200")] {
            let p = open_policy(&[format!("ws://127.0.0.1:{}/", port)]);
            let e = connect(&p, &NoLookup, &url(&format!("ws://127.0.0.1:{}/", port)), &[]).await.err().unwrap();
            assert!(e.contains(status) || e.contains("ハンドシェイク"), "{}: {}", status, e);
        }
    }

    #[tokio::test]
    async fn unreachable_port_fails_cleanly() {
        // 空いているポートを取って閉じる
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let p = open_policy(&[format!("ws://127.0.0.1:{}/", port)]);
        assert!(connect(&p, &NoLookup, &url(&format!("ws://127.0.0.1:{}/", port)), &[]).await.is_err());
    }

    #[tokio::test]
    async fn subprotocol_is_offered() {
        let port = serve_ws(|ws| async move {
            // accept_async は Sec-WebSocket-Protocol を見ない。要求側が出したことは別経路で確かめるので、ここは接続だけ
            drop(ws);
        })
        .await;
        let p = open_policy(&[format!("ws://127.0.0.1:{}/", port)]);
        // サーバーが選ばなかった場合も、接続自体は成立する
        let _ = connect(&p, &NoLookup, &url(&format!("ws://127.0.0.1:{}/", port)), &["v1.json".to_string()]).await;
    }

    /// 実機確認用（公開のエコーサーバーへ wss で繋ぐ。ネットワークが要るので通常は走らせない）:
    /// `cargo test --lib app_ws::tests::live_wss_echo -- --ignored --nocapture`
    /// 本番相当のポリシー（ループバック不可）・OS の名前解決・OS の証明書検証を通す。
    #[tokio::test]
    #[ignore]
    async fn live_wss_echo() {
        let p = policy(&["wss://echo.websocket.org/"]);
        let ws = connect(&p, &SystemLookup, &url("wss://echo.websocket.org/"), &[]).await.unwrap();
        let (tx, rx) = mpsc::channel(SEND_QUEUE_DEPTH);
        let (sink, events) = collector();
        let task = tokio::spawn(async move { pump("live", ws, rx, &sink).await });
        tx.send(Out::Text("oretachi-ws-check".into())).await.unwrap();
        tx.send(Out::Binary(vec![0, 1, 2, 254, 255])).await.unwrap();
        // サーバーは最初に挨拶（Request served by …）を返すので、エコーが含まれるまで待つ
        for _ in 0..200 {
            let got = events.lock().unwrap().iter().any(|e| matches!(e, WsEvent::Message { data, binary: false, .. } if data == "oretachi-ws-check"))
                && events.lock().unwrap().iter().any(|e| matches!(e, WsEvent::Message { binary: true, .. }));
            if got {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        println!("events: {:?}", events.lock().unwrap());
        assert!(events.lock().unwrap().iter().any(|e| matches!(e, WsEvent::Message { data, binary: false, .. } if data == "oretachi-ws-check")));
        tx.send(Out::Close(Some((1000, "bye".into())))).await.unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
        println!("ended: code={:?} reason={:?} clean={}", ended.code, ended.reason, ended.clean);
        assert!(ended.clean);
        // 証明書の検証が効いている（別のホスト名宛の証明書では繋がらない）ことも確かめる:
        // echo.websocket.org の IP へ、関係ない名前で TLS しても失敗する
        let bad = policy(&["wss://wrong.host.badssl.example/"]);
        let lookup = FixedLookup(vec!["8.8.8.8".parse().unwrap()]);
        assert!(connect(&bad, &lookup, &url("wss://wrong.host.badssl.example/"), &[]).await.is_err());
    }

    // ── 台帳 ──

    #[test]
    fn registry_enforces_limits_and_isolation() {
        let reg = AppWsRegistry::default();
        let mut slots = Vec::new();
        for i in 0..MAX_PER_INSTANCE {
            slots.push(reg.begin("appinst-a", &format!("c{}", i), "ws://x/", "w", "a", "win").unwrap());
        }
        // インスタンスごとの上限
        assert!(reg.begin("appinst-a", "extra", "ws://x/", "w", "a", "win").is_err());
        // 同じ ID は重ねて使えない（別インスタンスなら別物）
        assert!(reg.begin("appinst-a", "c0", "ws://x/", "w", "a", "win").is_err());
        let _b = reg.begin("appinst-b", "c0", "ws://x/", "w", "b", "win").unwrap();
        // 他のインスタンスの ID は引けない
        assert!(reg.get("appinst-b", "c1").is_none());
        assert!(reg.get("appinst-a", "c1").is_some());
        assert!(!reg.kill("appinst-b", "c1"));
        // 終わったぶんは空く
        reg.end("appinst-a", "c0");
        assert!(reg.begin("appinst-a", "again", "ws://x/", "w", "a", "win").is_ok());
    }

    #[test]
    fn registry_total_limit_spans_instances() {
        let reg = AppWsRegistry::default();
        let mut keep = Vec::new();
        'outer: for inst in 0.. {
            for c in 0..MAX_PER_INSTANCE {
                if keep.len() == MAX_TOTAL {
                    break 'outer;
                }
                keep.push(reg.begin(&format!("appinst-{}", inst), &format!("c{}", c), "ws://x/", "w", "a", "win").unwrap());
            }
        }
        let e = reg.begin("appinst-new", "c", "ws://x/", "w", "a", "win").err().unwrap();
        assert!(e.contains("全体"), "{}", e);
    }

    #[tokio::test]
    async fn abort_all_cancels_and_stop_blocks_new_connections() {
        let reg = AppWsRegistry::default();
        let slot = reg.begin("appinst-a", "c1", "ws://x/", "w", "a", "win").unwrap();
        let fut = Abortable::new(std::future::pending::<()>(), slot.abort_reg);
        let closed = reg.abort_all("appinst-a", false);
        assert_eq!(closed.len(), 1);
        assert!(fut.await.is_err());
        assert_eq!(reg.count("appinst-a"), 0);
        // 再読み込み（停止ではない）は開き直せる
        assert!(reg.begin("appinst-a", "c2", "ws://x/", "w", "a", "win").is_ok());
        // 停止すると、以後は開けない
        reg.abort_all("appinst-a", true);
        assert!(reg.begin("appinst-a", "c3", "ws://x/", "w", "a", "win").is_err());
    }

    #[test]
    fn abort_window_only_closes_that_windows_connections() {
        let reg = AppWsRegistry::default();
        let _a = reg.begin("appinst-a", "c1", "ws://x/", "w", "a", "appinst-a").unwrap();
        let _b = reg.begin("appinst-a", "c2", "ws://x/", "w", "a", "appinst-a-w1").unwrap();
        let _c = reg.begin("appinst-b", "c1", "ws://x/", "w", "b", "appinst-a").unwrap();
        let closed = reg.abort_window("appinst-a", "appinst-a");
        assert_eq!(closed.len(), 1);
        assert!(reg.get("appinst-a", "c1").is_none());
        assert!(reg.get("appinst-a", "c2").is_some());
        assert!(reg.get("appinst-b", "c1").is_some());
    }

    #[tokio::test]
    async fn abort_before_the_task_starts_still_cancels() {
        // ハンドシェイク中（Abortable を作る前）の中断でも、後から作った future が即座に落ちる
        let reg = AppWsRegistry::default();
        let slot = reg.begin("appinst-a", "c1", "ws://x/", "w", "a", "win").unwrap();
        reg.abort_all("appinst-a", false);
        assert!(Abortable::new(std::future::pending::<()>(), slot.abort_reg).await.is_err());
    }

    #[tokio::test]
    async fn send_queue_applies_backpressure() {
        let reg = AppWsRegistry::default();
        let slot = reg.begin("appinst-a", "c1", "ws://x/", "w", "a", "win").unwrap();
        let (_url, tx) = reg.get("appinst-a", "c1").unwrap();
        // 読み手（pump）がいないキューは、深さぶんで満杯になる
        for _ in 0..SEND_QUEUE_DEPTH {
            tx.try_send(Out::Text("x".into())).unwrap();
        }
        assert!(matches!(tx.try_send(Out::Text("x".into())), Err(mpsc::error::TrySendError::Full(_))));
        drop(slot);
    }
}
