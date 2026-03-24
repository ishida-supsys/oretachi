use crate::process_utils::{make_command, make_async_command, CancellableManager};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AiAgentKind {
    ClaudeCode,
    GeminiCli,
    CodexCli,
    ClineCli,
}

pub fn default_model(kind: &AiAgentKind) -> &'static str {
    match kind {
        AiAgentKind::ClaudeCode => "claude-haiku-4-5",
        AiAgentKind::GeminiCli => "gemini-2.5-flash",
        AiAgentKind::CodexCli => "o4-mini",
        AiAgentKind::ClineCli => "",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAgentInfo {
    pub kind: AiAgentKind,
    pub name: String,
    pub command: String,
    pub full_path: Option<String>,
}

struct AiAgentDef {
    kind: AiAgentKind,
    name: &'static str,
    commands: &'static [&'static str],
}

const AI_AGENT_DEFINITIONS: &[AiAgentDef] = &[
    AiAgentDef {
        kind: AiAgentKind::ClaudeCode,
        name: "Claude Code",
        commands: &["claude.cmd", "claude"],
    },
    AiAgentDef {
        kind: AiAgentKind::GeminiCli,
        name: "Gemini CLI",
        commands: &["gemini.cmd", "gemini"],
    },
    AiAgentDef {
        kind: AiAgentKind::CodexCli,
        name: "Codex CLI",
        commands: &["codex.cmd", "codex"],
    },
    AiAgentDef {
        kind: AiAgentKind::ClineCli,
        name: "Cline CLI",
        commands: &["cline.cmd", "cline"],
    },
];

/// コマンド名からフルパスを解決する。見つからなければ None。
fn resolve_command_path(name: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    let which_cmd = "where";
    #[cfg(not(target_os = "windows"))]
    let which_cmd = "which";

    make_command(which_cmd)
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
}

/// AiAgentKind に対応するコマンドのフルパスを解決する。
/// Windows では `.cmd` を先に試す。見つからなければ短いコマンド名を返す。
pub fn resolve_agent_command(kind: &AiAgentKind) -> String {
    let short_name = match kind {
        AiAgentKind::ClaudeCode => "claude",
        AiAgentKind::GeminiCli => "gemini",
        AiAgentKind::CodexCli => "codex",
        AiAgentKind::ClineCli => "cline",
    };

    #[cfg(target_os = "windows")]
    {
        if let Some(path) = resolve_command_path(&format!("{}.cmd", short_name)) {
            return path;
        }
    }

    resolve_command_path(short_name).unwrap_or_else(|| short_name.to_string())
}

pub fn detect_ai_agents() -> Vec<AiAgentInfo> {
    let mut result = Vec::new();

    for def in AI_AGENT_DEFINITIONS {
        for &cmd in def.commands {
            if let Some(full_path) = resolve_command_path(cmd) {
                result.push(AiAgentInfo {
                    kind: def.kind.clone(),
                    name: def.name.to_string(),
                    command: cmd.to_string(),
                    full_path: Some(full_path),
                });
                break;
            }
        }
    }

    result
}

pub struct AiExecutionPlan {
    pub program: String,
    pub args: Vec<String>,
    pub stdin_content: String,
}

/// Windows では `cmd /c <name>`, それ以外では `<name>` を返す
pub fn make_platform_cmd(name: &str) -> (String, Vec<String>) {
    #[cfg(target_os = "windows")]
    return (
        "cmd".to_string(),
        vec!["/c".to_string(), name.to_string()],
    );
    #[cfg(not(target_os = "windows"))]
    return (name.to_string(), vec![]);
}

fn json_schema_prompt_suffix(json_schema: &str) -> String {
    format!(
        "\n\nYou MUST respond with ONLY valid JSON matching this schema: {}\nDo not include any other text, markdown formatting, or code fences.",
        json_schema
    )
}

pub fn build_execution_plan(
    kind: &AiAgentKind,
    prompt: &str,
    json_schema: &str,
    model: &str,
    disable_mcp: bool,
) -> AiExecutionPlan {
    let resolved = resolve_agent_command(kind);
    match kind {
        AiAgentKind::ClaudeCode => {
            let (program, mut args) = make_platform_cmd(&resolved);
            args.extend([
                "--model".to_string(),
                model.to_string(),
                "-p".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
                "--json-schema".to_string(),
                json_schema.to_string(),
            ]);
            if disable_mcp {
                args.push("--strict-mcp-config".to_string());
            }
            AiExecutionPlan { program, args, stdin_content: prompt.to_string() }
        }
        AiAgentKind::GeminiCli => {
            let (program, mut args) = make_platform_cmd(&resolved);
            args.extend(["--model".to_string(), model.to_string()]);
            AiExecutionPlan {
                program,
                args,
                stdin_content: format!("{}{}", prompt, json_schema_prompt_suffix(json_schema)),
            }
        }
        AiAgentKind::CodexCli => {
            let (program, mut args) = make_platform_cmd(&resolved);
            args.extend(["--model".to_string(), model.to_string(), "-q".to_string()]);
            AiExecutionPlan {
                program,
                args,
                stdin_content: format!("{}{}", prompt, json_schema_prompt_suffix(json_schema)),
            }
        }
        AiAgentKind::ClineCli => {
            let (program, mut args) = make_platform_cmd(&resolved);
            args.extend(["--prompt".to_string(), format!("{}{}", prompt, json_schema_prompt_suffix(json_schema))]);
            AiExecutionPlan { program, args, stdin_content: String::new() }
        }
    }
}

pub fn parse_response(kind: &AiAgentKind, stdout: &str) -> Result<serde_json::Value, String> {
    match kind {
        AiAgentKind::ClaudeCode => {
            let response: serde_json::Value = serde_json::from_str(stdout.trim())
                .map_err(|e| format!("Failed to parse JSON: {}", e))?;
            let structured = response.get("structured_output").unwrap_or(&response).clone();
            Ok(structured)
        }
        AiAgentKind::GeminiCli | AiAgentKind::CodexCli | AiAgentKind::ClineCli => {
            extract_json_from_text(stdout).ok_or_else(|| {
                format!(
                    "No JSON found in {} output",
                    match kind {
                        AiAgentKind::GeminiCli => "Gemini CLI",
                        AiAgentKind::CodexCli => "Codex CLI",
                        AiAgentKind::ClineCli => "Cline CLI",
                        _ => unreachable!(),
                    }
                )
            })
        }
    }
}

/// AI コマンドを実行して stdout を返す共通ヘルパー。
/// - プロセスのスポーン・stdin 書き込み・PID 登録・タイムアウト待機・キャンセル処理を一元化。
pub async fn run_ai_command(
    plan: &AiExecutionPlan,
    state: &CancellableManager,
    key: &str,
    working_dir: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let mut cmd = make_async_command(&plan.program);
    cmd.args(&plan.args);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if !working_dir.is_empty() {
        cmd.current_dir(working_dir);
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

    if let Some(pid) = child.id() {
        state.register(key.to_string(), pid)?;
    }

    let wait_result = timeout(
        Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await;

    if wait_result.is_err() {
        let _ = state.cancel(key);
    }
    let _ = state.remove(key);

    let output = wait_result
        .map_err(|_| format!("AI agent timed out after {}s", timeout_secs))?
        .map_err(|e| format!("Failed to wait for AI agent: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("AI agent exited with {}: {}", output.status, stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn extract_json_from_text(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let chars: Vec<char> = text[start..].chars().collect();
    let mut end = None;

    for (i, &c) in chars.iter().enumerate() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let end = end?;
    let json_str: String = chars[..=end].iter().collect();
    serde_json::from_str(&json_str).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_pure() {
        let text = r#"{"key": "value"}"#;
        let result = extract_json_from_text(text);
        assert!(result.is_some());
        assert_eq!(result.unwrap()["key"], "value");
    }

    #[test]
    fn test_extract_json_with_surrounding_text() {
        let text = r#"Here is the result: {"key": "value"} and some extra."#;
        let result = extract_json_from_text(text);
        assert!(result.is_some());
        assert_eq!(result.unwrap()["key"], "value");
    }

    #[test]
    fn test_extract_json_nested() {
        let text = r#"{"outer": {"inner": 42}}"#;
        let result = extract_json_from_text(text);
        assert!(result.is_some());
        assert_eq!(result.unwrap()["outer"]["inner"], 42);
    }

    #[test]
    fn test_extract_json_no_json() {
        let text = "no json here";
        assert!(extract_json_from_text(text).is_none());
    }

    #[test]
    fn test_extract_json_invalid() {
        let text = "{invalid json}";
        assert!(extract_json_from_text(text).is_none());
    }

    #[test]
    fn test_parse_response_claude_code_with_structured_output() {
        let stdout = r#"{"structured_output": {"result": "ok"}}"#;
        let result = parse_response(&AiAgentKind::ClaudeCode, stdout);
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["result"], "ok");
    }

    #[test]
    fn test_parse_response_claude_code_without_structured_output() {
        let stdout = r#"{"result": "ok"}"#;
        let result = parse_response(&AiAgentKind::ClaudeCode, stdout);
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["result"], "ok");
    }

    #[test]
    fn test_parse_response_gemini_with_json() {
        let stdout = r#"Some text {"result": "ok"} more text"#;
        let result = parse_response(&AiAgentKind::GeminiCli, stdout);
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["result"], "ok");
    }

    #[test]
    fn test_parse_response_gemini_no_json() {
        let stdout = "no json here";
        let result = parse_response(&AiAgentKind::GeminiCli, stdout);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No JSON found"));
    }

    #[test]
    fn test_parse_response_codex_no_json() {
        let result = parse_response(&AiAgentKind::CodexCli, "plain text");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_response_cline_no_json() {
        let result = parse_response(&AiAgentKind::ClineCli, "plain text");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_execution_plan_claude_code() {
        let plan = build_execution_plan(&AiAgentKind::ClaudeCode, "my prompt", "{}", "model-x", false);
        assert!(plan.args.contains(&"--model".to_string()));
        assert!(plan.args.contains(&"model-x".to_string()));
        assert!(plan.args.contains(&"--output-format".to_string()));
        assert!(plan.args.contains(&"json".to_string()));
        assert!(plan.args.contains(&"--json-schema".to_string()));
        assert_eq!(plan.stdin_content, "my prompt");
        assert!(!plan.args.contains(&"--max-tokens".to_string()));
    }

    #[test]
    fn test_build_execution_plan_gemini() {
        let plan = build_execution_plan(&AiAgentKind::GeminiCli, "my prompt", "{}", "model-x", false);
        assert!(plan.args.contains(&"--model".to_string()));
        assert!(plan.stdin_content.contains("my prompt"));
        assert!(plan.stdin_content.contains("{}"));
    }

    #[test]
    fn test_build_execution_plan_codex() {
        let plan = build_execution_plan(&AiAgentKind::CodexCli, "my prompt", "{}", "model-x", false);
        assert!(plan.args.contains(&"-q".to_string()));
        assert!(plan.stdin_content.contains("my prompt"));
    }

    #[test]
    fn test_build_execution_plan_cline() {
        let plan = build_execution_plan(&AiAgentKind::ClineCli, "my prompt", "{}", "model-x", false);
        assert!(plan.stdin_content.is_empty());
        assert!(plan.args.contains(&"--prompt".to_string()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_build_execution_plan_windows_uses_cmd() {
        let plan = build_execution_plan(&AiAgentKind::ClaudeCode, "p", "{}", "m", false);
        assert_eq!(plan.program, "cmd");
        assert!(plan.args.contains(&"/c".to_string()));
        // resolve_agent_command によりフルパスまたは短い名前が入る
        assert!(plan.args.iter().any(|a| a.contains("claude")));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_build_execution_plan_non_windows_program() {
        let plan = build_execution_plan(&AiAgentKind::ClaudeCode, "p", "{}", "m", false);
        assert_eq!(plan.program, "claude");
    }
}
