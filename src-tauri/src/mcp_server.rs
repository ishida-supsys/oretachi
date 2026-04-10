use std::{collections::HashMap, fs, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}}, time::{SystemTime, UNIX_EPOCH}};
use tokio::fs as tokio_fs;

use axum::{extract::{Request, State}, http::StatusCode, middleware::{self, Next}, response::Response, routing::post, Json};
use rmcp::{
    schemars, ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    service::{NotificationContext, Peer, RoleServer},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::sync::{oneshot, watch, RwLock};

use crate::git_worktree::get_git_remotes;
use crate::settings::SettingsManager;

const PORT_FILE: &str = "mcp-port";
const SERVER_INFO_FILE: &str = "mcp-server.json";

// ─── Peer Registry (接続中クライアントの管理) ─────────────────────────────────

pub type PeerMap = Arc<RwLock<HashMap<u64, Peer<RoleServer>>>>;
/// ピアごとの連続タイムアウト回数を記録する。3回連続でタイムアウトしたピアを dead と判定する。
pub type PeerTimeoutCounts = Arc<Mutex<HashMap<u64, u32>>>;
const PEER_TIMEOUT_THRESHOLD: u32 = 3;
const PEER_NOTIFY_TIMEOUT_SECS: u64 = 5;

/// 接続中のMCPクライアントのPeerを保持するTauri managed state
pub struct McpPeerRegistry(pub PeerMap);

static PEER_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

// ─── MCP Server Manager ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct McpStatus {
    pub running: bool,
    pub port: Option<u16>,
}

pub struct McpServerManager {
    shutdown_tx: Mutex<Option<watch::Sender<bool>>>,
    /// サーバーが実際に停止したときに通知される oneshot receiver
    shutdown_complete_rx: Mutex<Option<oneshot::Receiver<()>>>,
    status: Arc<Mutex<McpStatus>>,
    /// restart_mcp_server の同時呼び出しを防ぐ排他ロック
    restart_lock: tokio::sync::Mutex<()>,
    /// サーバー起動のたびにインクリメントされる世代カウンタ
    /// 旧世代のタスクが status を上書きするのを防ぐ
    generation: Arc<AtomicU64>,
    /// worktree-archived リスナーID（再起動時にアンリジスターするために保持）
    archive_listener_id: Mutex<Option<tauri::EventId>>,
    /// worktree-added リスナーID（再起動時にアンリジスターするために保持）
    added_listener_id: Mutex<Option<tauri::EventId>>,
}

impl McpServerManager {
    pub fn new() -> Self {
        Self {
            shutdown_tx: Mutex::new(None),
            shutdown_complete_rx: Mutex::new(None),
            status: Arc::new(Mutex::new(McpStatus { running: false, port: None })),
            restart_lock: tokio::sync::Mutex::new(()),
            generation: Arc::new(AtomicU64::new(0)),
            archive_listener_id: Mutex::new(None),
            added_listener_id: Mutex::new(None),
        }
    }

    pub async fn acquire_restart_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.restart_lock.lock().await
    }

    pub fn stop(&self) {
        if let Ok(guard) = self.shutdown_tx.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(true);
            }
        }
    }

    /// stop() を呼び出してから、サーバーが実際に停止するまで待つ。
    /// タイムアウト内に停止すれば true を返す。
    pub async fn stop_and_wait(&self, timeout: std::time::Duration) -> bool {
        self.stop();
        let rx = self.shutdown_complete_rx.lock().ok().and_then(|mut g| g.take());
        if let Some(rx) = rx {
            tokio::time::timeout(timeout, rx).await.is_ok()
        } else {
            true
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
    #[schemars(description = "コンテンツの種類 (create時必須): application/vnd.ant.code, text/markdown, text/html, image/svg+xml, application/vnd.ant.mermaid, application/vnd.ant.react (Tailwind CSSユーティリティクラス利用可)")]
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CloseWorktreeParams {
    #[schemars(description = "クローズするワークツリーの名前")]
    pub worktree_name: String,
    #[schemars(description = "ワークツリーID（oretachi_get_worktree_statusで取得）。同名ワークツリーが複数ある場合はIDで特定する")]
    pub worktree_id: Option<String>,
    #[schemars(description = "削除前にマージするブランチ名（省略時はマージなし）")]
    pub merge_to: Option<String>,
    #[schemars(description = "ワークツリー削除後にブランチを削除するか（省略時は false）")]
    pub delete_branch: Option<bool>,
    #[schemars(description = "未マージでもブランチを強制削除するか（省略時は false）")]
    pub force_branch: Option<bool>,
}

#[derive(Debug, Serialize, Clone)]
struct CloseWorktreeEvent {
    worktree_id: String,
    worktree_name: String,
    merge_to: String,
    delete_branch: bool,
    force_branch: bool,
}

// ─── MCP Service ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct NotifyService {
    app_handle: AppHandle,
    tool_router: ToolRouter<NotifyService>,
    peer_registry: PeerMap,
}

#[tool_router]
impl NotifyService {
    pub fn new(app_handle: AppHandle, peer_registry: PeerMap) -> Self {
        Self {
            app_handle,
            tool_router: Self::tool_router(),
            peer_registry,
        }
    }

    #[tool(description = "アーティファクトを操作する。create: 新規作成, update: 差分更新(old_str→new_str), rewrite: 全置換, get: 1件取得(全フィールド含む)")]
    async fn artifact(
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
        let worktree_id = wt.id.clone();

        // artifact ID のパストラバーサル防止
        if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
            return Err(McpError::invalid_params("不正なアーティファクトIDです".to_string(), None));
        }

        let artifacts_dir = self
            .app_handle
            .path()
            .app_data_dir()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .join("artifacts")
            .join(&worktree_id);

        let artifact_path = artifacts_dir.join(format!("{}.json", id));

        if command == "get" {
            let raw = tokio_fs::read_to_string(&artifact_path).await
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
                tokio_fs::create_dir_all(&artifacts_dir).await
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
                let raw = tokio_fs::read_to_string(&artifact_path).await
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
                let raw = tokio_fs::read_to_string(&artifact_path).await
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
        tokio_fs::write(&artifact_path, &json).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        log::info!("[mcp] artifact command={} id={} worktree_id={}", command, id, worktree_id);
        if let Err(e) = self.app_handle.emit("artifact-changed", serde_json::json!({
                "worktreeId": worktree_id,
                "artifactId": id,
                "command": command,
            })) {
            log::warn!("Failed to emit artifact-changed: {}", e);
        }
        if command == "create" {
            if let Some(pool) = self.app_handle.try_state::<crate::report_db::ReportPool>() {
                let _ = crate::report_db::insert(&pool.inner().0, "artifact_change:create", &id).await;
            }
        }
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
    async fn oretachi_list_repository(
        &self,
        Parameters(_params): Parameters<ListRepositoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let paths: Vec<(String, String)> = settings
            .repositories
            .iter()
            .map(|repo| (repo.name.clone(), repo.path.clone()))
            .collect();
        let repos: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
            paths
                .iter()
                .map(|(name, path)| {
                    let remotes = get_git_remotes(path);
                    serde_json::json!({ "name": name, "remotes": remotes })
                })
                .collect()
        })
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&repos)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_list_repository: {} repos", repos.len());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "アーティファクトを検索する。queryを省略すると全件返却。title/content/type/languageを対象に部分一致検索。結果はcontentを除いたメタデータのみ")]
    async fn search_artifact(
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
        let worktree_id = wt.id.clone();

        let artifacts_dir = self
            .app_handle
            .path()
            .app_data_dir()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .join("artifacts")
            .join(&worktree_id);

        let query_log = query.clone();
        let results: Vec<serde_json::Value> = if artifacts_dir.exists() {
            tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, McpError> {
                let mut results = Vec::new();
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
                Ok(results)
            })
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))??
        } else {
            Vec::new()
        };

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!(
            "[mcp] search_artifact query={:?} worktree_id={} count={}",
            query_log, worktree_id, results.len()
        );
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "タスク追加リクエストを送信する。AIがタスクコードを生成し、ワークツリー作成やエージェント実行を非同期で行う")]
    fn oretachi_add_task(
        &self,
        Parameters(AddTaskParams { prompt, remote_exec }): Parameters<AddTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(McpError::invalid_params("prompt must not be empty", None));
        }
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
            "タスク追加リクエストを送信しました。タスクの生成・実行は非同期に行われます。",
        )]))
    }

    #[tool(description = "ワークツリーをアーカイブ（クローズ）する。アーカイブDBに記録してgitワークツリーを削除する")]
    fn oretachi_close_worktree(
        &self,
        Parameters(CloseWorktreeParams { worktree_name, worktree_id, merge_to, delete_branch, force_branch }): Parameters<CloseWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_name = worktree_name.trim().to_string();
        if worktree_name.is_empty() {
            return Err(McpError::invalid_params("worktree_name must not be empty", None));
        }

        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();

        let wt = if let Some(id) = worktree_id.as_deref() {
            // IDが指定されている場合はIDで特定
            settings.worktrees.iter().find(|w| w.id == id).ok_or_else(|| {
                McpError::invalid_params(format!("worktree id '{}' not found", id), None)
            })?
        } else {
            // 名前で特定（同名が複数ある場合はエラー）
            let matches: Vec<_> = settings.worktrees.iter().filter(|w| w.name == worktree_name).collect();
            match matches.len() {
                0 => {
                    let names: Vec<&str> = settings.worktrees.iter().map(|w| w.name.as_str()).collect();
                    return Err(McpError::invalid_params(
                        format!("worktree '{}' not found. available: [{}]", worktree_name, names.join(", ")),
                        None,
                    ));
                }
                1 => matches[0],
                _ => {
                    let ids: Vec<&str> = matches.iter().map(|w| w.id.as_str()).collect();
                    return Err(McpError::invalid_params(
                        format!("multiple worktrees named '{}'. specify worktree_id to disambiguate: [{}]", worktree_name, ids.join(", ")),
                        None,
                    ));
                }
            }
        };

        let event = CloseWorktreeEvent {
            worktree_id: wt.id.clone(),
            worktree_name: wt.name.clone(),
            merge_to: merge_to.unwrap_or_default(),
            delete_branch: delete_branch.unwrap_or(false),
            force_branch: force_branch.unwrap_or(false),
        };
        self.app_handle
            .emit("mcp-close-worktree", &event)
            .map_err(|e: tauri::Error| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_close_worktree: name={}", worktree_name);
        Ok(CallToolResult::success(vec![Content::text(
            "ワークツリーのクローズリクエストを送信しました。処理は非同期に行われます。",
        )]))
    }
}

#[tool_handler]
impl ServerHandler for NotifyService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_logging()
                .build(),
            server_info: Implementation {
                name: "oretachi".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                title: Some("oretachi 通知サーバー".to_string()),
                ..Default::default()
            },
            instructions: Some("ワークツリーへの通知を管理します".to_string()),
        }
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let peer = context.peer.clone();
        let registry = self.peer_registry.clone();
        async move {
            let id = PEER_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
            registry.write().await.insert(id, peer);
            log::info!("[mcp] client connected, peer_id={}", id);
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

// ─── API Key Authentication Middleware ───────────────────────────────────────

async fn api_key_auth(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // axum の Extensions から API キーを取得
    let expected_key = request
        .extensions()
        .get::<ApiKeyState>()
        .map(|s| s.0.clone())
        .unwrap_or_default();

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    // APIキーが未設定の場合は全リクエストを拒否（空文字による認証バイパスを防ぐ）
    if expected_key.is_empty() {
        log::warn!("[mcp] API key not configured, rejecting all requests");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let authorized = match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let provided = header[7..].as_bytes();
            let expected = expected_key.as_bytes();
            // 定数時間比較でタイミング攻撃を防ぐ
            use subtle::ConstantTimeEq;
            provided.len() == expected.len() && provided.ct_eq(expected).into()
        }
        _ => false,
    };

    if authorized {
        Ok(next.run(request).await)
    } else {
        log::warn!("[mcp] unauthorized request: missing or invalid API key");
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[derive(Clone)]
struct ApiKeyState(String);

// ─── MCP Notification Broadcast ──────────────────────────────────────────────

/// 全接続クライアントに通知を送信する共通ヘルパー。
/// タイムアウトと明示的エラーで dead peer を管理する。
async fn broadcast_notification(peer_registry: &PeerMap, timeout_counts: &PeerTimeoutCounts, params: LoggingMessageNotificationParam) {
    // readロックを保持したままawaitしないよう、先にPeerをcloneしてロックを解放する
    let peer_snapshot: Vec<(u64, Peer<RoleServer>)> = {
        let peers = peer_registry.read().await;
        peers.iter().map(|(k, v)| (*k, v.clone())).collect()
    };

    let mut dead_peers: Vec<u64> = Vec::new();
    for (peer_id, peer) in &peer_snapshot {
        match tokio::time::timeout(
            std::time::Duration::from_secs(PEER_NOTIFY_TIMEOUT_SECS),
            peer.notify_logging_message(params.clone()),
        )
        .await
        {
            Ok(Ok(())) => {
                // 成功したらタイムアウトカウンタをリセット
                let mut counts = timeout_counts.lock().unwrap_or_else(|e| e.into_inner());
                counts.remove(peer_id);
            }
            Ok(Err(e)) => {
                // 明示的な送信エラーは即 dead と判定して削除する
                log::warn!("[mcp] notify_logging_message failed for peer_id={}: {}", peer_id, e);
                dead_peers.push(*peer_id);
            }
            Err(_) => {
                // 連続タイムアウトが閾値を超えたら dead と判定する
                let count = {
                    let mut counts = timeout_counts.lock().unwrap_or_else(|e| e.into_inner());
                    let c = counts.entry(*peer_id).or_insert(0);
                    *c += 1;
                    *c
                };
                log::warn!("[mcp] notify_logging_message timed out for peer_id={} (count={})", peer_id, count);
                if count >= PEER_TIMEOUT_THRESHOLD {
                    log::warn!("[mcp] removing peer_id={} after {} consecutive timeouts", peer_id, count);
                    dead_peers.push(*peer_id);
                }
            }
        }
    }
    if !dead_peers.is_empty() {
        let mut peers = peer_registry.write().await;
        for peer_id in &dead_peers {
            peers.remove(peer_id);
        }
        let mut counts = timeout_counts.lock().unwrap_or_else(|e| e.into_inner());
        for peer_id in dead_peers {
            counts.remove(&peer_id);
        }
    }
}

/// アーカイブされたワークツリーの情報を全接続クライアントに通知する
async fn broadcast_worktree_archived(peer_registry: &PeerMap, timeout_counts: &PeerTimeoutCounts, name: &str, id: &str, branch: &str) {
    let params = LoggingMessageNotificationParam {
        level: LoggingLevel::Warning,
        logger: Some("oretachi".to_string()),
        data: serde_json::json!({
            "event": "worktree_archived",
            "worktreeId": id,
            "worktreeName": name,
            "branchName": branch,
        }),
    };
    broadcast_notification(peer_registry, timeout_counts, params).await;
}

async fn broadcast_worktree_added(peer_registry: &PeerMap, timeout_counts: &PeerTimeoutCounts, name: &str, id: &str, branch: &str) {
    let params = LoggingMessageNotificationParam {
        level: LoggingLevel::Info,
        logger: Some("oretachi".to_string()),
        data: serde_json::json!({
            "event": "worktree_added",
            "worktreeId": id,
            "worktreeName": name,
            "branchName": branch,
        }),
    };
    broadcast_notification(peer_registry, timeout_counts, params).await;
}

// ─── Port file management ─────────────────────────────────────────────────────

fn port_file_path(app_handle: &AppHandle) -> Option<PathBuf> {
    app_handle
        .path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(PORT_FILE))
}

fn server_info_file_path(app_handle: &AppHandle) -> Option<PathBuf> {
    app_handle
        .path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(SERVER_INFO_FILE))
}

fn write_server_info_file(app_handle: &AppHandle, port: u16, api_key: &str) {
    // MCP_PORT_OVERWRITE=false はポートの上書きのみを制限する。
    // APIキーは再生成後の再起動でも常に最新値が必要なため、常に書き込む。
    let overwrite_port = std::env::var("MCP_PORT_OVERWRITE")
        .map(|v| v != "false")
        .unwrap_or(true);

    // mcp-server.json を書き込む（ポート確定値 or キー更新のため常に更新）
    if let Some(path) = server_info_file_path(app_handle) {
        // ポート上書き禁止かつ既存ファイルがある場合: ポートは既存値を使い、APIキーのみ更新
        let effective_port = if !overwrite_port && path.exists() {
            // 既存ファイルからポートを読み取る
            fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|v| v["port"].as_u64())
                .map(|p| p as u16)
                .unwrap_or(port)
        } else {
            port
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let info = serde_json::json!({ "port": effective_port, "apiKey": api_key });
        if let Err(e) = fs::write(&path, serde_json::to_string_pretty(&info).unwrap_or_default()) {
            log::warn!("Failed to write server info file: {}", e);
        }
    }

    // 後方互換: 旧 mcp-port テキストファイルも書き込む
    // 後方互換: 旧 mcp-port テキストファイルも書き込む（ポート上書き制限を適用）
    if let Some(path) = port_file_path(app_handle) {
        if overwrite_port || !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::write(&path, port.to_string()) {
                log::warn!("Failed to write port file: {}", e);
            }
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
    if let Some(path) = server_info_file_path(app_handle) {
        let _ = fs::remove_file(path);
    }
}

// ─── Server startup ───────────────────────────────────────────────────────────

pub fn start_mcp_server(app_handle: AppHandle, port: u16, remote_access: bool) {
    let manager = app_handle.state::<McpServerManager>();

    // 既存サーバーを停止
    manager.stop();

    // 新しいシャットダウンチャンネルを作成
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    if let Ok(mut tx) = manager.shutdown_tx.lock() {
        *tx = Some(shutdown_tx);
    }

    // サーバー停止完了通知用 oneshot チャンネル
    let (complete_tx, complete_rx) = oneshot::channel::<()>();
    if let Ok(mut rx_guard) = manager.shutdown_complete_rx.lock() {
        *rx_guard = Some(complete_rx);
    }

    // 世代をインクリメントして旧タスクによる status 上書きを防ぐ
    let my_generation = manager.generation.fetch_add(1, Ordering::SeqCst) + 1;

    // Arc クローンをタスクに渡す
    let status = Arc::clone(&manager.status);
    let generation = Arc::clone(&manager.generation);

    // 前回起動時のリスナーがあればアンリジスター（再起動による重複防止）
    if let Ok(mut guard) = manager.archive_listener_id.lock() {
        if let Some(old_id) = guard.take() {
            app_handle.unlisten(old_id);
        }
    }
    if let Ok(mut guard) = manager.added_listener_id.lock() {
        if let Some(old_id) = guard.take() {
            app_handle.unlisten(old_id);
        }
    }

    drop(manager);

    // APIキーをsettingsから読み取り
    let api_key = {
        let settings_manager = app_handle.state::<SettingsManager>();
        settings_manager.get().mcp_api_key.clone()
    };

    // peer レジストリを取得（managed state から）
    let peer_map = app_handle.state::<McpPeerRegistry>().0.clone();
    // ピアごとの連続タイムアウトカウンタ（両リスナー間で共有）
    let timeout_counts: PeerTimeoutCounts = Arc::new(Mutex::new(HashMap::new()));

    // ワークツリーアーカイブ時に全クライアントへ通知
    let peer_map_for_listener = peer_map.clone();
    let timeout_counts_for_listener = timeout_counts.clone();
    let listener_id = app_handle.listen("worktree-archived", move |event: tauri::Event| {
        let registry = peer_map_for_listener.clone();
        let tc = timeout_counts_for_listener.clone();
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) {
            let name = payload["name"].as_str().unwrap_or("unknown").to_string();
            let id = payload["id"].as_str().unwrap_or("").to_string();
            let branch = payload["branchName"].as_str().unwrap_or("").to_string();
            tauri::async_runtime::spawn(async move {
                broadcast_worktree_archived(&registry, &tc, &name, &id, &branch).await;
            });
        }
    });

    // リスナーIDを保存して次回再起動時にアンリジスターできるようにする
    let manager = app_handle.state::<McpServerManager>();
    if let Ok(mut guard) = manager.archive_listener_id.lock() {
        *guard = Some(listener_id);
    }
    drop(manager);

    // ワークツリー追加時に全クライアントへ通知
    let peer_map_for_added_listener = peer_map.clone();
    let timeout_counts_for_added_listener = timeout_counts.clone();
    let added_listener_id = app_handle.listen("worktree-added", move |event: tauri::Event| {
        let registry = peer_map_for_added_listener.clone();
        let tc = timeout_counts_for_added_listener.clone();
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) {
            let name = payload["name"].as_str().unwrap_or("unknown").to_string();
            let id = payload["id"].as_str().unwrap_or("").to_string();
            let branch = payload["branchName"].as_str().unwrap_or("").to_string();
            tauri::async_runtime::spawn(async move {
                broadcast_worktree_added(&registry, &tc, &name, &id, &branch).await;
            });
        }
    });

    let manager = app_handle.state::<McpServerManager>();
    if let Ok(mut guard) = manager.added_listener_id.lock() {
        *guard = Some(added_listener_id);
    }
    drop(manager);

    tauri::async_runtime::spawn(async move {
        let service = StreamableHttpService::new(
            {
                let ah = app_handle.clone();
                let peers = peer_map.clone();
                move || Ok(NotifyService::new(ah.clone(), peers.clone()))
            },
            LocalSessionManager::default().into(),
            Default::default(),
        );

        let api_key_state = ApiKeyState(api_key.clone());
        let router = axum::Router::new()
            .nest_service("/mcp", service)
            .route("/notify", post(notify_handler))
            .with_state(app_handle.clone())
            .layer(middleware::from_fn(move |mut req: Request, next: Next| {
                let key = api_key_state.clone();
                async move {
                    req.extensions_mut().insert(key);
                    api_key_auth(req, next).await
                }
            }));

        // 固定ポートの場合は最大5回リトライ、ポート0はOS割り当てなので1回のみ
        let bind_addr = if remote_access { "0.0.0.0" } else { "127.0.0.1" };
        let max_retries = if port == 0 { 1 } else { 5 };
        let mut listener_opt = None;
        for attempt in 0..max_retries {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            match tokio::net::TcpListener::bind(format!("{}:{}", bind_addr, port)).await {
                Ok(l) => { listener_opt = Some(l); break; }
                Err(e) => {
                    log::warn!("MCP bind attempt {}/{} failed: {}", attempt + 1, max_retries, e);
                }
            }
        }
        let listener = match listener_opt {
            Some(l) => l,
            None => {
                log::error!("Failed to bind MCP server on port {} after {} attempts", port, max_retries);
                if generation.load(Ordering::SeqCst) == my_generation {
                    if let Ok(mut s) = status.lock() {
                        s.running = false;
                        s.port = None;
                    }
                }
                let _ = complete_tx.send(());
                return;
            }
        };

        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                log::error!("Failed to get MCP server local addr: {}", e);
                if generation.load(Ordering::SeqCst) == my_generation {
                    if let Ok(mut s) = status.lock() {
                        s.running = false;
                        s.port = None;
                    }
                }
                let _ = complete_tx.send(());
                return;
            }
        };

        write_server_info_file(&app_handle, port, &api_key);
        log::info!("MCP server listening on http://{}:{}/mcp", bind_addr, port);

        // ステータス: 起動中（世代が一致する場合のみ更新）
        if generation.load(Ordering::SeqCst) == my_generation {
            if let Ok(mut s) = status.lock() {
                s.running = true;
                s.port = Some(port);
            }
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

        // peer_map はクリアしない。
        // 次世代のサーバーに接続したクライアントが既に登録されている可能性があり、
        // ここでクリアすると新世代のpeerも失われる（世代間でpeer_mapは共有される）。
        // 切断済みpeerはbroadcast時にnotify失敗で検知しlazyに除去される。

        // ステータス: 停止（世代が一致する場合のみ更新 — 新世代が既に起動済みなら上書きしない）
        if generation.load(Ordering::SeqCst) == my_generation {
            if let Ok(mut s) = status.lock() {
                s.running = false;
                s.port = None;
            }
        }

        // 停止完了を通知
        let _ = complete_tx.send(());
    });
}

// ─── CLI notification sender (standalone, no AppHandle needed) ───────────────

pub fn send_notification_standalone(worktree_name: &str, kind: Option<&str>) -> Result<(), String> {
    let (port, api_key) = read_server_info_standalone()?;
    let body = serde_json::json!({
        "worktree": worktree_name,
        "kind": kind.unwrap_or("general"),
    }).to_string();
    let request = format!(
        "POST /notify HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAuthorization: Bearer {api_key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    use std::io::{Read, Write};
    use std::time::Duration;

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| format!("Cannot connect to oretachi MCP server: {}", e))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set read timeout: {}", e))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set write timeout: {}", e))?;
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
    // ステータス行 "HTTP/1.x 200 " を確認（単純な contains("200") は誤検知の恐れ）
    let first_line = response_str.lines().next().unwrap_or("");
    if !first_line.starts_with("HTTP/") || !first_line.contains(" 200 ") {
        return Err(format!("Server returned unexpected response: {}", response_str));
    }
    Ok(())
}

fn read_server_info_standalone() -> Result<(u16, String), String> {
    #[cfg(target_os = "windows")]
    let base = {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| "APPDATA environment variable not set".to_string())?;
        PathBuf::from(appdata).join("com.ia.oretachi")
    };

    #[cfg(not(target_os = "windows"))]
    let base = {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME environment variable not set".to_string())?;
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("com.ia.oretachi")
    };

    // mcp-server.json を優先して読む
    let json_path = base.join(SERVER_INFO_FILE);
    if json_path.exists() {
        let content = fs::read_to_string(&json_path)
            .map_err(|e| format!("Cannot read server info file: {}", e))?;
        let info: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("Invalid server info JSON: {}", e))?;
        let port = info["port"].as_u64()
            .ok_or_else(|| "Missing port in server info".to_string())? as u16;
        let api_key = info["apiKey"].as_str()
            .ok_or_else(|| "Missing apiKey in server info".to_string())?
            .to_string();
        return Ok((port, api_key));
    }

    // mcp-server.json が存在しない場合、APIキーが取得できないためエラーとする
    Err("Cannot read API key: mcp-server.json not found. Please restart oretachi to regenerate the server info file.".to_string())
}
