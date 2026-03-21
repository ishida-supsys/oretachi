use std::{fs, path::PathBuf, sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH}};

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
pub struct ArtifactParams {
    #[schemars(description = "操作の種類: \"create\"(新規作成) / \"update\"(差分更新) / \"rewrite\"(全置換) / \"get\"(1件取得)")]
    pub command: String,
    #[schemars(description = "アーティファクトを識別する一意なID")]
    pub id: String,
    #[schemars(description = "リポジトリ名")]
    pub repository: String,
    #[schemars(description = "ブランチ名")]
    pub branch: String,
    #[schemars(description = "コンテンツの種類 (create時必須): application/vnd.ant.code, text/markdown, text/html, image/svg+xml, application/vnd.ant.mermaid, application/vnd.ant.react")]
    #[serde(rename = "type")]
    pub content_type: Option<String>,
    #[schemars(description = "アーティファクトのタイトル (create時必須)")]
    pub title: Option<String>,
    #[schemars(description = "アーティファクトの中身 (create/rewrite時必須)")]
    pub content: Option<String>,
    #[schemars(description = "コード言語 (type=application/vnd.ant.code の時のみ)")]
    pub language: Option<String>,
    #[schemars(description = "update時: 置き換え元の文字列 (アーティファクト内に1箇所だけ存在すること)")]
    pub old_str: Option<String>,
    #[schemars(description = "update時: 置き換え後の文字列")]
    pub new_str: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArtifactData {
    id: String,
    #[serde(rename = "type")]
    content_type: String,
    title: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    created_at: u64,
    updated_at: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArtifactParams {
    #[schemars(description = "リポジトリ名")]
    pub repository: String,
    #[schemars(description = "ブランチ名")]
    pub branch: String,
    #[schemars(description = "検索キーワード (省略時は全件返却)。title, content, type, language を対象に部分一致検索")]
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListRepositoryParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetWorktreeStatusParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTaskParams {
    #[schemars(description = "タスクのプロンプト (AIに実行させたい作業の説明)")]
    pub prompt: String,
    #[schemars(description = "リモート実行するかどうか (省略時は false)")]
    pub remote_exec: Option<bool>,
}

#[derive(Debug, Serialize, Clone)]
struct AddTaskEvent {
    prompt: String,
    remote_exec: bool,
}

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

    #[tool(description = "アーティファクトを操作する。create: 新規作成, update: 差分更新(old_str→new_str), rewrite: 全置換, get: 1件取得(全フィールド含む)")]
    fn artifact(
        &self,
        Parameters(ArtifactParams {
            command,
            id,
            repository,
            branch,
            content_type,
            title,
            content,
            language,
            old_str,
            new_str,
        }): Parameters<ArtifactParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let wt = settings
            .worktrees
            .iter()
            .find(|wt| wt.repository_name == repository && wt.branch_name == branch)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "repository='{}', branch='{}' に一致するワークツリーが存在しません",
                        repository, branch
                    ),
                    None,
                )
            })?;
        let worktree_id = &wt.id;

        let artifacts_dir = self
            .app_handle
            .path()
            .app_data_dir()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .join("artifacts")
            .join(worktree_id);

        let artifact_path = artifacts_dir.join(format!("{}.json", id));

        if command == "get" {
            let raw = fs::read_to_string(&artifact_path)
                .map_err(|_| McpError::invalid_params(format!("アーティファクト '{}' が存在しません", id), None))?;
            log::info!("[mcp] artifact command=get id={} worktree_id={}", id, worktree_id);
            return Ok(CallToolResult::success(vec![Content::text(raw)]));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let data = match command.as_str() {
            "create" => {
                let content_type = content_type.ok_or_else(|| {
                    McpError::invalid_params("create には type が必須です".to_string(), None)
                })?;
                let title = title.ok_or_else(|| {
                    McpError::invalid_params("create には title が必須です".to_string(), None)
                })?;
                let content = content.ok_or_else(|| {
                    McpError::invalid_params("create には content が必須です".to_string(), None)
                })?;
                fs::create_dir_all(&artifacts_dir)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                ArtifactData { id: id.clone(), content_type, title, content, language, created_at: now, updated_at: now }
            }
            "update" => {
                let old_str = old_str.ok_or_else(|| {
                    McpError::invalid_params("update には old_str が必須です".to_string(), None)
                })?;
                let new_str = new_str.ok_or_else(|| {
                    McpError::invalid_params("update には new_str が必須です".to_string(), None)
                })?;
                let raw = fs::read_to_string(&artifact_path)
                    .map_err(|_| McpError::invalid_params(format!("アーティファクト '{}' が存在しません", id), None))?;
                let mut data: ArtifactData = serde_json::from_str(&raw)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let count = data.content.matches(&old_str as &str).count();
                if count == 0 {
                    return Err(McpError::invalid_params("old_str がアーティファクト内に見つかりません".to_string(), None));
                }
                if count > 1 {
                    return Err(McpError::invalid_params("old_str がアーティファクト内に複数箇所存在します。より長い文字列を指定してください".to_string(), None));
                }
                data.content = data.content.replacen(&old_str as &str, &new_str, 1);
                data.updated_at = now;
                data
            }
            "rewrite" => {
                let content = content.ok_or_else(|| {
                    McpError::invalid_params("rewrite には content が必須です".to_string(), None)
                })?;
                let raw = fs::read_to_string(&artifact_path)
                    .map_err(|_| McpError::invalid_params(format!("アーティファクト '{}' が存在しません", id), None))?;
                let mut data: ArtifactData = serde_json::from_str(&raw)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                data.content = content;
                data.updated_at = now;
                data
            }
            other => return Err(McpError::invalid_params(
                format!("不明なコマンド '{}'. create / update / rewrite / get のいずれかを指定してください", other),
                None,
            )),
        };

        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        fs::write(&artifact_path, &json)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        log::info!("[mcp] artifact command={} id={} worktree_id={}", command, id, worktree_id);
        self.app_handle
            .emit("artifact-changed", serde_json::json!({
                "worktreeId": worktree_id,
                "artifactId": id,
                "command": command,
            }))
            .ok();
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
                    "id": wt.id,
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

    #[tool(description = "アーティファクトを検索する。queryを省略すると全件返却。title/content/type/languageを対象に部分一致検索。結果はcontentを除いたメタデータのみ")]
    fn search_artifact(
        &self,
        Parameters(SearchArtifactParams { repository, branch, query }): Parameters<SearchArtifactParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let wt = settings
            .worktrees
            .iter()
            .find(|wt| wt.repository_name == repository && wt.branch_name == branch)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "repository='{}', branch='{}' に一致するワークツリーが存在しません",
                        repository, branch
                    ),
                    None,
                )
            })?;
        let worktree_id = &wt.id;

        let artifacts_dir = self
            .app_handle
            .path()
            .app_data_dir()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .join("artifacts")
            .join(worktree_id);

        let mut results: Vec<serde_json::Value> = Vec::new();

        if artifacts_dir.exists() {
            let entries = fs::read_dir(&artifacts_dir)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            for entry in entries {
                let entry = entry.map_err(|e| McpError::internal_error(e.to_string(), None))?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let raw = match fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let data: ArtifactData = match serde_json::from_str(&raw) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                if let Some(ref q) = query {
                    let q_lower = q.to_lowercase();
                    let matches = data.title.to_lowercase().contains(&q_lower)
                        || data.content.to_lowercase().contains(&q_lower)
                        || data.content_type.to_lowercase().contains(&q_lower)
                        || data.language.as_deref().unwrap_or("").to_lowercase().contains(&q_lower);
                    if !matches {
                        continue;
                    }
                }
                let mut meta = serde_json::json!({
                    "id": data.id,
                    "type": data.content_type,
                    "title": data.title,
                    "created_at": data.created_at,
                    "updated_at": data.updated_at,
                });
                if let Some(lang) = data.language {
                    meta["language"] = serde_json::Value::String(lang);
                }
                results.push(meta);
            }
            results.sort_by(|a, b| {
                let a_time = a.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0);
                let b_time = b.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0);
                b_time.cmp(&a_time)
            });
        }

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!(
            "[mcp] search_artifact query={:?} worktree_id={} count={}",
            query, worktree_id, results.len()
        );
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "タスクを追加して実行する。プロンプトを元にAIがタスクコードを生成し、ワークツリー作成やエージェント実行を自動で行う")]
    fn oretachi_add_task(
        &self,
        Parameters(AddTaskParams { prompt, remote_exec }): Parameters<AddTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let remote = remote_exec.unwrap_or(false);
        let event = AddTaskEvent {
            prompt: prompt.clone(),
            remote_exec: remote,
        };
        self.app_handle
            .emit("mcp-add-task", &event)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_add_task: prompt={} remote_exec={}", prompt, remote);
        Ok(CallToolResult::success(vec![Content::text(
            "タスクを追加しました。フロントエンドで生成・実行が開始されます。",
        )]))
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

pub fn start_mcp_server(app_handle: AppHandle, port: u16) {
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

        let listener = match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await {
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
