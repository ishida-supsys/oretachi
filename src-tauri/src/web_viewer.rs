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
// #337 (JSON API) 以降はこのルータへ追加する形で実装する。
//
// セルフレビューで挙がった申し送り事項(#336 の範囲では対応不要、後続 sub-issue で要考慮):
// - このルータは今のところ GET のみ・副作用なしなので CSRF は問題にならないが、
//   Cookie は `SameSite=Strict` のみで守られている。RFC 6265 の Cookie 分離も
//   schemeful-same-site 判定もポート単位ではないため、同一ホストの別ポートで動く
//   別インスタンス/別ローカルサーバーから見ると同一サイト scoped になりうる。
//   #337 以降で書き込み系(store・callTool ブリッジ)を足すときは、
//   SameSite=Strict だけに頼らず Origin/Sec-Fetch-Site 検証か CSRF トークンを併用すること。
// - `remote_access`(0.0.0.0 bind)時は TLS 終端が無いため `?token=` の URL も
//   発行後の Cookie も LAN 上を平文で流れる(Bearer 経路と同じ既知のリスクだが、
//   ブラウザへ直接貼る導線が増える分、履歴・オートコンプリートに残る経路が広がる)。
//   #341 の「ブラウザで開く」導線で remote_access 時の注意喚起を検討すること。
//   `Secure` 属性はこのアプリが HTTPS 非対応なため付けられない(付けると
//   localhost の通常利用まで Cookie が送られなくなる)。

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    middleware::{self, Next},
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::get,
    Router,
};
use hmac::{Hmac, Mac};
use serde::Serialize;
use serde_json::json;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Manager};

use crate::settings::{Repository, SettingsManager, WorktreeEntry};
use crate::{
    artifacts_dir_in, list_artifacts_in_dir, list_repo_artifacts_in_dir, read_artifact_store,
    repo_artifacts_dir_in, repo_artifacts_key, validate_path_component,
};

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

/// テスト・実装から共通で使うルータ組み立て。
fn build_router(
    key: KeyProvider,
    cookie_name: String,
    assets: AssetSource,
    dev_proxy: DevProxyTarget,
    data_dir: PathBuf,
    settings: SettingsProvider,
) -> Router {
    let state = ViewerState {
        key: key.clone(),
        assets,
        dev_proxy: dev_proxy.clone(),
        cookie_name,
        data_dir,
        settings,
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
        .fallback(get(static_fallback))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, viewer_auth))
}

/// 本番用: Bearer layer の外で merge する閲覧ルータを組み立てる。
///
/// `port` は bind 後の実ポート(0 指定時も実際の値)を渡すこと。Cookie 名に含めることで、
/// 同一ホストで動く別インスタンス(本番/dev)の Cookie を混同しないようにする。
pub fn router(app_handle: AppHandle, port: u16) -> Router {
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

    build_router(key, cookie_name, assets, dev_proxy, data_dir, settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request as HttpRequest;
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

    fn router_for_test() -> Router {
        build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
            temp_data_dir("basic"),
            empty_settings(),
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
}
