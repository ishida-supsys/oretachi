use crate::ai_provider::{self, AiAgentKind};
use crate::process_utils::CancellableManager;
use crate::settings::SettingsManager;
use tauri::State;

const PROMPT_TEMPLATE: &str = r#"You are an expert at writing extremely concise worktree descriptions.
The following is an implementation plan that an AI agent produced for a git worktree.
Summarize WHAT this worktree is about into a single short line.

Plan:
{{PLAN}}

Rules:
- Output a single line, no line breaks
- Use the SAME language as the plan (if the plan is Japanese, answer in Japanese)
- Keep it short and scannable (roughly 15-40 characters for Japanese, ~60 chars for English)
- Describe the goal/subject, not the steps
- No trailing punctuation, no quotes, no prefixes like "概要:" or "Summary:"

Respond with ONLY valid JSON: {"description": "..."}.
Do not add any other fields or text."#;

const JSON_SCHEMA: &str = r#"{"type":"object","properties":{"description":{"type":"string"}},"required":["description"]}"#;

pub struct DescriptionManager(CancellableManager);

impl DescriptionManager {
    pub fn new() -> Self {
        Self(CancellableManager::new())
    }
}

impl std::ops::Deref for DescriptionManager {
    type Target = CancellableManager;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// プラン（ExitPlanMode で渡されたプラン本文）を AI 要約して、
/// ワークツリーの説明に使える 1 行テキストを返す。
#[tauri::command]
pub async fn generate_description_from_plan(
    state: State<'_, DescriptionManager>,
    settings_state: State<'_, SettingsManager>,
    worktree_id: String,
    plan: String,
) -> Result<String, String> {
    // ワークツリー単位で重複実行を防止（複数ワークツリーの並行 ExitPlanMode に対応）
    if state.is_in_progress(&worktree_id)? {
        return Err("already in progress".to_string());
    }

    // 大きすぎるプランは先頭 20000 文字に切り詰める（バイト境界スライスはマルチバイトで
    // パニックするため文字単位で切る）
    if plan.trim().is_empty() {
        return Err("plan is empty".to_string());
    }
    let plan_content = if plan.chars().count() > 20000 {
        let truncated: String = plan.chars().take(20000).collect();
        format!("{}... (truncated)", truncated)
    } else {
        plan
    };

    let prompt = PROMPT_TEMPLATE.replace("{{PLAN}}", &plan_content);

    let agent_kind = settings_state
        .get()
        .ai_agent
        .and_then(|s| s.approval_agent)
        .unwrap_or(AiAgentKind::ClaudeCode);

    let exec = ai_provider::build_execution_plan(
        &agent_kind,
        &prompt,
        JSON_SCHEMA,
        ai_provider::default_model(&agent_kind),
        true,
    );
    let worktree_base_dir = settings_state.get().worktree_base_dir.clone();

    let stdout = ai_provider::run_ai_command(
        &exec,
        &state,
        &worktree_id,
        &worktree_base_dir,
        settings_state.get().get_ai_timeout_secs(),
    )
    .await?;
    log::debug!("[Description] AI agent response: {}", stdout.trim());

    let structured = ai_provider::parse_response(&agent_kind, &stdout)?;

    let description = structured
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if description.is_empty() {
        return Err("AI returned empty description".to_string());
    }
    Ok(description)
}

#[tauri::command]
pub async fn cancel_description_generation(
    state: State<'_, DescriptionManager>,
    worktree_id: String,
) -> Result<(), String> {
    let pid = state.remove(&worktree_id)?;
    if let Some(pid) = pid {
        log::debug!("[Description] cancelling PID={} for worktree_id={}", pid, worktree_id);
        crate::process_utils::kill_process_tree(pid);
    }
    Ok(())
}
