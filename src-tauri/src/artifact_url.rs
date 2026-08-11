//! URL アーティファクト (`text/uri-list`) の抽出・正規化・登録。
//!
//! Claude Code のライフサイクルフック (PreToolUse / PostToolUse) が `/notify` に運んでくる
//! hook JSON から URL を拾い、ワークツリーのアーティファクトとして登録する。
//! 保存形式は他のアーティファクトと同じ `artifacts/<worktreeId>/<id>.json`。

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager};

/// URL アーティファクトの content_type。フロント側の定数は
/// `src/types/artifact.ts` の `URL_ARTIFACT_CONTENT_TYPE` と対応する。
pub const URL_ARTIFACT_CONTENT_TYPE: &str = "text/uri-list";

/// URL 末尾に紛れ込みやすい区切り文字。Markdown の `(...)` や JSON の `"` を落とす。
const TRAILING_TRIM: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"', '<', '>', '\\'];

/// テキスト中の http(s) URL を出現順に抽出する（重複は除去）。
pub fn extract_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let rest = &text[i..];
        let scheme_len = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            // 次のバイト境界へ進む
            i += 1;
            while i < bytes.len() && !text.is_char_boundary(i) {
                i += 1;
            }
            continue;
        };

        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`' || c == '<' || c == '>')
            .unwrap_or(rest.len());
        let raw = &rest[..end];
        let trimmed = raw.trim_end_matches(TRAILING_TRIM);
        // スキーマだけ（ホスト無し）は URL とみなさない
        if trimmed.len() > scheme_len {
            let normalized = normalize_url(trimmed);
            if !out.contains(&normalized) {
                out.push(normalized);
            }
        }
        i += end.max(1);
    }
    out
}

/// 重複判定に使う正規化。末尾スラッシュを落とすだけの控えめな正規化に留める
/// （クエリやフラグメントは意味を持ちうるので保持する）。
pub fn normalize_url(url: &str) -> String {
    let t = url.trim();
    let t = t.trim_end_matches('/');
    t.to_string()
}

/// git remote URL / GitHub の web URL から `owner/repo` を取り出す。
/// 対応形式: `git@github.com:o/r.git`, `ssh://git@github.com/o/r.git`,
/// `https://github.com/o/r(.git)`, `https://github.com/o/r/issues/1`
pub fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let s = url.trim();
    // scp 形式: [user@]host:owner/repo
    let path = if let Some(rest) = s.strip_prefix("git@") {
        let (host, p) = rest.split_once(':')?;
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        p.to_string()
    } else {
        let after_scheme = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))
            .or_else(|| s.strip_prefix("ssh://"))
            .or_else(|| s.strip_prefix("git://"))?;
        // authority を先に切り出す。パス側に '@' があっても authority と誤認しないよう、
        // 認証情報の除去は authority の中だけで行う。
        let (authority, p) = after_scheme.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, r)| r);
        // ポート付きホストも許容
        let host = host.split(':').next().unwrap_or(host);
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        p.to_string()
    };

    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_ascii_lowercase(), repo.to_ascii_lowercase()))
}

/// GitHub の issue / PR URL を `(owner, repo, number)` に分解する。
pub fn parse_github_issue_or_pr(url: &str) -> Option<(String, String, u64)> {
    let after_scheme = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let (host, path) = after_scheme.split_once('/')?;
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    let kind = parts.next()?;
    if kind != "issues" && kind != "pull" {
        return None;
    }
    // `/issues/113#issuecomment-1` や `/issues/113?x=1` のようにクエリ・フラグメントが
    // 付いていても番号を取り出せるようにする
    let number_part = parts.next()?;
    let number_part = number_part.split(['#', '?']).next().unwrap_or(number_part);
    let number: u64 = number_part.parse().ok()?;
    Some((owner.to_ascii_lowercase(), repo.to_ascii_lowercase(), number))
}

/// アーティファクトのタイトル。GitHub の issue/PR は `owner/repo#123`、
/// それ以外は URL をそのまま使う。
pub fn derive_title(url: &str) -> String {
    match parse_github_issue_or_pr(url) {
        Some((owner, repo, number)) => format!("{}/{}#{}", owner, repo, number),
        None => url.to_string(),
    }
}

/// 正規化 URL から決定的なアーティファクト ID を作る。
/// 同じ URL は必ず同じ ID になるため、ファイル存在チェックだけで重複登録を防げる。
pub fn artifact_id_for(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(normalize_url(url).as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{:02x}", b)).collect();
    format!("url-{}", hex)
}

fn artifacts_dir(app: &AppHandle, worktree_id: &str) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("artifacts")
        .join(worktree_id))
}

/// 同じ URL の `text/uri-list` アーティファクトが既にあるか。
///
/// ID だけの判定では不十分。MCP の `artifact` ツールは任意 ID でアーティファクトを
/// 作れるため、同じ URL が別 ID（例 `issue-113`）で先に登録されていることがある。
/// issue #113 の「URL が重複する時は登録をスキップする」を満たすには内容で突き合わせる。
fn url_already_registered(dir: &std::path::Path, url: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if val.get("type").and_then(|v| v.as_str()) != Some(URL_ARTIFACT_CONTENT_TYPE) {
            continue;
        }
        let existing = val
            .get("content")
            .and_then(|v| v.as_str())
            .map(|c| normalize_url(c.lines().next().unwrap_or("")))
            .unwrap_or_default();
        if existing == url {
            return true;
        }
    }
    false
}

/// URL アーティファクトを登録する。既に同じ URL が登録済みなら何もせず `false` を返す。
pub async fn register_url_artifact(
    app: &AppHandle,
    worktree_id: &str,
    url: &str,
) -> Result<bool, String> {
    let url = normalize_url(url);
    let id = artifact_id_for(&url);
    let dir = artifacts_dir(app, worktree_id)?;
    let path = dir.join(format!("{}.json", id));

    // ロック取得前の早期判定（大半のケースはここで抜ける）
    if path.exists() {
        return Ok(false);
    }

    let _guard = crate::mcp_server::ARTIFACT_WRITE_LOCK.lock().await;
    // ID 一致に加えて内容一致も見る。ロック取得待ちの間に他スレッドが書いた場合も
    // ここで拾えるため、path.exists() の再チェックは不要。
    {
        let dir = dir.clone();
        let url = url.clone();
        let exists = tokio::task::spawn_blocking(move || url_already_registered(&dir, &url))
            .await
            .map_err(|e| e.to_string())?;
        if exists {
            return Ok(false);
        }
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "id": id,
        "type": URL_ARTIFACT_CONTENT_TYPE,
        "title": derive_title(&url),
        "content": url,
        "created_at": now,
        "updated_at": now,
    }))
    .map_err(|e| e.to_string())?;

    crate::mcp_server::write_artifact_atomic(&path, &json)
        .await
        .map_err(|e| e.to_string())?;
    drop(_guard);

    log::info!("[url-artifact] registered id={} worktree_id={} url={}", id, worktree_id, url);

    // autoOpen: false — フックによる自動登録は副作用なので、AI が明示的に作った
    // アーティファクトと違いビューアウィンドウを開いてユーザーの作業に割り込まない。
    if let Err(e) = app.emit(
        "artifact-changed",
        serde_json::json!({
            "worktreeId": worktree_id,
            "artifactId": id,
            "command": "create",
            "autoOpen": false,
        }),
    ) {
        log::warn!("Failed to emit artifact-changed: {}", e);
    }
    if let Some(pool) = app.try_state::<crate::report_db::ReportPool>() {
        let _ = crate::report_db::insert(&pool.inner().0, "artifact_change:create", &id).await;
    }

    Ok(true)
}

// ─── フック (PreToolUse / PostToolUse) からの自動登録 ─────────────────────────

/// フック処理に必要なワークツリー情報。`AppSettings` の借用を跨いで
/// `tokio::spawn` へ渡すため値でコピーして持つ。
#[derive(Debug, Clone)]
pub struct HookWorktree {
    pub id: String,
    pub repository_name: String,
    pub branch_name: String,
    pub path: String,
}

/// 直近何件のタスクを URL 突合の対象にするか。
const TASK_PROMPT_SCAN_LIMIT: i64 = 200;

/// タスクプロンプト URL キャッシュの有効期間。
/// PreToolUse は「http を含む全ツール呼び出し」で走るため、毎回 tasks.db を
/// 200 件走査すると無駄が大きい。タスクの追加は稀なので短い TTL で十分。
const PROMPT_URL_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

type PromptUrlCache =
    std::collections::HashMap<String, (std::time::Instant, std::collections::HashSet<String>)>;

/// `"<repository>\0<branch>"` → (取得時刻, プロンプト中の URL 集合)
static PROMPT_URL_CACHE: std::sync::Mutex<Option<PromptUrlCache>> = std::sync::Mutex::new(None);

fn cached_prompt_urls(key: &str) -> Option<std::collections::HashSet<String>> {
    let guard = PROMPT_URL_CACHE.lock().ok()?;
    let (fetched_at, urls) = guard.as_ref()?.get(key)?;
    if fetched_at.elapsed() > PROMPT_URL_CACHE_TTL {
        return None;
    }
    Some(urls.clone())
}

fn store_prompt_urls(key: &str, urls: &std::collections::HashSet<String>) {
    if let Ok(mut guard) = PROMPT_URL_CACHE.lock() {
        guard
            .get_or_insert_with(Default::default)
            .insert(key.to_string(), (std::time::Instant::now(), urls.clone()));
    }
}

/// JSON 値に含まれる文字列リーフを再帰的に集める。
///
/// `Value::to_string()` を直接 `extract_urls` に渡すと、改行が `\n` という
/// 2 文字のエスケープ列として URL の直後に残り、URL の末尾に混入する
/// （`gh issue create` の stdout がまさにこの形）。エスケープを解いた
/// 生の文字列単位で走査するために使う。
fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_strings(v, out);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_strings(v, out);
            }
        }
        _ => {}
    }
}

/// JSON 値に含まれる全文字列から URL を抽出する（重複は除去）。
fn extract_urls_from_json(value: &serde_json::Value) -> Vec<String> {
    let mut strings = Vec::new();
    collect_strings(value, &mut strings);
    let mut out: Vec<String> = Vec::new();
    for s in strings {
        for url in extract_urls(&s) {
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

/// Claude Code のツールフック JSON を見て、条件に一致する URL を自動登録する。
/// `/notify` から `tokio::spawn` で呼ばれる（通知パスをブロックしない）。
pub async fn handle_tool_hook(app: AppHandle, wt: HookWorktree, event: String, body: String) {
    let Ok(hook) = serde_json::from_str::<serde_json::Value>(&body) else {
        return;
    };
    match event.as_str() {
        "PreToolUse" => handle_pre_tool_use(&app, &wt, &hook).await,
        "PostToolUse" => handle_post_tool_use(&app, &wt, &hook).await,
        _ => {}
    }
}

/// issue を開く: ツール入力に現れた URL が、このワークツリー宛タスクの
/// プロンプトに含まれる URL と一致する時だけ登録する。
async fn handle_pre_tool_use(app: &AppHandle, wt: &HookWorktree, hook: &serde_json::Value) {
    let Some(tool_input) = hook.get("tool_input") else {
        return;
    };
    let urls = extract_urls_from_json(tool_input);
    if urls.is_empty() {
        return;
    }

    let cache_key = format!("{}\0{}", wt.repository_name, wt.branch_name);
    let prompt_urls = match cached_prompt_urls(&cache_key) {
        Some(urls) => urls,
        None => {
            let Some(pool) = app.try_state::<crate::task_db::TaskPool>() else {
                return;
            };
            let prompts = match crate::task_db::list_prompts_for_worktree(
                &pool.inner().0,
                &wt.repository_name,
                &wt.branch_name,
                TASK_PROMPT_SCAN_LIMIT,
            )
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("[url-artifact] failed to load task prompts: {}", e);
                    return;
                }
            };
            let urls: std::collections::HashSet<String> =
                prompts.iter().flat_map(|p| extract_urls(p)).collect();
            store_prompt_urls(&cache_key, &urls);
            urls
        }
    };
    if prompt_urls.is_empty() {
        return;
    }

    for url in urls.into_iter().filter(|u| prompt_urls.contains(u)) {
        if let Err(e) = register_url_artifact(app, &wt.id, &url).await {
            log::warn!("[url-artifact] register failed url={} error={}", url, e);
        }
    }
}

/// issue / PR を作成する: `gh issue create` / `gh pr create` の出力に現れた
/// GitHub URL が、このワークツリーのリポジトリと一致する時だけ登録する。
async fn handle_post_tool_use(app: &AppHandle, wt: &HookWorktree, hook: &serde_json::Value) {
    if hook.get("tool_name").and_then(|v| v.as_str()) != Some("Bash") {
        return;
    }
    let command = hook
        .get("tool_input")
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !is_gh_create_command(command) {
        return;
    }
    let Some(response) = hook.get("tool_response") else {
        return;
    };

    let candidates: Vec<(String, String, String)> = extract_urls_from_json(response)
        .into_iter()
        .filter_map(|url| {
            parse_github_issue_or_pr(&url).map(|(owner, repo, _)| (url, owner, repo))
        })
        .collect();
    if candidates.is_empty() {
        return;
    }

    let repo_path = wt.path.clone();
    let remotes = match tokio::task::spawn_blocking(move || {
        crate::git_worktree::get_git_remotes(&repo_path)
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[url-artifact] failed to read git remotes: {}", e);
            return;
        }
    };
    let owned: std::collections::HashSet<(String, String)> = remotes
        .iter()
        .filter_map(|r| r.get("url").and_then(|v| v.as_str()))
        .filter_map(parse_github_owner_repo)
        .collect();
    if owned.is_empty() {
        return;
    }

    for (url, owner, repo) in candidates {
        if !owned.contains(&(owner, repo)) {
            continue;
        }
        if let Err(e) = register_url_artifact(app, &wt.id, &url).await {
            log::warn!("[url-artifact] register failed url={} error={}", url, e);
        }
    }
}

/// `gh issue create` / `gh pr create` を含むシェルコマンドか。
/// `cd x && gh pr create ...` のような連結にも当たるよう部分一致で見る。
pub fn is_gh_create_command(command: &str) -> bool {
    let normalized = command.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.contains("gh issue create") || normalized.contains("gh pr create")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_urls_picks_http_and_https() {
        let text = r#"see https://github.com/o/r/issues/1 and http://example.com/a."#;
        assert_eq!(
            extract_urls(text),
            vec![
                "https://github.com/o/r/issues/1".to_string(),
                "http://example.com/a".to_string(),
            ]
        );
    }

    #[test]
    fn extract_urls_strips_json_quotes_and_dedupes() {
        let text = r#"{"command":"gh issue view https://github.com/o/r/issues/7","x":"https://github.com/o/r/issues/7/"}"#;
        assert_eq!(
            extract_urls(text),
            vec!["https://github.com/o/r/issues/7".to_string()]
        );
    }

    #[test]
    fn extract_urls_ignores_bare_scheme() {
        assert!(extract_urls("https:// and http://").is_empty());
    }

    #[test]
    fn extract_urls_handles_multibyte_text() {
        let text = "日本語の説明 https://github.com/o/r/pull/12 を参照";
        assert_eq!(
            extract_urls(text),
            vec!["https://github.com/o/r/pull/12".to_string()]
        );
    }

    #[test]
    fn parse_github_owner_repo_scp_form() {
        assert_eq!(
            parse_github_owner_repo("git@github.com:Ishida-Supsys/Oretachi.git"),
            Some(("ishida-supsys".to_string(), "oretachi".to_string()))
        );
    }

    #[test]
    fn parse_github_owner_repo_https_form() {
        assert_eq!(
            parse_github_owner_repo("https://github.com/o/r.git"),
            Some(("o".to_string(), "r".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://user:token@github.com/o/r"),
            Some(("o".to_string(), "r".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("ssh://git@github.com/o/r.git"),
            Some(("o".to_string(), "r".to_string()))
        );
    }

    #[test]
    fn parse_github_owner_repo_rejects_other_hosts() {
        assert_eq!(parse_github_owner_repo("git@gitlab.com:o/r.git"), None);
        assert_eq!(parse_github_owner_repo("https://example.com/o/r"), None);
    }

    #[test]
    fn parse_github_issue_or_pr_works() {
        assert_eq!(
            parse_github_issue_or_pr("https://github.com/o/r/issues/113"),
            Some(("o".to_string(), "r".to_string(), 113))
        );
        assert_eq!(
            parse_github_issue_or_pr("https://github.com/o/r/pull/9"),
            Some(("o".to_string(), "r".to_string(), 9))
        );
        assert_eq!(parse_github_issue_or_pr("https://github.com/o/r"), None);
    }

    #[test]
    fn parse_github_issue_or_pr_tolerates_query_and_fragment() {
        assert_eq!(
            parse_github_issue_or_pr("https://github.com/o/r/issues/113#issuecomment-1"),
            Some(("o".to_string(), "r".to_string(), 113))
        );
        assert_eq!(
            parse_github_issue_or_pr("https://github.com/o/r/pull/9?w=1"),
            Some(("o".to_string(), "r".to_string(), 9))
        );
    }

    #[test]
    fn parse_github_owner_repo_ignores_at_in_path() {
        // '@' がパス側にあるだけの他ホスト URL を github.com と誤認しない
        assert_eq!(
            parse_github_owner_repo("https://evil.com/p@github.com/o/r"),
            None
        );
    }

    #[test]
    fn url_already_registered_matches_by_content_not_id() {
        let dir = std::env::temp_dir().join(format!("oretachi-url-artifact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // MCP の artifact ツールで付けられた任意 ID。ハッシュ ID とは一致しない。
        std::fs::write(
            dir.join("issue-113.json"),
            r#"{"id":"issue-113","type":"text/uri-list","title":"x","content":"https://github.com/o/r/issues/113"}"#,
        )
        .unwrap();

        assert!(url_already_registered(&dir, "https://github.com/o/r/issues/113"));
        // 末尾スラッシュ違いも正規化して同一とみなす
        assert!(url_already_registered(
            &dir,
            &normalize_url("https://github.com/o/r/issues/113/")
        ));
        assert!(!url_already_registered(&dir, "https://github.com/o/r/issues/114"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_already_registered_ignores_other_content_types() {
        let dir = std::env::temp_dir()
            .join(format!("oretachi-url-artifact-other-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("doc.json"),
            r#"{"id":"doc","type":"text/markdown","title":"x","content":"https://github.com/o/r/issues/113"}"#,
        )
        .unwrap();

        assert!(!url_already_registered(&dir, "https://github.com/o/r/issues/113"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derive_title_formats_issue() {
        assert_eq!(derive_title("https://github.com/o/r/issues/113"), "o/r#113");
        assert_eq!(derive_title("https://example.com/x"), "https://example.com/x");
    }

    #[test]
    fn extract_urls_from_json_unescapes_before_scanning() {
        // gh の stdout は末尾に改行が付く。JSON を to_string() してから走査すると
        // エスケープ列 "\n" が URL に食い込むため、文字列リーフ単位で走査する。
        let response: serde_json::Value = serde_json::from_str(
            r#"{"stdout":"https://github.com/o/r/issues/5\n","stderr":"","interrupted":false}"#,
        )
        .unwrap();
        assert_eq!(
            extract_urls_from_json(&response),
            vec!["https://github.com/o/r/issues/5".to_string()]
        );
    }

    #[test]
    fn extract_urls_from_json_walks_nested_values() {
        let input: serde_json::Value = serde_json::from_str(
            r#"{"a":{"b":["https://e.com/1"]},"c":"https://e.com/2","d":1}"#,
        )
        .unwrap();
        assert_eq!(
            extract_urls_from_json(&input),
            vec!["https://e.com/1".to_string(), "https://e.com/2".to_string()]
        );
    }

    #[test]
    fn is_gh_create_command_matches_variants() {
        assert!(is_gh_create_command("gh issue create --title x"));
        assert!(is_gh_create_command("cd /repo && gh pr create --fill"));
        assert!(is_gh_create_command("gh  pr   create"));
        assert!(!is_gh_create_command("gh issue view 113"));
        assert!(!is_gh_create_command("gh pr list"));
    }

    #[test]
    fn artifact_id_is_stable_and_normalized() {
        let a = artifact_id_for("https://github.com/o/r/issues/1");
        let b = artifact_id_for("https://github.com/o/r/issues/1/");
        assert_eq!(a, b);
        assert!(a.starts_with("url-"));
        assert_ne!(a, artifact_id_for("https://github.com/o/r/issues/2"));
    }
}
