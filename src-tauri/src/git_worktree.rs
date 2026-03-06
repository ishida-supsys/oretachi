use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn make_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

pub fn validate_repo(path: &str) -> Result<bool, String> {
    let output = make_command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    Ok(output.status.success())
}

pub fn worktree_add(
    repo_path: &str,
    worktree_path: &str,
    branch_name: &str,
) -> Result<(), String> {
    let output = make_command("git")
        .args(["worktree", "add", "-b", branch_name, worktree_path])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git worktree add failed: {}", stderr))
    }
}

pub fn worktree_remove(repo_path: &str, worktree_path: &str) -> Result<(), String> {
    let output = make_command("git")
        .args(["worktree", "remove", "--force", "--force", worktree_path])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    log::warn!("git worktree remove failed (falling back to directory removal): {}", stderr);

    let path = std::path::Path::new(worktree_path);
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("failed to remove worktree directory: {}", e))?;
    }

    // メタデータ掃除
    let _ = make_command("git")
        .args(["worktree", "prune"])
        .current_dir(repo_path)
        .output();

    Ok(())
}
