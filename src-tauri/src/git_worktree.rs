use crate::process_utils::{kill_external_processes_in_dir, make_command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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

pub fn git_pull(repo_path: &str) -> Result<(), String> {
    // fast-forward pull でローカルブランチを更新する。
    let output = make_command("git")
        .args(["pull", "--ff-only"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git command error: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);

    // upstream が未設定または detached HEAD の場合のみ fetch にフォールバック。
    // dirty branch や diverged branch はエラーをそのまま返す。
    let is_no_upstream = stderr.contains("no tracking information")
        || stderr.contains("There is no tracking information")
        || stderr.contains("no upstream configured")
        || stderr.contains("HEAD detached")
        || stderr.contains("You are not currently on a branch");

    if is_no_upstream {
        run_git_in(repo_path, &["fetch"]).map(|_| ())
    } else {
        Err(format!("git pull --ff-only failed: {}", stderr))
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

/// 与えられたパスを含む git リポジトリのルート（メインワークツリー）を返す。
/// validate_repo は `--is-inside-work-tree` なのでサブディレクトリでも成功する。
/// リポジトリ登録時にここを通してルートへ正規化しないと、`git worktree list` が返す
/// メインワークツリーと登録パスが食い違い、メインワークツリーが「未登録のワークツリー」
/// として取り込み候補に現れてしまう。
pub fn repo_root(path: &str) -> Result<String, String> {
    let stdout = run_git_in(path, &["rev-parse", "--show-toplevel"])?;
    let root = stdout.trim();
    if root.is_empty() {
        return Err("git rev-parse --show-toplevel returned empty".to_string());
    }
    Ok(root.to_string())
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

/// `git worktree list --porcelain` の 1 エントリ。
#[derive(Debug, Clone, Serialize)]
pub struct GitWorktreeInfo {
    pub path: String,
    /// `refs/heads/` を剥がしたブランチ名。detached HEAD や bare では None
    pub branch: Option<String>,
    pub head: Option<String>,
    /// メインの bare リポジトリ本体（ワークツリーとして取り込む対象ではない）
    pub bare: bool,
    pub detached: bool,
    /// メインワークツリー（git は必ず先頭に出力する）。
    /// `git worktree remove` できない特別な存在なので、取り込み対象から必ず外すこと。
    pub is_main: bool,
    /// 実体ディレクトリが消えている等で `git worktree prune` の対象になっている。
    /// 取り込んでも存在しないパスが登録されるだけなので候補から外すこと。
    pub prunable: bool,
    pub locked: bool,
}

/// リポジトリに紐づく git ワークツリーを全件返す（メインワークツリー自身を含む）。
/// パスは git が返す絶対パスそのままなので、oretachi の追加先ディレクトリ外にあるものも拾える。
pub fn list_worktrees(repo_path: &str) -> Result<Vec<GitWorktreeInfo>, String> {
    let stdout = run_git_in(repo_path, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_list(&stdout))
}

/// `git worktree list --porcelain` の出力をパースする。
fn parse_worktree_list(stdout: &str) -> Vec<GitWorktreeInfo> {
    let mut result: Vec<GitWorktreeInfo> = Vec::new();

    // porcelain は 1 エントリが空行区切り。`worktree <path>` が必ずエントリの先頭に来る。
    for line in stdout.lines() {
        let line = line.trim_end();
        if let Some(path) = line.strip_prefix("worktree ") {
            result.push(GitWorktreeInfo {
                path: path.to_string(),
                branch: None,
                head: None,
                bare: false,
                detached: false,
                // git は必ずメインワークツリーを先頭に出力する
                is_main: result.is_empty(),
                prunable: false,
                locked: false,
            });
            continue;
        }
        let Some(current) = result.last_mut() else {
            continue;
        };
        if let Some(head) = line.strip_prefix("HEAD ") {
            current.head = Some(head.to_string());
        } else if let Some(branch) = line.strip_prefix("branch ") {
            current.branch = Some(branch.strip_prefix("refs/heads/").unwrap_or(branch).to_string());
        } else if line == "bare" {
            current.bare = true;
        } else if line == "detached" {
            current.detached = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            // 理由付き (`prunable gitdir file points to non-existent location`) と
            // 理由なしの両方がありうる
            current.prunable = true;
        } else if line == "locked" || line.starts_with("locked ") {
            current.locked = true;
        }
    }

    result
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
        // detached HEAD の場合はブランチ名が取れないため、戻り先としてコミットハッシュを使う
        let original_branch = match current_branch(repo_path) {
            Some(branch) => branch,
            None => {
                let head_output = make_command("git")
                    .args(["rev-parse", "HEAD"])
                    .current_dir(repo_path)
                    .output()
                    .map_err(|e| format!("git command error: {}", e))?;
                String::from_utf8_lossy(&head_output.stdout).trim().to_string()
            }
        };

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

/// ローカルブランチが存在するか（refs/heads/<name> の有無で判定）
pub fn branch_exists(repo_path: &str, branch_name: &str) -> bool {
    let refname = format!("refs/heads/{}", branch_name);
    make_command("git")
        .args(["rev-parse", "--verify", "--quiet", &refname])
        .current_dir(repo_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// ワークツリーがチェックアウトしているブランチ名（detached HEAD の場合は None）
pub fn current_branch(worktree_path: &str) -> Option<String> {
    let output = make_command("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() || name == "HEAD" {
        None
    } else {
        Some(name)
    }
}

/// ブランチを削除する。戻り値は「実際に削除したか」（false = 既に存在せずスキップ）。
/// ブランチが無い状態は目的の状態に到達済みなので成功として扱う（冪等）。
pub fn delete_branch(repo_path: &str, branch_name: &str, force: bool) -> Result<bool, String> {
    if !branch_exists(repo_path, branch_name) {
        log::info!(
            "[worktree] branch '{}' already absent in {}, skip delete",
            branch_name, repo_path
        );
        return Ok(false);
    }

    let flag = if force { "-D" } else { "-d" };
    match run_git_in(repo_path, &["branch", flag, branch_name]) {
        Ok(_) => Ok(true),
        Err(e) => {
            // 存在確認との競合（別プロセスが先に削除した等）は成功扱い
            if e.contains("not found") {
                log::info!(
                    "[worktree] branch '{}' disappeared before delete in {}, treated as deleted",
                    branch_name, repo_path
                );
                return Ok(false);
            }
            Err(e)
        }
    }
}

// ─── コードレビュー用 Git 関数 ───────────────────────────────────────────────

/// QuickOpen 用のファイル一覧を返す。
/// 「追跡 + 未追跡（.gitignore 尊重）」に加え、.gitignore に *ファイルとして直接記載* された
/// ものだけを追加する（例: `.claude/CLAUDE.md`）。`node_modules` 等の ignore ディレクトリ
/// 配下は含めない。
pub fn list_quick_open_files(repo_path: &str) -> Result<Vec<String>, String> {
    let stdout = run_git_in(
        repo_path,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
    )?;
    let mut files: std::collections::HashSet<String> = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    // .gitignore に直接記載されたファイル（glob でもディレクトリでもないもの）を追加
    for entry in read_gitignore(repo_path)? {
        if entry.contains('*')
            || entry.contains('?')
            || entry.contains('[')
            || entry.ends_with('/')
            || entry.contains("..")
        {
            continue;
        }
        let full = std::path::Path::new(repo_path).join(&entry);
        if full.is_file() {
            // パス区切りを forward slash に正規化
            files.insert(entry.replace('\\', "/"));
        }
    }

    let mut result: Vec<String> = files.into_iter().collect();
    result.sort();
    Ok(result)
}

/// ディレクトリ内のエントリ（ツリーの遅延読み込み用）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// 指定ディレクトリ直下のエントリを列挙する。`rel_path` が空文字ならリポジトリルート。
/// `.git` ディレクトリは除外する。ツリーの遅延読み込みに使用。
pub fn list_dir_entries(repo_path: &str, rel_path: &str) -> Result<Vec<DirEntry>, String> {
    // パストラバーサル検証（read_file_content と同じ規則）
    let normalized = std::path::Path::new(rel_path);
    for component in normalized.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err("パスに '..' は使用できません".to_string());
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err("絶対パスは使用できません".to_string());
            }
            _ => {}
        }
    }

    let dir = std::path::Path::new(repo_path).join(rel_path);
    let read = std::fs::read_dir(&dir).map_err(|e| format!("ディレクトリ読み込みエラー: {}", e))?;

    let mut entries: Vec<DirEntry> = Vec::new();
    for item in read {
        let item = match item {
            Ok(i) => i,
            Err(_) => continue,
        };
        let name = item.file_name().to_string_lossy().into_owned();
        // .git は除外
        if name == ".git" {
            continue;
        }
        // シンボリックリンクを辿らずに種別を判定
        let is_dir = match item.file_type() {
            Ok(ft) => ft.is_dir(),
            Err(_) => false,
        };
        let path = if rel_path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", rel_path, name)
        };
        entries.push(DirEntry { name, path, is_dir });
    }

    // ディレクトリ優先 → 名前の大文字小文字無視昇順
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
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
pub struct WorktreeInspection {
    /// 未コミット変更のあるファイル数（staged / unstaged / untracked を重複排除した実ファイル数）。
    /// 0 なら clean。同一ファイルが staged と unstaged の両方に該当しても 1 と数える。
    #[serde(rename = "dirtyCount")]
    pub dirty_count: usize,
    /// 現在のブランチ名（detached HEAD の場合は None）
    pub branch: Option<String>,
    /// マージ済み判定に使ったベースブランチ
    #[serde(rename = "baseBranch")]
    pub base_branch: Option<String>,
    /// ベースブランチにマージ済みなら Some(ベースブランチ名)。未マージ・判定不能なら None
    #[serde(rename = "mergedInto")]
    pub merged_into: Option<String>,
    /// 最終コミット日時 (ISO8601)
    #[serde(rename = "lastCommitAt")]
    pub last_commit_at: Option<String>,
    /// 最終コミットの件名
    #[serde(rename = "lastCommitSubject")]
    pub last_commit_subject: Option<String>,
    /// ベースブランチに対する ahead / behind のコミット数
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

/// ワークツリーの棚卸しに必要な git 情報をまとめて返す。
/// 不要ワークツリー判定（マージ済み・未コミット変更なし・最終更新）の根拠として使う。
///
/// `base_branch` 未指定時は origin/HEAD → main → master の順で既定ブランチを推定する。
/// マージ済み判定やベースブランチ解決に失敗しても全体はエラーにせず、該当フィールドを None にする
/// （リモート未設定・単独ブランチなど正常な構成でも失敗しうるため）。
pub fn inspect_worktree(
    worktree_path: &str,
    base_branch: Option<&str>,
) -> Result<WorktreeInspection, String> {
    // get_status は 1 ファイルにつき staged / unstaged / untracked を別エントリで返すため、
    // パスで重複排除して「変更のあるファイル数」にそろえる
    let dirty_count = get_status(worktree_path)?
        .into_iter()
        .map(|e| e.path)
        .collect::<std::collections::HashSet<_>>()
        .len();

    let branch = run_git_in(worktree_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD");

    let last_commit_at = run_git_in(worktree_path, &["log", "-1", "--format=%cI"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let last_commit_subject = run_git_in(worktree_path, &["log", "-1", "--format=%s"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let resolved_base = base_branch
        .map(|b| b.to_string())
        .or_else(|| detect_base_branch(worktree_path));

    let mut merged_into = None;
    let mut ahead = None;
    let mut behind = None;

    if let (Some(base), Some(cur)) = (resolved_base.as_deref(), branch.as_deref()) {
        if base != cur {
            // マージ済み判定: base に cur の HEAD が含まれているか
            if let Ok(out) = run_git_in(worktree_path, &["branch", "--merged", base]) {
                let is_merged = out
                    .lines()
                    .map(|l| l.trim_start_matches('*').trim())
                    .any(|l| l == cur);
                if is_merged {
                    merged_into = Some(base.to_string());
                }
            }
            // ahead/behind: "<behind>\t<ahead>" 形式
            if let Ok(out) = run_git_in(
                worktree_path,
                &["rev-list", "--left-right", "--count", &format!("{}...{}", base, cur)],
            ) {
                let mut it = out.split_whitespace();
                behind = it.next().and_then(|s| s.parse::<u32>().ok());
                ahead = it.next().and_then(|s| s.parse::<u32>().ok());
            }
        }
    }

    Ok(WorktreeInspection {
        dirty_count,
        branch,
        base_branch: resolved_base,
        merged_into,
        last_commit_at,
        last_commit_subject,
        ahead,
        behind,
    })
}

/// 既定ブランチを推定する。origin/HEAD → main → master の順に候補を試し、
/// **実際に解決できる ref だけを返す**（ローカルブランチが無ければ `origin/<name>` にフォールバック）。
/// 解決できない名前を返すと `branch --merged` / `rev-list` が黙って失敗し、
/// mergedInto や ahead/behind が恒常的に None になってしまうため。
fn detect_base_branch(worktree_path: &str) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(out) = run_git_in(worktree_path, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        if let Some(stripped) = out.trim().strip_prefix("origin/") {
            if !stripped.is_empty() {
                candidates.push(stripped.to_string());
            }
        }
    }
    candidates.push("main".to_string());
    candidates.push("master".to_string());

    for name in candidates {
        if ref_exists(worktree_path, &format!("refs/heads/{}", name)) {
            return Some(name);
        }
        if ref_exists(worktree_path, &format!("refs/remotes/origin/{}", name)) {
            return Some(format!("origin/{}", name));
        }
    }
    None
}

fn ref_exists(worktree_path: &str, full_ref: &str) -> bool {
    run_git_in(worktree_path, &["rev-parse", "--verify", "--quiet", full_ref]).is_ok()
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

/// リポジトリ内の .tsbuildinfo ファイルを検出して相対パスのリストを返す。
/// .git, target は除外。node_modules 内はパッケージディレクトリを除外し、
/// .cache 等のドットディレクトリのみ検索する。
pub fn detect_tsbuildinfo_files(repo_path: &str) -> Result<Vec<String>, String> {
    let repo = std::path::Path::new(repo_path);
    let pattern = format!("{}/**/*.tsbuildinfo", repo_path.replace('\\', "/"));
    let mut results = Vec::new();

    let exclude_dirs = [".git", "target"];

    let iter = match glob::glob(&pattern) {
        Ok(iter) => iter,
        Err(e) => {
            log::warn!("tsbuildinfo glob error: {}", e);
            return Ok(results);
        }
    };

    for entry in iter.filter_map(|r| r.ok()) {
        if !entry.is_file() {
            continue;
        }
        let rel = entry.strip_prefix(repo).unwrap_or(&entry);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if exclude_dirs.iter().any(|d| {
            rel_str.starts_with(&format!("{}/", d)) || rel_str.contains(&format!("/{}/", d))
        }) {
            continue;
        }
        // node_modules 内のパッケージディレクトリを除外（.cache 等のドットディレクトリは許可）
        if is_in_node_modules_package(&rel_str) {
            continue;
        }
        results.push(rel_str);
    }

    Ok(results)
}

/// node_modules/ 直下のパッケージディレクトリ内かどうかを判定する。
/// `node_modules/.cache/` 等のドットディレクトリは許可、それ以外（パッケージ）は除外。
fn is_in_node_modules_package(rel_str: &str) -> bool {
    let nm = "node_modules/";
    let mut search_from = 0;
    while let Some(pos) = rel_str[search_from..].find(nm) {
        let after_nm = search_from + pos + nm.len();
        if after_nm < rel_str.len() {
            let next_char = rel_str.as_bytes()[after_nm];
            // ドットで始まるディレクトリ (.cache 等) は許可、それ以外はパッケージ
            if next_char != b'.' {
                return true;
            }
        }
        search_from = after_nm;
    }
    false
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
    // index にのみ存在する（ワークツリーからは削除された）ステージ済みファイル（AD 等）
    let mut files_to_restore_from_index: Vec<String> = Vec::new();

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
        // rename(R) のみ旧パスをターゲットから削除する（copy(C) は元ファイルが残存するため削除しない）
        let is_rename_or_copy = x == "R" || x == "C";
        if is_rename_or_copy {
            if i < tokens.len() {
                let old_path = tokens[i].to_string();
                i += 1;
                // rename のみ旧パスをターゲットから削除対象に追加
                if x == "R" && !old_path.is_empty() {
                    files_to_delete.push(old_path);
                }
            }
        }

        // index側が削除 / worktree側が削除かで分岐
        let staged_deleted = x == "D";   // index から削除（staged delete）
        let worktree_deleted = y == "D"; // ワークツリーから削除（unstaged delete）

        // index にあるがワークツリーに存在しないファイル（例: AD）は
        // git show でindex版を取得する必要がある
        let has_staged_content = x != " " && x != "?" && !staged_deleted;
        let index_only = has_staged_content && worktree_deleted;

        if staged_deleted {
            // staged delete: ターゲットからも削除
            if !new_path.is_empty() {
                files_to_delete.push(new_path.clone());
            }
        } else if !new_path.is_empty() {
            if index_only {
                // ワークツリーに存在しないがindexにある（AD 等）: index版コピー対象
                files_to_restore_from_index.push(new_path.clone());
            } else if !worktree_deleted {
                files_to_copy.push(new_path.clone());
            }
            // worktree_deleted かつ staged_deleted でない場合は files_to_delete に入れず無視
            // (ADステータスは files_to_restore_from_index で処理済み)
        }

        // ステージ済みファイル（x が空白・?・D 以外）
        if has_staged_content && !new_path.is_empty() && !index_only {
            staged_files.push(new_path.clone());
        }
    }

    // 重複除去
    files_to_copy.sort();
    files_to_copy.dedup();
    files_to_restore_from_index.sort();
    files_to_restore_from_index.dedup();
    staged_files.sort();
    staged_files.dedup();

    /// git status 出力パスに '..' が含まれないことを確認する（パストラバーサル防止）
    fn validate_path(path: &str) -> Result<(), String> {
        for component in std::path::Path::new(path).components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(format!("パストラバーサルを含むパスは使用できません: {}", path));
            }
        }
        Ok(())
    }

    let mut count = 0u32;

    // ファイルをコピー
    for file in &files_to_copy {
        validate_path(file)?;
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

    // index にのみ存在するファイル（AD 等）を git show でindex版から取得してコピー
    for file in &files_to_restore_from_index {
        validate_path(file)?;
        let spec = format!(":{}", file);
        let show_output = make_command("git")
            .args(["show", &spec])
            .current_dir(source_path)
            .output()
            .map_err(|e| format!("git show error for {}: {}", file, e))?;
        if show_output.status.success() {
            let dst_file = target.join(file);
            if let Some(parent) = dst_file.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create dir for {}: {}", file, e))?;
            }
            std::fs::write(&dst_file, &show_output.stdout)
                .map_err(|e| format!("failed to write index content for {}: {}", file, e))?;
            // git add してステージ済み状態を再現
            let _ = run_git_in(target_path, &["add", "--", file]);
            count += 1;
        }
    }

    // 削除されたファイル・リネーム旧ファイルをターゲットからも削除
    for file in &files_to_delete {
        validate_path(file)?;
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

/// 既存ブランチのワークツリーを再作成する（ブランチ削除失敗時のロールバック用）
pub fn worktree_restore(repo_path: &str, worktree_path: &str, branch_name: &str) -> Result<(), String> {
    run_git_in(repo_path, &["worktree", "add", worktree_path, branch_name])?;
    Ok(())
}

/// `git worktree remove` がメインワークツリーを理由に失敗したか。
/// この場合にディレクトリ直接削除へフォールバックすると .git ごとリポジトリ本体を消してしまう。
///
/// 「実体ディレクトリが既に無い」系のエラー (`is not a working tree`) はここに含めない。
/// あちらはフォールバック側の prune 処理で掃除するのが正しい挙動のため。
fn is_main_worktree_error(stderr: &str) -> bool {
    stderr.to_lowercase().contains("is a main working tree")
}

fn is_lock_error(stderr: &str) -> bool {
    // Windows でプロセス終了直後にファイルハンドルが残留している場合に git や OS が返すエラー文言
    let lower = stderr.to_lowercase();
    lower.contains("permission denied")
        || lower.contains("being used by another process")
        || lower.contains("access is denied")
        || lower.contains("cannot remove")
        || lower.contains("failed to remove")
}

pub fn worktree_remove(repo_path: &str, worktree_path: &str) -> Result<(), String> {
    // Windows では PTY プロセス終了直後にファイルハンドルが残留することがあるため、
    // ロック起因のエラーに対してはバックオフ付きリトライを行う（最大5回）。
    // 遅延: 0, 100, 200, 400, 800ms（合計最大 1.5 秒）
    const MAX_ATTEMPTS: u32 = 5;
    let mut last_dir_error = String::new();

    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            let delay_ms = 100u64 * (1 << (attempt - 1)); // 100, 200, 400, 800
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }

        let output = make_command("git")
            .args(["worktree", "remove", "--force", "--force", worktree_path])
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git command error: {}", e))?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);

        // メインワークツリーはディレクトリ直接削除へフォールバックしてはいけない。
        // ここで remove_dir_all に落ちると .git ごとリポジトリ本体が消える。
        if is_main_worktree_error(&stderr) {
            return Err(format!(
                "git worktree remove failed (メインワークツリーのため中止しました): {}",
                stderr.trim()
            ));
        }

        // ロック起因のエラーで試行回数が残っている場合はリトライ
        if attempt < MAX_ATTEMPTS - 1 && is_lock_error(&stderr) {
            log::info!(
                "git worktree remove attempt {}/{} failed due to file lock, retrying: {}",
                attempt + 1, MAX_ATTEMPTS, stderr.trim()
            );
            continue;
        }

        // 最終試行またはロック以外のエラー: ディレクトリ直接削除にフォールバック
        log::warn!("git worktree remove failed (falling back to directory removal): {}", stderr);

        let path = std::path::Path::new(worktree_path);
        if !path.exists() {
            // ディレクトリが既に存在しない場合はメタデータ掃除のみ
            let _ = make_command("git")
                .args(["worktree", "prune"])
                .current_dir(repo_path)
                .output();
            return Ok(());
        }

        match std::fs::remove_dir_all(path) {
            Ok(()) => {
                // メタデータ掃除
                let _ = make_command("git")
                    .args(["worktree", "prune"])
                    .current_dir(repo_path)
                    .output();
                return Ok(());
            }
            Err(e) => {
                last_dir_error = format!("failed to remove worktree directory: {}", e);
                if attempt < MAX_ATTEMPTS - 1 {
                    log::info!(
                        "remove_dir_all attempt {}/{} failed, retrying: {}",
                        attempt + 1, MAX_ATTEMPTS, last_dir_error
                    );
                    // continue でループの先頭に戻り、次の遅延後に git worktree remove を再試行
                }
            }
        }
    }

    Err(last_dir_error)
}

/// ワークツリーを削除する（永続リトライ付き）。
///
/// Phase 1: 既存の `worktree_remove`（最大5回リトライ）を試みる。
/// Phase 2: 失敗した場合、`on_enter_persistent` を呼び出した後、
///          cancel_flag が true になるまで無限にリトライを続ける。
///          各反復で `kill_external_processes_in_dir` を呼びプロセスをkillしてから削除を試みる。
///
/// キャンセルされた場合は `Err("cancelled")` を返す。
pub fn worktree_remove_persistent(
    repo_path: &str,
    worktree_path: &str,
    cancel_flag: Arc<AtomicBool>,
    on_enter_persistent: Option<&dyn Fn()>,
) -> Result<(), String> {
    // Phase 1: 通常の5回リトライ
    let phase1_err = match worktree_remove(repo_path, worktree_path) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };

    // ロック起因エラーの場合のみ永続リトライへ移行する。
    // メタデータ破損・リポジトリパス不正など回復不能なエラーは即座に返す。
    if !is_lock_error(&phase1_err) {
        return Err(phase1_err);
    }

    log::warn!(
        "worktree_remove failed with lock error, entering persistent retry: {}",
        phase1_err
    );

    // Phase 2: 永続リトライ（ロックエラー専用）
    if let Some(cb) = on_enter_persistent {
        cb();
    }

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }

        // CWD ベースのプロセスkill
        let killed = kill_external_processes_in_dir(worktree_path);
        if killed > 0 {
            log::info!(
                "worktree_remove_persistent: killed {} processes in {}",
                killed, worktree_path
            );
            // ファイルハンドル解放を待つ（200ms×5回、各スリープ前にキャンセルチェック）
            for _ in 0..5 {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err("cancelled".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        if cancel_flag.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }

        // ディレクトリが既に無ければメタデータ掃除だけ
        let path = std::path::Path::new(worktree_path);
        if !path.exists() {
            let _ = make_command("git")
                .args(["worktree", "prune"])
                .current_dir(repo_path)
                .output();
            return Ok(());
        }

        // git worktree remove を試みる
        let git_ok = make_command("git")
            .args(["worktree", "remove", "--force", "--force", worktree_path])
            .current_dir(repo_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if git_ok {
            return Ok(());
        }

        // 直接ディレクトリ削除にフォールバック
        match std::fs::remove_dir_all(path) {
            Ok(()) => {
                let _ = make_command("git")
                    .args(["worktree", "prune"])
                    .current_dir(repo_path)
                    .output();
                return Ok(());
            }
            Err(e) => {
                log::info!("worktree_remove_persistent: remove_dir_all failed: {}", e);
            }
        }

        // 2秒待機（200ms×10、各スリープでキャンセルチェック）
        for _ in 0..10 {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err("cancelled".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_worktree_list_handles_main_branch_detached_and_bare() {
        // git worktree list --porcelain の実出力形式（エントリは空行区切り）
        let stdout = "\
worktree C:/repo
HEAD abc123
branch refs/heads/main

worktree X:/devel/worktree/feature-a
HEAD def456
branch refs/heads/feature/a

worktree D:/elsewhere/detached-wt
HEAD 789abc
detached

worktree C:/bare-repo
bare
";
        let list = parse_worktree_list(stdout);
        assert_eq!(list.len(), 4);

        assert_eq!(list[0].path, "C:/repo");
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[0].head.as_deref(), Some("abc123"));
        assert!(!list[0].bare && !list[0].detached);
        // 先頭がメインワークツリー。取り込み候補から外す判定に使う
        assert!(list[0].is_main);
        assert!(!list[1].is_main && !list[2].is_main && !list[3].is_main);

        // refs/heads/ を剥がしつつ、スラッシュを含むブランチ名を壊さない
        assert_eq!(list[1].branch.as_deref(), Some("feature/a"));
        // 追加先ディレクトリ外（別ドライブ）のパスもそのまま拾える
        assert_eq!(list[2].path, "D:/elsewhere/detached-wt");
        assert!(list[2].detached);
        assert_eq!(list[2].branch, None);

        assert!(list[3].bare);
    }

    #[test]
    fn parse_worktree_list_flags_prunable_and_locked() {
        // 実体ディレクトリを消したワークツリーには理由付きの prunable 行が出る。
        // locked も porcelain に出力される（理由の有無は状況次第）。
        let stdout = "\
worktree C:/repo
HEAD abc123
branch refs/heads/main

worktree C:/gone-wt
HEAD def456
branch refs/heads/gone
prunable gitdir file points to non-existent location

worktree C:/locked-wt
HEAD 789abc
branch refs/heads/locked
locked because it is on a removable device

worktree C:/plain-locked
HEAD 111222
detached
locked
";
        let list = parse_worktree_list(stdout);
        assert_eq!(list.len(), 4);
        assert!(!list[0].prunable && !list[0].locked);
        assert!(list[1].prunable, "理由付き prunable を拾えていない");
        assert!(list[2].locked, "理由付き locked を拾えていない");
        assert!(list[3].locked, "理由なし locked を拾えていない");
        // locked は実体が存在するので取り込み対象から外さない
        assert!(!list[2].prunable && !list[3].prunable);
    }

    #[test]
    fn is_main_worktree_error_only_matches_main_worktree() {
        // メインワークツリーはディレクトリ直接削除へフォールバックさせてはいけない
        assert!(is_main_worktree_error(
            "fatal: 'C:/repo' is a main working tree\n"
        ));
        // 実体が既に無いケースはフォールバック側の prune 処理に任せる（誤って止めない）
        assert!(!is_main_worktree_error(
            "fatal: 'C:/gone' is not a working tree\n"
        ));
        assert!(!is_main_worktree_error("error: Permission denied\n"));
    }

    #[test]
    fn parse_worktree_list_returns_empty_for_blank_output() {
        assert!(parse_worktree_list("").is_empty());
        // worktree 行より前の孤立した属性行はどのエントリにも属さず無視される
        assert!(parse_worktree_list("HEAD abc\nbare\n").is_empty());
    }
}
