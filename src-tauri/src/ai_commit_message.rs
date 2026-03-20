use crate::ai_provider::{self, AiAgentKind};
use crate::process_utils::CancellableManager;
use crate::settings::SettingsManager;
use tauri::State;

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

pub type CommitMessageManager = CancellableManager;

#[tauri::command]
pub async fn generate_commit_message(
    state: State<'_, CommitMessageManager>,
    settings_state: State<'_, SettingsManager>,
    repo_path: String,
) -> Result<String, String> {
    // 重複防止
    if state.is_in_progress(&repo_path)? {
        return Err("already in progress".to_string());
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

    let plan = ai_provider::build_execution_plan(&agent_kind, &prompt, JSON_SCHEMA, ai_provider::default_model(&agent_kind), true);
    let worktree_base_dir = settings_state.get().worktree_base_dir.clone();

    let stdout = ai_provider::run_ai_command(&plan, &state, &repo_path, &worktree_base_dir, TIMEOUT_SECS).await?;
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
    let pid = state.remove(&repo_path)?;
    if let Some(pid) = pid {
        log::debug!("[CommitMessage] cancelling PID={} for repo_path={}", pid, repo_path);
        crate::process_utils::kill_process_tree(pid);
    }
    Ok(())
}
