use serde::Serialize;
use std::collections::HashMap;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Serialize)]
pub struct ScriptResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

fn make_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

pub fn execute_script(script_path: &str, cwd: &str, envs: &HashMap<String, String>) -> Result<ScriptResult, String> {
    log::info!("[execScript] path={}, cwd={}", script_path, cwd);
    for (k, v) in envs {
        log::info!("[execScript] env: {}={}", k, v);
    }

    #[cfg(target_os = "windows")]
    let output = make_command("powershell.exe")
        .args(["-ExecutionPolicy", "Bypass", "-File", script_path])
        .current_dir(cwd)
        .envs(envs)
        .output()
        .map_err(|e| format!("powershell.exe の起動に失敗しました: {}", e))?;

    #[cfg(not(target_os = "windows"))]
    let output = make_command("sh")
        .arg(script_path)
        .current_dir(cwd)
        .envs(envs)
        .output()
        .map_err(|e| format!("sh の起動に失敗しました: {}", e))?;

    let result = ScriptResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };

    log::info!("[execScript] success={}", result.success);
    if !result.stdout.is_empty() {
        log::info!("[execScript] stdout:\n{}", result.stdout);
    }
    if !result.stderr.is_empty() {
        log::warn!("[execScript] stderr:\n{}", result.stderr);
    }

    Ok(result)
}
