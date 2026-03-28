use crate::process_utils::make_command;
use crate::settings::NotificationHookEntry;
use serde::Serialize;

/// git コマンドを repo_path で実行して stdout を返す共通ヘルパー
fn run_git_in(repo_path: &str, args: &[&str]) -> Result<String, String> {
    let output = make_command("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut msg = format!("git {} failed: {}", args.join(" "), stderr);
        // index.lock 競合の場合にリトライを促すヒントを追加
        if stderr.contains("index.lock") || stderr.contains("Unable to create") {
            msg.push_str("\n（別の git 操作が進行中の可能性があります。しばらく待ってから再試行してください）");
        }
        return Err(msg);
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

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

/// リモート名を抽出する: "<remote>/<branch>" 形式の場合にリモート名を返す
fn extract_remote_name(repo_path: &str, branch: &str) -> Option<String> {
    if !branch.contains('/') {
        return None;
    }
    let remotes = get_git_remotes(repo_path);
    for remote in &remotes {
        if let Some(name) = remote["name"].as_str() {
            let prefix = format!("{}/", name);
            if branch.starts_with(&prefix) {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub fn worktree_add(
    repo_path: &str,
    worktree_path: &str,
    branch_name: &str,
    source_branch: Option<&str>,
) -> Result<bool, String> {
    // リモートブランチが指定された場合はフェッチ
    if let Some(sb) = source_branch {
        if let Some(remote) = extract_remote_name(repo_path, sb) {
            let branch_part = &sb[remote.len() + 1..];
            // refs/remotes/<remote>/<branch> を明示的に更新して worktree add で参照できるようにする
            let refspec = format!("+{}:refs/remotes/{}/{}", branch_part, remote, branch_part);
            log::info!("[worktree_add] fetching remote={} refspec={}", remote, refspec);
            let fetch_output = make_command("git")
                .args(["fetch", &remote, &refspec])
                .current_dir(repo_path)
                .output()
                .map_err(|e| format!("git fetch error: {}", e))?;
            if !fetch_output.status.success() {
                let stderr = String::from_utf8_lossy(&fetch_output.stderr);
                return Err(format!("git fetch {}/{} failed: {}", remote, branch_part, stderr));
            }
        }
    }

    let mut args = vec!["worktree", "add", "-b", branch_name, worktree_path];
    if let Some(sb) = source_branch {
        args.push(sb);
    }

    let output = make_command("git")
        .args(&args)
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
        .args(&args)
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
    let stdout = run_git_in(repo_path, &["branch", "--format=%(refname:short)"])?;
    let branches = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(branches)
}

fn find_branch_worktree(repo_path: &str, branch_name: &str) -> Result<Option<String>, String> {
    let stdout = run_git_in(repo_path, &["worktree", "list", "--porcelain"])?;
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
    run_git_in(repo_path, &["branch", flag, branch_name])?;
    Ok(())
}

// ─── コードレビュー用 Git 関数 ───────────────────────────────────────────────

pub fn list_tracked_files(repo_path: &str) -> Result<Vec<String>, String> {
    let stdout = run_git_in(repo_path, &["ls-files"])?;
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
    // file_path にパストラバーサル用コンポーネントが含まれていないか検証
    let normalized = std::path::Path::new(file_path);
    for component in normalized.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err("ファイルパスに '..' は使用できません".to_string());
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err("絶対パスは使用できません".to_string());
            }
            _ => {}
        }
    }

    if let Some(rev) = revision {
        // revision がオプションインジェクション（'-' で始まる）になっていないか確認
        if rev.starts_with('-') {
            return Err("revision にハイフンで始まる値は使用できません".to_string());
        }
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

pub fn get_merge_message(repo_path: &str) -> Result<Option<String>, String> {
    let output = make_command("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        return Err("Not a git repository".to_string());
    }

    let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let git_dir_path = std::path::Path::new(repo_path).join(&git_dir);

    if !git_dir_path.join("MERGE_HEAD").exists() {
        return Ok(None);
    }

    let merge_msg_path = git_dir_path.join("MERGE_MSG");
    match std::fs::read_to_string(&merge_msg_path) {
        Ok(content) => Ok(Some(content)),
        Err(_) => Ok(Some(String::new())),
    }
}

pub fn get_status(repo_path: &str) -> Result<Vec<GitStatusEntry>, String> {
    let stdout = run_git_in(repo_path, &["status", "--porcelain=v1", "-uall"])?;
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
    let format = "%H%x00%h%x00%an%x00%ai%x00%s%x00%P%x00%D%x1e";
    let fmt_arg = format!("--format={}", format);
    let skip_arg = format!("--skip={}", skip);
    let limit_arg = format!("-n{}", limit);

    let stdout = run_git_in(repo_path, &["log", "--all", &fmt_arg, &skip_arg, &limit_arg])?;
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

#[derive(Serialize)]
pub struct CommitFileEntry {
    pub path: String,
    pub status: String,
    pub old_path: Option<String>,
}

pub fn get_commit_files(repo_path: &str, hash: &str) -> Result<Vec<CommitFileEntry>, String> {
    if hash.starts_with('-') {
        return Err("hash にハイフンで始まる値は使用できません".to_string());
    }

    // first parent hash を取得してマージコミットを正確に処理する
    let parent_output = make_command("git")
        .args(["log", "--pretty=%P", "-1", hash])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;
    let first_parent = if parent_output.status.success() {
        let s = String::from_utf8_lossy(&parent_output.stdout);
        s.split_whitespace().next().unwrap_or("").to_string()
    } else {
        String::new()
    };

    // 初回コミット: diff-tree --root / それ以外: git diff <first-parent> <hash>
    // (-m を使わないことでマージコミットでも first-parent との差分のみを正確に一覧表示)
    let stdout = if first_parent.is_empty() {
        run_git_in(
            repo_path,
            &["diff-tree", "--no-commit-id", "-r", "--root", "--name-status", hash],
        )?
    } else {
        run_git_in(
            repo_path,
            &["diff", "--name-status", &first_parent, hash],
        )?
    };
    let mut entries = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.splitn(3, '\t').collect();
        if fields.is_empty() {
            continue;
        }
        let status_raw = fields[0].trim();
        let status_char = status_raw.chars().next().unwrap_or('M');
        let status = status_char.to_string();
        // R / C はフィールドが 3 つ: status, old_path, new_path
        if (status_char == 'R' || status_char == 'C') && fields.len() == 3 {
            let old = fields[1].trim();
            let new = fields[2].trim();
            if new.is_empty() {
                continue;
            }
            entries.push(CommitFileEntry {
                path: new.to_string(),
                status,
                old_path: Some(old.to_string()),
            });
        } else if fields.len() >= 2 {
            let path = fields[1].trim();
            if path.is_empty() {
                continue;
            }
            entries.push(CommitFileEntry { path: path.to_string(), status, old_path: None });
        }
    }
    Ok(entries)
}

pub fn get_commit_file_diff(repo_path: &str, hash: &str, file_path: &str, old_file_path: Option<&str>) -> Result<FileDiff, String> {
    if hash.starts_with('-') {
        return Err("hash にハイフンで始まる値は使用できません".to_string());
    }
    for path_to_check in [file_path].iter().chain(old_file_path.iter()) {
        let normalized = std::path::Path::new(path_to_check);
        for component in normalized.components() {
            match component {
                std::path::Component::ParentDir => {
                    return Err("ファイルパスに '..' は使用できません".to_string());
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    return Err("絶対パスは使用できません".to_string());
                }
                _ => {}
            }
        }
    }

    // parent hash を取得
    let parent_output = make_command("git")
        .args(["log", "--pretty=%P", "-1", hash])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;
    let parent_hash = if parent_output.status.success() {
        let s = String::from_utf8_lossy(&parent_output.stdout);
        s.split_whitespace().next().unwrap_or("").to_string()
    } else {
        String::new()
    };

    // リネーム/コピーの場合は parent 側のパス (old_file_path) を使う
    let parent_path = old_file_path.unwrap_or(file_path);

    let old_bytes = if parent_hash.is_empty() {
        // 初回コミット: old は空
        vec![]
    } else {
        let spec = format!("{}:{}", parent_hash, parent_path);
        let output = make_command("git")
            .args(["show", &spec])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;
        if output.status.success() { output.stdout } else { vec![] }
    };

    let new_spec = format!("{}:{}", hash, file_path);
    let new_output = make_command("git")
        .args(["show", &new_spec])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;
    let new_bytes = if new_output.status.success() { new_output.stdout } else { vec![] };

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
    run_git_in(repo_path, &["add", "-A"])?;
    Ok(())
}

pub fn commit(repo_path: &str, message: &str) -> Result<String, String> {
    run_git_in(repo_path, &["commit", "-m", message])?;
    let stdout = run_git_in(repo_path, &["rev-parse", "--short", "HEAD"])?;
    Ok(stdout.trim().to_string())
}

pub fn detect_package_manager(repo_path: &str) -> Result<Vec<String>, String> {
    let path = std::path::Path::new(repo_path);
    let mut detected = Vec::new();
    if path.join("pnpm-lock.yaml").exists() {
        detected.push("pnpm".to_string());
    }
    if path.join("package-lock.json").exists() {
        detected.push("npm".to_string());
    }
    if path.join("yarn.lock").exists() {
        detected.push("yarn".to_string());
    }
    if path.join("bun.lockb").exists() || path.join("bun.lock").exists() {
        detected.push("bun".to_string());
    }
    Ok(detected)
}

pub fn read_gitignore(repo_path: &str) -> Result<Vec<String>, String> {
    let gitignore_path = std::path::Path::new(repo_path).join(".gitignore");
    if !gitignore_path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(&gitignore_path)
        .map_err(|e| format!("failed to read .gitignore: {}", e))?;
    let entries = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
        .map(|l| l.to_string())
        .collect();
    Ok(entries)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<u32, String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("failed to create dir {:?}: {}", dst, e))?;
    let mut count = 0u32;
    for entry in std::fs::read_dir(src).map_err(|e| format!("failed to read dir {:?}: {}", src, e))? {
        let entry = entry.map_err(|e| format!("dir entry error: {}", e))?;
        let ty = entry.file_type().map_err(|e| format!("file type error: {}", e))?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            count += copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(&entry.path(), &dst_path)
                .map_err(|e| format!("failed to copy {:?}: {}", entry.path(), e))?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn copy_gitignore_targets(
    repo_path: &str,
    worktree_path: &str,
    targets: Vec<String>,
) -> Result<u32, String> {
    let repo = std::path::Path::new(repo_path);
    let worktree = std::path::Path::new(worktree_path);
    let mut total = 0u32;

    for target in &targets {
        let normalized = target.replace('\\', "/").trim_end_matches('/').to_string();
        let repo_unix = repo_path.replace('\\', "/");

        // '/' を含まないパターン（例: *.local, node_modules）は再帰パターンも追加
        let has_slash = normalized.contains('/');
        let mut patterns = vec![format!("{}/{}", repo_unix, normalized)];
        if !has_slash {
            patterns.push(format!("{}/**/{}", repo_unix, normalized));
        }

        // 各パターンをglob展開して重複を除去したパスセットを構築
        let mut matched: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        for pattern in &patterns {
            match glob::glob(pattern) {
                Ok(iter) => {
                    for path in iter.filter_map(|r| r.ok()) {
                        matched.insert(path);
                    }
                }
                Err(e) => {
                    log::warn!("invalid glob pattern '{}': {}", pattern, e);
                }
            }
        }

        if matched.is_empty() {
            log::debug!("copy target not found, skipping: {}", normalized);
            continue;
        }

        for src in matched {
            if !src.exists() {
                continue;
            }
            let rel = src.strip_prefix(repo).unwrap_or(&src);
            let dst = worktree.join(rel);
            if src.is_dir() {
                total += copy_dir_recursive(&src, &dst)?;
            } else {
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create parent dir: {}", e))?;
                }
                std::fs::copy(&src, &dst)
                    .map_err(|e| format!("failed to copy {:?}: {}", src, e))?;
                total += 1;
            }
        }
    }

    Ok(total)
}

/// ソースワークツリーの未コミット変更（ステージ済み・未ステージ・untracked）をターゲットへコピーする。
/// ソースには副作用を与えない（stash不使用）。
pub fn copy_working_changes(source_path: &str, target_path: &str) -> Result<u32, String> {
    // -z: NUL区切り・引用符なし出力（スペースや非ASCII文字を含むパス名に対応）
    let output = make_command("git")
        .args(["status", "--porcelain=v1", "-uall", "-z"])
        .current_dir(source_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    if raw.trim_matches('\0').is_empty() {
        return Ok(0);
    }

    let source = std::path::Path::new(source_path);
    let target = std::path::Path::new(target_path);

    let mut files_to_copy: Vec<String> = Vec::new();
    let mut staged_files: Vec<String> = Vec::new();
    let mut files_to_delete: Vec<String> = Vec::new();

    // NUL区切りでトークン列にする
    // リネーム/コピーエントリのフォーマット: "XY new-path\0old-path\0"
    // 通常エントリのフォーマット: "XY path\0"
    let tokens: Vec<&str> = raw.split('\0').collect();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        i += 1;

        if token.len() < 3 {
            continue;
        }

        let x = &token[..1]; // index (staged) status
        let y = &token[1..2]; // worktree (unstaged) status
        let new_path = token[3..].to_string();

        // staged rename/copy の場合、次のトークンが旧パス
        // 旧パスはターゲットから削除する（リネーム後に旧ファイルが残らないように）
        let is_rename_or_copy = x == "R" || x == "C";
        if is_rename_or_copy {
            if i < tokens.len() {
                let old_path = tokens[i].to_string();
                i += 1;
                // 旧パスをターゲットから削除対象に追加
                if !old_path.is_empty() {
                    files_to_delete.push(old_path);
                }
            }
        }

        let is_deleted = x == "D" || y == "D";

        if is_deleted {
            files_to_delete.push(new_path.clone());
        } else if !new_path.is_empty() {
            files_to_copy.push(new_path.clone());
        }

        // ステージ済みファイル（x が空白・?・D 以外）
        if x != " " && x != "?" && x != "D" && !new_path.is_empty() {
            staged_files.push(new_path.clone());
        }
    }

    // 重複除去
    files_to_copy.sort();
    files_to_copy.dedup();
    staged_files.sort();
    staged_files.dedup();

    let mut count = 0u32;

    // ファイルをコピー
    for file in &files_to_copy {
        let src_file = source.join(file);
        let dst_file = target.join(file);
        if src_file.exists() {
            if let Some(parent) = dst_file.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create dir for {}: {}", file, e))?;
            }
            std::fs::copy(&src_file, &dst_file)
                .map_err(|e| format!("failed to copy {}: {}", file, e))?;
            count += 1;
        }
    }

    // 削除されたファイル・リネーム旧ファイルをターゲットからも削除
    for file in &files_to_delete {
        let dst_file = target.join(file);
        if dst_file.exists() {
            let _ = std::fs::remove_file(&dst_file);
        }
    }

    // ステージ済み状態を復元
    if !staged_files.is_empty() {
        let mut args: Vec<&str> = vec!["add", "--"];
        let refs: Vec<&str> = staged_files.iter().map(|s| s.as_str()).collect();
        args.extend(&refs);
        // エラーは無視（ファイルが存在しない場合など）
        let _ = run_git_in(target_path, &args);
    }

    Ok(count)
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

/// ワークツリーの `.claude/settings.local.json` に Claude Code 通知フックを書き込む
pub fn write_claude_hooks(
    worktree_path: &str,
    worktree_name: &str,
    hooks: Vec<NotificationHookEntry>,
) -> Result<(), String> {
    use std::path::Path;

    let claude_dir = Path::new(worktree_path).join(".claude");
    std::fs::create_dir_all(&claude_dir)
        .map_err(|e| format!("Failed to create .claude dir: {}", e))?;

    let settings_path = claude_dir.join("settings.local.json");

    // 既存ファイルを読み込む（存在しなければ空オブジェクト）
    let mut json: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)
            .map_err(|e| format!("Failed to read settings.local.json: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse settings.local.json: {}", e))?
    } else {
        serde_json::json!({})
    };

    // oretachi.exe のパスを取得してforward slashに統一
    let exe_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "oretachi".to_string());

    // oretachiが管理するイベント一覧（このリスト外は保持）
    const MANAGED_EVENTS: &[&str] =
        &["Stop", "Notification", "SubagentStop", "PreToolUse", "PostToolUse", "PermissionRequest"];

    // 既存のhooksオブジェクトを取得し、oretachiが管理するeventキーのみ上書き（他は保持）
    let mut hooks_obj = json
        .get("hooks")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    // 有効なイベントを上書き
    for entry in &hooks {
        let command = format!(
            "\"{}\" --notify \"{}\" --kind {}",
            exe_path, worktree_name, entry.kind
        );
        let hook_entry = serde_json::json!([{
            "matcher": "",
            "hooks": [{ "type": "command", "command": command }]
        }]);
        hooks_obj.insert(entry.event.clone(), hook_entry);
    }

    // oretachi管理イベントのうち無効化されたものを既存hooksから削除
    let enabled: std::collections::HashSet<&str> = hooks.iter().map(|h| h.event.as_str()).collect();
    for &ev in MANAGED_EVENTS {
        if !enabled.contains(ev) {
            hooks_obj.remove(ev);
        }
    }

    json["hooks"] = serde_json::Value::Object(hooks_obj);

    let content = serde_json::to_string_pretty(&json)
        .map_err(|e| format!("Failed to serialize settings.local.json: {}", e))?;
    std::fs::write(&settings_path, content)
        .map_err(|e| format!("Failed to write settings.local.json: {}", e))?;

    Ok(())
}

/// パスを Claude Code のプロジェクトディレクトリ名に変換する
/// CCManager と同じロジック: `/`, `\`, `.` をすべて `-` に置換
fn path_to_claude_project_name(path: &str) -> String {
    path.replace('/', "-").replace('\\', "-").replace('.', "-").replace(':', "-")
}

/// ソースワークツリーの Claude Code セッションデータをターゲットにコピーする
/// `~/.claude/projects/[encoded-source]/` → `~/.claude/projects/[encoded-target]/`
pub fn copy_claude_session_data(
    source_worktree_path: &str,
    target_worktree_path: &str,
) -> Result<u32, String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Could not determine home directory".to_string())?;
    let projects_dir = std::path::Path::new(&home).join(".claude").join("projects");

    let source_name = path_to_claude_project_name(source_worktree_path);
    let target_name = path_to_claude_project_name(target_worktree_path);
    let source_dir = projects_dir.join(&source_name);
    let target_dir = projects_dir.join(&target_name);

    if !source_dir.exists() {
        log::info!("[copy_claude_session] source not found, skipping: {:?}", source_dir);
        return Ok(0);
    }

    // ソースとターゲットが同じディレクトリなら自己コピーになるためスキップ
    if source_name == target_name {
        log::info!("[copy_claude_session] source and target are identical, skipping");
        return Ok(0);
    }

    log::info!("[copy_claude_session] copying {:?} -> {:?}", source_dir, target_dir);

    // 一時ディレクトリにコピーしてから置換することで、コピー失敗時のデータ損失を防ぐ
    let tmp_dir = projects_dir.join(format!("{}_tmp_{}", target_name, std::process::id()));
    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)
            .map_err(|e| format!("failed to remove stale tmp dir: {}", e))?;
    }

    let count = copy_dir_recursive(&source_dir, &tmp_dir)?;

    // コピー成功後に既存ターゲットを削除してから一時ディレクトリを移動
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)
            .map_err(|e| format!("failed to remove existing target session dir: {}", e))?;
    }
    std::fs::rename(&tmp_dir, &target_dir)
        .map_err(|e| format!("failed to rename tmp dir to target: {}", e))?;

    Ok(count)
}
