use serde::{Deserialize, Serialize};
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AiAgentKind {
    ClaudeCode,
    GeminiCli,
    CodexCli,
    ClineCli,
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

fn make_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

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
