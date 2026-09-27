// ─── アーティファクト Web 閲覧: 閲覧用ルータの土台 ─────────────────────────────
//
// 既存の MCP サーバー(mcp_server.rs)は全ルートに Bearer 認証を要求するが、
// ブラウザの直接遷移ではヘッダーを付けられない。このモジュールは
// Bearer layer の**外**で merge される閲覧専用サブルータを提供する。
//
// 認証: `?token=<mcp_api_key>` で開くと Cookie (HMAC 由来の派生値、key そのものは
// 保存しない) を発行し、token を除いた URL へリダイレクトする。以降は Cookie のみで
// 認証する。`regenerate_mcp_api_key` で key が変われば旧 Cookie は自動的に無効になる。
//
// #340 で書き込み系(memory 保存・callTool ブリッジ)を追加した。POST は `csrf_guard`
// middleware(`viewer_auth` の内側)で Origin/Host/Sec-Fetch-Site を検証しており、
// SameSite=Strict の Cookie だけに頼っていない(csrf_guard のドキュメントコメント参照)。

use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{
        sse::{Event as SseEvent, KeepAlive},
        Html, IntoResponse, Json, Redirect, Response, Sse,
    },
    routing::{get, post},
    Router,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{broadcast, watch};

use crate::settings::{Repository, SettingsManager, WorktreeEntry};
use crate::{
    artifacts_dir_in, list_artifacts_in_dir, list_repo_artifacts_in_dir, read_artifact_store,
    repo_artifacts_dir_in, repo_artifacts_key, set_artifact_memory_in, validate_path_component,
};

/// アーティファクトの置き場所ワークツリーへスコープ固定して MCP ツールを呼ぶ。
/// リポジトリスコープからは呼べない(呼び出し側で弾く)。
pub type ToolCaller = Arc<
    dyn Fn(
            String, /* worktree_id */
            String, /* artifact_id */
            String, /* tool */
            serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// memory 書き込み後、開いている Tauri ビューアへ知らせる(mcp_server.rs の
/// `artifact-state-changed` 発行と同じ目的: 知らせないと Tauri 側 iframe が古い
/// スナップショットで書き戻し、Web 側の保存を消してしまう)。
/// このイベントは mcp_server.rs の `artifact-state-changed` リスナー経由で
/// `/api/events` (SSE, #341) へも中継される(web_viewer 自身は broadcast へ直接送らない)。
pub type StateChangedNotifier = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

/// `/api/events` (SSE) が中継するイベント。`artifact-changed` / `repo-artifact-changed` /
/// `artifact-state-changed` の3つの Tauri イベントをこの形に正規化して配信する。
///
/// `scope` は worktree の実 ID を積む。repository のときはそれに加えて `repo_key`
/// (URL に使う `repo_artifacts_key` のハッシュ) も積む。フロントは URL に使う
/// key と一覧 API が返す実 ID の両方を持つ必要があるため(`resolveScope` 参照)。
#[derive(Clone, Debug, Serialize)]
pub struct ViewerEvent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub scope: &'static str,
    #[serde(rename = "scopeId")]
    pub scope_id: String,
    #[serde(rename = "repoKey", skip_serializing_if = "Option::is_none")]
    pub repo_key: Option<String>,
    #[serde(rename = "artifactId")]
    pub artifact_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// `artifact-changed` のみ意味を持つ。フックによる URL 自動登録等の副産物としての
    /// 追加は `false` になり、ビューア側は「開く」導線を出さずに黙って一覧へ取り込む
    /// (`ArtifactViewerApp.vue` の `refreshSelected` 参照)。欠落時は `true` 相当として扱う
    /// (デスクトップ版 `tauriArtifactDataSource.ts` の `!== false` と同じ既定)。
    #[serde(rename = "autoOpen", skip_serializing_if = "Option::is_none")]
    pub auto_open: Option<bool>,
}

impl ViewerEvent {
    pub fn artifact_changed_worktree(
        worktree_id: String,
        artifact_id: String,
        command: String,
        auto_open: bool,
    ) -> Self {
        Self {
            kind: "artifact-changed",
            scope: "worktree",
            scope_id: worktree_id,
            repo_key: None,
            artifact_id,
            command: Some(command),
            auto_open: Some(auto_open),
        }
    }

    pub fn artifact_changed_repository(
        repository_id: String,
        artifact_id: String,
        command: String,
    ) -> Self {
        let repo_key = repo_artifacts_key(&repository_id);
        Self {
            kind: "artifact-changed",
            scope: "repository",
            scope_id: repository_id,
            repo_key: Some(repo_key),
            artifact_id,
            command: Some(command),
            // repo-artifact-changed は Rust 側でそもそも autoOpen を積んでいない
            // (常に "開く" 前提の transfer/import/delete のみが emit 元)。
            auto_open: None,
        }
    }

    pub fn state_changed(scope: &str, scope_id: String, artifact_id: String) -> Self {
        let repo_key = if scope == "repository" {
            Some(repo_artifacts_key(&scope_id))
        } else {
            None
        };
        Self {
            kind: "state-changed",
            scope: if scope == "repository" { "repository" } else { "worktree" },
            scope_id,
            repo_key,
            artifact_id,
            command: None,
            auto_open: None,
        }
    }
}

/// 現在の MCP API key を返す。設定は再起動なしで変わりうるため、毎リクエスト読む。
pub type KeyProvider = Arc<dyn Fn() -> String + Send + Sync>;

/// 埋め込み(または dev の frontendDist)アセットを `(bytes, mime_type)` で返す。
/// 見つからなければ `None`。
pub type AssetSource = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;

/// dev モードでの devUrl (例: `http://localhost:1420`)。`None` ならプロキシしない
/// (release ビルド、または dist が既に存在する dev)。
pub type DevProxyTarget = Option<String>;

/// `/api/*` が読む設定のスナップショットを返す。設定は再起動なしで変わりうるため、
/// 毎リクエスト読む(`KeyProvider` と同じ方針)。
pub type SettingsProvider = Arc<dyn Fn() -> (Vec<WorktreeEntry>, Vec<Repository>) + Send + Sync>;

const COOKIE_HMAC_CONTEXT: &[u8] = b"oretachi-web-viewer-session-v1";

#[derive(Clone)]
struct ViewerState {
    key: KeyProvider,
    assets: AssetSource,
    dev_proxy: Option<String>,
    cookie_name: String,
    data_dir: PathBuf,
    settings: SettingsProvider,
    port: u16,
    tool_caller: ToolCaller,
    notify_state_changed: StateChangedNotifier,
    events: broadcast::Sender<ViewerEvent>,
    shutdown: watch::Receiver<bool>,
}

fn session_cookie_value(key: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
        .expect("HMAC は任意長キーを受け付ける");
    mac.update(COOKIE_HMAC_CONTEXT);
    let bytes = mac.finalize().into_bytes();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn constant_time_eq_str(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// クッキーヘッダーから指定名の値を取り出す(単純パーサ。RFC 完全準拠は不要)。
fn find_cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')) {
            return Some(v);
        }
    }
    None
}

/// 未認証時の 401。`/api/*` は既存どおり空ボディのまま(フロントは status だけを見る)、
/// それ以外(SPA シェル・アセット)は認証下にあるアセットを使わずに描ける
/// インライン HTML の 401 画面を返す(ワイヤーフレームの「未認証」画面に相当)。
fn unauthorized_response(uri: &Uri) -> Response {
    if uri.path().starts_with("/api/") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    const BODY: &str = include_str!("web_viewer_unauthorized.html");
    let mut response = (StatusCode::UNAUTHORIZED, Html(BODY)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// 閲覧ルータ全体にかける認証 middleware。
async fn viewer_auth(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    uri: Uri,
    request: Request,
    next: Next,
) -> Response {
    let key = (state.key)();
    if key.is_empty() {
        log::warn!("[web_viewer] API key not configured, rejecting all requests");
        return unauthorized_response(&uri);
    }

    // `?token=` が付いていれば、Cookie へ移し替えて token 抜きの URL へリダイレクトする。
    let query = uri.query().unwrap_or("");
    let mut token: Option<String> = None;
    let mut remaining_pairs = Vec::new();
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        if let Some(v) = pair.strip_prefix("token=") {
            token = Some(
                percent_decode(v),
            );
        } else {
            remaining_pairs.push(pair);
        }
    }

    if let Some(provided) = token {
        if !constant_time_eq_str(&provided, &key) {
            log::warn!("[web_viewer] token mismatch");
            return unauthorized_response(&uri);
        }

        let mut location = uri.path().to_string();
        if !remaining_pairs.is_empty() {
            location.push('?');
            location.push_str(&remaining_pairs.join("&"));
        }

        let cookie_value = session_cookie_value(&key);
        let cookie = format!(
            "{}={}; HttpOnly; SameSite=Strict; Path=/",
            state.cookie_name, cookie_value
        );

        let mut response = Redirect::to(&location).into_response();
        let response_headers = response.headers_mut();
        if let Ok(hv) = HeaderValue::from_str(&cookie) {
            response_headers.insert(header::SET_COOKIE, hv);
        }
        response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response_headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        return response;
    }

    // token が無ければ Cookie を見る。
    let cookie_value = find_cookie(&headers, &state.cookie_name);
    let expected = session_cookie_value(&key);
    let authorized = match cookie_value {
        Some(v) => constant_time_eq_str(v, &expected),
        None => false,
    };

    if authorized {
        next.run(request).await
    } else {
        log::warn!("[web_viewer] unauthorized request: no valid session cookie");
        unauthorized_response(&uri)
    }
}

/// 書き込み系(GET/HEAD/OPTIONS 以外)の CSRF 対策 middleware。`viewer_auth` の内側
/// (Cookie 認証が通った後)に layer する。
///
/// Cookie は `SameSite=Strict` のみで守られているが、RFC 6265 の Cookie 分離も
/// schemeful-same-site 判定もポート単位ではないため、同一ホストの別ポートで動く
/// 別インスタンス/別ローカルサーバー(vite dev server 等)から見ると同一サイト
/// scoped になりうる。そのため Origin と Host(port 込み)の完全一致を要求する。
///
/// DNS rebinding(`evil.example:<port>` が 127.0.0.1 を指す)については、Origin も
/// Host も `evil.example:<port>` で一致してしまいここは通過するが、Cookie は
/// host-only 属性で `evil.example` へは送られないため `viewer_auth` の Cookie 検証
/// (このガードより外側)で 401 になる。この防御が csrf_guard 単体では閉じないことに注意。
async fn csrf_guard(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    method: Method,
    request: Request,
    next: Next,
) -> Response {
    if matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(request).await;
    }

    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());

    let Some(host) = host else {
        return api_error(StatusCode::FORBIDDEN, "Host ヘッダーがありません");
    };
    let Some(origin) = origin else {
        return api_error(StatusCode::FORBIDDEN, "Origin ヘッダーがありません");
    };

    let host_port = host
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse::<u16>().ok())
        .unwrap_or(80);
    if host_port != state.port {
        return api_error(StatusCode::FORBIDDEN, "Host のポートが一致しません");
    }

    let expected_origin = format!("http://{}", host);
    if origin != expected_origin {
        return api_error(StatusCode::FORBIDDEN, "Origin が一致しません");
    }

    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if site != "same-origin" {
            return api_error(StatusCode::FORBIDDEN, "Sec-Fetch-Site が same-origin ではありません");
        }
    }

    next.run(request).await
}

/// `application/x-www-form-urlencoded` 相当の最小限のパーセントデコード。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(
                    std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                    16,
                ) {
                    out.push(byte);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// アセットパスの正規化。`..` や `.`、空セグメント、`\`、`:` を含むものは拒否する。
fn sanitize_asset_path(path: &str) -> Option<String> {
    if path.contains('\\') || path.contains(':') {
        return None;
    }
    let mut segments = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return None;
        }
        segments.push(seg);
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

fn asset_response(bytes: Vec<u8>, mime_type: String) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_type)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// SPA シェルのアセット名。`index.html` (Tauri 版) とは別エントリ (#339)。
const WEB_SPA_SHELL: &str = "web.html";

async fn serve_index(State(state): State<ViewerState>) -> Response {
    match (state.assets)(WEB_SPA_SHELL) {
        Some((bytes, mime_type)) => {
            let mut response = asset_response(bytes, mime_type);
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            response
        }
        None => {
            if let Some(target) = &state.dev_proxy {
                return proxy_to_dev(target, &format!("/{}", WEB_SPA_SHELL)).await;
            }
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

/// `/assets/*`・`/vendor/*` に加え、`index.html` がルート直下から参照するファイル
/// (`/vite.svg` 等)もここで拾う。 明示的なルートに一致しなかった GET はすべてここへ来る。
async fn static_fallback(State(state): State<ViewerState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    match sanitize_asset_path(path) {
        Some(sanitized) => match (state.assets)(&sanitized) {
            Some((bytes, mime_type)) => asset_response(bytes, mime_type),
            None => proxy_or_not_found(&state, &uri).await,
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn proxy_or_not_found(state: &ViewerState, uri: &Uri) -> Response {
    if let Some(target) = &state.dev_proxy {
        proxy_to_dev(target, &uri.to_string()).await
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// dev モード用: dist に無いパスを devUrl (vite) へプロキシする。
/// release ビルドではこの関数は呼ばれない( `dev_proxy` が `None` のため)。
async fn proxy_to_dev(target: &str, path_and_query: &str) -> Response {
    let url = format!("{}{}", target.trim_end_matches('/'), path_and_query);
    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .cloned();
            match resp.bytes().await {
                Ok(bytes) => {
                    let mut builder = Response::builder().status(status);
                    if let Some(ct) = content_type {
                        builder = builder.header(header::CONTENT_TYPE, ct);
                    }
                    builder
                        .body(Body::from(bytes))
                        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
                }
                Err(e) => {
                    log::warn!("[web_viewer] dev proxy body read failed: {}", e);
                    StatusCode::BAD_GATEWAY.into_response()
                }
            }
        }
        Err(e) => {
            log::warn!("[web_viewer] dev proxy request to {} failed: {}", url, e);
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

async fn spa_shell(state: State<ViewerState>) -> Response {
    serve_index(state).await
}

async fn root_redirect() -> impl IntoResponse {
    Redirect::to("/worktrees")
}

// ─── /api/* (読み取り JSON API) ────────────────────────────────────────────
//
// このセクションはすべて `viewer_auth` の配下 (Cookie 認証必須) にある。
// GET のみで副作用がないため CSRF は問題にならない (web_viewer.rs 冒頭のコメント参照)。

fn json_response(status: StatusCode, value: impl Serialize) -> Response {
    let mut response = (status, Json(value)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    json_response(status, json!({ "error": message.into() }))
}

fn last_updated_at(items: &[serde_json::Value]) -> u64 {
    items
        .iter()
        .filter_map(|v| v.get("updated_at").and_then(|x| x.as_u64()))
        .max()
        .unwrap_or(0)
}

/// `GET /api/worktrees`: ワークツリー一覧(件数・最終更新込み)と、
/// アーティファクトを 1 件以上持つリポジトリの一覧を返す。
async fn api_list_worktrees(State(state): State<ViewerState>) -> Response {
    let data_dir = state.data_dir.clone();
    let (worktrees, repositories) = (state.settings)();
    let result = tokio::task::spawn_blocking(move || {
        let worktree_entries: Vec<serde_json::Value> = worktrees
            .iter()
            .map(|w| {
                let artifacts = artifacts_dir_in(&data_dir, &w.id)
                    .ok()
                    .map(|dir| {
                        list_artifacts_in_dir(&dir).unwrap_or_else(|e| {
                            // 1件でも壊れていると全体 Err になる（list_artifacts_in_dir の仕様）。
                            // 件数が原因不明のまま 0 に見えないよう、握りつぶさずログに残す。
                            log::warn!(
                                "[web_viewer] worktree {} のアーティファクト一覧取得に失敗: {}",
                                w.id,
                                e
                            );
                            vec![]
                        })
                    })
                    .unwrap_or_default();
                json!({
                    "id": w.id,
                    "name": w.name,
                    "repositoryId": w.repository_id,
                    "repositoryName": w.repository_name,
                    "branchName": w.branch_name,
                    "description": w.description,
                    "isHome": w.is_home,
                    "isRepository": w.is_repository,
                    "artifactCount": artifacts.len(),
                    "lastUpdatedAt": last_updated_at(&artifacts),
                })
            })
            .collect();

        let repository_entries: Vec<serde_json::Value> = repositories
            .iter()
            .filter_map(|r| {
                let dir = repo_artifacts_dir_in(&data_dir, &r.id).ok()?;
                let artifacts = list_repo_artifacts_in_dir(&dir);
                if artifacts.is_empty() {
                    return None;
                }
                Some(json!({
                    "key": repo_artifacts_key(&r.id),
                    "id": r.id,
                    "name": r.name,
                    "artifactCount": artifacts.len(),
                    "lastUpdatedAt": last_updated_at(&artifacts),
                }))
            })
            .collect();

        json!({ "worktrees": worktree_entries, "repositories": repository_entries })
    })
    .await;

    match result {
        Ok(body) => json_response(StatusCode::OK, body),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("task join error: {}", e)),
    }
}

/// `GET /api/worktrees/{id}/artifacts`
async fn api_list_worktree_artifacts(
    State(state): State<ViewerState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    if let Err(e) = validate_path_component(&id) {
        return api_error(StatusCode::BAD_REQUEST, e);
    }
    let (worktrees, _repositories) = (state.settings)();
    if !worktrees.iter().any(|w| w.id == id) {
        return api_error(StatusCode::NOT_FOUND, "ワークツリーが見つかりません");
    }
    let dir = match artifacts_dir_in(&state.data_dir, &id) {
        Ok(dir) => dir,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, e),
    };
    match tokio::task::spawn_blocking(move || list_artifacts_in_dir(&dir)).await {
        Ok(Ok(list)) => json_response(StatusCode::OK, list),
        Ok(Err(e)) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("task join error: {}", e)),
    }
}

/// `GET /api/repositories/{repo_key}/artifacts`
async fn api_list_repo_artifacts(
    State(state): State<ViewerState>,
    AxumPath(repo_key): AxumPath<String>,
) -> Response {
    if let Err(e) = validate_path_component(&repo_key) {
        return api_error(StatusCode::BAD_REQUEST, e);
    }
    let (_worktrees, repositories) = (state.settings)();
    let Some(repository) = repositories
        .iter()
        .find(|r| repo_artifacts_key(&r.id) == repo_key)
    else {
        return api_error(StatusCode::NOT_FOUND, "リポジトリが見つかりません");
    };
    let dir = match repo_artifacts_dir_in(&state.data_dir, &repository.id) {
        Ok(dir) => dir,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, e),
    };
    let list = tokio::task::spawn_blocking(move || list_repo_artifacts_in_dir(&dir)).await;
    match list {
        Ok(list) => json_response(StatusCode::OK, list),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("task join error: {}", e)),
    }
}

/// 本体 JSON・memory・memoryUpdatedAt をまとめて読む。ファイルが無ければ `Ok(None)`。
///
/// 存在確認と読み込みを分けると、その間に削除された場合に 404 ではなく 500 になる
/// (TOCTOU)。ここでは 1 回の読み込みの結果だけで判定する。
fn read_artifact_payload(
    dir: &std::path::Path,
    artifact_id: &str,
) -> Result<Option<serde_json::Value>, String> {
    validate_path_component(artifact_id)?;
    let path = dir.join(format!("{}.json", artifact_id));
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let artifact: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let (memory, memory_updated_at) = read_artifact_store(dir, artifact_id);
    Ok(Some(json!({
        "artifact": artifact,
        "memory": memory,
        "memoryUpdatedAt": memory_updated_at,
    })))
}

async fn respond_with_artifact(dir: std::path::PathBuf, artifact_id: String) -> Response {
    match tokio::task::spawn_blocking(move || read_artifact_payload(&dir, &artifact_id)).await {
        Ok(Ok(Some(body))) => json_response(StatusCode::OK, body),
        Ok(Ok(None)) => api_error(StatusCode::NOT_FOUND, "アーティファクトが見つかりません"),
        Ok(Err(e)) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("task join error: {}", e)),
    }
}

/// `GET /api/worktrees/{id}/artifacts/{artifact_id}`
async fn api_read_worktree_artifact(
    State(state): State<ViewerState>,
    AxumPath((id, artifact_id)): AxumPath<(String, String)>,
) -> Response {
    if let Err(e) = validate_path_component(&id) {
        return api_error(StatusCode::BAD_REQUEST, e);
    }
    if let Err(e) = validate_path_component(&artifact_id) {
        return api_error(StatusCode::BAD_REQUEST, e);
    }
    let (worktrees, _repositories) = (state.settings)();
    if !worktrees.iter().any(|w| w.id == id) {
        return api_error(StatusCode::NOT_FOUND, "ワークツリーが見つかりません");
    }
    let dir = match artifacts_dir_in(&state.data_dir, &id) {
        Ok(dir) => dir,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, e),
    };
    respond_with_artifact(dir, artifact_id).await
}

/// `GET /api/repositories/{repo_key}/artifacts/{artifact_id}`
async fn api_read_repo_artifact(
    State(state): State<ViewerState>,
    AxumPath((repo_key, artifact_id)): AxumPath<(String, String)>,
) -> Response {
    if let Err(e) = validate_path_component(&repo_key) {
        return api_error(StatusCode::BAD_REQUEST, e);
    }
    if let Err(e) = validate_path_component(&artifact_id) {
        return api_error(StatusCode::BAD_REQUEST, e);
    }
    let (_worktrees, repositories) = (state.settings)();
    let Some(repository) = repositories
        .iter()
        .find(|r| repo_artifacts_key(&r.id) == repo_key)
    else {
        return api_error(StatusCode::NOT_FOUND, "リポジトリが見つかりません");
    };
    let dir = match repo_artifacts_dir_in(&state.data_dir, &repository.id) {
        Ok(dir) => dir,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, e),
    };
    respond_with_artifact(dir, artifact_id).await
}

// ─── /api/* (書き込み JSON API、#340) ───────────────────────────────────────
//
// `csrf_guard`(Origin/Host 検証)と `viewer_auth`(Cookie 認証)の両方を通った
// リクエストのみここへ届く。

#[derive(Deserialize)]
struct SetMemoryBody {
    memory: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CallToolBody {
    tool: String,
    #[serde(default)]
    params: serde_json::Value,
}

fn resolve_worktree_dir(state: &ViewerState, id: &str) -> Result<PathBuf, Response> {
    validate_path_component(id).map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let (worktrees, _repositories) = (state.settings)();
    if !worktrees.iter().any(|w| w.id == id) {
        return Err(api_error(StatusCode::NOT_FOUND, "ワークツリーが見つかりません"));
    }
    artifacts_dir_in(&state.data_dir, id).map_err(|e| api_error(StatusCode::BAD_REQUEST, e))
}

/// リポジトリ key からディレクトリと実 ID(通知の scopeId に使う)を解決する。
fn resolve_repo_dir(state: &ViewerState, repo_key: &str) -> Result<(PathBuf, String), Response> {
    validate_path_component(repo_key).map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let (_worktrees, repositories) = (state.settings)();
    let Some(repository) = repositories
        .iter()
        .find(|r| repo_artifacts_key(&r.id) == repo_key)
    else {
        return Err(api_error(StatusCode::NOT_FOUND, "リポジトリが見つかりません"));
    };
    let dir = repo_artifacts_dir_in(&state.data_dir, &repository.id)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    Ok((dir, repository.id.clone()))
}

fn memory_response(updated_at: Option<u64>) -> Response {
    json_response(StatusCode::OK, json!({ "memoryUpdatedAt": updated_at.unwrap_or(0) }))
}

/// `POST /api/worktrees/{id}/artifacts/{artifact_id}/memory`
async fn api_set_worktree_memory(
    State(state): State<ViewerState>,
    AxumPath((id, artifact_id)): AxumPath<(String, String)>,
    Json(body): Json<SetMemoryBody>,
) -> Response {
    let dir = match resolve_worktree_dir(&state, &id) {
        Ok(dir) => dir,
        Err(resp) => return resp,
    };
    match set_artifact_memory_in(dir, &artifact_id, body.memory).await {
        Ok(updated_at) => {
            if updated_at.is_some() {
                (state.notify_state_changed)("worktree", &id, &artifact_id);
            }
            memory_response(updated_at)
        }
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

/// `POST /api/repositories/{repo_key}/artifacts/{artifact_id}/memory`
async fn api_set_repo_memory(
    State(state): State<ViewerState>,
    AxumPath((repo_key, artifact_id)): AxumPath<(String, String)>,
    Json(body): Json<SetMemoryBody>,
) -> Response {
    let (dir, repository_id) = match resolve_repo_dir(&state, &repo_key) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match set_artifact_memory_in(dir, &artifact_id, body.memory).await {
        Ok(updated_at) => {
            if updated_at.is_some() {
                (state.notify_state_changed)("repository", &repository_id, &artifact_id);
            }
            memory_response(updated_at)
        }
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

/// `POST /api/worktrees/{id}/artifacts/{artifact_id}/call-tool`
async fn api_call_tool_worktree(
    State(state): State<ViewerState>,
    AxumPath((id, artifact_id)): AxumPath<(String, String)>,
    Json(body): Json<CallToolBody>,
) -> Response {
    if let Err(resp) = resolve_worktree_dir(&state, &id) {
        return resp;
    }
    match (state.tool_caller)(id, artifact_id, body.tool, body.params).await {
        Ok(result) => json_response(StatusCode::OK, json!({ "result": result })),
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

/// `POST /api/repositories/{repo_key}/artifacts/{artifact_id}/call-tool`
///
/// リポジトリ保管庫のアーティファクトには紐づくワークツリーが無く、スコープを強制できない
/// ため常に拒否する(`lib.rs` の `artifact_call_mcp_tool` と同じ方針)。
async fn api_call_tool_repo() -> Response {
    api_error(
        StatusCode::BAD_REQUEST,
        "MCP ツール呼び出しはワークツリーのアーティファクトからのみ使えます（現在のスコープ: repository）",
    )
}

/// `GET /api/events`: アーティファクトの変更を SSE で中継する。
///
/// `viewer_auth` の配下にあるため Cookie 認証が必須。接続が `broadcast::Receiver` の
/// バッファ(256件)を溢れさせて `Lagged` になった場合は、個々の差分を追わせる代わりに
/// `{"type":"resync"}` を1件送って呼び出し元に一覧・本体の再取得を促す。
/// サーバー再起動(`shutdown` が true になる)を検知したら接続を閉じる
/// (開いたままの SSE 接続があると graceful shutdown が完了しない)。
async fn api_events(State(state): State<ViewerState>) -> Response {
    let rx = state.events.subscribe();
    let shutdown = state.shutdown.clone();
    let stream = futures_util::stream::unfold((rx, shutdown), |(mut rx, mut shutdown)| async move {
        loop {
            tokio::select! {
                result = rx.recv() => {
                    return match result {
                        Ok(payload) => {
                            let data = serde_json::to_string(&payload).unwrap_or_default();
                            Some((Ok::<_, Infallible>(SseEvent::default().data(data)), (rx, shutdown)))
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("[web_viewer] SSE lagged, {} events dropped; sending resync", n);
                            let data = json!({ "type": "resync" }).to_string();
                            Some((Ok(SseEvent::default().data(data)), (rx, shutdown)))
                        }
                        Err(broadcast::error::RecvError::Closed) => None,
                    };
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return None;
                    }
                    // false → false の変化は基本無いはずだが、安全側でループを続ける。
                }
            }
        }
    });

    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// テスト・実装から共通で使うルータ組み立て。
fn build_router(
    key: KeyProvider,
    cookie_name: String,
    assets: AssetSource,
    dev_proxy: DevProxyTarget,
    data_dir: PathBuf,
    settings: SettingsProvider,
    port: u16,
    tool_caller: ToolCaller,
    notify_state_changed: StateChangedNotifier,
    events: broadcast::Sender<ViewerEvent>,
    shutdown: watch::Receiver<bool>,
) -> Router {
    let state = ViewerState {
        key: key.clone(),
        assets,
        dev_proxy: dev_proxy.clone(),
        cookie_name,
        data_dir,
        settings,
        port,
        tool_caller,
        notify_state_changed,
        events,
        shutdown,
    };

    // `/assets/*`・`/vendor/*`・ルート直下の静的ファイル(`/vite.svg` 等)は
    // すべて `static_fallback` が処理する。dev モードでは dist に無いパス
    // (`/src/*`、`/@vite/*` 等)を devUrl (vite) へプロキシするフォールバックも兼ねる。
    Router::new()
        .route("/", get(root_redirect))
        .route("/worktrees", get(spa_shell))
        .route("/worktrees/{*rest}", get(spa_shell))
        .route("/repositories/{*rest}", get(spa_shell))
        .route("/api/worktrees", get(api_list_worktrees))
        .route("/api/worktrees/{id}/artifacts", get(api_list_worktree_artifacts))
        .route(
            "/api/worktrees/{id}/artifacts/{artifact_id}",
            get(api_read_worktree_artifact),
        )
        .route(
            "/api/repositories/{repo_key}/artifacts",
            get(api_list_repo_artifacts),
        )
        .route(
            "/api/repositories/{repo_key}/artifacts/{artifact_id}",
            get(api_read_repo_artifact),
        )
        .route(
            "/api/worktrees/{id}/artifacts/{artifact_id}/memory",
            post(api_set_worktree_memory),
        )
        .route(
            "/api/repositories/{repo_key}/artifacts/{artifact_id}/memory",
            post(api_set_repo_memory),
        )
        .route(
            "/api/worktrees/{id}/artifacts/{artifact_id}/call-tool",
            post(api_call_tool_worktree),
        )
        .route(
            "/api/repositories/{repo_key}/artifacts/{artifact_id}/call-tool",
            post(api_call_tool_repo),
        )
        .route("/api/events", get(api_events))
        .fallback(get(static_fallback))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state.clone(), csrf_guard))
        .layer(middleware::from_fn_with_state(state, viewer_auth))
}

/// 本番用: Bearer layer の外で merge する閲覧ルータを組み立てる。
///
/// `port` は bind 後の実ポート(0 指定時も実際の値)を渡すこと。Cookie 名に含めることで、
/// 同一ホストで動く別インスタンス(本番/dev)の Cookie を混同しないようにする。
/// `events` は `/api/events` (SSE) が中継するイベントの送信側、`shutdown` はサーバー
/// 再起動時に SSE 接続を閉じるための監視チャンネル(呼び出し元の `shutdown_rx` を渡す)。
pub fn router(
    app_handle: AppHandle,
    port: u16,
    events: broadcast::Sender<ViewerEvent>,
    shutdown: watch::Receiver<bool>,
) -> Router {
    let key_handle = app_handle.clone();
    let key: KeyProvider = Arc::new(move || {
        key_handle
            .state::<SettingsManager>()
            .get()
            .mcp_api_key
            .clone()
    });

    let asset_handle = app_handle.clone();
    let assets: AssetSource = Arc::new(move |path: &str| {
        let asset = asset_handle.asset_resolver().get(path.to_string())?;
        Some((asset.bytes, asset.mime_type))
    });

    let dev_proxy = if tauri::is_dev() {
        // dist が実在すれば `assets` 側で解決できるので、プロキシは
        // 「dist に無いパス」のフォールバックとしてのみ使う。
        app_handle
            .config()
            .build
            .dev_url
            .as_ref()
            .map(|u| u.to_string())
    } else {
        None
    };

    let cookie_name = format!("oretachi_viewer_{}", port);

    let settings_handle = app_handle.clone();
    let settings: SettingsProvider = Arc::new(move || {
        let s = settings_handle.state::<SettingsManager>().get();
        (s.worktrees, s.repositories)
    });

    let data_dir = app_handle
        .path()
        .app_data_dir()
        .expect("app_data_dir は起動時に必ず解決できる");

    let tool_caller_handle = app_handle.clone();
    let tool_caller: ToolCaller = Arc::new(move |worktree_id, artifact_id, tool, params| {
        let handle = tool_caller_handle.clone();
        Box::pin(async move {
            crate::mcp_server::call_tool_for_artifact(&handle, &worktree_id, &artifact_id, &tool, params)
                .await
        })
    });

    // `artifact-state-changed` の emit は mcp_server.rs のリスナー経由で `/api/events` (SSE) へも
    // 中継される(#341)。ここでは Tauri イベントを発行するだけでよい。
    let notify_handle = app_handle.clone();
    let notify_state_changed: StateChangedNotifier = Arc::new(move |scope, scope_id, artifact_id| {
        if let Err(e) = notify_handle.emit(
            "artifact-state-changed",
            json!({ "scope": scope, "scopeId": scope_id, "artifactId": artifact_id }),
        ) {
            log::warn!("[web_viewer] Failed to emit artifact-state-changed: {}", e);
        }
    });

    build_router(
        key,
        cookie_name,
        assets,
        dev_proxy,
        data_dir,
        settings,
        port,
        tool_caller,
        notify_state_changed,
        events,
        shutdown,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request as HttpRequest;
    use futures_util::StreamExt;
    use std::time::Duration;
    use tower::ServiceExt;

    const TEST_KEY: &str = "test-api-key-1234567890";

    fn key_provider(key: &str) -> KeyProvider {
        let key = key.to_string();
        Arc::new(move || key.clone())
    }

    fn stub_assets() -> AssetSource {
        Arc::new(|path: &str| match path {
            "web.html" => Some((b"<html>index</html>".to_vec(), "text/html".to_string())),
            "assets/app.js" => Some((b"console.log(1)".to_vec(), "text/javascript".to_string())),
            _ => None,
        })
    }

    fn empty_settings() -> SettingsProvider {
        Arc::new(|| (vec![], vec![]))
    }

    fn test_events() -> broadcast::Sender<ViewerEvent> {
        broadcast::channel(16).0
    }

    /// send 側を保持せず戻すと、watch チャンネルが即クローズ扱いになり
    /// `changed()` が(shutdown を送っていないのに)即座に `Err` で解決してしまう
    /// (`tokio::select!` はどちらの分岐が先に ready でも取り得るため、これが
    /// `/api/events` のストリームを起動直後に終了させるフレーキーな失敗の原因になっていた)。
    /// send 側は使わないが、`forget` して drop させないことでチャンネルを開いたままにする。
    fn test_shutdown() -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        std::mem::forget(tx);
        rx
    }

    /// テストごとに衝突しない一時ディレクトリを用意する(既存があれば作り直す)。
    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oretachi-web-viewer-test-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_worktree(id: &str) -> WorktreeEntry {
        WorktreeEntry {
            id: id.to_string(),
            name: format!("name-{}", id),
            repository_id: "repo-1".to_string(),
            repository_name: "repo-name".to_string(),
            path: format!("/tmp/{}", id),
            branch_name: "main".to_string(),
            hotkey_char: None,
            auto_approval: None,
            auto_approval_prompt: None,
            description: None,
            description_open: None,
            workgroup_id: None,
            tray_notification: None,
            is_home: false,
            is_repository: false,
        }
    }

    fn test_repository(id: &str) -> Repository {
        Repository {
            id: id.to_string(),
            name: format!("repo-name-{}", id),
            path: id.to_string(),
            exec_script: None,
            copy_targets: None,
            package_manager: None,
            package_manager_args: None,
            notification_hooks: None,
            pull_before_add: None,
            branch_name_pattern: None,
        }
    }

    fn write_test_artifact(dir: &std::path::Path, artifact_id: &str, updated_at: u64) {
        std::fs::create_dir_all(dir).unwrap();
        let content = json!({
            "id": artifact_id,
            "updated_at": updated_at,
            "content": "hello",
            "modules": {},
        });
        std::fs::write(
            dir.join(format!("{}.json", artifact_id)),
            serde_json::to_string(&content).unwrap(),
        )
        .unwrap();
    }

    fn cookie_for(key: &str) -> String {
        format!("oretachi_viewer_test={}", session_cookie_value(key))
    }

    const TEST_PORT: u16 = 34567;

    fn test_host() -> String {
        format!("localhost:{}", TEST_PORT)
    }

    fn test_origin() -> String {
        format!("http://{}", test_host())
    }

    /// テスト用 tool_caller。渡された引数を記録し、固定の結果 (または注入されたエラー) を返す。
    fn stub_tool_caller(
        calls: Arc<std::sync::Mutex<Vec<(String, String, String, serde_json::Value)>>>,
        result: Result<String, String>,
    ) -> ToolCaller {
        Arc::new(move |worktree_id, artifact_id, tool, params| {
            calls
                .lock()
                .unwrap()
                .push((worktree_id, artifact_id, tool, params));
            let result = result.clone();
            Box::pin(async move { result })
        })
    }

    fn noop_state_notifier() -> StateChangedNotifier {
        Arc::new(|_, _, _| {})
    }

    /// 呼ばれた引数 `(scope, scopeId, artifactId)` を記録するテスト用 notifier。
    fn recording_state_notifier(
        calls: Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
    ) -> StateChangedNotifier {
        Arc::new(move |scope, scope_id, artifact_id| {
            calls
                .lock()
                .unwrap()
                .push((scope.to_string(), scope_id.to_string(), artifact_id.to_string()));
        })
    }

    fn router_for_test() -> Router {
        build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            temp_data_dir("basic"),
            empty_settings(),
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        )
    }

    async fn get(router: &Router, uri: &str, cookie: Option<&str>) -> Response {
        let mut builder = HttpRequest::builder().uri(uri).method("GET");
        if let Some(c) = cookie {
            builder = builder.header(header::COOKIE, c);
        }
        let req = builder.body(Body::empty()).unwrap();
        router.clone().oneshot(req).await.unwrap()
    }

    /// POST リクエストを送る。`origin`/`host` を渡すと CSRF ガード用ヘッダーとして付ける。
    async fn post_json(
        router: &Router,
        uri: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
        host: Option<&str>,
        body: serde_json::Value,
    ) -> Response {
        let mut builder = HttpRequest::builder().uri(uri).method("POST");
        if let Some(c) = cookie {
            builder = builder.header(header::COOKIE, c);
        }
        if let Some(o) = origin {
            builder = builder.header(header::ORIGIN, o);
        }
        if let Some(h) = host {
            builder = builder.header(header::HOST, h);
        }
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        let req = builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        router.clone().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn token_redirects_and_sets_cookie() {
        let router = router_for_test();
        let resp = get(&router, "/worktrees?token=test-api-key-1234567890&x=1", None).await;
        // axum Redirect::to は 303 (SEE_OTHER) を返す
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let location = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        assert_eq!(location, "/worktrees?x=1");
        let set_cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Strict"));
        assert!(set_cookie.contains("Path=/"));
        assert!(!set_cookie.contains(TEST_KEY));
    }

    #[tokio::test]
    async fn cookie_grants_access_without_token() {
        let router = router_for_test();
        let cookie_value = session_cookie_value(TEST_KEY);
        let cookie = format!("oretachi_viewer_test={}", cookie_value);
        let resp = get(&router, "/worktrees", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"<html>index</html>");

        let resp = get(&router, "/assets/app.js", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn no_token_no_cookie_is_unauthorized() {
        let router = router_for_test();
        let resp = get(&router, "/worktrees", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unauthorized_non_api_path_returns_html_page() {
        let router = router_for_test();
        let resp = get(&router, "/worktrees", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap().to_str().unwrap(),
            "no-store"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("<html"));
        assert!(body.contains("Authentication required"));
    }

    #[tokio::test]
    async fn unauthorized_api_path_returns_empty_body_as_before() {
        let router = router_for_test();
        let resp = get(&router, "/api/worktrees", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.is_empty());
    }

    #[tokio::test]
    async fn wrong_token_is_unauthorized() {
        let router = router_for_test();
        let resp = get(&router, "/worktrees?token=wrong", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tampered_cookie_is_unauthorized() {
        let router = router_for_test();
        let resp = get(&router, "/worktrees", Some("oretachi_viewer_test=deadbeef")).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn empty_key_rejects_everything() {
        let router = build_router(
            key_provider(""),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            temp_data_dir("empty-key"),
            empty_settings(),
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        );
        let resp = get(&router, "/worktrees?token=", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn key_rotation_invalidates_old_cookie() {
        let cookie_value = session_cookie_value(TEST_KEY);
        let cookie = format!("oretachi_viewer_test={}", cookie_value);

        // key を差し替えられる KeyProvider(regenerate_mcp_api_key 相当)
        let current = Arc::new(std::sync::Mutex::new(TEST_KEY.to_string()));
        let current_for_provider = current.clone();
        let key: KeyProvider = Arc::new(move || current_for_provider.lock().unwrap().clone());
        let router = build_router(
            key,
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            temp_data_dir("key-rotation"),
            empty_settings(),
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        );

        let resp = get(&router, "/worktrees", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        *current.lock().unwrap() = "rotated-key".to_string();
        let resp = get(&router, "/worktrees", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn path_traversal_in_asset_path_is_rejected() {
        let router = router_for_test();
        let cookie_value = session_cookie_value(TEST_KEY);
        let cookie = format!("oretachi_viewer_test={}", cookie_value);
        let resp = get(&router, "/assets/../secrets.txt", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn existing_bearer_routes_are_unaffected_by_viewer_auth() {
        use axum::routing::post;

        async fn bearer_protected() -> &'static str {
            "ok"
        }

        async fn bearer_auth(req: Request, next: Next) -> Result<Response, StatusCode> {
            let authorized = req
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(|h| h == "Bearer test-api-key-1234567890")
                .unwrap_or(false);
            if authorized {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }

        let bearer_router = Router::new()
            .route("/notify", post(bearer_protected))
            .layer(middleware::from_fn(bearer_auth));

        let combined = bearer_router.merge(router_for_test());

        // Bearer ルートは Bearer 必須のまま。
        let req = HttpRequest::builder()
            .uri("/notify")
            .method("POST")
            .body(Body::empty())
            .unwrap();
        let resp = combined.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let req = HttpRequest::builder()
            .uri("/notify")
            .method("POST")
            .header(header::AUTHORIZATION, "Bearer test-api-key-1234567890")
            .body(Body::empty())
            .unwrap();
        let resp = combined.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 閲覧ルートに Bearer を付けても、Cookie が無ければ 401(閲覧ルートは Bearer を認識しない)。
        let req = HttpRequest::builder()
            .uri("/worktrees")
            .method("GET")
            .header(header::AUTHORIZATION, "Bearer test-api-key-1234567890")
            .body(Body::empty())
            .unwrap();
        let resp = combined.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    async fn json_body(resp: Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn api_worktrees_requires_cookie() {
        let router = router_for_test();
        let resp = get(&router, "/api/worktrees", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn api_worktrees_returns_counts_and_repositories_with_artifacts() {
        let data_dir = temp_data_dir("worktrees-api");
        let wt = test_worktree("wt-a");
        let repo_with = test_repository("D:/git/has-artifacts");
        let repo_without = test_repository("D:/git/empty");

        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-2", 200);
        let repo_key = repo_artifacts_key(&repo_with.id);
        write_test_artifact(&data_dir.join("repo-artifacts").join(&repo_key), "repo-art-1", 50);

        let wt_for_settings = wt.clone();
        let repos_for_settings = vec![repo_with.clone(), repo_without.clone()];
        let settings: SettingsProvider =
            Arc::new(move || (vec![wt_for_settings.clone()], repos_for_settings.clone()));

        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            data_dir,
            settings,
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        );
        let cookie = cookie_for(TEST_KEY);
        let resp = get(&router, "/api/worktrees", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;

        assert_eq!(body["worktrees"][0]["id"], "wt-a");
        assert_eq!(body["worktrees"][0]["artifactCount"], 2);
        assert_eq!(body["worktrees"][0]["lastUpdatedAt"], 200);

        let repositories = body["repositories"].as_array().unwrap();
        assert_eq!(repositories.len(), 1, "アーティファクトの無いリポジトリは出ない");
        assert_eq!(repositories[0]["key"], repo_key);
        assert_eq!(repositories[0]["artifactCount"], 1);
    }

    #[tokio::test]
    async fn api_worktree_list_and_read_roundtrip_with_memory() {
        let data_dir = temp_data_dir("worktree-list-read");
        let wt = test_worktree("wt-b");
        let artifacts_dir = data_dir.join("artifacts").join(&wt.id);
        write_test_artifact(&artifacts_dir, "art-1", 100);
        std::fs::write(
            artifacts_dir.join("art-1.state"),
            serde_json::to_string(&json!({ "memory": { "foo": "bar" }, "memoryUpdatedAt": 42 }))
                .unwrap(),
        )
        .unwrap();

        let wt_for_settings = wt.clone();
        let settings: SettingsProvider = Arc::new(move || (vec![wt_for_settings.clone()], vec![]));
        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            data_dir,
            settings,
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = get(&router, "/api/worktrees/wt-b/artifacts", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let list = json_body(resp).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["content"], "hello");

        let resp = get(
            &router,
            "/api/worktrees/wt-b/artifacts/art-1",
            Some(&cookie),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        assert_eq!(body["artifact"]["id"], "art-1");
        assert_eq!(body["memory"]["foo"], "bar");
        assert_eq!(body["memoryUpdatedAt"], 42);

        let resp = get(
            &router,
            "/api/worktrees/wt-b/artifacts/does-not-exist",
            Some(&cookie),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_repository_list_drops_content_and_modules_except_url_artifacts() {
        let data_dir = temp_data_dir("repo-list");
        let repo = test_repository("D:/git/repo-list");
        let repo_key = repo_artifacts_key(&repo.id);
        let dir = data_dir.join("repo-artifacts").join(&repo_key);
        write_test_artifact(&dir, "art-1", 100);
        std::fs::write(
            dir.join("art-2.json"),
            serde_json::to_string(&json!({
                "id": "art-2",
                "updated_at": 200,
                "type": "text/uri-list",
                "content": "https://example.com",
                "modules": {},
            }))
            .unwrap(),
        )
        .unwrap();

        let repo_for_settings = repo.clone();
        let settings: SettingsProvider = Arc::new(move || (vec![], vec![repo_for_settings.clone()]));
        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            data_dir,
            settings,
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = get(
            &router,
            &format!("/api/repositories/{}/artifacts", repo_key),
            Some(&cookie),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let list = json_body(resp).await;
        let by_id = |id: &str| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == id)
                .unwrap()
                .clone()
        };
        assert!(by_id("art-1").get("content").is_none());
        assert!(by_id("art-1").get("modules").is_none());
        assert_eq!(by_id("art-2")["content"], "https://example.com");

        let resp = get(
            &router,
            &format!("/api/repositories/{}/artifacts/art-1", repo_key),
            Some(&cookie),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        // read エンドポイントは list と違い本体をそのまま返すので content が残る
        assert_eq!(body["artifact"]["content"], "hello");
    }

    #[tokio::test]
    async fn api_rejects_path_traversal_and_unknown_ids() {
        let data_dir = temp_data_dir("api-traversal");
        let wt = test_worktree("wt-c");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 1);

        let wt_for_settings = wt.clone();
        let settings: SettingsProvider = Arc::new(move || (vec![wt_for_settings.clone()], vec![]));
        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            data_dir,
            settings,
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        );
        let cookie = cookie_for(TEST_KEY);

        // `..` を含む id
        let resp = get(
            &router,
            "/api/worktrees/..%2Fetc/artifacts",
            Some(&cookie),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // `\` を含む artifact_id
        let resp = get(
            &router,
            "/api/worktrees/wt-c/artifacts/foo%5Cbar",
            Some(&cookie),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // settings に無いワークツリー id
        let resp = get(&router, "/api/worktrees/unknown/artifacts", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // settings に無いリポジトリ key
        let resp = get(&router, "/api/repositories/unknown/artifacts", Some(&cookie)).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ─── #340: memory 書き込み ──────────────────────────────────────────────

    fn worktree_router_with(
        data_dir: PathBuf,
        wt: WorktreeEntry,
        tool_caller: ToolCaller,
        notify: StateChangedNotifier,
    ) -> Router {
        let settings: SettingsProvider = Arc::new(move || (vec![wt.clone()], vec![]));
        build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            data_dir,
            settings,
            TEST_PORT,
            tool_caller,
            notify,
            test_events(),
            test_shutdown(),
        )
    }

    #[tokio::test]
    async fn set_worktree_memory_roundtrips_and_notifies() {
        let data_dir = temp_data_dir("set-memory-worktree");
        let wt = test_worktree("wt-mem");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);

        let notify_calls = Arc::new(std::sync::Mutex::new(vec![]));
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            recording_state_notifier(notify_calls.clone()),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-mem/artifacts/art-1/memory",
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "memory": { "foo": "bar" } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        let updated_at = body["memoryUpdatedAt"].as_u64().unwrap();
        assert!(updated_at > 0);
        assert_eq!(
            notify_calls.lock().unwrap().as_slice(),
            [("worktree".to_string(), "wt-mem".to_string(), "art-1".to_string())]
        );

        // read で読み直せる
        let resp = get(&router, "/api/worktrees/wt-mem/artifacts/art-1", Some(&cookie)).await;
        let body = json_body(resp).await;
        assert_eq!(body["memory"]["foo"], "bar");
        assert_eq!(body["memoryUpdatedAt"], updated_at);

        // null で削除
        let resp = post_json(
            &router,
            "/api/worktrees/wt-mem/artifacts/art-1/memory",
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "memory": null }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = get(&router, "/api/worktrees/wt-mem/artifacts/art-1", Some(&cookie)).await;
        let body = json_body(resp).await;
        assert!(body["memory"].is_null());
    }

    #[tokio::test]
    async fn set_worktree_memory_rejects_non_object() {
        let data_dir = temp_data_dir("set-memory-invalid");
        let wt = test_worktree("wt-mem2");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-mem2/artifacts/art-1/memory",
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "memory": "not-an-object" }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn set_memory_on_orphan_artifact_does_not_create_sidecar() {
        let data_dir = temp_data_dir("set-memory-orphan");
        let wt = test_worktree("wt-mem3");
        // 本体 JSON を作らない(存在しないアーティファクトIDへ書き込む)
        let notify_calls = Arc::new(std::sync::Mutex::new(vec![]));
        let router = worktree_router_with(
            data_dir.clone(),
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            recording_state_notifier(notify_calls.clone()),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-mem3/artifacts/does-not-exist/memory",
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "memory": { "foo": "bar" } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        assert_eq!(body["memoryUpdatedAt"], 0);
        assert!(notify_calls.lock().unwrap().is_empty(), "孤児書き込みでは通知しない");
        assert!(!data_dir
            .join("artifacts")
            .join("wt-mem3")
            .join("does-not-exist.state")
            .exists());
    }

    #[tokio::test]
    async fn set_repo_memory_roundtrips() {
        let data_dir = temp_data_dir("set-memory-repo");
        let repo = test_repository("D:/git/set-memory-repo");
        let repo_key = repo_artifacts_key(&repo.id);
        write_test_artifact(&data_dir.join("repo-artifacts").join(&repo_key), "art-1", 100);

        let repo_for_settings = repo.clone();
        let settings: SettingsProvider = Arc::new(move || (vec![], vec![repo_for_settings.clone()]));
        let notify_calls = Arc::new(std::sync::Mutex::new(vec![]));
        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            data_dir,
            settings,
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            recording_state_notifier(notify_calls.clone()),
            test_events(),
            test_shutdown(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            &format!("/api/repositories/{}/artifacts/art-1/memory", repo_key),
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "memory": { "foo": "bar" } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            notify_calls.lock().unwrap().as_slice(),
            [("repository".to_string(), repo.id.clone(), "art-1".to_string())]
        );
    }

    // ─── #340: callTool ブリッジ ────────────────────────────────────────────

    #[tokio::test]
    async fn call_tool_worktree_forwards_to_tool_caller() {
        let data_dir = temp_data_dir("call-tool-worktree");
        let wt = test_worktree("wt-tool");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);

        let calls = Arc::new(std::sync::Mutex::new(vec![]));
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(calls.clone(), Ok("tool-result".to_string())),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-tool/artifacts/art-1/call-tool",
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "tool": "oretachi_get_worktree_status", "params": { "query": "x" } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        assert_eq!(body["result"], "tool-result");

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "wt-tool");
        assert_eq!(recorded[0].1, "art-1");
        assert_eq!(recorded[0].2, "oretachi_get_worktree_status");
        assert_eq!(recorded[0].3, json!({ "query": "x" }));
    }

    #[tokio::test]
    async fn call_tool_worktree_propagates_error() {
        let data_dir = temp_data_dir("call-tool-error");
        let wt = test_worktree("wt-tool2");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(
                Arc::new(std::sync::Mutex::new(vec![])),
                Err("ホワイトリストにないツールです".to_string()),
            ),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-tool2/artifacts/art-1/call-tool",
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "tool": "not_whitelisted", "params": {} }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], "ホワイトリストにないツールです");
    }

    #[tokio::test]
    async fn call_tool_repository_is_always_rejected() {
        let data_dir = temp_data_dir("call-tool-repo");
        let repo = test_repository("D:/git/call-tool-repo");
        let repo_key = repo_artifacts_key(&repo.id);
        write_test_artifact(&data_dir.join("repo-artifacts").join(&repo_key), "art-1", 100);

        let repo_for_settings = repo.clone();
        let settings: SettingsProvider = Arc::new(move || (vec![], vec![repo_for_settings.clone()]));
        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            data_dir,
            settings,
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            test_shutdown(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            &format!("/api/repositories/{}/artifacts/art-1/call-tool", repo_key),
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "tool": "anything", "params": {} }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ─── #340: CSRF ガード ──────────────────────────────────────────────────

    #[tokio::test]
    async fn csrf_guard_rejects_missing_origin() {
        let data_dir = temp_data_dir("csrf-no-origin");
        let wt = test_worktree("wt-csrf1");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-csrf1/artifacts/art-1/memory",
            Some(&cookie),
            None,
            Some(&test_host()),
            json!({ "memory": { "a": 1 } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn csrf_guard_rejects_mismatched_origin_port() {
        let data_dir = temp_data_dir("csrf-wrong-port");
        let wt = test_worktree("wt-csrf2");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        // vite dev サーバ等、別ポートの localhost からの Origin
        let resp = post_json(
            &router,
            "/api/worktrees/wt-csrf2/artifacts/art-1/memory",
            Some(&cookie),
            Some("http://localhost:1420"),
            Some(&test_host()),
            json!({ "memory": { "a": 1 } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn csrf_guard_rejects_mismatched_host() {
        let data_dir = temp_data_dir("csrf-wrong-host");
        let wt = test_worktree("wt-csrf3");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-csrf3/artifacts/art-1/memory",
            Some(&cookie),
            Some(&test_origin()),
            Some(&format!("evil.example:{}", TEST_PORT)),
            json!({ "memory": { "a": 1 } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn csrf_guard_rejects_cross_site_sec_fetch_site() {
        let data_dir = temp_data_dir("csrf-cross-site");
        let wt = test_worktree("wt-csrf4");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        let req = HttpRequest::builder()
            .uri("/api/worktrees/wt-csrf4/artifacts/art-1/memory")
            .method("POST")
            .header(header::COOKIE, &cookie)
            .header(header::ORIGIN, test_origin())
            .header(header::HOST, test_host())
            .header("sec-fetch-site", "cross-site")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&json!({ "memory": { "a": 1 } })).unwrap()))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn csrf_guard_allows_matching_origin_and_host() {
        let data_dir = temp_data_dir("csrf-ok");
        let wt = test_worktree("wt-csrf5");
        write_test_artifact(&data_dir.join("artifacts").join(&wt.id), "art-1", 100);
        let router = worktree_router_with(
            data_dir,
            wt,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
        );
        let cookie = cookie_for(TEST_KEY);

        let resp = post_json(
            &router,
            "/api/worktrees/wt-csrf5/artifacts/art-1/memory",
            Some(&cookie),
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "memory": { "a": 1 } }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn csrf_guard_is_bypassed_for_get_but_not_post_without_cookie() {
        let router = router_for_test();
        // Cookie が無ければ csrf_guard より外側の viewer_auth で 401 になる
        // (POST でも Origin 検証の前に弾かれる)。
        let resp = post_json(
            &router,
            "/api/worktrees/unknown/artifacts/art-1/memory",
            None,
            Some(&test_origin()),
            Some(&test_host()),
            json!({ "memory": {} }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ─── #341: SSE (/api/events) ────────────────────────────────────────────

    #[tokio::test]
    async fn events_requires_cookie() {
        let router = router_for_test();
        let resp = get(&router, "/api/events", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn events_streams_broadcast_payload_as_sse_data() {
        let events_tx = test_events();
        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            temp_data_dir("events-basic"),
            empty_settings(),
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            events_tx.clone(),
            test_shutdown(),
        );
        let cookie = cookie_for(TEST_KEY);
        let req = HttpRequest::builder()
            .uri("/api/events")
            .method("GET")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap().to_str().unwrap(),
            "no-store"
        );

        let mut stream = resp.into_body().into_data_stream();

        // ハンドラは `subscribe()` してから Response を返すので、この時点で
        // 送信すれば取りこぼさない(ストリームをまだ poll していなくても届く)。
        events_tx
            .send(ViewerEvent::artifact_changed_worktree(
                "wt-1".to_string(),
                "art-1".to_string(),
                "create".to_string(),
                true,
            ))
            .unwrap();

        let chunk = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("SSE chunk が届かない")
            .expect("ストリームが予期せず終了した")
            .unwrap();
        let text = String::from_utf8(chunk.to_vec()).unwrap();
        assert!(text.contains("data:"));
        assert!(text.contains("\"type\":\"artifact-changed\""));
        assert!(text.contains("\"scopeId\":\"wt-1\""));
        assert!(text.contains("\"artifactId\":\"art-1\""));
    }

    #[tokio::test]
    async fn events_stream_ends_on_shutdown() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let router = build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            temp_data_dir("events-shutdown"),
            empty_settings(),
            TEST_PORT,
            stub_tool_caller(Arc::new(std::sync::Mutex::new(vec![])), Ok("ok".to_string())),
            noop_state_notifier(),
            test_events(),
            shutdown_rx,
        );
        let cookie = cookie_for(TEST_KEY);
        let req = HttpRequest::builder()
            .uri("/api/events")
            .method("GET")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        let mut stream = resp.into_body().into_data_stream();

        shutdown_tx.send(true).unwrap();

        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("shutdown を送っても SSE ストリームが終わらない");
        assert!(next.is_none(), "shutdown 後はストリームが終わるはず");
    }
}
