use crate::process_utils::make_command;

pub fn get_git_remotes(repo_path: &str) -> Vec<serde_json::Value> {
    let output = make_command("git")
        .args(["remote", "-v"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut seen = std::collections::HashMap::<String, String>::new();
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    seen.entry(parts[0].to_string())
                        .or_insert_with(|| parts[1].to_string());
                }
            }
            seen.into_iter()
                .map(|(name, url)| serde_json::json!({"name": name, "url": url}))
                .collect()
        }
        _ => vec![],
    }
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
) -> Result<bool, String> {
    let output = make_command("git")
        .args(["worktree", "add", "-b", branch_name, worktree_path])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if output.status.success() {
        return Ok(false);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);

    // LFS smudge filterエラーでなければそのまま返す
    if !stderr.contains("smudge") && !stderr.contains("filter") {
        return Err(format!("git worktree add failed: {}", stderr));
    }

    log::warn!(
        "git worktree add failed due to LFS smudge filter, retrying with GIT_LFS_SKIP_SMUDGE=1: {}",
        stderr
    );

    // クリーンアップ: 失敗したワークツリーとブランチを除去
    let _ = make_command("git")
        .args(["worktree", "remove", "--force", worktree_path])
        .current_dir(repo_path)
        .output();

    let path = std::path::Path::new(worktree_path);
    if path.exists() {
        let _ = std::fs::remove_dir_all(path);
    }

    let _ = make_command("git")
        .args(["worktree", "prune"])
        .current_dir(repo_path)
        .output();

    let _ = make_command("git")
        .args(["branch", "-D", branch_name])
        .current_dir(repo_path)
        .output();

    // LFS smudgeをスキップしてリトライ
    let retry_output = make_command("git")
        .args(["worktree", "add", "-b", branch_name, worktree_path])
        .current_dir(repo_path)
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if retry_output.status.success() {
        Ok(true)
    } else {
        let retry_stderr = String::from_utf8_lossy(&retry_output.stderr);
        Err(format!("git worktree add failed: {}", retry_stderr))
    }
}

pub fn list_branches(repo_path: &str) -> Result<Vec<String>, String> {
    let output = make_command("git")
        .args(["branch", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git branch failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let branches = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(branches)
}

fn find_branch_worktree(repo_path: &str, branch_name: &str) -> Result<Option<String>, String> {
    let output = make_command("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut current_path: Option<String> = None;

    for line in stdout.lines() {
        if line.starts_with("worktree ") {
            current_path = Some(line["worktree ".len()..].to_string());
        } else if line.starts_with("branch refs/heads/") {
            let b = &line["branch refs/heads/".len()..];
            if b == branch_name {
                return Ok(current_path);
            }
        }
    }
    Ok(None)
}

pub fn merge_branch(repo_path: &str, source_branch: &str, target_branch: &str) -> Result<(), String> {
    if let Some(target_worktree_path) = find_branch_worktree(repo_path, target_branch)? {
        // target_branch がチェックアウトされているワークツリーで直接 merge
        let merge_output = make_command("git")
            .args(["merge", source_branch, "--no-edit"])
            .current_dir(&target_worktree_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;

        if !merge_output.status.success() {
            let _ = make_command("git")
                .args(["merge", "--abort"])
                .current_dir(&target_worktree_path)
                .output();
            let stderr = String::from_utf8_lossy(&merge_output.stderr);
            return Err(format!("git merge failed: {}", stderr));
        }
    } else {
        // target_branch がどのワークツリーにもチェックアウトされていない → repo_path で checkout して merge
        let head_output = make_command("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;
        let original_branch = String::from_utf8_lossy(&head_output.stdout).trim().to_string();

        let checkout_output = make_command("git")
            .args(["checkout", target_branch])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;

        if !checkout_output.status.success() {
            let stderr = String::from_utf8_lossy(&checkout_output.stderr);
            return Err(format!("git checkout failed: {}", stderr));
        }

        let merge_output = make_command("git")
            .args(["merge", source_branch, "--no-edit"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;

        if !merge_output.status.success() {
            let _ = make_command("git")
                .args(["merge", "--abort"])
                .current_dir(repo_path)
                .output();
            let _ = make_command("git")
                .args(["checkout", &original_branch])
                .current_dir(repo_path)
                .output();
            let stderr = String::from_utf8_lossy(&merge_output.stderr);
            return Err(format!("git merge failed: {}", stderr));
        }

        let _ = make_command("git")
            .args(["checkout", &original_branch])
            .current_dir(repo_path)
            .output();
    }

    Ok(())
}

pub fn delete_branch(repo_path: &str, branch_name: &str, force: bool) -> Result<(), String> {
    let flag = if force { "-D" } else { "-d" };
    let output = make_command("git")
        .args(["branch", flag, branch_name])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git branch {} failed: {}", flag, stderr));
    }

    Ok(())
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
