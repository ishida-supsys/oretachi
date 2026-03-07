use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn save_terminal_session(
    app_handle: AppHandle,
    worktree_id: String,
    data_json: String,
) -> Result<(), String> {
    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir error: {}", e))?
        .join("terminal-sessions");

    std::fs::create_dir_all(&dir).map_err(|e| format!("Dir create error: {}", e))?;

    let file_path = dir.join(format!("{}.json", worktree_id));
    std::fs::write(&file_path, &data_json).map_err(|e| format!("Write error: {}", e))?;

    Ok(())
}

#[tauri::command]
pub fn load_terminal_session(
    app_handle: AppHandle,
    worktree_id: String,
) -> Result<Option<String>, String> {
    let file_path = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir error: {}", e))?
        .join("terminal-sessions")
        .join(format!("{}.json", worktree_id));

    if !file_path.exists() {
        return Ok(None);
    }

    let content =
        std::fs::read_to_string(&file_path).map_err(|e| format!("Read error: {}", e))?;

    Ok(Some(content))
}

#[tauri::command]
pub fn delete_terminal_session(
    app_handle: AppHandle,
    worktree_id: String,
) -> Result<(), String> {
    let file_path = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir error: {}", e))?
        .join("terminal-sessions")
        .join(format!("{}.json", worktree_id));

    if file_path.exists() {
        std::fs::remove_file(&file_path).map_err(|e| format!("Delete error: {}", e))?;
    }

    Ok(())
}
