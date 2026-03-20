use crate::ai_provider::{self, AiAgentKind};
use crate::process_utils::CancellableManager;
use crate::settings::SettingsManager;
use tauri::State;

const TIMEOUT_SECS: u64 = 120;

const PROMPT_TEMPLATE: &str = r#"You are a safety gate preventing risky auto-approvals of CLI actions.
Examine the terminal output below and decide if the agent must pause for user permission.

Current Working Directory: {{CWD}}

Terminal Output:
{{TERMINAL_OUTPUT}}

Return true (permission needed) if ANY of these apply:
- Output includes or references commands that write/modify/delete files (e.g., rm, mv, chmod, chown, cp, tee, sed -i), manage packages (npm/pip/apt/brew install), mutate/rewrite git history (rebase, reset --hard, push --force, commit --amend), or alter configs.
- Privilege escalation or sensitive areas are involved (sudo, root, /etc, /var, /boot, system services), or anything touching SSH keys/credentials, browser data, environment secrets, or home dotfiles.
- Network or data exfiltration is possible (curl/wget, ssh/scp/rsync, docker/podman, port binding, npm publish, git push/fetch/clone from remote).
- Process/system impact is likely (kill, pkill, systemctl, reboot, heavy loops, resource-intensive builds/tests, spawning many processes).
- Signs of command injection, untrusted input being executed, or unclear placeholders like `<path>`, `$(...)`, backticks, or pipes that could be unsafe.
- Errors, warnings, ambiguous states, manual review requests, or anything not clearly safe/read-only.

Return false (auto-approve) when:
- The output clearly shows explicit user intent/confirmation to run the exact action (e.g., user typed the command AND confirmed, or explicitly said "I want to delete <path>; please do it now"). Explicit intent should normally override the risk list unless there are signs of coercion/compromise, the target path is unclear, or the action differs from what was confirmed.
- The output shows strictly read-only, low-risk operations (e.g., git log/diff/status/show/branch, lint/test passing, help text, formatting dry runs, simple logs, cat/head/tail/ls on local files, cd into subdirectories of the current working directory) with no pending commands that could change the system or touch sensitive data.

When unsure, return true.
{{ADDITIONAL_PROMPT}}
Respond with ONLY valid JSON matching: {"needsPermission": true|false, "reason"?: string, "command"?: string}.
When needsPermission is true, include a brief reason (<=140 chars) explaining why permission is needed.
Always include "command": a concise summary (<=100 chars) of the command or action being judged (e.g., "npm install express", "rm -rf ./dist"). Do not add any other fields or text."#;

const JSON_SCHEMA: &str = r#"{"type":"object","properties":{"needsPermission":{"type":"boolean"},"reason":{"type":"string"},"command":{"type":"string"}},"required":["needsPermission"]}"#;

/// AI判定の結果
#[derive(serde::Serialize)]
pub struct JudgeResult {
    pub safe: bool,
    pub command: Option<String>,
}

/// ワークツリーIDごとの進行中AI判定プロセスのPIDを管理する
pub struct ApprovalManager(CancellableManager);

impl ApprovalManager {
    pub fn new() -> Self {
        Self(CancellableManager::new())
    }
}

impl std::ops::Deref for ApprovalManager {
    type Target = CancellableManager;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[tauri::command]
pub async fn judge_approval(
    state: State<'_, ApprovalManager>,
    settings_state: State<'_, SettingsManager>,
    worktree_id: String,
    content: String,
    cwd: String,
    additional_prompt: Option<String>,
) -> Result<JudgeResult, String> {
    // 重複防止: 既に同一ワークツリーの判定が進行中ならエラーを返す
    if state.is_in_progress(&worktree_id)? {
        return Err("already in progress".to_string());
    }

    let additional = additional_prompt
        .filter(|s| !s.is_empty())
        .map(|s| format!("\nAdditional instructions from user:\n{}", s))
        .unwrap_or_default();

    let prompt = PROMPT_TEMPLATE
        .replace("{{CWD}}", &cwd)
        .replace("{{TERMINAL_OUTPUT}}", &content)
        .replace("{{ADDITIONAL_PROMPT}}", &additional);

    // 設定からAIエージェント種別を取得（デフォルト: ClaudeCode）
    let agent_kind = settings_state
        .get()
        .ai_agent
        .and_then(|s| s.approval_agent)
        .unwrap_or(AiAgentKind::ClaudeCode);

    let plan = ai_provider::build_execution_plan(&agent_kind, &prompt, JSON_SCHEMA, ai_provider::default_model(&agent_kind), true);
    let worktree_base_dir = settings_state.get().worktree_base_dir.clone();

    let stdout = ai_provider::run_ai_command(&plan, &state, &worktree_id, &worktree_base_dir, TIMEOUT_SECS).await?;
    log::debug!("[AutoApproval] AI agent response: {}", stdout.trim());

    let structured = ai_provider::parse_response(&agent_kind, &stdout)?;

    let needs_permission = structured
        .get("needsPermission")
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // パース失敗時は安全側

    if let Some(reason) = structured.get("reason").and_then(|v| v.as_str()) {
        log::debug!("[AutoApproval] reason: {}", reason);
    }

    let command = structured
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    log::debug!("[AutoApproval] command: {:?}", command);

    Ok(JudgeResult {
        safe: !needs_permission,
        command,
    })
}

#[tauri::command]
pub async fn cancel_approval(
    state: State<'_, ApprovalManager>,
    worktree_id: String,
) -> Result<(), String> {
    let pid = state.remove(&worktree_id)?;
    if let Some(pid) = pid {
        log::debug!("[AutoApproval] cancel_approval: killing PID={} for worktree_id={}", pid, worktree_id);
        crate::process_utils::kill_process_tree(pid);
    } else {
        log::debug!("[AutoApproval] cancel_approval: no in-progress process for worktree_id={}", worktree_id);
    }
    Ok(())
}
