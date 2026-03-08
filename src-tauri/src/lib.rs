mod ai_judge;
mod ai_provider;
mod git_worktree;
mod ide_launcher;
pub mod mcp_server;
mod process_utils;
mod pty_manager;
mod settings;
mod task_executor;
mod terminal_session;

use pty_manager::PtyManager;
use settings::{AppSettings, SettingsManager};
use tauri::{Manager, State};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

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
    state.spawn(app_handle, rows, cols, shell, cwd)
}

#[tauri::command]
fn pty_write(state: State<PtyManager>, session_id: u32, data: Vec<u8>) -> Result<(), String> {
    state.write(session_id, data)
}

#[tauri::command]
fn pty_resize(state: State<PtyManager>, session_id: u32, rows: u16, cols: u16) -> Result<(), String> {
    state.resize(session_id, rows, cols)
}

#[tauri::command]
fn pty_kill(state: State<PtyManager>, session_id: u32) -> Result<(), String> {
    state.kill(session_id)
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
    repo_path: String,
    worktree_path: String,
    branch_name: String,
) -> Result<bool, String> {
    run_git(move || git_worktree::worktree_add(&repo_path, &worktree_path, &branch_name)).await
}

#[tauri::command]
async fn git_worktree_remove(repo_path: String, worktree_path: String) -> Result<(), String> {
    run_git(move || git_worktree::worktree_remove(&repo_path, &worktree_path)).await
}

#[tauri::command]
async fn git_list_branches(repo_path: String) -> Result<Vec<String>, String> {
    run_git(move || git_worktree::list_branches(&repo_path)).await
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
    {
        let manager = app_handle.state::<mcp_server::McpServerManager>();
        manager.stop();
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    mcp_server::start_mcp_server(app_handle.clone());
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let manager = app_handle.state::<mcp_server::McpServerManager>();
    Ok(manager.get_status())
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

    let app = tauri::Builder::default()
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
                .with_denylist(&["tray-popup"])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init())
        .manage(PtyManager::new())
        .manage(SettingsManager::new())
        .manage(mcp_server::McpServerManager::new())
        .manage(ai_judge::ApprovalManager::new())
        .invoke_handler(tauri::generate_handler![
            pty_spawn,
            pty_write,
            pty_resize,
            pty_kill,
            get_settings,
            save_settings,
            git_validate_repo,
            git_worktree_add,
            git_worktree_remove,
            git_list_branches,
            git_merge_branch,
            git_delete_branch,
            git_list_files,
            git_read_file,
            git_get_status,
            git_get_file_diff,
            git_get_log,
            detect_ides,
            detect_ai_agents,
            open_in_ide,
            get_mcp_status,
            restart_mcp_server,
            ai_judge::judge_approval,
            ai_judge::cancel_approval,
            terminal_session::save_terminal_session,
            terminal_session::load_terminal_session,
            terminal_session::delete_terminal_session,
            task_executor::task_generate,
            task_executor::write_temp_prompt,
        ])
        .setup(|app| {
            if let Ok(log_dir) = app.path().app_log_dir() {
                log::info!("Application log directory: {:?}", log_dir);
            }
            let settings_manager = app.state::<SettingsManager>();
            settings_manager.init(app.handle());

            // 通常モード: MCP サーバー起動 + ウィンドウ表示
            mcp_server::start_mcp_server(app.handle().clone());

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
        }
    });
}
