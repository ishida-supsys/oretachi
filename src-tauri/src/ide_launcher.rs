use crate::process_utils::make_command;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct IdeInfo {
    pub id: String,
    pub name: String,
    pub command: String,
}

// (id, display_name, candidate_commands)
const IDE_DEFINITIONS: &[(&str, &str, &[&str])] = &[
    ("cursor", "Cursor", &["cursor.cmd", "cursor"]),
    ("vscode", "VS Code", &["code.cmd", "code"]),
    ("antigravity", "Antigravity", &["antigravity.cmd", "antigravity"]),
];

pub fn detect_ides() -> Vec<IdeInfo> {
    let mut result = Vec::new();

    for &(id, name, candidates) in IDE_DEFINITIONS {
        for &cmd in candidates {
            #[cfg(target_os = "windows")]
            let which_cmd = "where";
            #[cfg(not(target_os = "windows"))]
            let which_cmd = "which";

            let output = make_command(which_cmd)
                .arg(cmd)
                .output();

            if let Ok(out) = output {
                if out.status.success() {
                    result.push(IdeInfo {
                        id: id.to_string(),
                        name: name.to_string(),
                        command: cmd.to_string(),
                    });
                    break; // 最初に見つかったコマンドを使用
                }
            }
        }
    }

    result
}

pub fn open_in_ide(command: &str, path: &str) -> Result<(), String> {
    // ホワイトリスト: IDE_DEFINITIONS に含まれるコマンド名のみ許可
    let allowed: Vec<&str> = IDE_DEFINITIONS.iter()
        .flat_map(|(_, _, candidates)| candidates.iter().copied())
        .collect();
    if !allowed.contains(&command) {
        return Err(format!("許可されていないIDEコマンドです: {}", command));
    }
    make_command(command)
        .args(["--reuse-window", path])
        .spawn()
        .map_err(|e| format!("IDE の起動に失敗しました: {}", e))?;
    Ok(())
}
