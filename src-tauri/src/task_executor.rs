use crate::ai_provider::{self, AiAgentKind};
use crate::mcp_server::McpServerManager;
use crate::settings::SettingsManager;
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;
use tokio::time::{timeout, Duration};

const TIMEOUT_SECS: u64 = 120;

const SYSTEM_PROMPT_TEMPLATE: &str = r#"You are a task planner acting as both the manager of AI agents and the contact point
for user requests. Follow these steps to understand the relationship between the
request and repositories, and perform appropriate worktree operations.

1. Retrieve current state: Use oretachi_list_repository to get available repositories,
   and use oretachi_get_worktree_status to get the list of existing worktrees.
2. Generate a list of specific tasks from the request and select the repository for
   each task.
3. Generate the task process code and output it as JSON.

## Task List Generation Rules
- When an issue URL is specified, compare it with the remote information from
  oretachi_list_repository to select the repository. Do NOT look into the issue or
  pull request contents - only compare repository names and remote information. Leave
  precise matching to later flows.
- The "prompt" field in agent_worktree MUST contain the user's original words verbatim.
  Do NOT rephrase, summarize, elaborate, or make the request more specific.
- If the request maps to a single task, copy the entire user request as-is into the prompt field.
- If splitting into multiple tasks, extract the relevant portion of the user's original text for each
  task. Do not rewrite or interpret it.
- NEVER generate your own instructions or ideas to put in the prompt. Your role is routing and
  splitting only — the downstream AI agent will interpret the prompt.
- When only an issue URL is provided or context is unclear, pass the full text as-is.
- By default, each task creates a new worktree (add_worktree + agent_worktree).
- Use an EXISTING worktree (agent_worktree only, no add_worktree) in these cases:
  - A pull request URL is provided: fetch the PR's branch name and check if it matches
    an existing worktree's branch. If it matches, use that worktree.
  - The user explicitly refers to a previous task (e.g. "continue the task for X"):
    use the existing worktree only if the repository AND branch/name exactly match.
    Do not reuse a worktree just because the repository is the same.

## Task Process Code Schema
{
  "code": [
    { "type": "add_worktree", "repository": "<repo_name>", "branch": "<branch_name>", "source_branch": "<source_branch_optional>" },
    { "type": "agent_worktree", "repository": "<repo_name>", "branch": "<branch_name>", "prompt": "<instruction>" }
  ]
}

- add_worktree: Add a worktree for the specified repository and branch. Optionally specify source_branch to base the new worktree on a specific branch (e.g. "main", "origin/develop"). If source_branch starts with a remote name (e.g. "origin/..."), a fetch will be performed automatically.
- agent_worktree: Launch an AI agent on the worktree's terminal with the given prompt. The worktree must already exist (either pre-existing or created via add_worktree).
- When you want to add a NEW worktree and launch an agent, output BOTH add_worktree and agent_worktree as separate entries in the code array, in order.
- When targeting an EXISTING worktree, output only agent_worktree (no add_worktree needed).
- Branch names for new worktrees should follow the pattern "worktree/{descriptive-name}".
- Repository names must match exactly what is in the repository list.

## User Request
{{USER_PROMPT}}"#;

const JSON_SCHEMA: &str = r#"{"type":"object","properties":{"code":{"type":"array","items":{"oneOf":[{"type":"object","properties":{"type":{"const":"add_worktree"},"repository":{"type":"string"},"branch":{"type":"string"},"source_branch":{"type":"string"}},"required":["type","repository","branch"]},{"type":"object","properties":{"type":{"const":"agent_worktree"},"repository":{"type":"string"},"branch":{"type":"string"},"prompt":{"type":"string"}},"required":["type","repository","branch","prompt"]}]}}},"required":["code"]}"#;

fn build_worktree_list_text(settings: &crate::settings::AppSettings) -> String {
    if settings.worktrees.is_empty() {
        return "Existing worktrees:\n(none)".to_string();
    }
    let lines: Vec<String> = settings
        .worktrees
        .iter()
        .map(|wt| {
            format!(
                "- {} (repository: {}, branch: {})",
                wt.name, wt.repository_name, wt.branch_name
            )
        })
        .collect();
    format!("Existing worktrees:\n{}", lines.join("\n"))
}

fn build_repo_list_text(settings: &crate::settings::AppSettings) -> String {
    if settings.repositories.is_empty() {
        return "Available repositories:\n(none)".to_string();
    }
    let lines: Vec<String> = settings
        .repositories
        .iter()
        .map(|repo| {
            let remotes = crate::git_worktree::get_git_remotes(&repo.path);
            let remote_strs: Vec<String> = remotes
                .iter()
                .filter_map(|r| {
                    let name = r["name"].as_str()?;
                    let url = r["url"].as_str()?;
                    Some(format!("{}={}", name, url))
                })
                .collect();
            if remote_strs.is_empty() {
                format!("- {} (no remotes)", repo.name)
            } else {
                format!("- {} (remotes: {})", repo.name, remote_strs.join(", "))
            }
        })
        .collect();
    format!("Available repositories:\n{}", lines.join("\n"))
}

#[tauri::command]
pub async fn task_generate(
    _app_handle: AppHandle,
    settings_state: State<'_, SettingsManager>,
    mcp_state: State<'_, McpServerManager>,
    prompt: String,
) -> Result<String, String> {
    let settings = settings_state.get();
    let mcp_status = mcp_state.get_status();

    let agent_kind = settings
        .ai_agent
        .as_ref()
        .and_then(|s| s.approval_agent.as_ref())
        .cloned()
        .unwrap_or(AiAgentKind::ClaudeCode);

    let full_prompt = SYSTEM_PROMPT_TEMPLATE.replace("{{USER_PROMPT}}", &prompt);

    let use_mcp =
        agent_kind == AiAgentKind::ClaudeCode && mcp_status.running && mcp_status.port.is_some();

    // For non-MCP agents, embed repo list and worktree list directly in prompt
    let final_prompt = if use_mcp {
        full_prompt
    } else {
        format!(
            "{}\n\n{}\n\n{}",
            full_prompt,
            build_repo_list_text(&settings),
            build_worktree_list_text(&settings)
        )
    };

    // Build command and args
    let (program, args, stdin_content) = if use_mcp {
        let port = mcp_status.port.unwrap();

        let mcp_config = serde_json::json!({
            "mcpServers": {
                "oretachi": {
                    "type": "http",
                    "url": format!("http://127.0.0.1:{}/mcp", port)
                }
            }
        });
        let config_str = serde_json::to_string(&mcp_config).map_err(|e| {
            log::error!("[TaskGenerate] Failed to serialize MCP config: {}", e);
            format!("Failed to serialize MCP config: {}", e)
        })?;

        let config_path =
            std::env::temp_dir().join(format!("oretachi-task-mcp-{}.json", std::process::id()));
        std::fs::write(&config_path, &config_str).map_err(|e| {
            log::error!("[TaskGenerate] Failed to write MCP config: {}", e);
            format!("Failed to write MCP config: {}", e)
        })?;

        let (prog, mut a) = ai_provider::make_platform_cmd(&ai_provider::resolve_agent_command(&AiAgentKind::ClaudeCode));

        a.extend([
            "-p".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--json-schema".to_string(),
            JSON_SCHEMA.to_string(),
            "--mcp-config".to_string(),
            config_path.to_string_lossy().to_string(),
        ]);

        (prog, a, final_prompt)
    } else {
        let plan = ai_provider::build_execution_plan(
            &agent_kind,
            &final_prompt,
            JSON_SCHEMA,
            ai_provider::default_model(&agent_kind),
            false,
        );
        (plan.program, plan.args, plan.stdin_content)
    };

    let worktree_base_dir = settings.worktree_base_dir.clone();

    let mut cmd = crate::process_utils::make_async_command(&program);
    cmd.args(&args);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if !worktree_base_dir.is_empty() {
        cmd.current_dir(&worktree_base_dir);
    }

    let mut child = cmd.spawn().map_err(|e| {
        log::error!("[TaskGenerate] Failed to spawn AI agent: {}", e);
        format!("Failed to spawn AI agent: {}", e)
    })?;

    if !stdin_content.is_empty() {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(stdin_content.as_bytes())
                .await
                .map_err(|e| {
                    log::error!("[TaskGenerate] Failed to write stdin: {}", e);
                    format!("Failed to write stdin: {}", e)
                })?;
        }
    }

    let pid = child.id();

    let wait_result = timeout(Duration::from_secs(TIMEOUT_SECS), child.wait_with_output()).await;

    // タイムアウト時はプロセスをkill
    if wait_result.is_err() {
        if let Some(pid) = pid {
            crate::process_utils::kill_process_tree(pid);
        }
    }

    let output = wait_result
        .map_err(|_| {
            log::error!("[TaskGenerate] AI agent timed out after {}s", TIMEOUT_SECS);
            format!("AI agent timed out after {}s", TIMEOUT_SECS)
        })?
        .map_err(|e| {
            log::error!("[TaskGenerate] Failed to wait for AI agent: {}", e);
            format!("Failed to wait for AI agent: {}", e)
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!(
            "[TaskGenerate] AI agent exited with {}: {}",
            output.status,
            stderr
        );
        return Err(format!(
            "AI agent exited with {}: {}",
            output.status, stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    log::debug!("[TaskGenerate] AI agent response: {}", stdout.trim());

    let structured = ai_provider::parse_response(&agent_kind, &stdout)?;

    serde_json::to_string(&structured).map_err(|e| {
        log::error!("[TaskGenerate] Failed to serialize response: {}", e);
        format!("Failed to serialize response: {}", e)
    })
}

#[tauri::command]
pub fn write_temp_prompt(content: String) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!(
        "oretachi-prompt-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::write(&path, content.as_bytes())
        .map_err(|e| format!("Failed to write temp file: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AppSettings, WorktreeEntry};

    fn make_worktree_entry(name: &str, repo_name: &str, branch: &str) -> WorktreeEntry {
        WorktreeEntry {
            id: "1".to_string(),
            name: name.to_string(),
            repository_id: "repo1".to_string(),
            repository_name: repo_name.to_string(),
            path: "/path".to_string(),
            branch_name: branch.to_string(),
            hotkey_char: None,
            auto_approval: None,
            auto_approval_prompt: None,
        }
    }

    #[test]
    fn test_build_worktree_list_text_empty() {
        let settings = AppSettings::default();
        let text = build_worktree_list_text(&settings);
        assert!(text.contains("(none)"));
        assert!(text.contains("Existing worktrees:"));
    }

    #[test]
    fn test_build_worktree_list_text_with_entries() {
        let mut settings = AppSettings::default();
        settings.worktrees = vec![make_worktree_entry(
            "my-feature",
            "myrepo",
            "worktree/my-feature",
        )];
        let text = build_worktree_list_text(&settings);
        assert!(text.contains("my-feature"));
        assert!(text.contains("myrepo"));
        assert!(!text.contains("(none)"));
    }

    #[test]
    fn test_build_worktree_list_text_multiple_entries() {
        let mut settings = AppSettings::default();
        settings.worktrees = vec![
            make_worktree_entry("feat-a", "repo-x", "worktree/feat-a"),
            make_worktree_entry("feat-b", "repo-y", "worktree/feat-b"),
        ];
        let text = build_worktree_list_text(&settings);
        assert!(text.contains("feat-a"));
        assert!(text.contains("repo-x"));
        assert!(text.contains("feat-b"));
        assert!(text.contains("repo-y"));
    }
}
