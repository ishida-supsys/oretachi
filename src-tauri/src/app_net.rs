//! アプリ形式アーティファクトのネットワーク API（Rust 側の HTTP プロキシ）。
//!
//! iframe の CSP（`connect-src 'none'`）は変えず、アプリの通信はここを経由する。関門は 1 箇所で、
//! `app_net_fetch` が **承認済みの URL プレフィックス（`AppGrantStore` の `resolved.network`）だけ**を
//! 見て通す。アーティファクト JSON 内の manifest は見ない。
//!
//! # 検査（すべて Rust 側）
//!
//! - URL を承認済みプレフィックスと照合する（scheme / host / port 完全一致 + path の前方一致）。
//! - リダイレクトのたびに同じ検査をやり直す（最大 [`MAX_REDIRECTS`] 回）。
//! - ループバック・プライベート IP・oretachi 自身の axum ポート（MCP と Web ビューアは同一リスナー）
//!   を拒否する。許すとネットワーク権限が oretachi の HTTP API への到達権限になる。
//!   **v1 に解除オプションはない**（manifest からも ResolvedNetwork からも緩められない）。
//! - 接続先 IP は名前解決の**後**で検査する（DNS リバインディング対策）。[`GuardedResolver`] が
//!   解決結果に禁止 IP が 1 つでも混じれば拒否し、reqwest はその検査済みアドレスへ接続する。
//!   IP リテラルの URL は resolver を通らないので URL の段階で検査する。
//! - システムプロキシは使わない（プロキシ越しだと接続先 IP の検査が意味を失うため）。
//! - メソッド・ヘッダー・本文サイズ・タイムアウト・同時実行数に上限を設ける。
//!
//! 拒否・失敗は、アプリのデバッグログとログファイルへ残す。

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::future::{AbortHandle, AbortRegistration, Abortable};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

// ─── 上限 ────────────────────────────────────────────────────────────

pub const MAX_PREFIXES: usize = 20;
const MAX_REDIRECTS: usize = 5;
const MAX_URL_CHARS: usize = 4096;
const MAX_REQUEST_HEADERS: usize = 32;
const MAX_HEADER_VALUE_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_HEADERS: usize = 100;
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;
/// 全体のタイムアウト。iframe ブリッジの長い待ち（30 秒）より短くしておく
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONCURRENT_PER_INSTANCE: usize = 8;
const LOG_URL_CHARS: usize = 200;

const ALLOWED_METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
/// アプリから指定できないヘッダー（接続・フレーミング・プロキシ制御に関わるもの）
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "connection",
    "content-length",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "keep-alive",
    "expect",
];

// ─── プレフィックスの正規化（manifest） ────────────────────────────────

/// path に含めてはいけない（サーバー側でデコードされると prefix を抜けうる）エスケープ。
fn path_has_ambiguous_escape(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") || path.contains('\\')
}

fn host_is_localhost_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    h == "localhost" || h.ends_with(".localhost")
}

/// manifest の `network.allow` の 1 要素を検査して正規化する（`Url::as_str` の形）。
/// 承認ダイアログへ行く前に、使えないと分かっている申請は manifest 不正として返す。
pub fn normalize_prefix(raw: &str) -> Result<String, String> {
    if raw.chars().count() > MAX_URL_CHARS {
        return Err(format!("URL プレフィックスが長すぎます（{} 文字まで）", MAX_URL_CHARS));
    }
    let url = reqwest::Url::parse(raw)
        .map_err(|e| format!("URL プレフィックス '{}' を解釈できません: {}", raw, e))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("URL プレフィックス '{}' は http / https だけ指定できます", raw));
    }
    let Some(host) = url.host() else {
        return Err(format!("URL プレフィックス '{}' にホストがありません", raw));
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("URL プレフィックス '{}' に認証情報（user:pass@）は書けません", raw));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("URL プレフィックス '{}' に ? / # は書けません（ホストとパスだけ）", raw));
    }
    if path_has_ambiguous_escape(url.path()) {
        return Err(format!("URL プレフィックス '{}' のパスに %2e / %2f / %5c / \\ は使えません", raw));
    }
    match host {
        url::Host::Domain(d) => {
            if host_is_localhost_name(d) {
                return Err(format!(
                    "URL プレフィックス '{}' はループバックです。ネットワーク権限では oretachi 自身や同じ PC のサービスへは届きません",
                    raw
                ));
            }
        }
        url::Host::Ipv4(ip) => reject_literal_ip(raw, IpAddr::V4(ip))?,
        url::Host::Ipv6(ip) => reject_literal_ip(raw, IpAddr::V6(ip))?,
    }
    Ok(url.as_str().to_string())
}

fn reject_literal_ip(raw: &str, ip: IpAddr) -> Result<(), String> {
    if ip_is_forbidden(ip) {
        return Err(format!(
            "URL プレフィックス '{}' はループバック／プライベート／予約済みアドレスです。ネットワーク権限では届きません",
            raw
        ));
    }
    Ok(())
}

// ─── IP の検査 ───────────────────────────────────────────────────────

fn v4_is_forbidden(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || o[0] == 0 // 0.0.0.0/8
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 (CGNAT)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24 (IETF)
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15 (benchmark)
        || o[0] >= 240 // 240.0.0.0/4 (reserved)
}

/// IPv6 に埋め込まれた IPv4（mapped / compat / NAT64 / 6to4）を取り出す。
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let v4 = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    // ::ffff:a.b.c.d（mapped）
    if s[..5] == [0, 0, 0, 0, 0] && s[5] == 0xffff {
        return Some(v4(s[6], s[7]));
    }
    // ::ffff:0:a.b.c.d（IPv4-translated, ::ffff:0:0/96）
    if s[..4] == [0, 0, 0, 0] && s[4] == 0xffff && s[5] == 0 {
        return Some(v4(s[6], s[7]));
    }
    // ::a.b.c.d（compat。::1 と :: は別途 v6 側で弾く）
    if s[..6] == [0, 0, 0, 0, 0, 0] {
        return Some(v4(s[6], s[7]));
    }
    // 64:ff9b::/96（NAT64）
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return Some(v4(s[6], s[7]));
    }
    // 2002::/16（6to4）。埋め込みは s[1], s[2]
    if s[0] == 0x2002 {
        return Some(v4(s[1], s[2]));
    }
    None
}

fn v6_is_forbidden(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    let s = ip.segments();
    if (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || (s[0] & 0xffc0) == 0xfec0 {
        return true; // ULA fc00::/7, link-local fe80::/10, site-local fec0::/10
    }
    // discard 100::/64, Teredo 2001::/32（トンネル経由で内側の v4 に化けうる）
    if (s[0] == 0x100 && s[1..4] == [0, 0, 0]) || (s[0] == 0x2001 && s[1] == 0) {
        return true;
    }
    // 64:ff9b:1::/48（ローカル用 NAT64）は内包アドレスが分からないので一律拒否
    if s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1 {
        return true;
    }
    if let Some(v4) = embedded_v4(ip) {
        return v4_is_forbidden(v4);
    }
    false
}

/// 接続してはいけないアドレスか。
pub fn ip_is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v4_is_forbidden(v),
        IpAddr::V6(v) => v6_is_forbidden(v),
    }
}

// ─── ポリシー（URL の照合） ─────────────────────────────────────────────

/// 1 リクエストぶんの検査ルール。**manifest の値からは作れる範囲が決まっている**（緩める口はない）。
#[derive(Clone)]
pub struct Policy {
    prefixes: Vec<reqwest::Url>,
    /// oretachi 自身が listen しているポート。host を問わず拒否する（別名・別 IP での到達を塞ぐ）
    own_ports: Vec<u16>,
    /// テストだけがループバックの許可を足せる。本番ビルドにはこのフィールド自体が無い
    #[cfg(test)]
    allow_private: bool,
}

impl Policy {
    pub fn new(prefixes: &[String], own_ports: Vec<u16>) -> Self {
        Policy {
            // 保存済みの値はパースできるはずだが、できないものは捨てる（＝何も許さない側）
            prefixes: prefixes.iter().filter_map(|p| reqwest::Url::parse(p).ok()).collect(),
            own_ports,
            #[cfg(test)]
            allow_private: false,
        }
    }

    fn ip_blocked(&self, ip: IpAddr) -> bool {
        #[cfg(test)]
        if self.allow_private {
            return false;
        }
        ip_is_forbidden(ip)
    }

    /// 初回の URL と、リダイレクトの各ホップで呼ぶ。
    pub fn check_url(&self, url: &reqwest::Url) -> Result<(), String> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("http / https 以外のスキームは使えません: {}", url.scheme()));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("URL に認証情報（user:pass@）は含められません".to_string());
        }
        if path_has_ambiguous_escape(url.path()) {
            return Err("URL のパスに %2e / %2f / %5c / \\ は使えません".to_string());
        }
        if !self.prefixes.iter().any(|p| prefix_matches(p, url)) {
            return Err("承認された URL プレフィックスに含まれていません".to_string());
        }
        if let Some(port) = url.port_or_known_default() {
            if self.own_ports.contains(&port) {
                return Err(format!("oretachi 自身が使っているポート（{}）へは接続できません", port));
            }
        }
        match url.host() {
            Some(url::Host::Ipv4(ip)) if self.ip_blocked(IpAddr::V4(ip)) => {
                Err(format!("ループバック／プライベート／予約済みアドレス（{}）へは接続できません", ip))
            }
            Some(url::Host::Ipv6(ip)) if self.ip_blocked(IpAddr::V6(ip)) => {
                Err(format!("ループバック／プライベート／予約済みアドレス（{}）へは接続できません", ip))
            }
            Some(url::Host::Domain(d)) if host_is_localhost_name(d) && !self.localhost_allowed() => {
                Err(format!("ループバックの名前（{}）へは接続できません", d))
            }
            Some(_) => Ok(()),
            None => Err("URL にホストがありません".to_string()),
        }
    }

    fn localhost_allowed(&self) -> bool {
        #[cfg(test)]
        {
            self.allow_private
        }
        #[cfg(not(test))]
        {
            false
        }
    }
}

/// scheme / host / port が完全一致し、path が前方一致するか。
/// プレフィックスの path が `/` で終わらないときは、境界（`/` か末尾）を要求する
/// （`/v1` が `/v1beta` に当たらないように）。
fn prefix_matches(prefix: &reqwest::Url, url: &reqwest::Url) -> bool {
    if prefix.scheme() != url.scheme()
        || prefix.host_str() != url.host_str()
        || prefix.port_or_known_default() != url.port_or_known_default()
    {
        return false;
    }
    let (p, u) = (prefix.path(), url.path());
    if p.ends_with('/') {
        u.starts_with(p)
    } else {
        u == p || (u.starts_with(p) && u[p.len()..].starts_with('/'))
    }
}

// ─── 名前解決（DNS 後の検査） ───────────────────────────────────────────

type LookupFuture = Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send>>;

/// 名前 → IP。本番は OS の resolver、テストは差し替えてリバインディングを再現する。
pub trait Lookup: Send + Sync {
    fn lookup(&self, host: String) -> LookupFuture;
}

pub struct SystemLookup;

impl Lookup for SystemLookup {
    fn lookup(&self, host: String) -> LookupFuture {
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addrs.map(|a| a.ip()).collect())
        })
    }
}

/// 解決結果に禁止 IP が 1 つでも混じれば、その名前への接続を拒否する resolver。
/// 拒否の理由は `denied` に残す（reqwest のエラーは原因が深く埋もれるため）。
struct GuardedResolver {
    policy: Arc<Policy>,
    lookup: Arc<dyn Lookup>,
    denied: Arc<Mutex<Option<String>>>,
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let policy = self.policy.clone();
        let lookup = self.lookup.clone();
        let denied = self.denied.clone();
        Box::pin(async move {
            let ips = lookup.lookup(host.clone()).await.map_err(|e| {
                Box::<dyn std::error::Error + Send + Sync>::from(format!("名前解決に失敗しました（{}）: {}", host, e))
            })?;
            if ips.is_empty() {
                return Err(format!("名前解決の結果が空です（{}）", host).into());
            }
            if let Some(bad) = ips.iter().find(|ip| policy.ip_blocked(**ip)) {
                let msg = format!("{} はループバック／プライベート／予約済みアドレス（{}）へ解決されるため接続できません", host, bad);
                if let Ok(mut d) = denied.lock() {
                    d.get_or_insert_with(|| msg.clone());
                }
                return Err(msg.into());
            }
            let addrs: Vec<std::net::SocketAddr> = ips.into_iter().map(|ip| std::net::SocketAddr::new(ip, 0)).collect();
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

// ─── リクエスト / レスポンス ────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FetchInit {
    pub method: Option<String>,
    /// `[[name, value], ...]`（同名ヘッダーを重ねられる）
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    /// "utf8"（既定）| "base64"
    pub body_encoding: Option<String>,
    /// "text"（既定）| "base64"
    pub response_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchResult {
    pub status: u16,
    pub status_text: String,
    /// リダイレクトを辿った後の最終 URL
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// "text" | "base64"
    pub body_encoding: String,
}

/// 検査済みの要求から実際に通信する。コマンドとテストの両方がここを通る。
pub async fn perform(
    policy: Arc<Policy>,
    lookup: Arc<dyn Lookup>,
    url: &str,
    init: FetchInit,
) -> Result<FetchResult, String> {
    if url.chars().count() > MAX_URL_CHARS {
        return Err(format!("URL が長すぎます（{} 文字まで）", MAX_URL_CHARS));
    }
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("URL を解釈できません: {}", e))?;
    policy.check_url(&parsed)?;

    let method_name = init.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
    if !ALLOWED_METHODS.contains(&method_name.as_str()) {
        return Err(format!("メソッド '{}' は使えません（{}）", method_name, ALLOWED_METHODS.join(" / ")));
    }
    let method = reqwest::Method::from_bytes(method_name.as_bytes()).map_err(|e| e.to_string())?;

    if init.headers.len() > MAX_REQUEST_HEADERS {
        return Err(format!("ヘッダーは {} 個までです", MAX_REQUEST_HEADERS));
    }
    let mut headers = reqwest::header::HeaderMap::new();
    for (k, v) in &init.headers {
        let lower = k.to_ascii_lowercase();
        if FORBIDDEN_HEADERS.contains(&lower.as_str()) || lower.starts_with("proxy-") {
            return Err(format!("ヘッダー '{}' は指定できません", k));
        }
        if v.len() > MAX_HEADER_VALUE_BYTES {
            return Err(format!("ヘッダー '{}' の値が長すぎます（{} バイトまで）", k, MAX_HEADER_VALUE_BYTES));
        }
        let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
            .map_err(|_| format!("ヘッダー名 '{}' が不正です", k))?;
        let value = reqwest::header::HeaderValue::from_str(v)
            .map_err(|_| format!("ヘッダー '{}' の値が不正です", k))?;
        headers.append(name, value);
    }

    let body: Option<Vec<u8>> = match init.body {
        None => None,
        Some(b) => {
            if matches!(method_name.as_str(), "GET" | "HEAD") {
                return Err(format!("{} に本文は付けられません", method_name));
            }
            let bytes = match init.body_encoding.as_deref().unwrap_or("utf8") {
                "utf8" => b.into_bytes(),
                "base64" => base64::engine::general_purpose::STANDARD
                    .decode(b.as_bytes())
                    .map_err(|e| format!("body が base64 として不正です: {}", e))?,
                other => return Err(format!("bodyEncoding '{}' は使えません（utf8 / base64）", other)),
            };
            if bytes.len() > MAX_REQUEST_BODY_BYTES {
                return Err(format!(
                    "リクエスト本文が大きすぎます: {} バイト（上限 {}）",
                    bytes.len(),
                    MAX_REQUEST_BODY_BYTES
                ));
            }
            Some(bytes)
        }
    };
    let as_base64 = match init.response_type.as_deref().unwrap_or("text") {
        "text" => false,
        "base64" => true,
        other => return Err(format!("responseType '{}' は使えません（text / base64）", other)),
    };

    let denied: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let redirect_policy = {
        let policy = policy.clone();
        let denied = denied.clone();
        let origin = parsed.clone();
        reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error(format!("リダイレクトが {} 回を超えました", MAX_REDIRECTS));
            }
            // 別オリジンへのリダイレクトは、承認済みのプレフィックスであっても追わない。reqwest が
            // 落とすのは Authorization / Cookie 等だけで、アプリが付けた x-api-key などの
            // ヘッダーや本文（307/308）が別のホストへ渡ってしまうため
            let result = policy.check_url(attempt.url()).and_then(|()| check_redirect_origin(&origin, attempt.url()));
            match result {
                Ok(()) => attempt.follow(),
                Err(e) => {
                    let msg = format!("リダイレクト先 {} は許可されていません: {}", loggable_url(attempt.url().as_str()), e);
                    if let Ok(mut d) = denied.lock() {
                        d.get_or_insert_with(|| msg.clone());
                    }
                    attempt.error(msg)
                }
            }
        })
    };
    ensure_crypto_provider();
    let client = reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(GuardedResolver { policy: policy.clone(), lookup, denied: denied.clone() })
        .redirect(redirect_policy)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .user_agent(concat!("oretachi-app/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("HTTP クライアントを作れませんでした: {}", e))?;

    let mut req = client.request(method, parsed).headers(headers);
    if let Some(b) = body {
        req = req.body(b);
    }
    let take_denied = || denied.lock().ok().and_then(|mut d| d.take());
    let mut resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Err(take_denied().unwrap_or_else(|| describe_error(&e))),
    };

    if let Some(len) = resp.content_length() {
        if len as usize > MAX_RESPONSE_BODY_BYTES {
            return Err(format!("レスポンスが大きすぎます: {} バイト（上限 {}）", len, MAX_RESPONSE_BODY_BYTES));
        }
    }
    let status = resp.status();
    let final_url = resp.url().to_string();
    let resp_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .take(MAX_RESPONSE_HEADERS)
        .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();

    let mut buf: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len() + chunk.len() > MAX_RESPONSE_BODY_BYTES {
                    return Err(format!("レスポンスが上限（{} バイト）を超えました", MAX_RESPONSE_BODY_BYTES));
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(take_denied().unwrap_or_else(|| describe_error(&e))),
        }
    }

    let (body, body_encoding) = if as_base64 {
        (base64::engine::general_purpose::STANDARD.encode(&buf), "base64")
    } else {
        (String::from_utf8_lossy(&buf).into_owned(), "text")
    };
    Ok(FetchResult {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        url: final_url,
        headers: resp_headers,
        body,
        body_encoding: body_encoding.to_string(),
    })
}

/// リダイレクト先が元のリクエストと同じオリジンか（http → https への格上げだけは、同じホストなら許す）。
fn check_redirect_origin(origin: &reqwest::Url, next: &reqwest::Url) -> Result<(), String> {
    let same_origin = origin.scheme() == next.scheme()
        && origin.host_str() == next.host_str()
        && origin.port_or_known_default() == next.port_or_known_default();
    let https_upgrade = origin.scheme() == "http" && next.scheme() == "https" && origin.host_str() == next.host_str();
    if same_origin || https_upgrade {
        Ok(())
    } else {
        Err("別のオリジン（スキーム・ホスト・ポート）へのリダイレクトは追いません".to_string())
    }
}

/// reqwest は `rustls-no-provider` で使っているので、暗号プロバイダ（ring）が要る。起動時（`lib.rs`）にも
/// 入れるが、クライアントを作る直前にも確かめる（入っていなければクライアントの構築が panic する）。
/// すでに入っていれば `Err` を返すだけなので無視してよい。
pub fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

fn describe_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return format!("タイムアウトしました（{} 秒）", TOTAL_TIMEOUT.as_secs());
    }
    // 原因の連鎖をたどって、いちばん具体的なメッセージを出す
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        msg = format!("{}: {}", msg, s);
        src = s.source();
    }
    format!("通信に失敗しました: {}", msg)
}

// ─── 実行中リクエストの台帳（再読み込み・停止での中断） ─────────────────────

#[derive(Default)]
struct Inner {
    next_id: u64,
    /// ウィンドウラベル → リクエスト ID → 中断ハンドル
    flights: HashMap<String, HashMap<u64, AbortHandle>>,
    /// 停止したインスタンスのラベル（ラベルは再利用されない）
    dead: HashSet<String>,
}

#[derive(Default)]
pub struct AppNetRegistry {
    inner: Mutex<Inner>,
}

impl AppNetRegistry {
    fn begin(&self, label: &str) -> Result<(u64, AbortRegistration), String> {
        let Ok(mut inner) = self.inner.lock() else {
            return Err("通信の台帳のロックが壊れています".to_string());
        };
        if inner.dead.contains(label) {
            return Err("アプリが停止しました".to_string());
        }
        if inner.flights.get(label).map(|m| m.len()).unwrap_or(0) >= MAX_CONCURRENT_PER_INSTANCE {
            return Err(format!("同時に出せる通信は {} 本までです", MAX_CONCURRENT_PER_INSTANCE));
        }
        let id = inner.next_id;
        inner.next_id += 1;
        let (handle, reg) = AbortHandle::new_pair();
        inner.flights.entry(label.to_string()).or_default().insert(id, handle);
        Ok((id, reg))
    }

    fn end(&self, label: &str, id: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(m) = inner.flights.get_mut(label) {
                m.remove(&id);
                if m.is_empty() {
                    inner.flights.remove(label);
                }
            }
        }
    }

    fn abort_all(&self, label: &str, stopped: bool) {
        let Ok(mut inner) = self.inner.lock() else { return };
        if stopped {
            inner.dead.insert(label.to_string());
        }
        if let Some(m) = inner.flights.remove(label) {
            for h in m.into_values() {
                h.abort();
            }
        }
    }
}

/// インスタンスの停止時（`app_instance::release`）。実行中の通信を中断し、以後は出せないようにする。
pub fn on_instance_released(app: &AppHandle, label: &str) {
    app.state::<AppNetRegistry>().abort_all(label, true);
}

/// ホットリロード・権限セットの入れ替え時。実行中の通信は中断する（新しい iframe が再接続する）。
pub fn on_reload(app: &AppHandle, label: &str) {
    app.state::<AppNetRegistry>().abort_all(label, false);
}

// ─── Tauri コマンド（appinst-* ウィンドウからだけ） ────────────────────

/// oretachi 自身が listen しているポート。リクエストごとに読む（MCP は再起動でポートが変わりうる）。
fn own_ports(app: &AppHandle) -> Vec<u16> {
    let mut ports = Vec::new();
    if let Some(mgr) = app.try_state::<crate::mcp_server::McpServerManager>() {
        if let Some(p) = mgr.get_status().port {
            ports.push(p);
        }
    }
    if let Some(sm) = app.try_state::<crate::settings::SettingsManager>() {
        let p = sm.get().mcp_port;
        if p != 0 {
            ports.push(p);
        }
    }
    #[cfg(debug_assertions)]
    {
        // dev ビルドの追加リスナー（tauri-plugin-mcp の TCP と vite）
        ports.push(crate::mcp_server::env_port_override("ORETACHI_TAURI_MCP_PORT", 4000));
        ports.extend([1420, 1421]);
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

fn caller_net(
    window: &tauri::Window,
    app: &AppHandle,
) -> Result<(crate::app_instance::InstanceContext, Vec<String>), String> {
    crate::app_instance::require_instance_window(window)?;
    let ctx = crate::app_instance::context_for_label(app, window.label())?;
    let grant = app
        .state::<crate::app_grant::AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .ok_or_else(|| "このアプリには承認がありません".to_string())?;
    let net = grant
        .resolved
        .network
        .ok_or_else(|| "このアプリにはネットワークの権限が承認されていません".to_string())?;
    Ok((ctx, net.allow))
}

/// ログ・エラー文に出す URL。クエリと fragment は落とす（API キーなどの秘密が載りうる）。
/// 長さも切る（リダイレクトの Location は任意長にできる）。
fn loggable_url(url: &str) -> String {
    let shown = match reqwest::Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string()
        }
        // 解釈できないものは、? 以降を落として出す
        Err(_) => url.split(['?', '#']).next().unwrap_or("").to_string(),
    };
    let mut s: String = shown.chars().take(LOG_URL_CHARS).collect();
    if shown.chars().count() > LOG_URL_CHARS {
        s.push('…');
    }
    s.replace(['\r', '\n'], " ")
}

/// 拒否・失敗はデバッグログ（アプリのログとアプリのログファイル）へ残す。成功は残さない。
fn log_failure(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, method: &str, url: &str, error: &str) {
    let note = if ctx.reapproval_pending { "（再承認待ちのため、新しい権限はまだ使えません）" } else { "" };
    let url = loggable_url(url);
    log::debug!("[app-net] 拒否 {} {}/{} {} — {}", method, ctx.worktree_id, ctx.artifact_id, url, error);
    crate::app_instance::add_log(
        app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        "warn",
        format!("net.fetch を拒否/失敗: {} {} — {}{}", method, url, error, note),
    );
}

#[tauri::command]
pub async fn app_net_fetch(
    window: tauri::Window,
    app: AppHandle,
    url: String,
    init: Option<FetchInit>,
) -> Result<FetchResult, String> {
    let init = init.unwrap_or_default();
    let method = init.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
    let (ctx, allow) = caller_net(&window, &app)?;
    let policy = Arc::new(Policy::new(&allow, own_ports(&app)));

    let reg = app.state::<AppNetRegistry>();
    let (id, abort_reg) = reg.begin(window.label()).map_err(|e| {
        log_failure(&app, &ctx, &method, &url, &e);
        e
    })?;
    let fut = perform(policy, Arc::new(SystemLookup), &url, init);
    let result = match Abortable::new(fut, abort_reg).await {
        Ok(r) => r,
        Err(_) => Err("アプリの再読み込みまたは停止で通信を中断しました".to_string()),
    };
    reg.end(window.label(), id);
    if let Err(e) = &result {
        log_failure(&app, &ctx, &method, &url, e);
    }
    result
}

// ─── テスト ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(prefixes: &[&str], own: Vec<u16>) -> Policy {
        let p: Vec<String> = prefixes.iter().map(|s| normalize_prefix(s).unwrap()).collect();
        Policy::new(&p, own)
    }

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    // ── normalize_prefix ──

    #[test]
    fn normalize_prefix_accepts_and_normalizes() {
        assert_eq!(normalize_prefix("https://api.example.com").unwrap(), "https://api.example.com/");
        assert_eq!(normalize_prefix("HTTPS://API.Example.com:443/v1/").unwrap(), "https://api.example.com/v1/");
        assert_eq!(normalize_prefix("http://example.com:8080/a").unwrap(), "http://example.com:8080/a");
        // `%2e%2e` は Url がドットセグメントとして解決するので、書いた場所より広がることはない
        assert_eq!(normalize_prefix("https://example.com/v1/%2e%2e/").unwrap(), "https://example.com/");
    }

    #[test]
    fn normalize_prefix_rejects_bad_inputs() {
        for bad in [
            "ftp://example.com/",
            "example.com/v1",
            "https://user:pw@example.com/",
            "https://user@example.com/",
            "https://example.com/?q=1",
            "https://example.com/#frag",
            "https://example.com/a%2fb",
            "http://localhost/",
            "http://foo.localhost/",
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://169.254.169.254/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            assert!(normalize_prefix(bad).is_err(), "{} should be rejected", bad);
        }
    }

    // ── prefix の照合 ──

    #[test]
    fn only_allowed_prefix_passes() {
        let p = policy(&["https://api.example.com/v1/"], vec![]);
        assert!(p.check_url(&url("https://api.example.com/v1/users")).is_ok());
        assert!(p.check_url(&url("https://api.example.com/v1/")).is_ok());
        // 他ホスト・他パス・他スキーム・他ポート
        assert!(p.check_url(&url("https://other.example.com/v1/users")).is_err());
        assert!(p.check_url(&url("https://api.example.com/v2/users")).is_err());
        assert!(p.check_url(&url("https://api.example.com/v1")).is_err());
        assert!(p.check_url(&url("http://api.example.com/v1/users")).is_err());
        assert!(p.check_url(&url("https://api.example.com:8443/v1/users")).is_err());
        // 前方一致を装ったホスト
        assert!(p.check_url(&url("https://api.example.com.evil.com/v1/users")).is_err());
        assert!(p.check_url(&url("https://evil.com/https://api.example.com/v1/")).is_err());
        assert!(p.check_url(&url("https://api.example.com@evil.com/v1/users")).is_err());
        assert!(p.check_url(&url("https://api.example.com:pw@evil.com/v1/")).is_err());
    }

    #[test]
    fn prefix_without_trailing_slash_requires_boundary() {
        let p = policy(&["https://api.example.com/v1"], vec![]);
        assert!(p.check_url(&url("https://api.example.com/v1")).is_ok());
        assert!(p.check_url(&url("https://api.example.com/v1/x")).is_ok());
        assert!(p.check_url(&url("https://api.example.com/v1beta")).is_err());
        assert!(p.check_url(&url("https://api.example.com/v10/x")).is_err());
    }

    #[test]
    fn default_port_is_equivalent() {
        let p = policy(&["https://api.example.com/"], vec![]);
        assert!(p.check_url(&url("https://api.example.com:443/x")).is_ok());
        assert!(p.check_url(&url("https://api.example.com:444/x")).is_err());
    }

    #[test]
    fn traversal_cannot_leave_the_prefix() {
        let p = policy(&["https://api.example.com/v1/"], vec![]);
        // `..` は Url が解決するので、プレフィックスの外になる
        assert!(p.check_url(&url("https://api.example.com/v1/../admin")).is_err());
        // エンコードされた区切り・ドットは、サーバー側でデコードされると抜けうるので拒否
        assert!(p.check_url(&url("https://api.example.com/v1/%2e%2e/admin")).is_err());
        assert!(p.check_url(&url("https://api.example.com/v1/..%2fadmin")).is_err());
        assert!(p.check_url(&url("https://api.example.com/v1/a%5cb")).is_err());
    }

    #[test]
    fn own_port_is_rejected_for_any_host() {
        // ポートは host を問わず拒否する（別名・DNS で自分へ戻ってくる経路を塞ぐ）
        let p = policy(&["https://public.example.com:7777/"], vec![7777]);
        assert!(p.check_url(&url("https://public.example.com:7777/")).is_err());
        // 既定ポートでも、自ポートと一致すれば拒否（port_or_known_default）
        let p = policy(&["https://public.example.com/"], vec![443]);
        assert!(p.check_url(&url("https://public.example.com/")).is_err());
        let p = policy(&["https://public.example.com/"], vec![9999]);
        assert!(p.check_url(&url("https://public.example.com/")).is_ok());
    }

    #[test]
    fn literal_private_and_loopback_urls_are_rejected() {
        // プレフィックスに書いてあっても（manifest を通らない保存値でも）、URL の段階で拒否する
        let raw = |s: &str| Policy::new(&[s.to_string()], vec![]);
        for u in [
            "http://127.0.0.1:8080/",
            "http://[::1]:8080/",
            "http://10.1.2.3/",
            "http://172.16.0.1/",
            "http://192.168.0.10/",
            "http://169.254.169.254/latest/meta-data/",
            "http://0.0.0.0/",
            "http://localhost/",
            "http://LOCALHOST./",
            "http://foo.localhost/",
        ] {
            assert!(raw(u).check_url(&url(u)).is_err(), "{} should be rejected", u);
        }
    }

    // ── ip_is_forbidden ──

    #[test]
    fn forbidden_ips() {
        let bad = [
            "127.0.0.1", "127.9.9.9", "0.0.0.0", "0.1.2.3", "10.0.0.1", "172.16.0.1", "172.31.255.255",
            "192.168.1.1", "169.254.1.1", "100.64.0.1", "100.127.255.255", "198.18.0.1", "198.19.255.255",
            "224.0.0.1", "255.255.255.255", "240.0.0.1", "192.0.0.8",
            "::1", "::", "fe80::1", "fc00::1", "fd12:3456::1", "ff02::1",
            "::ffff:127.0.0.1", "::ffff:10.0.0.1", "::ffff:169.254.169.254", "::127.0.0.1",
            "64:ff9b::7f00:1", "64:ff9b::a00:1", "64:ff9b:1::1",
            "::ffff:0:7f00:1", "::ffff:0:a00:1", "fec0::1", "100::1", "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2002:7f00:1::1", "2002:0a00:0001::",
        ];
        for s in bad {
            assert!(ip_is_forbidden(s.parse().unwrap()), "{} should be forbidden", s);
        }
        let ok = [
            "8.8.8.8", "1.1.1.1", "93.184.216.34", "172.15.0.1", "172.32.0.1", "100.63.0.1", "100.128.0.1",
            "198.17.0.1", "198.20.0.1", "2606:4700:4700::1111", "2001:4860:4860::8888",
            "64:ff9b::808:808", "2002:0808:0808::1", "::ffff:8.8.8.8",
        ];
        for s in ok {
            assert!(!ip_is_forbidden(s.parse().unwrap()), "{} should be allowed", s);
        }
    }

    // ── DNS 後の検査 ──

    struct StubLookup(HashMap<String, Vec<IpAddr>>);

    impl Lookup for StubLookup {
        fn lookup(&self, host: String) -> LookupFuture {
            let r = self
                .0
                .get(&host)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such host"));
            Box::pin(async move { r })
        }
    }

    fn stub(entries: &[(&str, &[&str])]) -> Arc<dyn Lookup> {
        Arc::new(StubLookup(
            entries
                .iter()
                .map(|(h, ips)| (h.to_string(), ips.iter().map(|s| s.parse().unwrap()).collect()))
                .collect(),
        ))
    }

    fn resolver(policy: Policy, lookup: Arc<dyn Lookup>) -> (GuardedResolver, Arc<Mutex<Option<String>>>) {
        let denied = Arc::new(Mutex::new(None));
        (GuardedResolver { policy: Arc::new(policy), lookup, denied: denied.clone() }, denied)
    }

    async fn resolve(r: &GuardedResolver, host: &str) -> Result<Vec<IpAddr>, String> {
        use reqwest::dns::Resolve;
        r.resolve(host.parse().unwrap())
            .await
            .map(|it| it.map(|a| a.ip()).collect())
            .map_err(|e| e.to_string())
    }

    #[tokio::test]
    async fn resolver_rejects_names_that_resolve_to_private_addresses() {
        let lookup = stub(&[
            ("public.example.com", &["93.184.216.34"]),
            // DNS リバインディング: 公開風の名前がプライベート / ループバック / メタデータを返す
            ("rebind.example.com", &["10.0.0.5"]),
            ("loop.example.com", &["127.0.0.1"]),
            ("meta.example.com", &["169.254.169.254"]),
            // 1 つでも混じれば拒否
            ("mixed.example.com", &["93.184.216.34", "127.0.0.1"]),
            ("v6.example.com", &["::1"]),
            ("mapped.example.com", &["::ffff:127.0.0.1"]),
            ("empty.example.com", &[]),
        ]);
        let (r, denied) = resolver(policy(&["https://public.example.com/"], vec![]), lookup);

        assert_eq!(resolve(&r, "public.example.com").await.unwrap(), vec!["93.184.216.34".parse::<IpAddr>().unwrap()]);
        assert!(denied.lock().unwrap().is_none());
        for host in ["rebind.example.com", "loop.example.com", "meta.example.com", "mixed.example.com", "v6.example.com", "mapped.example.com"] {
            let err = resolve(&r, host).await.unwrap_err();
            assert!(err.contains("接続できません"), "{}: {}", host, err);
            denied.lock().unwrap().take();
        }
        assert!(resolve(&r, "empty.example.com").await.is_err());
        assert!(resolve(&r, "unknown.example.com").await.is_err());
    }

    // ── 通信（テスト用ポリシーでループバックを許可した実サーバー） ──

    /// 1 リクエストずつ処理する最小の HTTP サーバー。`handler(path) -> 応答の生バイト列`。
    async fn serve(handler: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let _ = sock.write_all(&handler(&path)).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        port
    }

    fn ok(body: &str) -> Vec<u8> {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes()
    }

    fn redirect(location: &str) -> Vec<u8> {
        format!("HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", location).into_bytes()
    }

    /// ループバックを許すのはテストだけ（`allow_private` は `#[cfg(test)]` のフィールド）。
    fn open_policy(prefixes: &[String], own: Vec<u16>) -> Arc<Policy> {
        let mut p = Policy::new(prefixes, own);
        p.allow_private = true;
        Arc::new(p)
    }

    fn init() -> FetchInit {
        FetchInit::default()
    }

    #[tokio::test]
    async fn fetch_passes_allowed_prefix_only() {
        let port = serve(|path| ok(&format!("hello {}", path))).await;
        let prefix = format!("http://127.0.0.1:{}/api/", port);
        let policy = open_policy(&[prefix], vec![]);
        let lookup = stub(&[]);

        let r = perform(policy.clone(), lookup.clone(), &format!("http://127.0.0.1:{}/api/x", port), init())
            .await
            .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "hello /api/x");
        assert_eq!(r.body_encoding, "text");

        // 同じホスト・別パス / 別ポート は許可プレフィックスの外
        assert!(perform(policy.clone(), lookup.clone(), &format!("http://127.0.0.1:{}/other", port), init()).await.is_err());
        assert!(perform(policy.clone(), lookup.clone(), &format!("http://127.0.0.1:{}/api/x", port + 1), init()).await.is_err());
    }

    #[tokio::test]
    async fn redirect_hops_are_rechecked() {
        let port = serve(|path| match path {
            "/api/to-ok" => redirect("/api/final"),
            "/api/final" => ok("final"),
            // 許可プレフィックスの外へ
            "/api/to-outside" => redirect("/outside"),
            "/outside" => ok("should not be reached"),
            // 別ホスト（名前経由でループバックへ解決される）へ
            "/api/to-name" => redirect("http://rebind.test/api/final"),
            // 自ポート扱いのポートへ
            "/api/to-own" => redirect("http://127.0.0.1:1/api/x"),
            "/api/loop" => redirect("/api/loop"),
            _ => ok("?"),
        })
        .await;
        let prefix = format!("http://127.0.0.1:{}/api/", port);
        let policy = open_policy(&[prefix], vec![]);
        let lookup = stub(&[]);
        let base = format!("http://127.0.0.1:{}", port);

        let r = perform(policy.clone(), lookup.clone(), &format!("{}/api/to-ok", base), init()).await.unwrap();
        assert_eq!(r.body, "final");
        assert!(r.url.ends_with("/api/final"));

        let e = perform(policy.clone(), lookup.clone(), &format!("{}/api/to-outside", base), init()).await.unwrap_err();
        assert!(e.contains("リダイレクト先") && e.contains("許可されていません"), "{}", e);
        let e = perform(policy.clone(), lookup.clone(), &format!("{}/api/to-name", base), init()).await.unwrap_err();
        assert!(e.contains("リダイレクト先"), "{}", e);
        let e = perform(policy.clone(), lookup.clone(), &format!("{}/api/loop", base), init()).await.unwrap_err();
        assert!(e.contains("リダイレクト"), "{}", e);
    }

    #[test]
    fn cross_origin_redirects_are_refused_but_https_upgrade_is_allowed() {
        let o = url("http://a.example.com/x");
        assert!(check_redirect_origin(&o, &url("http://a.example.com/y")).is_ok());
        assert!(check_redirect_origin(&o, &url("https://a.example.com/y")).is_ok()); // 格上げ
        assert!(check_redirect_origin(&o, &url("http://b.example.com/y")).is_err());
        assert!(check_redirect_origin(&o, &url("http://a.example.com:8080/y")).is_err());
        let s = url("https://a.example.com/x");
        assert!(check_redirect_origin(&s, &url("http://a.example.com/y")).is_err()); // 格下げは不可
        assert!(check_redirect_origin(&s, &url("https://b.example.com/y")).is_err());
    }

    #[test]
    fn logged_urls_drop_query_fragment_and_credentials() {
        assert_eq!(loggable_url("https://a.example.com/p?key=SECRET#f"), "https://a.example.com/p");
        assert_eq!(loggable_url("https://u:pw@a.example.com/p?k=1"), "https://a.example.com/p");
        assert_eq!(loggable_url("not a url?secret=1"), "not a url");
        assert!(loggable_url(&format!("https://a.example.com/{}", "x".repeat(1000))).chars().count() <= LOG_URL_CHARS + 1);
    }

    #[tokio::test]
    async fn redirect_to_a_name_that_resolves_to_loopback_is_rejected() {
        // 本番相当のポリシー（ループバック不可）。許可プレフィックスは公開風の名前で、
        // その名前がループバックへ解決される（リバインディング）ケース
        let port = serve(|_| ok("secret")).await;
        let prefixes = vec![format!("http://rebind.test:{}/", port)];
        let policy = Arc::new(Policy::new(&prefixes, vec![]));
        let lookup = stub(&[("rebind.test", &["127.0.0.1"])]);
        let e = perform(policy, lookup, &format!("http://rebind.test:{}/", port), init()).await.unwrap_err();
        assert!(e.contains("ループバック／プライベート／予約済みアドレス"), "{}", e);
    }

    #[tokio::test]
    async fn default_policy_rejects_loopback_literal_and_localhost() {
        let port = serve(|_| ok("secret")).await;
        // 本番相当のポリシー（allow_private なし）。プレフィックスが保存値に紛れていても URL 段階で拒否
        for host in ["127.0.0.1", "localhost"] {
            let u = format!("http://{}:{}/", host, port);
            let policy = Arc::new(Policy::new(&[u.clone()], vec![]));
            let e = perform(policy, stub(&[]), &u, init()).await.unwrap_err();
            assert!(e.contains("ループバック"), "{}: {}", host, e);
        }
        // 実際の localhost 名前解決を通しても、名前の段階で拒否される
        let u = format!("http://localhost.:{}/", port);
        let policy = Arc::new(Policy::new(&[u.clone()], vec![]));
        let e = perform(policy, Arc::new(SystemLookup), &u, init()).await.unwrap_err();
        assert!(e.contains("ループバック"), "{}", e);
    }

    #[tokio::test]
    async fn own_axum_port_is_rejected_even_when_the_prefix_allows_it() {
        let port = serve(|_| ok("mcp")).await;
        let u = format!("http://127.0.0.1:{}/", port);
        // own_ports に自ポートを入れる（oretachi の MCP / Web ビューアのポートに相当）
        let policy = open_policy(&[u.clone()], vec![port]);
        let e = perform(policy, stub(&[]), &u, init()).await.unwrap_err();
        assert!(e.contains("oretachi 自身"), "{}", e);
    }

    #[tokio::test]
    async fn limits_are_enforced() {
        let big = "x".repeat(MAX_RESPONSE_BODY_BYTES + 1);
        let port = serve(move |path| if path == "/big" { ok(&big) } else { ok("small") }).await;
        let prefix = format!("http://127.0.0.1:{}/", port);
        let policy = open_policy(&[prefix], vec![]);
        let lookup = stub(&[]);
        let base = format!("http://127.0.0.1:{}", port);

        // レスポンス本文の上限
        let e = perform(policy.clone(), lookup.clone(), &format!("{}/big", base), init()).await.unwrap_err();
        assert!(e.contains("大きすぎます") || e.contains("上限"), "{}", e);

        // メソッド
        let mut i = init();
        i.method = Some("CONNECT".to_string());
        assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err());
        let mut i = init();
        i.method = Some("TRACE".to_string());
        assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err());

        // 禁止ヘッダー
        for h in ["Host", "content-length", "Transfer-Encoding", "Proxy-Authorization", "Connection"] {
            let mut i = init();
            i.headers = vec![(h.to_string(), "x".to_string())];
            assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err(), "{}", h);
        }
        // ヘッダーの数・値の長さ
        let mut i = init();
        i.headers = (0..=MAX_REQUEST_HEADERS).map(|n| (format!("x-h{}", n), "v".to_string())).collect();
        assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err());
        let mut i = init();
        i.headers = vec![("x-long".to_string(), "v".repeat(MAX_HEADER_VALUE_BYTES + 1))];
        assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err());

        // リクエスト本文の上限・GET の本文・base64 の不正
        let mut i = init();
        i.method = Some("POST".to_string());
        i.body = Some("x".repeat(MAX_REQUEST_BODY_BYTES + 1));
        assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err());
        let mut i = init();
        i.body = Some("x".to_string());
        assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err());
        let mut i = init();
        i.method = Some("POST".to_string());
        i.body = Some("!!!".to_string());
        i.body_encoding = Some("base64".to_string());
        assert!(perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.is_err());

        // 通る要求（POST + base64 応答）
        let mut i = init();
        i.method = Some("post".to_string());
        i.body = Some("hi".to_string());
        i.response_type = Some("base64".to_string());
        let r = perform(policy.clone(), lookup.clone(), &format!("{}/s", base), i).await.unwrap();
        assert_eq!(r.body_encoding, "base64");
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&r.body).unwrap(), b"small");
    }

    // ── 台帳 ──

    #[test]
    fn registry_limits_and_aborts() {
        let reg = AppNetRegistry::default();
        let mut regs = Vec::new();
        for _ in 0..MAX_CONCURRENT_PER_INSTANCE {
            regs.push(reg.begin("appinst-a").unwrap());
        }
        assert!(reg.begin("appinst-a").is_err());
        // 別インスタンスには影響しない
        assert!(reg.begin("appinst-b").is_ok());
        // 終わったぶんは空く
        reg.end("appinst-a", regs[0].0);
        assert!(reg.begin("appinst-a").is_ok());
        // 停止すると、以後は出せない
        reg.abort_all("appinst-a", true);
        assert!(reg.begin("appinst-a").is_err());
        // 再読み込み（停止ではない）は出せる
        reg.abort_all("appinst-b", false);
        assert!(reg.begin("appinst-b").is_ok());
    }

    #[tokio::test]
    async fn abort_all_cancels_in_flight_requests() {
        let reg = AppNetRegistry::default();
        let (_id, abort_reg) = reg.begin("appinst-a").unwrap();
        let fut = Abortable::new(std::future::pending::<()>(), abort_reg);
        reg.abort_all("appinst-a", false);
        assert!(fut.await.is_err());
    }
}
