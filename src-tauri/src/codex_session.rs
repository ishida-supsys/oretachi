//! codex の rollout ファイルから対話セッションの ID を引く (#157)。
//!
//! Claude Code は `~/.claude/sessions/<pid>.json` を書くので PID から一意に辿れるが、
//! codex にはそれが無い。残っているのは `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
//! だけで、ファイル名にも中身にも PID は入らない。そこで
//!
//! 1. codex プロセスの起動時刻以降に書かれた rollout に絞り、
//! 2. 先頭行の `session_meta` の `cwd` がタブの cwd と一致し、
//! 3. `originator` が対話 TUI のものだけを候補にする
//!
//! という消去法で辿る。**候補が 2 つ以上あるときは諦める**（`None` を返す）。
//! 同じワークツリーで codex タブを 2 枚開くと候補が割れるが、取り違えて
//! 別の会話を復元するくらいなら復元しないほうがましなので、そこは切る。
//!
//! なお `codex resume <id>` は**既存の rollout に追記し、session ID も変えない**
//! （v0.147.0 で実測）。復元したタブを再度保存しても ID は同じままでよい。

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// 起動時刻からどれだけ遡った更新まで候補に含めるか。時計のずれと、
/// 検出した PID が shim（`codex.cmd`）で実体の起動がわずかに後ろへずれる分の余裕。
const START_TOLERANCE_MS: i64 = 60_000;

/// 対話セッションの `originator`。`codex exec`（`codex_exec`）や
/// `codex_cli_rs` の非対話実行は resume しても意味が無いので候補から外す。
const INTERACTIVE_ORIGINATORS: &[&str] = &["codex-tui"];

/// 走査するファイル数の上限。`~/.codex/sessions` は消されないので単調に増える。
/// stat だけとはいえ無制限に舐めるのは避ける。
const MAX_SCANNED_FILES: usize = 20_000;

/// 解決に失敗した PID を再走査するまでの間隔。ポーリングは 10 秒周期なので、
/// 毎 tick フルスキャンしないための間引き。
const RETRY_INTERVAL_MS: i64 = 30_000;

/// PID 単位の解決結果キャッシュ。`start_polling` のループが持つ。
#[derive(Default)]
pub struct CodexSessionCache {
    /// agent_pid → (解決済み session ID, 最後に走査した時刻)
    entries: HashMap<u32, (Option<String>, i64)>,
}

impl CodexSessionCache {
    /// `agent_pid` の codex セッション ID を返す。解決済みなら走査しない。
    ///
    /// `now_ms` は呼び出し側の tick 時刻（`pty_manager::now_ms`）。
    pub fn resolve(&mut self, agent_pid: u32, cwd: Option<&str>, now_ms: i64) -> Option<String> {
        if let Some((cached, last_attempt)) = self.entries.get(&agent_pid) {
            if cached.is_some() || now_ms - last_attempt < RETRY_INTERVAL_MS {
                return cached.clone();
            }
        }
        let resolved = cwd.and_then(|c| find_codex_session_id(c, process_start_ms(agent_pid)));
        self.entries.insert(agent_pid, (resolved.clone(), now_ms));
        resolved
    }

    /// 生存していない PID のエントリを落とす。
    pub fn retain_live(&mut self, live_pids: &std::collections::HashSet<u32>) {
        self.entries.retain(|pid, _| live_pids.contains(pid));
    }
}

/// プロセスの起動時刻（epoch ms）。取れなければ `None`。
fn process_start_ms(pid: u32) -> Option<i64> {
    let target = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
    sys.process(target).map(|p| p.start_time() as i64 * 1000)
}

fn codex_sessions_dir() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let dir = Path::new(&home).join(".codex").join("sessions");
    dir.is_dir().then_some(dir)
}

/// パス比較用の正規化。Windows なので区切りと大文字小文字を潰す。
fn normalize_path(p: &str) -> String {
    p.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(ms as i64)
}

/// `since_ms` 以降に更新された rollout ファイルを集める。
fn collect_recent_rollouts(dir: &Path, since_ms: i64, scanned: &mut usize, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *scanned >= MAX_SCANNED_FILES {
            return;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => collect_recent_rollouts(&path, since_ms, scanned, out),
            Ok(ft) if ft.is_file() => {
                *scanned += 1;
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if mtime_ms(&path).map_or(false, |m| m >= since_ms) {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
}

/// rollout の先頭行（`session_meta`）から `(session_id, cwd, originator)` を読む。
///
/// 1 行目だけ読む。会話が伸びたファイルは数 MB になるので全部は読まない。
/// 派生スレッド（`parent_thread_id` 付き）は `None` を返して候補から落とす。
fn read_rollout_meta(path: &Path) -> Option<(String, String, String)> {
    let file = std::fs::File::open(path).ok()?;
    let mut first_line = String::new();
    BufReader::new(file).read_line(&mut first_line).ok()?;
    let v: serde_json::Value = serde_json::from_str(&first_line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("session_meta") {
        return None;
    }
    let payload = v.get("payload")?;
    // 派生スレッド（サブエージェント）はタブ本体の会話ではないので候補から外す。
    if payload.get("parent_thread_id").is_some_and(|v| !v.is_null()) {
        return None;
    }
    // `session_id` は cli 0.147.0 で確認。0.139.0 の rollout は `id` しか持たない。
    let session_id = payload
        .get("session_id")
        .or_else(|| payload.get("id"))
        .and_then(|s| s.as_str())?
        .to_string();
    let cwd = payload.get("cwd").and_then(|s| s.as_str())?.to_string();
    let originator = payload
        .get("originator")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    Some((session_id, cwd, originator))
}

/// `cwd` で動いている対話 codex セッションの ID を 1 つに絞れたら返す。
///
/// `proc_start_ms` が取れなかった場合は候補を絞る軸が無くなるので諦める。
fn find_codex_session_id(cwd: &str, proc_start_ms: Option<i64>) -> Option<String> {
    let start_ms = proc_start_ms?;
    let dir = codex_sessions_dir()?;
    let target_cwd = normalize_path(cwd);

    let mut candidates = Vec::new();
    let mut scanned = 0usize;
    collect_recent_rollouts(&dir, start_ms - START_TOLERANCE_MS, &mut scanned, &mut candidates);

    let mut matched: Vec<String> = Vec::new();
    for path in candidates {
        let Some((session_id, rollout_cwd, originator)) = read_rollout_meta(&path) else {
            continue;
        };
        if !INTERACTIVE_ORIGINATORS.contains(&originator.as_str()) {
            continue;
        }
        if normalize_path(&rollout_cwd) != target_cwd {
            continue;
        }
        if !matched.contains(&session_id) {
            matched.push(session_id);
        }
    }

    match matched.len() {
        1 => Some(matched.remove(0)),
        0 => None,
        n => {
            // 同じ cwd で codex を複数枚開いている。どれがこの PID のものか決められない。
            log::debug!(
                "[Terminal] codex セッションを一意に決められないため resume 情報を諦める cwd={} 候補数={}",
                cwd,
                n
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_は区切りと大文字小文字を潰す() {
        assert_eq!(normalize_path("X:/devel/Worktree/"), "x:\\devel\\worktree");
        assert_eq!(normalize_path("X:\\devel\\worktree"), "x:\\devel\\worktree");
    }

    #[test]
    fn read_rollout_meta_は先頭行から3つ組を返す() {
        let dir = std::env::temp_dir().join("oretachi-codex-session-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-test.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"session_id\":\"01a0-b2\",\"cwd\":\"X:\\\\w\",\"originator\":\"codex-tui\"}}\n{\"type\":\"event\"}\n",
        )
        .unwrap();
        let meta = read_rollout_meta(&path).unwrap();
        assert_eq!(meta.0, "01a0-b2");
        assert_eq!(meta.1, "X:\\w");
        assert_eq!(meta.2, "codex-tui");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_rollout_meta_は先頭行が_session_meta_でなければ_none() {
        let dir = std::env::temp_dir().join("oretachi-codex-session-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-broken.jsonl");
        std::fs::write(&path, "{\"type\":\"event\"}\n").unwrap();
        assert!(read_rollout_meta(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_rollout_meta_は_session_id_が無ければ_id_を使う() {
        // cli 0.139.0 の codex-tui rollout は `session_id` を持たない
        let dir = std::env::temp_dir().join("oretachi-codex-session-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-legacy.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"019efc72-f8fb\",\"cwd\":\"X:\\\\w\",\"originator\":\"codex-tui\"}}\n",
        )
        .unwrap();
        assert_eq!(read_rollout_meta(&path).unwrap().0, "019efc72-f8fb");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_rollout_meta_は派生スレッドを弾く() {
        let dir = std::env::temp_dir().join("oretachi-codex-session-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-child.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"session_id\":\"a-b\",\"parent_thread_id\":\"p-q\",\"cwd\":\"X:\\\\w\",\"originator\":\"codex-tui\"}}\n",
        )
        .unwrap();
        assert!(read_rollout_meta(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn 起動時刻が取れなければ諦める() {
        assert!(find_codex_session_id("X:\\w", None).is_none());
    }
}
