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

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Manager};

use crate::settings::SettingsManager;

/// 現在の MCP API key を返す。設定は再起動なしで変わりうるため、毎リクエスト読む。
pub type KeyProvider = Arc<dyn Fn() -> String + Send + Sync>;

/// 埋め込み(または dev の frontendDist)アセットを `(bytes, mime_type)` で返す。
/// 見つからなければ `None`。
pub type AssetSource = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;

/// dev モードでの devUrl (例: `http://localhost:1420`)。`None` ならプロキシしない
/// (release ビルド、または dist が既に存在する dev)。
pub type DevProxyTarget = Option<String>;

const COOKIE_HMAC_CONTEXT: &[u8] = b"oretachi-web-viewer-session-v1";

#[derive(Clone)]
struct ViewerState {
    key: KeyProvider,
    assets: AssetSource,
    dev_proxy: Option<String>,
    cookie_name: String,
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

/// 閲覧ルータ全体にかける認証 middleware。
async fn viewer_auth(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    uri: Uri,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let key = (state.key)();
    if key.is_empty() {
        log::warn!("[web_viewer] API key not configured, rejecting all requests");
        return Err(StatusCode::UNAUTHORIZED);
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
            return Err(StatusCode::UNAUTHORIZED);
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
        return Ok(response);
    }

    // token が無ければ Cookie を見る。
    let cookie_value = find_cookie(&headers, &state.cookie_name);
    let expected = session_cookie_value(&key);
    let authorized = match cookie_value {
        Some(v) => constant_time_eq_str(v, &expected),
        None => false,
    };

    if authorized {
        Ok(next.run(request).await)
    } else {
        log::warn!("[web_viewer] unauthorized request: no valid session cookie");
        Err(StatusCode::UNAUTHORIZED)
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

async fn serve_index(State(state): State<ViewerState>) -> Response {
    match (state.assets)("index.html") {
        Some((bytes, mime_type)) => {
            let mut response = asset_response(bytes, mime_type);
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            response
        }
        None => {
            if let Some(target) = &state.dev_proxy {
                return proxy_to_dev(target, "/").await;
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

/// テスト・実装から共通で使うルータ組み立て。
fn build_router(
    key: KeyProvider,
    cookie_name: String,
    assets: AssetSource,
    dev_proxy: DevProxyTarget,
) -> Router {
    let state = ViewerState {
        key: key.clone(),
        assets,
        dev_proxy: dev_proxy.clone(),
        cookie_name,
    };

    // `/assets/*`・`/vendor/*`・ルート直下の静的ファイル(`/vite.svg` 等)は
    // すべて `static_fallback` が処理する。dev モードでは dist に無いパス
    // (`/src/*`、`/@vite/*` 等)を devUrl (vite) へプロキシするフォールバックも兼ねる。
    Router::new()
        .route("/", get(root_redirect))
        .route("/worktrees", get(spa_shell))
        .route("/worktrees/{*rest}", get(spa_shell))
        .route("/repositories/{*rest}", get(spa_shell))
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

    build_router(key, cookie_name, assets, dev_proxy)
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
            "index.html" => Some((b"<html>index</html>".to_vec(), "text/html".to_string())),
            "assets/app.js" => Some((b"console.log(1)".to_vec(), "text/javascript".to_string())),
            _ => None,
        })
    }

    fn router_for_test() -> Router {
        build_router(
            key_provider(TEST_KEY),
            "oretachi_viewer_test".to_string(),
            stub_assets(),
            None,
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
}
