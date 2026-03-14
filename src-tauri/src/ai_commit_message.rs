use crate::ai_provider::{self, AiAgentKind};
use crate::settings::SettingsManager;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;
use tokio::io::AsyncWriteExt;
use tokio::time::{timeout, Duration};

const TIMEOUT_SECS: u64 = 120;

const PROMPT_TEMPLATE: &str = r#"You are an expert at writing concise git commit messages.
Analyze the following git diff and generate a commit message.

Recent commit messages from this repository (for style/language/pattern reference):
{{RECENT_COMMITS}}

Git Diff:
{{DIFF}}

Rules:
- Mimic the exact style of the recent commits above: language, prefix convention (e.g. "feat:", "fix:", type/scope format, or no prefix), capitalization, and tone
- Subject line: imperative mood, ≤72 chars, no period at end
- Body (optional): explain WHY, not WHAT, wrap at 72 chars
- If the diff is empty or has no meaningful changes, follow the same style with a minimal message

Respond with ONLY valid JSON: {"subject": "...", "body": "..."}.
The "body" field may be empty string. Do not add any other fields or text."#;

const JSON_SCHEMA: &str = r#"{"type":"object","properties":{"subject":{"type":"string"},"body":{"type":"string"}},"required":["subject","body"]}"#;

pub struct CommitMessageManager {
    in_progress: Mutex<HashMap<String, u32>>,
}

impl CommitMessageManager {
    pub fn new() -> Self {
        Self {
            in_progress: Mutex::new(HashMap::new()),
        }
    }
}

#[tauri::command]
pub async fn generate_commit_message(
    state: State<'_, CommitMessageManager>,
    settings_state: State<'_, SettingsManager>,
    repo_path: String,
) -> Result<String, String> {
    // 重複防止
    {
        let map = state.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        if map.contains_key(&repo_path) {
            return Err("already in progress".to_string());
        }
    }

    let diff = crate::git_worktree::get_diff_text(&repo_path)?;
    let diff_content = if diff.trim().is_empty() {
        "(no changes)".to_string()
    } else {
        // 大きすぎるdiffは先頭20000文字に切り詰める
        if diff.len() > 20000 {
            format!("{}... (truncated)", &diff[..20000])
        } else {
            diff
        }
    };

    // 直近20件のコミットメッセージを取得 (スタイル参照用)
    let recent_commits = {
        let rp = repo_path.clone();
        tokio::task::spawn_blocking(move || {
            crate::process_utils::make_command("git")
                .args(["log", "--oneline", "-20", "--no-decorate"])
                .current_dir(&rp)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    };
    let recent_commits_content = if recent_commits.trim().is_empty() {
        "(none)".to_string()
    } else {
        recent_commits
    };

    let prompt = PROMPT_TEMPLATE
        .replace("{{RECENT_COMMITS}}", &recent_commits_content)
        .replace("{{DIFF}}", &diff_content);

    let agent_kind = settings_state
        .get()
        .ai_agent
        .and_then(|s| s.approval_agent)
        .unwrap_or(AiAgentKind::ClaudeCode);

    let plan = ai_provider::build_execution_plan(&agent_kind, &prompt, JSON_SCHEMA, ai_provider::default_model(&agent_kind), true, Some(1024));

    let mut cmd = crate::process_utils::make_async_command(&plan.program);
    cmd.args(&plan.args);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let worktree_base_dir = settings_state.get().worktree_base_dir.clone();
    if !worktree_base_dir.is_empty() {
        cmd.current_dir(&worktree_base_dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn AI agent: {}", e))?;

    if !plan.stdin_content.is_empty() {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(plan.stdin_content.as_bytes())
                .await
                .map_err(|e| format!("Failed to write stdin: {}", e))?;
        }
    }

    let pid = child.id();
    if let Some(pid) = pid {
        let mut map = state.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        map.insert(repo_path.clone(), pid);
    }

    let wait_result = timeout(
        Duration::from_secs(TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await;

    if wait_result.is_err() {
        let map = state.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        if let Some(&pid) = map.get(&repo_path) {
            crate::process_utils::kill_process_tree(pid);
        }
    }

    {
        let mut map = state.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        map.remove(&repo_path);
    }

    let output = wait_result
        .map_err(|_| format!("AI agent timed out after {}s", TIMEOUT_SECS))?
        .map_err(|e| format!("Failed to wait for AI agent: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("AI agent exited with {}: {}", output.status, stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    log::debug!("[CommitMessage] AI agent response: {}", stdout.trim());

    let structured = ai_provider::parse_response(&agent_kind, &stdout)?;

    let subject = structured
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let body = structured
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if body.is_empty() {
        Ok(subject)
    } else {
        Ok(format!("{}\n\n{}", subject, body))
    }
}

#[tauri::command]
pub async fn cancel_commit_message_generation(
    state: State<'_, CommitMessageManager>,
    repo_path: String,
) -> Result<(), String> {
    let pid = {
        let mut map = state.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        map.remove(&repo_path)
    };

    if let Some(pid) = pid {
        log::debug!("[CommitMessage] cancelling PID={} for repo_path={}", pid, repo_path);
        crate::process_utils::kill_process_tree(pid);
    }

    Ok(())
}
