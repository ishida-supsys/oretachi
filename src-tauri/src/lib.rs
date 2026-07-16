mod ai_commit_message;
mod ai_description;
mod ai_judge;
mod ai_provider;
mod archive_db;
mod claude_plugin;
mod claude_plugin_skills;
mod fs_watcher;
mod git_worktree;
mod ide_launcher;
mod job_object;
mod main_thread_watch;
pub mod mcp_server;
mod process_utils;
mod pty_manager;
mod report_db;
mod settings;
mod system_metrics;
mod task_db;
mod task_executor;
mod terminal_session;

#[cfg(target_os = "windows")]
mod acrylic;

#[cfg(target_os = "windows")]
mod redirection_guard;

use fs_watcher::FsWatcherManager;
use process_utils::WorktreeRemoveManager;
use pty_manager::{AiAgentChangedPayload, PtyManager};
use settings::{AppSettings, SettingsManager};
use system_metrics::SystemMetricsState;
use tauri::{Emitter, Listener, Manager, State};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

/// パスコンポーネントに `..`、絶対パス区切り文字、NULバイトが含まれていないか検証する
fn validate_path_component(s: &str) -> Result<(), String> {
    if s.contains("..") || s.contains('/') || s.contains('\\') || s.contains('\0') || s.contains(':') {
        return Err(format!("不正なパス文字が含まれています: {}", s));
    }
    Ok(())
}

fn artifacts_dir(
    app_handle: &tauri::AppHandle,
    worktree_id: &str,
) -> Result<std::path::PathBuf, String> {
    validate_path_component(worktree_id)?;
    Ok(app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("artifacts")
        .join(worktree_id))
}

// ─── PTY コマンド ────────────────────────────────────────────────────────────

#[tauri::command]
async fn pty_spawn(
    app_handle: tauri::AppHandle,
    state: State<'_, PtyManager>,
    rows: u16,
    cols: u16,
    shell: Option<String>,
    cwd: Option<String>,
) -> Result<u32, String> {
    log::debug!("[Terminal] cmd::pty_spawn rows={} cols={} shell={:?} cwd={:?}", rows, cols, shell, cwd);
    // ConPTY 生成 + プロセス spawn はブロッキング I/O のため spawn_blocking で実行し、
    // 同期コマンドとしてメインスレッド (WebView2 UI スレッド) を塞がないようにする。
    let manager = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.spawn(app_handle, rows, cols, shell, cwd))
        .await
        .map_err(|e| format!("spawn_blocking join error: {}", e))?
}

// pty_write は enqueue のみで非ブロッキング (実 I/O はセッション毎の writer スレッド)。
// 同期コマンドのままにすることで、メインスレッド上の実行順 = チャネル投入順となり
// キー入力の順序が保証される (async 化すると並列実行で順序が壊れうる)。
#[tauri::command]
fn pty_write(state: State<PtyManager>, session_id: u32, data: Vec<u8>) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::PtyWrite);
    state.write(session_id, data)
}

#[tauri::command]
async fn pty_resize(state: State<'_, PtyManager>, session_id: u32, rows: u16, cols: u16) -> Result<(), String> {
    log::debug!("[Terminal] cmd::pty_resize session_id={} rows={} cols={}", session_id, rows, cols);
    // ConPTY の resize は conhost への RPC でブロックしうるため spawn_blocking で実行
    let manager = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.resize(session_id, rows, cols))
        .await
        .map_err(|e| format!("spawn_blocking join error: {}", e))?
}

#[tauri::command]
async fn pty_kill(state: State<'_, PtyManager>, session_id: u32) -> Result<(), String> {
    log::info!("[Terminal] cmd::pty_kill session_id={} source=webview-invoke", session_id);
    // taskkill /F /T は数秒かかることがあるため spawn_blocking で実行
    let manager = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.kill(session_id, "webview-invoke"))
        .await
        .map_err(|e| format!("spawn_blocking join error: {}", e))?
}

#[tauri::command]
fn pty_set_ai_agent(
    app_handle: tauri::AppHandle,
    state: State<PtyManager>,
    session_id: u32,
    is_agent: bool,
) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::PtySetAiAgent);
    state.set_ai_agent(session_id, is_agent)?;
    let mut sessions = std::collections::HashMap::new();
    sessions.insert(session_id, pty_manager::AiAgentInfo {
        is_agent,
        agent_name: None,
        session_id: None,
    });
    let _ = app_handle.emit("pty-ai-agent-changed", AiAgentChangedPayload { sessions });
    Ok(())
}

#[tauri::command]
fn pty_is_ai_agent(state: State<PtyManager>, session_id: u32) -> Result<bool, String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::PtyIsAiAgent);
    state.is_ai_agent(session_id)
}

// ─── 設定コマンド ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: State<SettingsManager>) -> AppSettings {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::GetSettings);
    state.get()
}

#[tauri::command]
fn save_settings(state: State<SettingsManager>, settings: AppSettings) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::SaveSettings);
    state.save(settings)
}

#[tauri::command]
fn apply_acrylic_effect(app_handle: tauri::AppHandle, r: u8, g: u8, b: u8, a: u8) {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::ApplyAcrylic);
    #[cfg(target_os = "windows")]
    {
        for (_, window) in app_handle.webview_windows() {
            if let Ok(hwnd) = window.hwnd() {
                acrylic::setup(hwnd.0, r, g, b, a);
            }
        }
    }
    let _ = (app_handle, r, g, b, a);
}

// ─── Git コマンド ─────────────────────────────────────────────────────────────

async fn run_git<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("task join error: {}", e))?
}

#[tauri::command]
async fn git_validate_repo(path: String) -> Result<bool, String> {
    run_git(move || git_worktree::validate_repo(&path)).await
}

#[tauri::command]
async fn git_pull(repo_path: String) -> Result<(), String> {
    run_git(move || git_worktree::git_pull(&repo_path)).await
}

#[tauri::command]
async fn git_worktree_add(
    app_handle: tauri::AppHandle,
    repo_path: String,
    worktree_path: String,
    branch_name: String,
    source_branch: Option<String>,
) -> Result<bool, String> {
    let bn = branch_name.clone();
    let result = run_git(move || git_worktree::worktree_add(&repo_path, &worktree_path, &branch_name, source_branch.as_deref())).await?;
    if let Some(pool) = app_handle.try_state::<report_db::ReportPool>() {
        let _ = report_db::insert(&pool.0, "worktree_change:add", &bn).await;
    }
    Ok(result)
}

#[tauri::command]
async fn git_worktree_remove(
    app_handle: tauri::AppHandle,
    pty_manager: State<'_, PtyManager>,
    remove_manager: State<'_, WorktreeRemoveManager>,
    repo_path: String,
    worktree_path: String,
) -> Result<(), String> {
    // 削除対象ディレクトリをcwdとして掴んでいるPTYセッションを先にkill
    // (taskkill 最大10秒 + watcher join を含むため tokio ワーカーを塞がないよう spawn_blocking)
    let killed = {
        let manager = pty_manager.inner().clone();
        let dir = worktree_path.clone();
        tauri::async_runtime::spawn_blocking(move || manager.kill_sessions_in_dir(&dir))
            .await
            .map_err(|e| format!("spawn_blocking join error: {}", e))?
    };
    if killed > 0 {
        // プロセスが完全に終了してファイルハンドルを解放するまで待機
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // NtAPI による外部プロセスkillは worktree_remove_persistent の Phase 2 ループ内で実行する。
    // ここで呼ぶと git がパスを正規のワークツリーと確認する前にプロセスをkillしてしまい、
    // 不正なパスを渡された場合に無関係なプロセスを終了するリスクがある。

    let cancel_flag = remove_manager.create_cancel_flag(&worktree_path);
    let wp = worktree_path.clone();
    let wp2 = worktree_path.clone();
    let ah = app_handle.clone();

    let result = tokio::task::spawn_blocking(move || {
        git_worktree::worktree_remove_persistent(
            &repo_path,
            &worktree_path,
            cancel_flag,
            Some(&|| {
                let _ = ah.emit(
                    "worktree-remove-retrying",
                    serde_json::json!({ "worktreePath": wp2 }),
                );
            }),
        )
    })
    .await
    .map_err(|e| format!("task join error: {}", e))?;

    remove_manager.remove(&wp);

    // キャンセルはエラーとして伝播（フロントエンドで特別処理）
    result?;

    if let Some(pool) = app_handle.try_state::<report_db::ReportPool>() {
        let _ = report_db::insert(&pool.0, "worktree_change:remove", &wp).await;
    }
    Ok(())
}

#[tauri::command]
fn cancel_worktree_remove(
    remove_manager: State<'_, WorktreeRemoveManager>,
    worktree_path: String,
) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::CancelWorktreeRemove);
    if remove_manager.cancel(&worktree_path) {
        Ok(())
    } else {
        // すでに完了または存在しない場合は無視
        Ok(())
    }
}

#[tauri::command]
async fn git_list_branches(repo_path: String) -> Result<Vec<String>, String> {
    run_git(move || git_worktree::list_branches(&repo_path)).await
}

#[tauri::command]
async fn detect_package_manager(repo_path: String) -> Result<Vec<String>, String> {
    run_git(move || git_worktree::detect_package_manager(&repo_path)).await
}

#[tauri::command]
async fn read_gitignore(repo_path: String) -> Result<Vec<String>, String> {
    run_git(move || git_worktree::read_gitignore(&repo_path)).await
}

#[tauri::command]
async fn detect_tsbuildinfo_files(repo_path: String) -> Result<Vec<String>, String> {
    run_git(move || git_worktree::detect_tsbuildinfo_files(&repo_path)).await
}

#[tauri::command]
async fn copy_gitignore_targets(
    repo_path: String,
    worktree_path: String,
    targets: Vec<String>,
) -> Result<u32, String> {
    run_git(move || git_worktree::copy_gitignore_targets(&repo_path, &worktree_path, targets)).await
}

#[tauri::command]
async fn write_claude_plugin_config(
    app_handle: tauri::AppHandle,
    worktree_path: String,
    worktree_name: String,
    hooks: Vec<crate::settings::NotificationHookEntry>,
) -> Result<(), String> {
    let marketplace_dir = claude_plugin::marketplace_dir(&app_handle)?
        .to_string_lossy()
        .replace('\\', "/");
    run_git(move || {
        claude_plugin::write_plugin_config(&worktree_path, &worktree_name, hooks, &marketplace_dir)
    })
    .await
}

#[tauri::command]
async fn copy_claude_session_data(
    source_worktree_path: String,
    target_worktree_path: String,
) -> Result<u32, String> {
    run_git(move || {
        git_worktree::copy_claude_session_data(&source_worktree_path, &target_worktree_path)
    })
    .await
}

#[tauri::command]
async fn copy_working_changes(
    source_path: String,
    target_path: String,
) -> Result<u32, String> {
    run_git(move || git_worktree::copy_working_changes(&source_path, &target_path)).await
}

#[tauri::command]
async fn git_merge_branch(
    repo_path: String,
    source_branch: String,
    target_branch: String,
) -> Result<(), String> {
    run_git(move || git_worktree::merge_branch(&repo_path, &source_branch, &target_branch)).await
}

#[tauri::command]
async fn git_delete_branch(repo_path: String, branch_name: String, force: bool) -> Result<(), String> {
    run_git(move || git_worktree::delete_branch(&repo_path, &branch_name, force)).await
}

#[tauri::command]
async fn git_worktree_restore(repo_path: String, worktree_path: String, branch_name: String) -> Result<(), String> {
    run_git(move || git_worktree::worktree_restore(&repo_path, &worktree_path, &branch_name)).await
}

// ─── コードレビュー用 Git コマンド ────────────────────────────────────────────

#[tauri::command]
async fn git_list_files(repo_path: String) -> Result<Vec<String>, String> {
    run_git(move || git_worktree::list_quick_open_files(&repo_path)).await
}

#[tauri::command]
async fn list_dir_entries(
    repo_path: String,
    rel_path: String,
) -> Result<Vec<git_worktree::DirEntry>, String> {
    run_git(move || git_worktree::list_dir_entries(&repo_path, &rel_path)).await
}

#[tauri::command]
async fn git_read_file(
    repo_path: String,
    file_path: String,
    revision: Option<String>,
) -> Result<String, String> {
    run_git(move || {
        git_worktree::read_file_content(&repo_path, &file_path, revision.as_deref())
    })
    .await
}

#[tauri::command]
async fn git_get_merge_message(repo_path: String) -> Result<Option<String>, String> {
    run_git(move || git_worktree::get_merge_message(&repo_path)).await
}

#[tauri::command]
async fn git_get_status(repo_path: String) -> Result<Vec<git_worktree::GitStatusEntry>, String> {
    run_git(move || git_worktree::get_status(&repo_path)).await
}

#[tauri::command]
async fn git_get_file_diff(
    repo_path: String,
    file_path: String,
    staged: bool,
) -> Result<git_worktree::FileDiff, String> {
    run_git(move || git_worktree::get_file_diff(&repo_path, &file_path, staged)).await
}

#[tauri::command]
async fn git_get_log(
    repo_path: String,
    skip: usize,
    limit: usize,
) -> Result<Vec<git_worktree::CommitEntry>, String> {
    run_git(move || git_worktree::get_log(&repo_path, skip, limit)).await
}

#[tauri::command]
async fn git_get_commit_files(
    repo_path: String,
    hash: String,
) -> Result<Vec<git_worktree::CommitFileEntry>, String> {
    run_git(move || git_worktree::get_commit_files(&repo_path, &hash)).await
}

#[tauri::command]
async fn git_get_commit_file_diff(
    repo_path: String,
    hash: String,
    file_path: String,
    old_file_path: Option<String>,
) -> Result<git_worktree::FileDiff, String> {
    run_git(move || git_worktree::get_commit_file_diff(&repo_path, &hash, &file_path, old_file_path.as_deref())).await
}

#[tauri::command]
async fn git_stage_all(repo_path: String) -> Result<(), String> {
    run_git(move || git_worktree::stage_all(&repo_path)).await
}

#[tauri::command]
async fn git_commit(repo_path: String, message: String) -> Result<String, String> {
    run_git(move || git_worktree::commit(&repo_path, &message)).await
}

// ─── IDE / AI エージェントコマンド ────────────────────────────────────────────

#[tauri::command]
fn detect_ides() -> Vec<ide_launcher::IdeInfo> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::DetectIdes);
    ide_launcher::detect_ides()
}

#[tauri::command]
fn detect_ai_agents() -> Vec<ai_provider::AiAgentInfo> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::DetectAiAgents);
    ai_provider::detect_ai_agents()
}

#[tauri::command]
fn open_in_ide(command: String, path: String) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::OpenInIde);
    ide_launcher::open_in_ide(&command, &path)
}

#[tauri::command]
fn open_in_file_explorer(path: String) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::OpenInFileExplorer);
    ide_launcher::open_in_file_explorer(&path)
}

// ─── MCP コマンド ─────────────────────────────────────────────────────────────

#[tauri::command]
fn regenerate_mcp_api_key(app_handle: tauri::AppHandle) -> Result<String, String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::RegenerateMcpApiKey);
    let settings_manager = app_handle.state::<SettingsManager>();
    let mut settings = settings_manager.get();
    settings.mcp_api_key = settings::generate_api_key();
    let new_key = settings.mcp_api_key.clone();
    settings_manager.save(settings)?;
    Ok(new_key)
}

#[tauri::command]
fn get_mcp_status(state: State<mcp_server::McpServerManager>) -> mcp_server::McpStatus {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::GetMcpStatus);
    state.get_status()
}

#[tauri::command]
async fn restart_mcp_server(app_handle: tauri::AppHandle) -> Result<mcp_server::McpStatus, String> {
    let manager = app_handle.state::<mcp_server::McpServerManager>();
    // 同時に複数の restart が走らないよう排他ロックを取得する
    let _lock = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        manager.acquire_restart_lock(),
    )
    .await
    .map_err(|_| "restart lock acquisition timed out".to_string())?;
    let (mcp_port, mcp_remote_access) = {
        let settings_manager = app_handle.state::<SettingsManager>();
        let s = settings_manager.get();
        (s.mcp_port, s.mcp_remote_access)
    };
    manager.stop_and_wait(std::time::Duration::from_secs(3)).await;
    mcp_server::start_mcp_server(app_handle.clone(), mcp_port, mcp_remote_access);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(manager.get_status())
}

#[tauri::command]
async fn download_and_install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    // 自前の on_before_exit を設定した Updater を構築する。
    // プラグイン既定の on_before_exit (cleanup_before_exit) を踏襲しつつ、
    // KILL_ON_JOB_CLOSE 解除を追加する。これは Windows で ShellExecute による
    // インストーラ起動と process::exit(0) の「前」に走る (tauri-plugin-updater updater.rs)。
    // 既存実装は relaunch 後の prepare_for_relaunch で解除していたが、Windows では
    // downloadAndInstall 内で process::exit(0) され JS に戻らないため解除されず、
    // インストーラが Job の巻き込み終了で殺されてアップデートが起動しなかった。
    let app_hook = app.clone();
    let updater = app
        .updater_builder()
        .on_before_exit(move || {
            // Windows ではこの後 process::exit(0) され RunEvent::Exit のクリーンアップ
            // (PtyManager::kill_all_fast 等) が走らない。KILL_ON_JOB_CLOSE を解除すると
            // Job のセーフティネットも外れるため、解除の「前」に PTY 子プロセス
            // (ターミナル / AI エージェント) を明示 kill してオーファン化を防ぐ。
            app_hook
                .state::<PtyManager>()
                .kill_all_fast("update-before-exit");
            app_hook.cleanup_before_exit();
            job_object::release_kill_on_close();
        })
        .build()
        .map_err(|e| e.to_string())?;

    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(()); // 更新なし
    };

    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;

    // install() は Windows では成功時に on_before_exit→process::exit(0) され戻らない。
    // 失敗 (extract 失敗等、インストーラ起動前) 時のみ Err を返す。ここで MCP を止めて
    // しまうと install 失敗時に MCP が落ちたまま残るため、MCP の停止は install 後
    // （= 非 Windows の成功パスでのみ到達）に行う。
    update.install(bytes).map_err(|e| e.to_string())?;

    // 非 Windows（mac 等）のみ到達。restart() 前に MCP を停止してポート競合を避け、
    // その後明示再起動する。restart() は `!` を返す。
    {
        let mcp = app.state::<mcp_server::McpServerManager>();
        let _lock = mcp.acquire_restart_lock().await;
        mcp.stop_and_wait(std::time::Duration::from_secs(3)).await;
    }
    app.restart();
}

// ─── アーティファクトコマンド ─────────────────────────────────────────────────

#[tauri::command]
async fn list_artifacts(
    app_handle: tauri::AppHandle,
    worktree_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    let artifacts_dir = artifacts_dir(&app_handle, &worktree_id)?;
    tokio::task::spawn_blocking(move || {
        if !artifacts_dir.exists() {
            return Ok(vec![]);
        }
        let mut artifacts = Vec::new();
        let entries = std::fs::read_dir(&artifacts_dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
                artifacts.push(val);
            }
        }
        // updated_at 降順でソート
        artifacts.sort_by(|a, b| {
            let a_time = a.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0);
            let b_time = b.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0);
            b_time.cmp(&a_time)
        });
        Ok(artifacts)
    })
    .await
    .map_err(|e| format!("task join error: {}", e))?
}

#[tauri::command]
async fn read_artifact(
    app_handle: tauri::AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<String, String> {
    validate_path_component(&artifact_id)?;
    let artifact_path = artifacts_dir(&app_handle, &worktree_id)?
        .join(format!("{}.json", artifact_id));
    tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&artifact_path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {}", e))?
}

#[tauri::command]
async fn delete_artifacts(app_handle: tauri::AppHandle, worktree_id: String) -> Result<(), String> {
    let dir = artifacts_dir(&app_handle, &worktree_id)?;
    let deleted = tokio::task::spawn_blocking(move || {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            Ok::<bool, String>(true)
        } else {
            Ok(false)
        }
    })
    .await
    .map_err(|e| format!("task join error: {}", e))??;
    if deleted {
        if let Some(pool) = app_handle.try_state::<report_db::ReportPool>() {
            let _ = report_db::insert(&pool.0, "artifact_change:delete", &worktree_id).await;
        }
    }
    Ok(())
}

// ─── レポートコマンド ─────────────────────────────────────────────────────────

#[tauri::command]
async fn get_report_summary(
    app_handle: tauri::AppHandle,
    date: String,
    tz_offset_min: i64,
) -> Result<report_db::ReportSummary, String> {
    let pool = app_handle
        .try_state::<report_db::ReportPool>()
        .ok_or_else(|| "Report DB not initialized".to_string())?;
    report_db::summary_for_date(&pool.0, &date, tz_offset_min).await
}

// ─── タスク DB コマンド ───────────────────────────────────────────────────────

#[tauri::command]
async fn save_task(
    app_handle: tauri::AppHandle,
    task: task_db::TaskRow,
) -> Result<(), String> {
    let pool = app_handle
        .try_state::<task_db::TaskPool>()
        .ok_or_else(|| "Task DB not initialized".to_string())?;
    task_db::save(&pool.0, &task).await
}

#[tauri::command]
async fn list_tasks(
    app_handle: tauri::AppHandle,
    search: String,
    offset: i64,
    limit: i64,
) -> Result<task_db::TaskListResult, String> {
    let pool = app_handle
        .try_state::<task_db::TaskPool>()
        .ok_or_else(|| "Task DB not initialized".to_string())?;
    task_db::list(&pool.0, &search, offset, limit).await
}

#[tauri::command]
async fn delete_task(
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let pool = app_handle
        .try_state::<task_db::TaskPool>()
        .ok_or_else(|| "Task DB not initialized".to_string())?;
    task_db::delete(&pool.0, &id).await
}

// ─── ファイルシステム ユーティリティ ─────────────────────────────────────────

#[tauri::command]
fn path_exists(path: String) -> bool {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::PathExists);
    std::path::Path::new(&path).exists()
}

// ─── アーカイブ DB コマンド ───────────────────────────────────────────────────

#[tauri::command]
async fn save_archive(
    app_handle: tauri::AppHandle,
    archive: archive_db::ArchiveRow,
) -> Result<(), String> {
    let pool = app_handle
        .try_state::<archive_db::ArchivePool>()
        .ok_or_else(|| "Archive DB not initialized".to_string())?;
    archive_db::save(&pool.0, &archive).await
}

/// ワークツリーのアーカイブ（削除）が完了した後にMCPクライアントへ通知する。
/// git worktree remove の成功確認後にフロントエンドから呼び出す。
#[tauri::command]
fn notify_worktree_archived(
    app_handle: tauri::AppHandle,
    id: String,
    name: String,
    branch_name: String,
) {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::NotifyWorktreeArchived);
    let _ = app_handle.emit("worktree-archived", serde_json::json!({
        "id": id,
        "name": name,
        "branchName": branch_name,
    }));
}

/// ワークツリーの追加が完了した後にMCPクライアントへ通知する。
/// git worktree add の成功確認後にフロントエンドから呼び出す。
#[tauri::command]
fn notify_worktree_added(
    app_handle: tauri::AppHandle,
    id: String,
    name: String,
    branch_name: String,
) {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::NotifyWorktreeAdded);
    let _ = app_handle.emit("worktree-added", serde_json::json!({
        "id": id,
        "name": name,
        "branchName": branch_name,
    }));
}

#[tauri::command]
async fn list_archives(
    app_handle: tauri::AppHandle,
    search: String,
    offset: i64,
    limit: i64,
) -> Result<archive_db::ArchiveListResult, String> {
    let pool = app_handle
        .try_state::<archive_db::ArchivePool>()
        .ok_or_else(|| "Archive DB not initialized".to_string())?;
    archive_db::list(&pool.0, &search, offset, limit).await
}

#[tauri::command]
async fn delete_archive(
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let pool = app_handle
        .try_state::<archive_db::ArchivePool>()
        .ok_or_else(|| "Archive DB not initialized".to_string())?;
    archive_db::delete(&pool.0, &id).await
}

// ─── FS ウォッチャーコマンド ──────────────────────────────────────────────────

#[tauri::command]
fn start_fs_watch(
    app_handle: tauri::AppHandle,
    state: State<FsWatcherManager>,
    worktree_id: String,
    worktree_path: String,
) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::StartFsWatch);
    state.start_watching(app_handle, worktree_id, worktree_path)
}

#[tauri::command]
fn stop_fs_watch(state: State<FsWatcherManager>, worktree_id: String) -> Result<(), String> {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::StopFsWatch);
    state.stop_watching(&worktree_id)
}

fn is_mcp_enabled() -> bool {
    std::env::var("MCP_SERVER_ENABLED")
        .map(|v| v != "false")
        .unwrap_or(true)
}

#[tauri::command]
fn set_debug_mode(enabled: bool) {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::SetDebugMode);
    if enabled {
        log::set_max_level(log::LevelFilter::Debug);
    } else {
        log::set_max_level(log::LevelFilter::Info);
    }
    log::info!("Debug mode changed to: {}", enabled);
}

#[tauri::command]
fn get_debug_mode() -> bool {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::GetDebugMode);
    log::max_level() >= log::LevelFilter::Debug
}

/// 動作確認用: 初回起動ウィザードを毎回表示するか (ORETACHI_FORCE_WIZARD)。
/// run() 冒頭で .env* 一式 (Vite 規約準拠、.env.development.local が最優先) を読むため、
/// dev では .env.development.local 等で true にすると毎回表示される。
/// インストール済みアプリでは .env* が存在しないため実質常に false になる。
#[tauri::command]
fn get_force_wizard() -> bool {
    let _bc = main_thread_watch::enter(main_thread_watch::Activity::GetForceWizard);
    std::env::var("ORETACHI_FORCE_WIZARD")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
}

/// 猫ターミナル向けシステムメトリクスのポーリングを開始する。
#[tauri::command]
fn system_metrics_start(app_handle: tauri::AppHandle, state: State<SystemMetricsState>) {
    state.start(app_handle);
}

/// システムメトリクスのポーリングを停止する。
#[tauri::command]
fn system_metrics_stop(state: State<SystemMetricsState>) {
    state.stop();
}

/// メインウィンドウの WebView2 にネイティブ診断イベントを登録する (Windows 限定)。
///
/// - `ProcessFailed`: browser/renderer プロセス破綻を ERROR ログ。完全アイドル中のフリーズ
///   (WebView2Feedback #4141: idle 時 browser プロセス破綻) を **アプリ側ログに初めて残す** 決定打。
/// - `NewBrowserVersionAvailable`: WebView2 ランタイム自動更新検知を WARN ログ。更新→破綻仮説の裏付け/反証用。
///
/// 失敗してもアプリ動作には影響させない (ログのみ)。**回復処理は一切行わない** (記録に徹する)。
#[cfg(target_os = "windows")]
fn subscribe_webview2_diagnostics(window: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2ProcessFailedEventArgs, COREWEBVIEW2_PROCESS_FAILED_KIND,
    };
    use webview2_com::{NewBrowserVersionAvailableEventHandler, ProcessFailedEventHandler};

    let result = window.with_webview(|webview| unsafe {
        // ProcessFailed は ICoreWebView2 直下のイベント。
        match webview.controller().CoreWebView2() {
            Ok(core) => {
                let handler = ProcessFailedEventHandler::create(Box::new(
                    |_sender, args: Option<ICoreWebView2ProcessFailedEventArgs>| {
                        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(-1);
                        if let Some(a) = args.as_ref() {
                            let _ = a.ProcessFailedKind(&mut kind);
                        }
                        log::error!(
                            "[webview2] ProcessFailed kind={} (browser/renderer プロセス破綻; アイドル時フリーズの疑い WebView2Feedback#4141)",
                            kind.0
                        );
                        Ok(())
                    },
                ));
                let mut token: i64 = 0;
                if let Err(e) = core.add_ProcessFailed(&handler, &mut token) {
                    log::warn!("[webview2] add_ProcessFailed failed: {}", e);
                } else {
                    log::info!("[webview2] ProcessFailed ハンドラ登録完了");
                }
            }
            Err(e) => log::warn!("[webview2] CoreWebView2() 取得失敗: {}", e),
        }

        // NewBrowserVersionAvailable は ICoreWebView2Environment のイベント。
        let env = webview.environment();
        let handler = NewBrowserVersionAvailableEventHandler::create(Box::new(|_env, _args| {
            log::warn!(
                "[webview2] NewBrowserVersionAvailable: WebView2 ランタイム自動更新を検知 (更新後のアイドル破綻に注意 WebView2Feedback#4141)"
            );
            Ok(())
        }));
        let mut token: i64 = 0;
        if let Err(e) = env.add_NewBrowserVersionAvailable(&handler, &mut token) {
            log::warn!("[webview2] add_NewBrowserVersionAvailable failed: {}", e);
        }
    });
    if let Err(e) = result {
        log::warn!("[webview2] with_webview failed (診断イベント未登録): {}", e);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Windows: インストーラ起動由来の RedirectionGuard 緩和策 (Enforce=1) を検出したら
    // explorer.exe 経由でセルフリローンチする (Issue #70)。リローンチ時は exit(0) して戻らない。
    // この時点ではロガー未初期化のため、結果は setup() に持ち越して記録する。
    #[cfg(target_os = "windows")]
    let redirection_guard_outcome = redirection_guard::relaunch_if_guarded();

    // Windows: tauri-plugin-notification はパスが \target\release で終わる場合に
    // AUMID を設定しないため、PowerShell として通知が表示される問題を回避する。
    #[cfg(target_os = "windows")]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        unsafe {
            let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from("com.ia.oretachi"));
        }
    }

    // メインスレッド・ブレッドクラムの単調時計を初期化する (以降の enter() より前に必須)。
    main_thread_watch::init();

    // .env 読み込み（Vite の .env 規約に準拠: 優先度 .{mode}.local > .{mode} > .local > base）
    // builder チェーンより前に読み込む必要があるためここで実施。
    // 読み込み順は「低優先 → 高優先」で、後勝ち（override）になるよう並べる。
    // from_filename(_override) は cwd とその親を探索するため、cwd が src-tauri でも
    // リポジトリルートの .env* 一式を解決する。
    let _ = dotenvy::from_filename(".env"); // base（既存のシェル環境変数は上書きしない）
    let _ = dotenvy::from_filename_override(".env.local"); // 全モード共通のローカル上書き
    if cfg!(debug_assertions) {
        let _ = dotenvy::from_filename_override(".env.development");
        let _ = dotenvy::from_filename_override(".env.development.local");
    }

    // ORETACHI_DEBUG 環境変数でデバッグモードを判定（起動時点で確定）
    let env_debug = std::env::var("ORETACHI_DEBUG")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    #[cfg(target_os = "windows")]
    let (acrylic_enabled, acrylic_opacity, acrylic_color) = acrylic::load_settings();
    #[cfg(not(target_os = "windows"))]
    let acrylic_enabled = true;

    // MCP の有効/無効をプロセス起動時に一度だけ評価し、セットアップと終了ハンドラで共有する。
    // 実行中に環境変数が変更された場合でも起動時の判断と終了時の判断が一致する。
    let mcp_enabled = is_mcp_enabled();

    let mut builder = tauri::Builder::default();

    // 多重起動防止 (重複起動すると同一ログ/settings/SQLite への並行書き込みと
    // ワークツリー復元による PTY/AI エージェントの重複 spawn が起きる)。
    // single-instance プラグインは「最初に登録する」ことが公式要件。
    // dev と本番は識別子 com.ia.oretachi を共有するため、debug ビルドでは登録しない
    // (本番稼働中に `pnpm run tauri dev` が起動できなくなるのを避ける)。
    #[cfg(not(debug_assertions))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log::warn!("[single-instance] 二重起動を検出。既存インスタンスの main ウィンドウを前面化する");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show(); // トレイ常駐で非表示のケースに対応
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }));
    }

    if acrylic_enabled {
        builder = builder.plugin(tauri_plugin_liquid_glass::init());

        #[cfg(target_os = "windows")]
        {
            let (r, g, b) = acrylic::parse_color(&acrylic_color);
            let a = acrylic_opacity;
            builder = builder.plugin(
                tauri::plugin::Builder::<tauri::Wry>::new("acrylic-effect")
                    .on_window_ready(move |window| {
                        let _bc = main_thread_watch::enter(main_thread_watch::Activity::WindowReady);
                        if let Ok(hwnd) = window.hwnd() {
                            acrylic::setup(hwnd.0, r, g, b, a);
                        }
                        let _ = window;
                    })
                    .build(),
            );
        }
    }
    let mut builder = builder
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Stdout),
                ])
                .rotation_strategy(RotationStrategy::KeepAll)
                .max_file_size(100_000_000) // 100MB
                .timezone_strategy(TimezoneStrategy::UseLocal)
                .level(log::LevelFilter::Debug)
                .build(),
        )
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        .difference(tauri_plugin_window_state::StateFlags::DECORATIONS),
                )
                .with_denylist(&["tray-popup"])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_prevent_default::Builder::new()
                .with_flags(tauri_plugin_prevent_default::Flags::CONTEXT_MENU)
                .build(),
        );

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp::init_with_config(
            tauri_plugin_mcp::PluginConfig::new("oretachi".to_string()).tcp_localhost(4000),
        ));
    }

    let app = builder
        .manage(PtyManager::new())
        .manage(WorktreeRemoveManager::new())
        .manage(SettingsManager::new())
        .manage(mcp_server::McpServerManager::new())
        .manage(mcp_server::McpPeerRegistry(std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()))))
        .manage(mcp_server::DetachedWorktreeRegistry::default())
        .manage(ai_judge::ApprovalManager::new())
        .manage(ai_commit_message::CommitMessageManager::new())
        .manage(ai_description::DescriptionManager::new())
        .manage(task_executor::TaskGenerateManager::new())
        .manage(FsWatcherManager::new())
        .manage(SystemMetricsState::new())
        // ReportPool は setup() 内で非同期初期化するため、ここでは登録しない
        .invoke_handler(tauri::generate_handler![
            pty_spawn,
            pty_write,
            pty_resize,
            pty_kill,
            pty_set_ai_agent,
            pty_is_ai_agent,
            get_settings,
            save_settings,
            git_validate_repo,
            git_pull,
            git_worktree_add,
            git_worktree_remove,
            cancel_worktree_remove,
            git_worktree_restore,
            git_list_branches,
            detect_package_manager,
            read_gitignore,
            detect_tsbuildinfo_files,
            copy_gitignore_targets,
            write_claude_plugin_config,
            copy_claude_session_data,
            copy_working_changes,
            git_merge_branch,
            git_delete_branch,
            git_list_files,
            list_dir_entries,
            git_read_file,
            git_get_merge_message,
            git_get_status,
            git_get_file_diff,
            git_get_commit_files,
            git_get_commit_file_diff,
            git_get_log,
            git_stage_all,
            git_commit,
            detect_ides,
            detect_ai_agents,
            open_in_ide,
            open_in_file_explorer,
            get_mcp_status,
            restart_mcp_server,
            regenerate_mcp_api_key,
            mcp_server::register_detached_worktree,
            mcp_server::unregister_detached_worktree,
            download_and_install_update,
            ai_judge::judge_approval,
            ai_judge::cancel_approval,
            ai_commit_message::generate_commit_message,
            ai_commit_message::cancel_commit_message_generation,
            ai_description::generate_description_from_plan,
            ai_description::cancel_description_generation,
            terminal_session::save_terminal_session,
            terminal_session::load_terminal_session,
            terminal_session::delete_terminal_session,
            task_executor::task_generate,
            task_executor::cancel_task_generate,
            task_executor::write_temp_prompt,
            list_artifacts,
            read_artifact,
            delete_artifacts,
            start_fs_watch,
            stop_fs_watch,
            settings::list_system_sounds,
            settings::copy_custom_sound,
            settings::read_audio_file,
            apply_acrylic_effect,
            get_report_summary,
            save_task,
            list_tasks,
            delete_task,
            path_exists,
            save_archive,
            notify_worktree_archived,
            notify_worktree_added,
            list_archives,
            delete_archive,
            set_debug_mode,
            get_debug_mode,
            get_force_wizard,
            system_metrics_start,
            system_metrics_stop,
        ])
        .setup(move |app| {
            // env var が false の場合は DB初期化などの起動ログを含め debug! を抑制する。
            // settings.debug_mode は後で読み込まれるため、env var のみで先行設定する。
            if !env_debug {
                log::set_max_level(log::LevelFilter::Info);
            }

            // 子プロセス（WebView2 等）を Job Object に取り込み、親終了時に OS レベルで
            // 巻き込み終了させる。MCP サーバ spawn や window.show() より前に割り当てて
            // 以降生成される全子プロセスを確実に Job 配下に入れる。
            job_object::assign_current_process_to_job();

            #[cfg(target_os = "windows")]
            match &redirection_guard_outcome {
                redirection_guard::GuardOutcome::NotGuarded => {}
                redirection_guard::GuardOutcome::RelaunchedPreviously => {
                    log::info!(
                        "[RedirectionGuard] 緩和策 (Enforce=1) を検出したため explorer.exe 経由で再起動した"
                    );
                }
                redirection_guard::GuardOutcome::StillGuarded => {
                    log::warn!(
                        "[RedirectionGuard] リローンチ後も緩和策 (Enforce=1) が残っている。ターミナル内のジャンクション操作 (pnpm install 等) が失敗する可能性がある"
                    );
                }
                redirection_guard::GuardOutcome::RelaunchSkipped(reason) => {
                    log::warn!(
                        "[RedirectionGuard] 緩和策 (Enforce=1) を検出したが起動を継続: {reason}。ターミナル内のジャンクション操作 (pnpm install 等) が失敗する可能性がある"
                    );
                }
            }

            // レポート DB を同期的に初期化（ファイル接続のみで高速、起動前に完了させる）
            {
                let handle = app.handle().clone();
                tauri::async_runtime::block_on(async move {
                    match report_db::init_report_db(&handle).await {
                        Ok(pool) => {
                            handle.manage(report_db::ReportPool(pool));
                            log::debug!("[ReportDB] Initialized successfully");
                        }
                        Err(e) => {
                            log::warn!("[ReportDB] Failed to initialize: {}", e);
                        }
                    }
                });
            }

            // タスク DB を同期的に初期化
            {
                let handle = app.handle().clone();
                tauri::async_runtime::block_on(async move {
                    match task_db::init_task_db(&handle).await {
                        Ok(pool) => {
                            handle.manage(task_db::TaskPool(pool));
                            log::debug!("[TaskDB] Initialized successfully");
                        }
                        Err(e) => {
                            log::warn!("[TaskDB] Failed to initialize: {}", e);
                        }
                    }
                });
            }

            // アーカイブ DB を同期的に初期化
            {
                let handle = app.handle().clone();
                tauri::async_runtime::block_on(async move {
                    match archive_db::init_archive_db(&handle).await {
                        Ok(pool) => {
                            handle.manage(archive_db::ArchivePool(pool));
                            log::debug!("[ArchiveDB] Initialized successfully");
                        }
                        Err(e) => {
                            log::warn!("[ArchiveDB] Failed to initialize: {}", e);
                        }
                    }
                });
            }

            // アップデート後の再起動でPATHが不完全になる問題を修正:
            // NSISインストーラー経由で再起動した場合、ユーザーの完全なPATHが
            // 継承されないため、レジストリから最新PATHを取得して上書きする。
            #[cfg(target_os = "windows")]
            match crate::process_utils::refresh_path_from_registry() {
                Ok(path) => {
                    std::env::set_var("PATH", &path);
                    log::debug!("PATH refreshed from registry");
                }
                Err(e) => {
                    log::warn!("Failed to refresh PATH from registry: {}", e);
                }
            }

            #[cfg(target_os = "macos")]
            match crate::process_utils::refresh_path_from_login_shell() {
                Ok(path) => {
                    std::env::set_var("PATH", &path);
                    log::debug!("PATH refreshed from login shell");
                }
                Err(e) => {
                    log::warn!("Failed to refresh PATH from login shell: {}", e);
                }
            }

            if let Ok(log_dir) = app.path().app_log_dir() {
                log::info!("Application log directory: {:?}", log_dir);
            }
            let settings_manager = app.state::<SettingsManager>();
            settings_manager.init(app.handle());

            // settings.debug_mode を反映: env=false かつ settings=true の場合は Debug に昇格
            {
                let settings_debug = settings_manager.get().debug_mode;
                let debug_enabled = env_debug || settings_debug;
                if debug_enabled && !env_debug {
                    // env_debug=false で既に Info に設定済みのため、settings で有効なら Debug に昇格
                    log::set_max_level(log::LevelFilter::Debug);
                } else if !debug_enabled {
                    // env_debug=true の場合は plugin が Debug にしているためここに来ない想定だが念のため
                    log::set_max_level(log::LevelFilter::Info);
                }
                log::info!(
                    "Debug mode: {} (env={}, settings={})",
                    debug_enabled, env_debug, settings_debug
                );
            }

            // Claude Code プラグインファイルを生成・更新（ORETACHI_PLUGIN_OVERWRITE=false で抑止）
            if claude_plugin::overwrite_enabled() {
                if let Err(e) = claude_plugin::generate_plugin_files(app.handle()) {
                    log::warn!("[ClaudePlugin] Failed to generate plugin files: {}", e);
                }
            } else {
                log::info!(
                    "[ClaudePlugin] ORETACHI_PLUGIN_OVERWRITE=false: プラグイン生成をスキップ"
                );
            }

            // AIエージェントインジケーター用ポーリング起動
            let pty_manager = app.state::<PtyManager>();
            pty_manager.start_polling(app.handle().clone());
            // PTY 出力 emit を 16ms 周期でコアレッシング（WebView2 IPC 飽和によるハング対策）
            pty_manager.start_output_flush(app.handle().clone());

            // Webview ハング診断: heartbeat ループ（30秒間隔で ping → pong のラウンドトリップ計測）
            {
                use std::sync::Arc;
                // NOTE: ハング自動復旧（reload / WebView 再作成）を一時無効化したため、
                // AtomicBool（recreate_attempted 用）は未使用となり import から除外している。
                use std::sync::atomic::{AtomicU64, Ordering};
                use std::time::{Duration, SystemTime, UNIX_EPOCH};

                let last_pong_ms = Arc::new(AtomicU64::new(0));
                let last_pong_for_listener = last_pong_ms.clone();
                let heartbeat_handle = app.handle().clone();

                // pong リスナー登録
                heartbeat_handle.listen("__webview-heartbeat-pong", move |event: tauri::Event| {
                    #[derive(serde::Deserialize)]
                    struct PongPayload {
                        ts: u64,
                        mem: Option<u64>,
                        #[serde(rename = "blockedMs")]
                        blocked_ms: Option<u64>,
                        terminals: Option<TerminalStats>,
                    }
                    #[derive(serde::Deserialize)]
                    struct TerminalStats {
                        active: u32,
                        #[serde(rename = "totalMounts")]
                        total_mounts: u32,
                        #[serde(rename = "totalUnmounts")]
                        total_unmounts: u32,
                    }
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    last_pong_for_listener.store(now_ms, Ordering::Relaxed);
                    if let Ok(payload) = serde_json::from_str::<PongPayload>(event.payload()) {
                        let rtt = now_ms.saturating_sub(payload.ts);
                        let mem_mb = payload.mem.unwrap_or(0) / 1024 / 1024;
                        let blocked_ms = payload.blocked_ms.unwrap_or(0);
                        let ai_in_flight = crate::ai_judge::ai_in_flight();
                        if let Some(t) = &payload.terminals {
                            log::debug!(
                                "[heartbeat] pong rtt={}ms mem={}MB blockedMs={} aiInFlight={} terminals(active={} mounts={} unmounts={})",
                                rtt, mem_mb, blocked_ms, ai_in_flight, t.active, t.total_mounts, t.total_unmounts
                            );
                        } else {
                            log::debug!(
                                "[heartbeat] pong rtt={}ms mem={}MB blockedMs={} aiInFlight={}",
                                rtt, mem_mb, blocked_ms, ai_in_flight
                            );
                        }
                        if rtt > 5000 {
                            log::warn!("[heartbeat] webview response slow: rtt={}ms", rtt);
                        }
                        if blocked_ms >= 5000 {
                            log::warn!(
                                "[heartbeat] main thread blocked since last pong: {}ms",
                                blocked_ms
                            );
                        }
                    }
                });

                // ping ループ
                let ping_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut ping_pending_since: Option<u64> = None;
                    let mut unresponsive_logged_until_secs: u64 = 0;
                    // [一時無効化] 自動復旧（reload / WebView 再作成）がハング回復に失敗して
                    // アプリ全体をクラッシュさせるため、復旧アクションを無効化している。
                    // それに伴い以下の状態変数も未使用となるためコメントアウト:
                    //   - reload_attempted    : reload() を試行したか
                    //   - recreate_attempted  : WebView 再作成を試行したか
                    //   - recreate_generation : 再作成バックオフタスクの世代カウンタ
                    // let mut reload_attempted = false;
                    // let recreate_attempted = Arc::new(AtomicBool::new(false));
                    // let recreate_generation = Arc::new(AtomicU64::new(0));
                    loop {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        let now_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        // pong が届いていれば pending をクリア（エラー判定より先に行う）
                        let last_pong = last_pong_ms.load(Ordering::Relaxed);
                        if last_pong > 0 && now_ms.saturating_sub(last_pong) < 35_000 {
                            ping_pending_since = None;
                            unresponsive_logged_until_secs = 0;
                            // [一時無効化] 復旧アクション無効化に伴い、復旧フラグの reset も不要。
                            // reload_attempted = false;
                            // recreate_attempted.store(false, Ordering::Relaxed);
                            // recreate_generation.fetch_add(1, Ordering::Relaxed);
                        }
                        // クリアされずに残っている場合のみ本当の未応答と判定
                        if let Some(pending_since) = ping_pending_since {
                            let unresponsive_secs = now_ms.saturating_sub(pending_since) / 1000;
                            // 90秒までは毎回、180秒で 1 回中間サイン、300秒で 1 回最終警告
                            // （無限連続出力を抑制しつつ、長時間フリーズ継続中かどうかの目安を残す）
                            let should_log = if unresponsive_secs <= 90 {
                                true
                            } else if unresponsive_secs >= 180
                                && unresponsive_logged_until_secs < 180
                            {
                                true
                            } else {
                                unresponsive_secs >= 300 && unresponsive_logged_until_secs < 300
                            };
                            if should_log {
                                log::error!(
                                    "[heartbeat] webview unresponsive, no pong for {}s aiInFlight={}",
                                    unresponsive_secs,
                                    crate::ai_judge::ai_in_flight()
                                );
                                if unresponsive_secs >= 300 {
                                    unresponsive_logged_until_secs = 300;
                                } else if unresponsive_secs >= 180 {
                                    unresponsive_logged_until_secs = 180;
                                }
                            }
                            // [一時無効化] 第1段階（30s 未応答）の WebView 強制リロード。
                            // 自動復旧がハング回復に失敗してアプリをクラッシュさせるため無効化。
                            // 診断ログ（上の未応答 ERROR ログ）は残し、復旧アクションのみ止める。
                            /*
                            // 第1段階（30s 未応答）: メインウィンドウの強制リロードを試みる。
                            // eval("location.reload()") は JS タスクキュー経由のため、JS メインスレッド
                            // が詰まっていると実行されない。WebviewWindow::reload() は Tauri runtime
                            // 経由で UI スレッドに post され、wry から ICoreWebView2::Reload() をネイ
                            // ティブ呼び出しするため、JS がブロック状態でも復帰できる場合がある。
                            if !reload_attempted && unresponsive_secs < 35 {
                                reload_attempted = true;
                                log::warn!("[heartbeat] attempting webview reload to recover from hang");
                                for (label, webview) in ping_handle.webview_windows() {
                                    if label == "main" {
                                        if let Err(e) = webview.reload() {
                                            log::error!("[heartbeat] reload failed for {}: {}", label, e);
                                        } else {
                                            log::info!("[heartbeat] reload triggered for {}", label);
                                        }
                                    }
                                }
                            }
                            */

                            // [一時無効化] 第2段階（95s 未応答）の WebView ウィンドウ destroy → 再作成。
                            // この復旧アクション（特に destroy → rebuild）がハング回復に失敗して
                            // アプリ全体をクラッシュさせる原因となっているため無効化する。
                            // 診断ログは残し、復旧アクションのみ止める。
                            /*
                            // 第2段階（95s 未応答）: reload が効かなかった場合は WebView ウィンドウ
                            // 自体を destroy → tauri.conf.json の設定から再作成する。WebView2 プロセス
                            // 自身が応答不能な場合、reload メッセージが届かずこの段階に到達する。
                            // PTY セッションは kill しない（既存挙動）。フロントは再マウント時に
                            // TerminalView.initialSessionId 経由で既存セッションに再 attach する。
                            //
                            // recreate_attempted は spawn 内タスクから成功/失敗に応じて書き戻す。
                            // 失敗時は 60s 後に false にリセットしてリトライ可能にし、成功までは
                            // 30s loop と相まって過度な連発を避けつつ復旧不能を防ぐ。
                            if !recreate_attempted.load(Ordering::Relaxed) && unresponsive_secs >= 95 {
                                recreate_attempted.store(true, Ordering::Relaxed);
                                // 自世代を確定して spawn 内に持ち込む。fetch_add は古い値を返すため +1。
                                let my_gen = recreate_generation.fetch_add(1, Ordering::Relaxed) + 1;
                                log::warn!(
                                    "[heartbeat] reload ineffective, attempting main webview recreate (unresponsive={}s gen={})",
                                    unresponsive_secs, my_gen
                                );
                                let recreate_handle = ping_handle.clone();
                                let recreate_attempted_inner = recreate_attempted.clone();
                                let recreate_generation_inner = recreate_generation.clone();
                                tauri::async_runtime::spawn(async move {
                                    let main_cfg = recreate_handle
                                        .config()
                                        .app
                                        .windows
                                        .iter()
                                        .find(|w| w.label == "main")
                                        .cloned();
                                    let Some(cfg) = main_cfg else {
                                        log::error!(
                                            "[heartbeat] recreate: main window config not found in tauri.conf.json"
                                        );
                                        // バックオフ後、世代一致時のみリトライ可能に戻す
                                        tokio::time::sleep(Duration::from_secs(60)).await;
                                        if recreate_generation_inner.load(Ordering::Relaxed) == my_gen {
                                            recreate_attempted_inner.store(false, Ordering::Relaxed);
                                        }
                                        return;
                                    };

                                    // 旧ウィンドウを destroy で強制破棄。close() は CloseRequested
                                    // イベント経由のソフトクローズで race の余地があるため避ける。
                                    if let Some(old) = recreate_handle.get_webview_window("main") {
                                        if let Err(e) = old.destroy() {
                                            log::error!("[heartbeat] recreate: destroy old window failed: {}", e);
                                        } else {
                                            log::info!("[heartbeat] recreate: old window destroyed");
                                        }
                                    }

                                    // label 解放を待つ
                                    tokio::time::sleep(Duration::from_millis(500)).await;

                                    // build を試行。WindowLabelAlreadyExists は label 解放前のため、
                                    // 1秒追加で待ってもう一度試す。
                                    let try_build = |handle: &tauri::AppHandle, cfg: &tauri::utils::config::WindowConfig| -> Result<tauri::WebviewWindow, String> {
                                        tauri::WebviewWindowBuilder::from_config(handle, cfg)
                                            .map_err(|e| format!("from_config: {}", e))?
                                            .build()
                                            .map_err(|e| format!("build: {}", e))
                                    };

                                    let result = match try_build(&recreate_handle, &cfg) {
                                        Ok(w) => Ok(w),
                                        Err(e) => {
                                            log::warn!("[heartbeat] recreate: first attempt failed ({}), retry after 1s", e);
                                            tokio::time::sleep(Duration::from_secs(1)).await;
                                            try_build(&recreate_handle, &cfg)
                                        }
                                    };

                                    match result {
                                        Ok(new_window) => {
                                            // tauri.conf.json で visible:false のため明示的に show する。
                                            // setup() (lib.rs の通常起動経路) と同じ振る舞いに揃える。
                                            if let Err(e) = new_window.show() {
                                                log::error!("[heartbeat] recreate: show failed: {}", e);
                                            }
                                            if let Err(e) = new_window.set_focus() {
                                                log::warn!("[heartbeat] recreate: set_focus failed: {}", e);
                                            }
                                            log::info!("[heartbeat] recreate: main webview rebuilt and shown (gen={})", my_gen);
                                            // 成功確定後の dead-end 救済: 新窓のフロント bundle ロード
                                            // 失敗 / pong listener 登録到達前のエラーで pong が永久に
                                            // 戻らないケースに備え、300秒経っても pong 復帰しなければ
                                            // フラグを false に戻して再試行可能にする。
                                            // 通常は pong 復帰で即 false にリセットされる経路の方が
                                            // 先に走るため、このタイマーは保険として機能する。
                                            // 自世代と一致するときだけ書き戻し、別 recreate に巻き込まれないようにする。
                                            tokio::time::sleep(Duration::from_secs(300)).await;
                                            if recreate_generation_inner.load(Ordering::Relaxed) == my_gen
                                                && recreate_attempted_inner.load(Ordering::Relaxed)
                                            {
                                                log::warn!(
                                                    "[heartbeat] recreate: 300s elapsed without pong recovery (gen={}), allowing retry",
                                                    my_gen
                                                );
                                                recreate_attempted_inner.store(false, Ordering::Relaxed);
                                            }
                                        }
                                        Err(e) => {
                                            log::error!("[heartbeat] recreate: build failed: {}", e);
                                            // 失敗 → 60秒後にリトライ可能に戻す（連発を抑制）。
                                            // 自世代と一致するときだけ書き戻す。
                                            tokio::time::sleep(Duration::from_secs(60)).await;
                                            if recreate_generation_inner.load(Ordering::Relaxed) == my_gen {
                                                recreate_attempted_inner.store(false, Ordering::Relaxed);
                                                log::info!("[heartbeat] recreate: backoff elapsed (gen={}), retry allowed", my_gen);
                                            }
                                        }
                                    }
                                });
                            }
                            */
                        }
                        match ping_handle.emit("__webview-heartbeat-ping", serde_json::json!({ "ts": now_ms })) {
                            Ok(_) => {
                                if ping_pending_since.is_none() {
                                    ping_pending_since = Some(now_ms);
                                }
                            }
                            Err(e) => {
                                log::error!("[heartbeat] ping emit failed: {}", e);
                                ping_pending_since = None;
                                unresponsive_logged_until_secs = 0;
                            }
                        }
                    }
                });
            }

            // メインスレッド watchdog: run_on_main_thread に投げた閉包の実行遅延を計測し、
            // tao メインスレッド (= WebView2 への emit/IPC 配送を担う UI スレッド) の
            // ブロックを検出する。heartbeat の「webview unresponsive」だけでは
            // 「JS 側フリーズ」と「Rust メインスレッドブロック」を区別できないため、
            // ハング時にどちらかをログから切り分けられるようにする診断機構。
            {
                use std::sync::Arc;
                use std::sync::atomic::{AtomicU64, Ordering};
                use std::time::{Duration, Instant};

                // SystemTime は NTP 補正等で巻き戻り、blocked の誤検知を生むため、
                // 単調な Instant 起点の経過 ms で比較する。
                let watchdog_start = Instant::now();
                let elapsed_ms = move || watchdog_start.elapsed().as_millis() as u64;

                let main_ack_ms = Arc::new(AtomicU64::new(0));
                let watchdog_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut blocked_since: Option<u64> = None;
                    let mut last_blocked_log_ms: u64 = 0;
                    loop {
                        tokio::time::sleep(Duration::from_secs(15)).await;
                        let sent = elapsed_ms();
                        let ack = main_ack_ms.clone();
                        if watchdog_handle
                            .run_on_main_thread(move || {
                                let _bc = main_thread_watch::enter(
                                    main_thread_watch::Activity::WatchdogProbe,
                                );
                                ack.store(elapsed_ms(), Ordering::Relaxed);
                            })
                            .is_err()
                        {
                            // アプリ終了中など。次周期で再試行する。
                            continue;
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        let acked = main_ack_ms.load(Ordering::Relaxed);
                        if acked >= sent {
                            if let Some(since) = blocked_since.take() {
                                log::warn!(
                                    "[main-thread-watchdog] main thread recovered after {}s",
                                    elapsed_ms().saturating_sub(since) / 1000
                                );
                            }
                            last_blocked_log_ms = 0;
                        } else {
                            let first_detection = blocked_since.is_none();
                            let since = *blocked_since.get_or_insert(sent);
                            let blocked_secs = elapsed_ms().saturating_sub(since) / 1000;
                            // ブロック初検知時のみ: UI スレッド ID をログし、プロセス全体の minidump を
                            // 1 回だけ書き出す。停止中のネイティブスタック (WebView2/wry/tray) を後で解析できる。
                            if first_detection {
                                #[cfg(target_os = "windows")]
                                {
                                    log::error!(
                                        "[main-thread-watchdog] UI thread id={} (minidump 内でこのスレッドのスタックを参照)",
                                        main_thread_watch::ui_thread_id()
                                    );
                                    if let Ok(dir) = watchdog_handle.path().app_log_dir() {
                                        let _ = std::fs::create_dir_all(&dir);
                                        let path = dir.join(format!("oretachi-hang-{}.dmp", since));
                                        main_thread_watch::write_hang_minidump_once(path);
                                    }
                                }
                            }
                            // 継続ブロック中の連続出力は 55 秒に 1 回へ抑制（初回は即時）
                            if elapsed_ms().saturating_sub(last_blocked_log_ms) >= 55_000 {
                                last_blocked_log_ms = elapsed_ms();
                                // ブロック中の UI スレッドの「最後のブレッドクラム」を併記する。
                                // label=="idle" なら停止はネイティブ event loop 側 (WebView2/wry/tray)、
                                // それ以外なら当該同期コマンド内での停止と切り分けられる。
                                let bc = main_thread_watch::snapshot();
                                log::error!(
                                    "[main-thread-watchdog] tao main thread blocked for {}s; last main-thread activity: {} (running {}ms) (run_on_main_thread closure not executed within 5s)",
                                    blocked_secs, bc.label, bc.age_ms
                                );
                            }
                        }
                    }
                });
            }

            // 通常モード: MCP サーバー起動 + ウィンドウ表示
            if mcp_enabled {
                let (mcp_port, mcp_remote_access) = {
                    let s = settings_manager.get();
                    (s.mcp_port, s.mcp_remote_access)
                };
                mcp_server::start_mcp_server(app.handle().clone(), mcp_port, mcp_remote_access);
            }

            if let Some(window) = app.get_webview_window("main") {
                // WebView2 ネイティブ診断イベント (ProcessFailed / NewBrowserVersionAvailable) を購読する。
                #[cfg(target_os = "windows")]
                subscribe_webview2_diagnostics(&window);
                let _ = window.show();
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(move |app_handle, event| {
        // RunEvent ハンドラは UI スレッドで走る。Guard の Drop で復元するため、ここを抜けると
        // ブレッドクラムは直前 (通常 idle) に戻る。RunEvent ディスパッチ「中」に停止した場合のみ
        // ブレッドクラムが run-event のまま残り、イベント待ち「中」の停止 (= idle) と区別できる。
        let _bc = main_thread_watch::enter(main_thread_watch::Activity::RunEvent);

        // UI スレッド起因のイベントを可視化する (フリーズ直前の操作を追える)。
        // 高頻度イベント (Moved/Resized/CursorMoved/MainEventsCleared 等) は意図的に除外する:
        // 無条件 DEBUG ログはログ肥大＋UI スレッドのログ I/O が新たなスタール要因になるため。
        match &event {
            tauri::RunEvent::Ready => log::debug!("[run-event] Ready"),
            tauri::RunEvent::Resumed => log::debug!("[run-event] Resumed"),
            tauri::RunEvent::ExitRequested { code, .. } => {
                log::debug!("[run-event] ExitRequested code={:?}", code)
            }
            tauri::RunEvent::WindowEvent { label, event, .. } => match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    log::debug!("[run-event] WindowEvent[{}] CloseRequested", label)
                }
                tauri::WindowEvent::Destroyed => {
                    log::debug!("[run-event] WindowEvent[{}] Destroyed", label)
                }
                tauri::WindowEvent::Focused(focused) => {
                    log::debug!("[run-event] WindowEvent[{}] Focused={}", label, focused)
                }
                tauri::WindowEvent::ThemeChanged(theme) => {
                    log::debug!("[run-event] WindowEvent[{}] ThemeChanged={:?}", label, theme)
                }
                // Moved/Resized/ScaleFactorChanged/CursorMoved 等の高頻度イベントは除外
                _ => {}
            },
            _ => {}
        }

        if let tauri::RunEvent::Exit = event {
            // ① MCP を最優先で停止し、ポート解放を有界に待つ。
            //    stop_and_wait は serve future 完了（= TcpListener drop = ポート解放）まで待つ。
            //    PTY kill が長引いてもポートは先に確実に解放されるため、再起動時の
            //    「固定ポートが掴まれたまま bind 失敗 → 停止中」を防ぐ。
            //    Exit 時点で tokio ランタイムは生存しており block_on は安全（有界 timeout で必ず復帰）。
            if mcp_enabled {
                let mcp_manager = app_handle.state::<mcp_server::McpServerManager>();
                let released = tauri::async_runtime::block_on(
                    mcp_manager.stop_and_wait(std::time::Duration::from_millis(1500)),
                );
                if !released {
                    log::warn!("[exit] MCP stop_and_wait timed out; port may still be held");
                }
                mcp_server::cleanup_port_file(app_handle);
            }

            // ② PTY セッションを終了時専用の高速経路で kill（並列 + 有界 deadline）。
            let pty_manager = app_handle.state::<PtyManager>();
            pty_manager.kill_all_fast("app-exit");

            // ③ ファイルシステムウォッチャーを停止。
            let fs_watcher = app_handle.state::<FsWatcherManager>();
            fs_watcher.stop_all();
        }
    });
}
