use crate::process_utils::make_command;
use serde::{Deserialize, Serialize};

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

pub fn detect_ai_agents() -> Vec<AiAgentInfo> {
    let mut result = Vec::new();

    for def in AI_AGENT_DEFINITIONS {
        for &cmd in def.commands {
            #[cfg(target_os = "windows")]
            let which_cmd = "where";
            #[cfg(not(target_os = "windows"))]
            let which_cmd = "which";

            let output = make_command(which_cmd).arg(cmd).output();

            if let Ok(out) = output {
                if out.status.success() {
                    result.push(AiAgentInfo {
                        kind: def.kind.clone(),
                        name: def.name.to_string(),
                        command: cmd.to_string(),
                    });
                    break;
                }
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
) -> AiExecutionPlan {
    match kind {
        AiAgentKind::ClaudeCode => {
            #[cfg(target_os = "windows")]
            let (program, mut args) = (
                "cmd".to_string(),
                vec!["/c".to_string(), "claude".to_string()],
            );
            #[cfg(not(target_os = "windows"))]
            let (program, mut args) = ("claude".to_string(), vec![]);

            args.extend([
                "--model".to_string(),
                model.to_string(),
                "-p".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
                "--json-schema".to_string(),
                json_schema.to_string(),
            ]);

            AiExecutionPlan {
                program,
                args,
                stdin_content: prompt.to_string(),
            }
        }
        AiAgentKind::GeminiCli => {
            #[cfg(target_os = "windows")]
            let (program, mut args) = (
                "cmd".to_string(),
                vec!["/c".to_string(), "gemini".to_string()],
            );
            #[cfg(not(target_os = "windows"))]
            let (program, mut args) = ("gemini".to_string(), vec![]);

            args.extend(["--model".to_string(), model.to_string()]);

            AiExecutionPlan {
                program,
                args,
                stdin_content: format!("{}{}", prompt, json_schema_prompt_suffix(json_schema)),
            }
        }
        AiAgentKind::CodexCli => {
            #[cfg(target_os = "windows")]
            let (program, mut args) = (
                "cmd".to_string(),
                vec!["/c".to_string(), "codex".to_string()],
            );
            #[cfg(not(target_os = "windows"))]
            let (program, mut args) = ("codex".to_string(), vec![]);

            args.extend([
                "--model".to_string(),
                model.to_string(),
                "-q".to_string(),
            ]);

            AiExecutionPlan {
                program,
                args,
                stdin_content: format!("{}{}", prompt, json_schema_prompt_suffix(json_schema)),
            }
        }
        AiAgentKind::ClineCli => {
            let prompt_with_schema =
                format!("{}{}", prompt, json_schema_prompt_suffix(json_schema));

            #[cfg(target_os = "windows")]
            let (program, mut args) = (
                "cmd".to_string(),
                vec!["/c".to_string(), "cline".to_string()],
            );
            #[cfg(not(target_os = "windows"))]
            let (program, mut args) = ("cline".to_string(), vec![]);

            args.extend(["--prompt".to_string(), prompt_with_schema]);

            AiExecutionPlan {
                program,
                args,
                stdin_content: String::new(),
            }
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
        let plan = build_execution_plan(&AiAgentKind::ClaudeCode, "my prompt", "{}", "model-x");
        assert!(plan.args.contains(&"--model".to_string()));
        assert!(plan.args.contains(&"model-x".to_string()));
        assert!(plan.args.contains(&"--output-format".to_string()));
        assert!(plan.args.contains(&"json".to_string()));
        assert!(plan.args.contains(&"--json-schema".to_string()));
        assert_eq!(plan.stdin_content, "my prompt");
    }

    #[test]
    fn test_build_execution_plan_gemini() {
        let plan = build_execution_plan(&AiAgentKind::GeminiCli, "my prompt", "{}", "model-x");
        assert!(plan.args.contains(&"--model".to_string()));
        assert!(plan.stdin_content.contains("my prompt"));
        assert!(plan.stdin_content.contains("{}"));
    }

    #[test]
    fn test_build_execution_plan_codex() {
        let plan = build_execution_plan(&AiAgentKind::CodexCli, "my prompt", "{}", "model-x");
        assert!(plan.args.contains(&"-q".to_string()));
        assert!(plan.stdin_content.contains("my prompt"));
    }

    #[test]
    fn test_build_execution_plan_cline() {
        let plan = build_execution_plan(&AiAgentKind::ClineCli, "my prompt", "{}", "model-x");
        assert!(plan.stdin_content.is_empty());
        assert!(plan.args.contains(&"--prompt".to_string()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_build_execution_plan_windows_uses_cmd() {
        let plan = build_execution_plan(&AiAgentKind::ClaudeCode, "p", "{}", "m");
        assert_eq!(plan.program, "cmd");
        assert!(plan.args.contains(&"/c".to_string()));
        assert!(plan.args.contains(&"claude".to_string()));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_build_execution_plan_non_windows_program() {
        let plan = build_execution_plan(&AiAgentKind::ClaudeCode, "p", "{}", "m");
        assert_eq!(plan.program, "claude");
    }
}
