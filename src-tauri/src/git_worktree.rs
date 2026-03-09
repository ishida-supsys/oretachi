use crate::process_utils::make_command;
use serde::Serialize;

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

// ─── コードレビュー用 Git 関数 ───────────────────────────────────────────────

pub fn list_tracked_files(repo_path: &str) -> Result<Vec<String>, String> {
    let output = make_command("git")
        .args(["ls-files"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git ls-files failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(files)
}

pub fn read_file_content(
    repo_path: &str,
    file_path: &str,
    revision: Option<&str>,
) -> Result<String, String> {
    if let Some(rev) = revision {
        let spec = format!("{}:{}", rev, file_path);
        let output = make_command("git")
            .args(["show", &spec])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git show failed: {}", stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let full_path = std::path::Path::new(repo_path).join(file_path);
        // バイナリファイルや巨大ファイルのガード (10MB)
        let meta = std::fs::metadata(&full_path)
            .map_err(|e| format!("file metadata error: {}", e))?;
        if meta.len() > 10 * 1024 * 1024 {
            return Err(format!("file too large: {} bytes", meta.len()));
        }
        std::fs::read_to_string(&full_path).map_err(|e| format!("file read error: {}", e))
    }
}

#[derive(Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    pub status: String,
    pub staged: bool,
}

pub fn get_status(repo_path: &str) -> Result<Vec<GitStatusEntry>, String> {
    let output = make_command("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git status failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        if line.len() < 3 {
            continue;
        }
        let xy = &line[..2];
        let path = line[3..].to_string();
        let x = &xy[..1]; // index (staged)
        let y = &xy[1..]; // worktree (unstaged)

        // staged change (index != ' ' && index != '?')
        if x != " " && x != "?" {
            entries.push(GitStatusEntry {
                path: path.clone(),
                status: x.to_string(),
                staged: true,
            });
        }
        // unstaged change (worktree != ' ' && worktree != '?')
        if y != " " && y != "?" {
            entries.push(GitStatusEntry {
                path: path.clone(),
                status: y.to_string(),
                staged: false,
            });
        }
        // untracked
        if xy == "??" {
            entries.push(GitStatusEntry {
                path: path.clone(),
                status: "??".to_string(),
                staged: false,
            });
        }
    }

    Ok(entries)
}

#[derive(Serialize)]
pub struct FileDiff {
    pub old_content: String,
    pub new_content: String,
    pub is_binary: bool,
}

pub fn get_file_diff(repo_path: &str, file_path: &str, staged: bool) -> Result<FileDiff, String> {
    let old_bytes = {
        let spec = format!("HEAD:{}", file_path);
        let output = make_command("git")
            .args(["show", &spec])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;
        if output.status.success() { output.stdout } else { vec![] }
    };

    let new_bytes = if staged {
        // staged: インデックスの内容
        let spec = format!(":{}", file_path);
        let output = make_command("git")
            .args(["show", &spec])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;
        if output.status.success() { output.stdout } else { vec![] }
    } else {
        // unstaged: ワーキングツリーの内容
        let full_path = std::path::Path::new(repo_path).join(file_path);
        std::fs::read(&full_path).unwrap_or_default()
    };

    if content_inspector::inspect(&old_bytes).is_binary()
        || content_inspector::inspect(&new_bytes).is_binary()
    {
        return Ok(FileDiff { old_content: String::new(), new_content: String::new(), is_binary: true });
    }

    Ok(FileDiff {
        old_content: String::from_utf8_lossy(&old_bytes).into_owned(),
        new_content: String::from_utf8_lossy(&new_bytes).into_owned(),
        is_binary: false,
    })
}

#[derive(Serialize)]
pub struct CommitEntry {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub parents: Vec<String>,
    pub refs: Vec<String>,
}

pub fn get_log(repo_path: &str, skip: usize, limit: usize) -> Result<Vec<CommitEntry>, String> {
    // セパレータを使ってフィールドを区切る
    let format = "%H%x00%h%x00%an%x00%ai%x00%s%x00%P%x00%D%x1e";
    let skip_arg = format!("--skip={}", skip);
    let limit_arg = format!("-n{}", limit);

    let output = make_command("git")
        .args([
            "log",
            "--all",
            &format!("--format={}", format),
            &skip_arg,
            &limit_arg,
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for record in stdout.split('\x1e') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.split('\x00').collect();
        if fields.len() < 7 {
            continue;
        }
        let parents = fields[5]
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let refs = fields[6]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        entries.push(CommitEntry {
            hash: fields[0].to_string(),
            short_hash: fields[1].to_string(),
            author: fields[2].to_string(),
            date: fields[3].to_string(),
            message: fields[4].to_string(),
            parents,
            refs,
        });
    }

    Ok(entries)
}

pub fn get_diff_text(repo_path: &str) -> Result<String, String> {
    // ステージ済み + 未ステージの全差分を取得
    let staged = make_command("git")
        .args(["diff", "--cached"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    let unstaged = make_command("git")
        .args(["diff"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    let mut result = String::new();
    if staged.status.success() {
        result.push_str(&String::from_utf8_lossy(&staged.stdout));
    }
    if unstaged.status.success() {
        result.push_str(&String::from_utf8_lossy(&unstaged.stdout));
    }
    Ok(result)
}

pub fn stage_all(repo_path: &str) -> Result<(), String> {
    let output = make_command("git")
        .args(["add", "-A"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git add -A failed: {}", stderr));
    }
    Ok(())
}

pub fn commit(repo_path: &str, message: &str) -> Result<String, String> {
    let output = make_command("git")
        .args(["commit", "-m", message])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git commit failed: {}", stderr));
    }

    // short hash を返す
    let hash_output = make_command("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    Ok(String::from_utf8_lossy(&hash_output.stdout).trim().to_string())
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
