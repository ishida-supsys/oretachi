mod ai_commit_message;
mod ai_judge;
mod ai_provider;
mod fs_watcher;
mod git_worktree;
mod ide_launcher;
pub mod mcp_server;
mod process_utils;
mod pty_manager;
mod report_db;
mod settings;
mod task_db;
mod task_executor;
mod terminal_session;

#[cfg(target_os = "windows")]
mod acrylic;

use fs_watcher::FsWatcherManager;
use pty_manager::{AiAgentChangedPayload, PtyManager};
use settings::{AppSettings, SettingsManager};
use tauri::{Emitter, Manager, State};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

/// パスコンポーネントに `..`、絶対パス区切り文字、NULバイトが含まれていないか検証する
fn validate_path_component(s: &str) -> Result<(), String> {
    if s.contains("..") || s.contains('/') || s.contains('\\') || s.contains('\0') {
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
fn pty_spawn(
    app_handle: tauri::AppHandle,
    state: State<PtyManager>,
    rows: u16,
    cols: u16,
    shell: Option<String>,
    cwd: Option<String>,
) -> Result<u32, String> {
    log::debug!("[Terminal] cmd::pty_spawn rows={} cols={} shell={:?} cwd={:?}", rows, cols, shell, cwd);
    state.spawn(app_handle, rows, cols, shell, cwd)
}

#[tauri::command]
fn pty_write(state: State<PtyManager>, session_id: u32, data: Vec<u8>) -> Result<(), String> {
    state.write(session_id, data)
}

#[tauri::command]
fn pty_resize(state: State<PtyManager>, session_id: u32, rows: u16, cols: u16) -> Result<(), String> {
    log::debug!("[Terminal] cmd::pty_resize session_id={} rows={} cols={}", session_id, rows, cols);
    state.resize(session_id, rows, cols)
}

#[tauri::command]
fn pty_kill(state: State<PtyManager>, session_id: u32) -> Result<(), String> {
    state.kill(session_id)
}

#[tauri::command]
fn pty_set_ai_agent(
    app_handle: tauri::AppHandle,
    state: State<PtyManager>,
    session_id: u32,
    is_agent: bool,
) -> Result<(), String> {
    state.set_ai_agent(session_id, is_agent)?;
    let mut sessions = std::collections::HashMap::new();
    sessions.insert(session_id, is_agent);
    let _ = app_handle.emit("pty-ai-agent-changed", AiAgentChangedPayload { sessions });
    Ok(())
}

#[tauri::command]
fn pty_is_ai_agent(state: State<PtyManager>, session_id: u32) -> Result<bool, String> {
    state.is_ai_agent(session_id)
}

// ─── 設定コマンド ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: State<SettingsManager>) -> AppSettings {
    state.get()
}

#[tauri::command]
fn save_settings(state: State<SettingsManager>, settings: AppSettings) -> Result<(), String> {
    state.save(settings)
}

#[tauri::command]
fn apply_acrylic_effect(app_handle: tauri::AppHandle, r: u8, g: u8, b: u8, a: u8) {
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
    repo_path: String,
    worktree_path: String,
) -> Result<(), String> {
    // 削除対象ディレクトリをcwdとして掴んでいる子プロセスを先にkill
    let killed = pty_manager.kill_sessions_in_dir(&worktree_path);
    if killed > 0 {
        // プロセスが完全に終了してファイルハンドルを解放するまで待機
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let wp = worktree_path.clone();
    run_git(move || git_worktree::worktree_remove(&repo_path, &worktree_path)).await?;
    if let Some(pool) = app_handle.try_state::<report_db::ReportPool>() {
        let _ = report_db::insert(&pool.0, "worktree_change:remove", &wp).await;
    }
    Ok(())
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
async fn copy_gitignore_targets(
    repo_path: String,
    worktree_path: String,
    targets: Vec<String>,
) -> Result<u32, String> {
    run_git(move || git_worktree::copy_gitignore_targets(&repo_path, &worktree_path, targets)).await
}

#[tauri::command]
async fn write_claude_hooks(
    worktree_path: String,
    worktree_name: String,
    hooks: Vec<crate::settings::NotificationHookEntry>,
) -> Result<(), String> {
    run_git(move || git_worktree::write_claude_hooks(&worktree_path, &worktree_name, hooks)).await
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

// ─── コードレビュー用 Git コマンド ────────────────────────────────────────────

#[tauri::command]
async fn git_list_files(repo_path: String) -> Result<Vec<String>, String> {
    run_git(move || git_worktree::list_tracked_files(&repo_path)).await
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
    ide_launcher::detect_ides()
}

#[tauri::command]
fn detect_ai_agents() -> Vec<ai_provider::AiAgentInfo> {
    ai_provider::detect_ai_agents()
}

#[tauri::command]
fn open_in_ide(command: String, path: String) -> Result<(), String> {
    ide_launcher::open_in_ide(&command, &path)
}

// ─── MCP コマンド ─────────────────────────────────────────────────────────────

#[tauri::command]
fn get_mcp_status(state: State<mcp_server::McpServerManager>) -> mcp_server::McpStatus {
    state.get_status()
}

#[tauri::command]
async fn restart_mcp_server(app_handle: tauri::AppHandle) -> Result<mcp_server::McpStatus, String> {
    let manager = app_handle.state::<mcp_server::McpServerManager>();
    // 同時に複数の restart が走らないよう排他ロックを取得する
    let _lock = manager.acquire_restart_lock().await;
    let mcp_port = {
        let settings_manager = app_handle.state::<SettingsManager>();
        settings_manager.get().mcp_port
    };
    manager.stop_and_wait(std::time::Duration::from_secs(3)).await;
    mcp_server::start_mcp_server(app_handle.clone(), mcp_port);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(manager.get_status())
}

#[tauri::command]
async fn prepare_for_relaunch(app_handle: tauri::AppHandle) -> Result<(), String> {
    let mcp_manager = app_handle.state::<mcp_server::McpServerManager>();
    let _lock = mcp_manager.acquire_restart_lock().await;
    let completed = mcp_manager
        .stop_and_wait(std::time::Duration::from_secs(3))
        .await;
    if !completed {
        return Err("MCP server did not shut down within timeout".into());
    }
    Ok(())
}

// ─── アーティファクトコマンド ─────────────────────────────────────────────────

#[tauri::command]
fn list_artifacts(
    app_handle: tauri::AppHandle,
    worktree_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    let artifacts_dir = artifacts_dir(&app_handle, &worktree_id)?;
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
}

#[tauri::command]
fn read_artifact(
    app_handle: tauri::AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<String, String> {
    validate_path_component(&artifact_id)?;
    let artifact_path = artifacts_dir(&app_handle, &worktree_id)?
        .join(format!("{}.json", artifact_id));
    std::fs::read_to_string(&artifact_path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_artifacts(app_handle: tauri::AppHandle, worktree_id: String) -> Result<(), String> {
    let dir = artifacts_dir(&app_handle, &worktree_id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
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

// ─── FS ウォッチャーコマンド ──────────────────────────────────────────────────

#[tauri::command]
fn start_fs_watch(
    app_handle: tauri::AppHandle,
    state: State<FsWatcherManager>,
    worktree_id: String,
    worktree_path: String,
) -> Result<(), String> {
    state.start_watching(app_handle, worktree_id, worktree_path)
}

#[tauri::command]
fn stop_fs_watch(state: State<FsWatcherManager>, worktree_id: String) -> Result<(), String> {
    state.stop_watching(&worktree_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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

    #[cfg(target_os = "windows")]
    let (acrylic_enabled, acrylic_opacity, acrylic_color) = acrylic::load_settings();
    #[cfg(not(target_os = "windows"))]
    let acrylic_enabled = true;

    let mut builder = tauri::Builder::default();
    if acrylic_enabled {
        builder = builder.plugin(tauri_plugin_liquid_glass::init());

        #[cfg(target_os = "windows")]
        {
            let (r, g, b) = acrylic::parse_color(&acrylic_color);
            let a = acrylic_opacity;
            builder = builder.plugin(
                tauri::plugin::Builder::<tauri::Wry>::new("acrylic-effect")
                    .on_window_ready(move |window| {
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
        .plugin(tauri_plugin_cli::init())
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
        .plugin(tauri_plugin_process::init());

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp::init_with_config(
            tauri_plugin_mcp::PluginConfig::new("oretachi".to_string()).tcp_localhost(4000),
        ));
    }

    let app = builder
        .manage(PtyManager::new())
        .manage(SettingsManager::new())
        .manage(mcp_server::McpServerManager::new())
        .manage(ai_judge::ApprovalManager::new())
        .manage(ai_commit_message::CommitMessageManager::new())
        .manage(task_executor::TaskGenerateManager::new())
        .manage(FsWatcherManager::new())
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
            git_worktree_add,
            git_worktree_remove,
            git_list_branches,
            detect_package_manager,
            read_gitignore,
            copy_gitignore_targets,
            write_claude_hooks,
            git_merge_branch,
            git_delete_branch,
            git_list_files,
            git_read_file,
            git_get_merge_message,
            git_get_status,
            git_get_file_diff,
            git_get_log,
            git_stage_all,
            git_commit,
            detect_ides,
            detect_ai_agents,
            open_in_ide,
            get_mcp_status,
            restart_mcp_server,
            prepare_for_relaunch,
            ai_judge::judge_approval,
            ai_judge::cancel_approval,
            ai_commit_message::generate_commit_message,
            ai_commit_message::cancel_commit_message_generation,
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
        ])
        .setup(|app| {
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

            // .env 読み込み（Vite の .env 規約に準拠）
            let _ = dotenvy::from_filename(".env");
            if cfg!(debug_assertions) {
                let _ = dotenvy::from_filename_override(".env.development");
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

            if let Ok(log_dir) = app.path().app_log_dir() {
                log::info!("Application log directory: {:?}", log_dir);
            }
            let settings_manager = app.state::<SettingsManager>();
            settings_manager.init(app.handle());

            // AIエージェントインジケーター用ポーリング起動
            let pty_manager = app.state::<PtyManager>();
            pty_manager.start_polling(app.handle().clone());

            // 通常モード: MCP サーバー起動 + ウィンドウ表示
            let mcp_port = settings_manager.get().mcp_port;
            mcp_server::start_mcp_server(app.handle().clone(), mcp_port);

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            let pty_manager = app_handle.state::<PtyManager>();
            pty_manager.kill_all();
            let mcp_manager = app_handle.state::<mcp_server::McpServerManager>();
            mcp_manager.stop();
            mcp_server::cleanup_port_file(app_handle);
            let fs_watcher = app_handle.state::<FsWatcherManager>();
            fs_watcher.stop_all();
        }
    });
}
