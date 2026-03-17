use std::{fs, path::PathBuf, sync::{Arc, Mutex}};

use axum::{extract::State, http::StatusCode, routing::post, Json};
use rmcp::{
    schemars, ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

use crate::git_worktree::get_git_remotes;
use crate::settings::SettingsManager;

const PORT_FILE: &str = "mcp-port";

// ─── MCP Server Manager ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct McpStatus {
    pub running: bool,
    pub port: Option<u16>,
}

pub struct McpServerManager {
    shutdown_tx: Mutex<Option<watch::Sender<bool>>>,
    status: Arc<Mutex<McpStatus>>,
}

impl McpServerManager {
    pub fn new() -> Self {
        Self {
            shutdown_tx: Mutex::new(None),
            status: Arc::new(Mutex::new(McpStatus { running: false, port: None })),
        }
    }

    pub fn stop(&self) {
        if let Ok(guard) = self.shutdown_tx.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(true);
            }
        }
    }

    pub fn get_status(&self) -> McpStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or(McpStatus { running: false, port: None })
    }
}

// ─── Request types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct NotifyPayload {
    pub worktree: String,
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NotifyWorktreeParams {
    #[schemars(description = "通知するワークツリー名")]
    pub worktree_name: String,
    #[schemars(description = "通知種別: \"approval\"(承認待ち) / \"completed\"(作業完了) / \"general\"(汎用)。省略時は \"general\"")]
    pub kind: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct NotifyWorktreeEvent {
    pub worktree_name: String,
    pub kind: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListRepositoryParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetWorktreeStatusParams {}

// ─── MCP Service ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct NotifyService {
    app_handle: AppHandle,
    tool_router: ToolRouter<NotifyService>,
}

#[tool_router]
impl NotifyService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "ワークツリーに通知を送信する")]
    fn notify_worktree(
        &self,
        Parameters(NotifyWorktreeParams { worktree_name, kind }): Parameters<NotifyWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let event = NotifyWorktreeEvent {
            worktree_name: worktree_name.clone(),
            kind: kind.unwrap_or_else(|| "general".to_string()),
        };
        self.app_handle
            .emit("notify-worktree", &event)
            .map_err(|e: tauri::Error| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] notify_worktree: {} kind={}", worktree_name, event.kind);
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(description = "登録済みワークツリーのステータス一覧を取得する")]
    fn oretachi_get_worktree_status(
        &self,
        Parameters(_params): Parameters<GetWorktreeStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let detached: std::collections::HashSet<&str> =
            settings.detached_worktree_ids.iter().map(|s| s.as_str()).collect();

        let results: Vec<serde_json::Value> = settings
            .worktrees
            .iter()
            .map(|wt| {
                serde_json::json!({
                    "name": wt.name,
                    "repositoryName": wt.repository_name,
                    "branchName": wt.branch_name,
                    "isDetached": detached.contains(wt.id.as_str()),
                    "autoApproval": wt.auto_approval,
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_get_worktree_status: {} entries", results.len());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all registered repositories with their names and git remote URLs")]
    fn oretachi_list_repository(
        &self,
        Parameters(_params): Parameters<ListRepositoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let repos: Vec<serde_json::Value> = settings
            .repositories
            .iter()
            .map(|repo| {
                let remotes = get_git_remotes(&repo.path);
                serde_json::json!({ "name": repo.name, "remotes": remotes })
            })
            .collect();
        let json = serde_json::to_string_pretty(&repos)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_list_repository: {} repos", repos.len());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for NotifyService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "oretachi".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                title: Some("oretachi 通知サーバー".to_string()),
                ..Default::default()
            },
            instructions: Some("ワークツリーへの通知を管理します".to_string()),
        }
    }
}

// ─── Simple REST endpoint (/notify) ──────────────────────────────────────────

async fn notify_handler(
    State(app_handle): State<AppHandle>,
    Json(payload): Json<NotifyPayload>,
) -> StatusCode {
    let event = NotifyWorktreeEvent {
        worktree_name: payload.worktree.clone(),
        kind: payload.kind.unwrap_or_else(|| "general".to_string()),
    };
    match app_handle.emit("notify-worktree", &event) {
        Ok(_) => {
            log::info!("[notify] worktree={} kind={}", payload.worktree, event.kind);
            StatusCode::OK
        }
        Err(e) => {
            log::error!("Failed to emit notify-worktree: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// ─── Port file management ─────────────────────────────────────────────────────

fn port_file_path(app_handle: &AppHandle) -> Option<PathBuf> {
    app_handle
        .path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(PORT_FILE))
}

fn write_port_file(app_handle: &AppHandle, port: u16) {
    if let Some(path) = port_file_path(app_handle) {
        // MCP_PORT_OVERWRITE=false なら既存ファイルを上書きしない
        let overwrite = std::env::var("MCP_PORT_OVERWRITE")
            .map(|v| v != "false")
            .unwrap_or(true);
        if !overwrite && path.exists() {
            log::info!("MCP port file already exists, skipping overwrite (MCP_PORT_OVERWRITE=false)");
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&path, port.to_string()) {
            log::warn!("Failed to write port file: {}", e);
        }
    }
}

pub fn read_port_file(app_handle: &AppHandle) -> Result<u16, String> {
    let path = port_file_path(app_handle)
        .ok_or_else(|| "Cannot determine app data dir".to_string())?;
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read port file (is oretachi running?): {}", e))?;
    content
        .trim()
        .parse::<u16>()
        .map_err(|e| format!("Invalid port in port file: {}", e))
}

pub fn cleanup_port_file(app_handle: &AppHandle) {
    if let Some(path) = port_file_path(app_handle) {
        let _ = fs::remove_file(path);
    }
}

// ─── Server startup ───────────────────────────────────────────────────────────

pub fn start_mcp_server(app_handle: AppHandle) {
    let manager = app_handle.state::<McpServerManager>();

    // 既存サーバーを停止
    manager.stop();

    // 新しいシャットダウンチャンネルを作成
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    if let Ok(mut tx) = manager.shutdown_tx.lock() {
        *tx = Some(shutdown_tx);
    }

    // Arc クローンをタスクに渡す
    let status = Arc::clone(&manager.status);
    drop(manager);

    tauri::async_runtime::spawn(async move {
        let service = StreamableHttpService::new(
            {
                let ah = app_handle.clone();
                move || Ok(NotifyService::new(ah.clone()))
            },
            LocalSessionManager::default().into(),
            Default::default(),
        );

        let router = axum::Router::new()
            .nest_service("/mcp", service)
            .route("/notify", post(notify_handler))
            .with_state(app_handle.clone());

        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(e) => {
                log::error!("Failed to bind MCP server: {}", e);
                return;
            }
        };

        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                log::error!("Failed to get MCP server local addr: {}", e);
                return;
            }
        };

        write_port_file(&app_handle, port);
        log::info!("MCP server listening on http://127.0.0.1:{}/mcp", port);

        // ステータス: 起動中
        if let Ok(mut s) = status.lock() {
            s.running = true;
            s.port = Some(port);
        }

        let mut rx = shutdown_rx;
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                // シャットダウン信号を待つ
                while rx.changed().await.is_ok() && !*rx.borrow() {}
            })
            .await
        {
            log::error!("MCP server exited with error: {}", e);
        }

        log::info!("[mcp] Shutdown signal received, server stopped");

        // ステータス: 停止
        if let Ok(mut s) = status.lock() {
            s.running = false;
            s.port = None;
        }
    });
}

// ─── CLI notification sender (standalone, no AppHandle needed) ───────────────

pub fn send_notification_standalone(worktree_name: &str, kind: Option<&str>) -> Result<(), String> {
    let port = read_port_standalone()?;
    let body = serde_json::json!({
        "worktree": worktree_name,
        "kind": kind.unwrap_or("general"),
    }).to_string();
    let request = format!(
        "POST /notify HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    use std::io::{Read, Write};
    use std::time::Duration;

    let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{}", port))
        .map_err(|e| format!("Cannot connect to oretachi MCP server: {}", e))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set timeout: {}", e))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("Failed to send notification: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush: {}", e))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let response_str = String::from_utf8_lossy(&response);
    if !response_str.contains("200") {
        return Err(format!("Server returned unexpected response: {}", response_str));
    }
    Ok(())
}

fn read_port_standalone() -> Result<u16, String> {
    #[cfg(target_os = "windows")]
    let path = {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| "APPDATA environment variable not set".to_string())?;
        PathBuf::from(appdata).join("com.ia.oretachi").join(PORT_FILE)
    };

    #[cfg(not(target_os = "windows"))]
    let path = {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME environment variable not set".to_string())?;
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("com.ia.oretachi")
            .join(PORT_FILE)
    };
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read port file (is oretachi running?): {}", e))?;
    content
        .trim()
        .parse::<u16>()
        .map_err(|e| format!("Invalid port in port file: {}", e))
}
