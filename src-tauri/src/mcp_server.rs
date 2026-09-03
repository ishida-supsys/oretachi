use std::{collections::{HashMap, HashSet}, fs, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}}, time::{SystemTime, UNIX_EPOCH}};
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
use tokio::sync::{broadcast, oneshot, watch, RwLock};

use crate::git_worktree::get_git_remotes;
use crate::pty_manager::PtyManager;
use crate::settings::{resolve_tray_notification, AppSettings, SettingsManager, Workgroup, WorktreeEntry};

/// artifact / artifact_module の read-modify-write を直列化するグローバルロック。
/// これらのツールは read_only_hint = true を宣言しているため Claude Code 側が
/// isConcurrencySafe = true とみなし、同一アーティファクトに対して並列に呼び出しうる。
/// NotifyService は接続ごとに生成されるためプロセス共有の static で持つ。
pub(crate) static ARTIFACT_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 一時ファイル名の衝突を避けるための連番。
static ARTIFACT_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// アーティファクト JSON をアトミックに書き込む。
///
/// `tokio_fs::write` は truncate → write なので、書き込み中のファイルを
/// 読んだ側が壊れた JSON を掴む。read_only_hint = true を宣言した結果
/// Claude Code が読み書きを並列実行しうるため、一時ファイル + rename にする。
/// rename は同一ディレクトリ内なら Windows/Unix ともに既存ファイルを置換する。
/// 一時ファイルの拡張子は `.json` にならないため search_artifact の走査対象外。
pub(crate) async fn write_artifact_atomic(
    path: &std::path::Path,
    contents: &str,
) -> Result<(), McpError> {
    let seq = ARTIFACT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp-{}-{}", std::process::id(), seq));
    let tmp_path = path.with_file_name(tmp_name);

    tokio_fs::write(&tmp_path, contents)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    if let Err(e) = tokio_fs::rename(&tmp_path, path).await {
        let _ = tokio_fs::remove_file(&tmp_path).await;
        return Err(McpError::internal_error(e.to_string(), None));
    }
    Ok(())
}

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

/// サブウィンドウへ移送中の worktree ID を保持。MCP ツールが silent failure せず
/// 即座にエラーを返すために、フロント側で detached/attached が変わるたびに更新する。
#[derive(Default)]
pub struct DetachedWorktreeRegistry(pub Mutex<HashSet<String>>);

impl DetachedWorktreeRegistry {
    pub fn is_detached(&self, worktree_id: &str) -> bool {
        match self.0.lock() {
            Ok(g) => g.contains(worktree_id),
            Err(e) => e.into_inner().contains(worktree_id),
        }
    }
}

#[tauri::command]
pub fn register_detached_worktree(
    worktree_id: String,
    registry: tauri::State<'_, DetachedWorktreeRegistry>,
) {
    if let Ok(mut g) = registry.0.lock() {
        g.insert(worktree_id);
    }
}

#[tauri::command]
pub fn unregister_detached_worktree(
    worktree_id: String,
    registry: tauri::State<'_, DetachedWorktreeRegistry>,
) {
    if let Ok(mut g) = registry.0.lock() {
        g.remove(&worktree_id);
    }
}

/// ワークツリークローズの最終結果。フロントエンドの status 文字列に対応する。
pub enum CloseWorktreeOutcome {
    Closed,
    /// ユーザーが削除リトライをキャンセルした（エラーではない）
    Cancelled,
    /// 同じワークツリーのクローズ処理が既に進行中だった（エラーではない）
    Busy,
    Failed(String),
}

/// oretachi_close_worktree の処理結果をフロントエンドから受け取るための oneshot 送信側を保持する。
/// MCP ツールは request_id ごとに receiver を待ち、フロント側が
/// mcp_close_worktree_result コマンドで実際の成否を返す。
#[derive(Default)]
pub struct CloseWorktreeAckRegistry(pub Mutex<HashMap<String, oneshot::Sender<CloseWorktreeOutcome>>>);

impl CloseWorktreeAckRegistry {
    fn register(&self, request_id: String) -> oneshot::Receiver<CloseWorktreeOutcome> {
        let (tx, rx) = oneshot::channel();
        match self.0.lock() {
            Ok(mut g) => { g.insert(request_id, tx); }
            Err(e) => { e.into_inner().insert(request_id, tx); }
        }
        rx
    }

    fn take(&self, request_id: &str) -> Option<oneshot::Sender<CloseWorktreeOutcome>> {
        match self.0.lock() {
            Ok(mut g) => g.remove(request_id),
            Err(e) => e.into_inner().remove(request_id),
        }
    }
}

/// フロントエンドがワークツリークローズの成否を MCP ツールへ返す。
/// status: "ok" | "cancelled" | "busy" | それ以外は失敗扱い。
/// 該当 request_id が既にタイムアウト等で除去済みの場合は何もしない。
#[tauri::command]
pub fn mcp_close_worktree_result(
    request_id: String,
    status: String,
    error: Option<String>,
    registry: tauri::State<'_, CloseWorktreeAckRegistry>,
) {
    if let Some(tx) = registry.take(&request_id) {
        let outcome = match status.as_str() {
            "ok" => CloseWorktreeOutcome::Closed,
            "cancelled" => CloseWorktreeOutcome::Cancelled,
            "busy" => CloseWorktreeOutcome::Busy,
            _ => CloseWorktreeOutcome::Failed(error.unwrap_or_else(|| "unknown error".to_string())),
        };
        let _ = tx.send(outcome);
    }
}

/// ワークツリー取り込みの最終結果。
pub enum ImportWorktreeOutcome {
    /// 登録できた（ワークツリーID）
    Imported(String),
    /// 既に登録済みだった（エラーではない）
    AlreadyRegistered,
    Failed(String),
}

/// oretachi_import_worktree の処理結果をフロントエンドから受け取る。
/// settings の所有権はフロント側にあるため、登録そのものはフロントに行わせて結果だけ受け取る。
#[derive(Default)]
pub struct ImportWorktreeAckRegistry(pub Mutex<HashMap<String, oneshot::Sender<ImportWorktreeOutcome>>>);

impl ImportWorktreeAckRegistry {
    fn register(&self, request_id: String) -> oneshot::Receiver<ImportWorktreeOutcome> {
        let (tx, rx) = oneshot::channel();
        match self.0.lock() {
            Ok(mut g) => { g.insert(request_id, tx); }
            Err(e) => { e.into_inner().insert(request_id, tx); }
        }
        rx
    }

    fn take(&self, request_id: &str) -> Option<oneshot::Sender<ImportWorktreeOutcome>> {
        match self.0.lock() {
            Ok(mut g) => g.remove(request_id),
            Err(e) => e.into_inner().remove(request_id),
        }
    }
}

/// フロントエンドがワークツリー取り込みの成否を MCP ツールへ返す。
/// status: "ok" | "already" | それ以外は失敗扱い。
#[tauri::command]
pub fn mcp_import_worktree_result(
    request_id: String,
    status: String,
    worktree_id: Option<String>,
    error: Option<String>,
    registry: tauri::State<'_, ImportWorktreeAckRegistry>,
) {
    if let Some(tx) = registry.take(&request_id) {
        let outcome = match status.as_str() {
            "ok" => ImportWorktreeOutcome::Imported(worktree_id.unwrap_or_default()),
            "already" => ImportWorktreeOutcome::AlreadyRegistered,
            _ => ImportWorktreeOutcome::Failed(error.unwrap_or_else(|| "unknown error".to_string())),
        };
        let _ = tx.send(outcome);
    }
}

static PEER_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static CLOSE_WORKTREE_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);
static IMPORT_WORKTREE_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);
/// 取り込み結果を待つ上限。登録はファイル I/O を伴わない settings 操作なので短くてよいが、
/// フロントが重い処理中でも取りこぼさないよう余裕を持たせる。
const IMPORT_WORKTREE_ACK_TIMEOUT_SECS: u64 = 30;
/// クローズ結果を待つ上限。worktree remove はロックエラー時にキャンセルまで無限リトライする
/// (git_worktree::worktree_remove_persistent) ため、タイムアウトしても失敗とは断定せず
/// 「継続中」として返す。MCP クライアント側のツールタイムアウト(既定 60 秒前後)に
/// 先んじて応答を返せるよう、それより短く設定する。
const CLOSE_WORKTREE_ACK_TIMEOUT_SECS: u64 = 45;

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
    /// notify-worktree リスナーID（再起動時にアンリジスターするために保持）
    notify_listener_id: Mutex<Option<tauri::EventId>>,
    /// hook 通知をWebView IPCを経由せずMCPピアへ直接配信するチャネル
    /// (WebView IPC を使うと UIスレッドに負荷がかかるため broadcast channel を使用)
    pub hook_tx: broadcast::Sender<NotifyWorktreeEvent>,
    /// 通知の rate limiting: (worktree_name, kind) → 最終送信時刻 (None=未送信)
    /// hook: 3秒、approval: 1秒 debounce。general/completed や任意の kind は
    /// debounce しない（MCP クライアントの意図的な通知を握り潰さないため）
    notify_last_sent: Mutex<HashMap<(String, String), Option<std::time::Instant>>>,
    /// /prompt-context のスロットル: worktree_id → 最終送信時刻。
    /// UserPromptSubmit はプロンプトごとに発火するため、期間内は skip を返して
    /// コンテキスト注入のノイズを抑える。
    prompt_context_last_sent: Mutex<HashMap<String, std::time::Instant>>,
}

/// kind ごとの debounce 秒数。None の kind は debounce しない（毎回送信）。
///
/// hook/approval は短時間に大量発火する可能性があり WebView イベントキューを
/// 圧迫するため debounce する。general/completed や任意のカスタム kind は
/// MCP クライアントの意図的な通知のため握り潰さない。
fn notify_debounce_secs(kind: &str) -> Option<u64> {
    match kind {
        "hook" => Some(3),
        "approval" => Some(1),
        _ => None,
    }
}

/// worktree_name × kind の組み合わせで debounce 判定する。
/// true を返したら送信すべき（初回、または前回送信から debounce 秒数以上経過、
/// または対象外 kind）。
fn should_send_notify(
    last_sent: &Mutex<HashMap<(String, String), Option<std::time::Instant>>>,
    worktree_name: &str,
    kind: &str,
) -> bool {
    let Some(debounce) = notify_debounce_secs(kind) else {
        return true;
    };
    let mut map = last_sent.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    let key = (worktree_name.to_string(), kind.to_string());
    // Option で初回を None として明示。Instant 減算による underflow panic を
    // 回避しつつ、初回は必ず送信する旧仕様の挙動も維持。
    let entry = map.entry(key).or_insert(None);
    let should = match *entry {
        None => true,
        Some(prev) => now.duration_since(prev).as_secs() >= debounce,
    };
    if should {
        *entry = Some(now);
        true
    } else {
        log::debug!(
            "[notify] debounced worktree={} kind={} (window={}s)",
            worktree_name, kind, debounce
        );
        false
    }
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
            notify_listener_id: Mutex::new(None),
            hook_tx: broadcast::channel::<NotifyWorktreeEvent>(256).0,
            notify_last_sent: Mutex::new(HashMap::new()),
            prompt_context_last_sent: Mutex::new(HashMap::new()),
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
    /// userConfig 非依存版の hook が送る CC 組み込み変数 ${CLAUDE_PROJECT_DIR}。
    /// これからワークツリー名を逆引きする。
    #[serde(default, rename = "projectDir")]
    pub project_dir: Option<String>,
    /// ライフサイクルイベント名（"Stop" 等）。これから kind を解決する。
    #[serde(default)]
    pub event: Option<String>,
    /// 後方互換: ワークツリー名を直接指定する旧形式 / MCP 経由。
    #[serde(default)]
    pub worktree: Option<String>,
    /// 後方互換: kind を直接指定する旧形式。
    #[serde(default)]
    pub kind: Option<String>,
    pub body: Option<String>,
    pub agent: Option<String>,
    /// 発火元 PTY タブの `terminal_id`。サイドカーが env `ORETACHI_TERMINAL_ID` から拾って付ける。
    /// 同一ワークツリーに複数タブがある場合に発火元を確定するために使う。
    /// oretachi 管理外のターミナルから起動されたエージェントでは付かない（`None`）。
    #[serde(default, rename = "terminalId")]
    pub terminal_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetDescriptionPayload {
    /// userConfig 非依存版の hook が送る ${CLAUDE_PROJECT_DIR}。
    #[serde(default, rename = "projectDir")]
    pub project_dir: Option<String>,
    /// 後方互換: ワークツリー名を直接指定する旧形式。
    #[serde(default)]
    pub worktree: Option<String>,
    /// ExitPlanMode フックが stdin で受け取った hook JSON 文字列（生）
    #[serde(rename = "hookJson")]
    pub hook_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionContextPayload {
    /// SessionStart フックの oretachi-notify が送る ${CLAUDE_PROJECT_DIR}。
    #[serde(default, rename = "projectDir")]
    pub project_dir: Option<String>,
    /// 発火元 PTY タブの `terminal_id`（env `ORETACHI_TERMINAL_ID` 由来）。
    #[serde(default, rename = "terminalId")]
    pub terminal_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SetWorktreeDescriptionEvent {
    pub worktree: String,
    /// ExitPlanMode 経路: フロント側で AI 要約してから description にセットする
    pub plan: Option<String>,
    /// MCP 直接セット経路 (oretachi_set_description): AI 要約をスキップしてそのまま採用する
    pub description: Option<String>,
}

/// Stop フック (--turn-context) が送るペイロード（#124）
#[derive(Debug, Deserialize)]
pub struct TurnContextPayload {
    #[serde(default, rename = "projectDir")]
    pub project_dir: Option<String>,
    /// 発火元 PTY タブの `terminal_id`（env `ORETACHI_TERMINAL_ID` 由来）。
    /// 配送はこのタブ宛の inbox だけを対象にする（同一ワークツリーの別タブ宛を抜き取らない）。
    #[serde(default, rename = "terminalId")]
    pub terminal_id: Option<String>,
    /// Stop フックの stdin JSON（生文字列）。`prompt_id` / `stop_hook_active` をここから読む。
    #[serde(default, rename = "hookJson")]
    pub hook_json: Option<String>,
}

/// UserPromptSubmit フック (--prompt-context) が送るペイロード
#[derive(Debug, Deserialize)]
pub struct PromptContextPayload {
    #[serde(default, rename = "projectDir")]
    pub project_dir: Option<String>,
    /// 発火元 PTY タブの `terminal_id`（env `ORETACHI_TERMINAL_ID` 由来）。
    #[serde(default, rename = "terminalId")]
    pub terminal_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NotifyWorktreeParams {
    #[schemars(description = "通知するワークツリー名")]
    pub worktree_name: String,
    #[schemars(description = "通知種別: \"approval\"(承認待ち) / \"completed\"(作業完了) / \"general\"(汎用) / \"hook\"(ライフサイクルフック)。省略時は \"general\"。これは画面に出すトーストの種別であり、購読イベントの種別(event_kind)とは別物")]
    pub kind: Option<String>,
    #[schemars(description = "通知本文（ライフサイクルフックのコンテキスト情報など）。省略可")]
    pub body: Option<String>,
    #[schemars(description = "購読イベントとしても発行する場合の**イベント種別**。現在は \"worktree.message\" のみ。トースト種別 kind とは名前空間が別。指定すると body が自由文メッセージとして、自分のワークツリーを購読している他のワークツリーへ配送される（宛先は購読者が決めるので worktree_name とは無関係）")]
    pub event_kind: Option<String>,
    #[schemars(description = "呼び出し元ターミナルの terminal_id。event_kind 指定時の発信元の同定に使う（セッション開始時に oretachi から注入されている）")]
    pub terminal_id: Option<String>,
    #[schemars(description = "呼び出し元の作業ディレクトリ絶対パス。event_kind 指定時に terminal_id を省略した場合のフォールバック同定に使う")]
    pub project_dir: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetWorktreeDescriptionParams {
    #[schemars(description = "ワークツリーの1行説明。このワークツリーで進めている作業全体の目的を簡潔に表す（改行なし、日本語なら15〜40文字程度）")]
    pub description: String,
    #[schemars(description = "ワークツリーのルートディレクトリ絶対パス（通常は自分の作業ディレクトリ）。worktree_name/worktree_id 未指定時はこれでワークツリーを特定する")]
    pub project_dir: Option<String>,
    #[schemars(description = "対象ワークツリー名（project_dir で特定できない場合に指定）")]
    pub worktree_name: Option<String>,
    #[schemars(description = "対象ワークツリーID（同名ワークツリーが複数ある場合に指定）")]
    pub worktree_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetTrayNotificationParams {
    #[schemars(description = "true=フック由来通知をトレイに出す / false=出さない / **省略時は「未設定」に戻して所属ワークグループの既定値（無ければ true）へフォールバックする**")]
    pub enabled: Option<bool>,
    #[schemars(description = "ワークツリーのルートディレクトリ絶対パス（通常は自分の作業ディレクトリ）。worktree_name/worktree_id 未指定時はこれでワークツリーを特定する")]
    pub project_dir: Option<String>,
    #[schemars(description = "対象ワークツリー名（project_dir で特定できない場合に指定）")]
    pub worktree_name: Option<String>,
    #[schemars(description = "対象ワークツリーID（同名ワークツリーが複数ある場合に指定）")]
    pub worktree_id: Option<String>,
}

/// トレイ通知設定の変更をフロント（App.vue）へ伝えるイベント。
/// フロント側が settings.json への永続化と UI 反映の両方を担う。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SetTrayNotificationEvent {
    pub worktree: String,
    pub worktree_id: String,
    /// `None` = 未設定へ戻す（ワークグループ既定値へフォールバック）
    pub tray_notification: Option<bool>,
}

fn default_true() -> bool { true }

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NotifyWorktreeEvent {
    pub worktree_name: String,
    pub kind: String,
    pub body: Option<String>,
    pub agent: Option<String>,
    /// トレイ通知として提示してよいか。`false` はフック由来通知を
    /// `trayNotification: false` のワークツリーで抑制するケースのみ。
    /// **イベント自体は drop しない**（自動承認が `notify-worktree` をトリガにしている）。
    /// MCP ブロードキャスト経路の `from_str::<NotifyWorktreeEvent>` との後方互換のため
    /// `default` が必須。
    #[serde(default = "default_true")]
    pub tray: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ArtifactParams {
    #[schemars(description = "操作の種類: \"create\"(新規作成) / \"update\"(差分更新) / \"rewrite\"(全置換) / \"get\"(1件取得) / \"outline\"(構造概要取得。contentを除く)")]
    pub command: String,
    #[schemars(description = "アーティファクトを識別する一意なID")]
    pub id: String,
    #[schemars(description = "現在の作業ディレクトリ。これを渡すのが最も確実。HOMEタブやリポジトリルートで作業している場合は repository/branch では特定できないため必須")]
    pub project_dir: Option<String>,
    #[schemars(description = "対象ワークツリーID（project_dir で特定できない場合に指定）")]
    pub worktree_id: Option<String>,
    #[schemars(description = "リポジトリ名。project_dir を渡す場合は不要。指定する場合は branch と両方セットで")]
    pub repository: Option<String>,
    #[schemars(description = "ブランチ名。project_dir を渡す場合は不要。指定する場合は repository と両方セットで")]
    pub branch: Option<String>,
    #[schemars(description = "コンテンツの種類 (create時必須): application/vnd.ant.code, text/markdown, text/html, image/svg+xml, application/vnd.ant.mermaid, application/vnd.ant.react (Tailwind CSSユーティリティクラス利用可), text/csv, text/tab-separated-values (1行目をヘッダとするテーブルビューアで表示), text/uri-list (content に URL を1行だけ書く。ビューアには「ブラウザで開く」ボタンだけが出る)")]
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
    #[schemars(description = "get時: 取得開始行 (0始まり、省略時は0)")]
    pub offset: Option<u32>,
    #[schemars(description = "get時: 取得する行数 (省略時は全行)")]
    pub limit: Option<u32>,
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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    modules: HashMap<String, String>,
    created_at: u64,
    updated_at: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArtifactParams {
    #[schemars(description = "現在の作業ディレクトリ。これを渡すのが最も確実。HOMEタブやリポジトリルートで作業している場合は repository/branch では特定できないため必須")]
    pub project_dir: Option<String>,
    #[schemars(description = "対象ワークツリーID（project_dir で特定できない場合に指定）")]
    pub worktree_id: Option<String>,
    #[schemars(description = "リポジトリ名。project_dir を渡す場合は不要。指定する場合は branch と両方セットで")]
    pub repository: Option<String>,
    #[schemars(description = "ブランチ名。project_dir を渡す場合は不要。指定する場合は repository と両方セットで")]
    pub branch: Option<String>,
    #[schemars(description = "検索キーワード (省略時は全件返却)。title, content, type, language を対象に部分一致検索")]
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ArtifactModuleParams {
    #[schemars(description = "操作の種類: \"list\"(一覧), \"get\"(取得), \"create\"(作成), \"update\"(差分更新), \"rewrite\"(全置換), \"delete\"(削除)")]
    pub command: String,
    #[schemars(description = "アーティファクトID")]
    pub id: String,
    #[schemars(description = "現在の作業ディレクトリ。これを渡すのが最も確実。HOMEタブやリポジトリルートで作業している場合は repository/branch では特定できないため必須")]
    pub project_dir: Option<String>,
    #[schemars(description = "対象ワークツリーID（project_dir で特定できない場合に指定）")]
    pub worktree_id: Option<String>,
    #[schemars(description = "リポジトリ名。project_dir を渡す場合は不要。指定する場合は branch と両方セットで")]
    pub repository: Option<String>,
    #[schemars(description = "ブランチ名。project_dir を渡す場合は不要。指定する場合は repository と両方セットで")]
    pub branch: Option<String>,
    #[schemars(description = "モジュール名 (例: \"components/Header\", \"screens/Login\")。list時は省略可")]
    pub module_name: Option<String>,
    #[schemars(description = "モジュールのソースコード (create/rewrite時必須)")]
    pub content: Option<String>,
    #[schemars(description = "update時: 置き換え元の文字列 (モジュール内に1箇所だけ存在すること)")]
    pub old_str: Option<String>,
    #[schemars(description = "update時: 置き換え後の文字列")]
    pub new_str: Option<String>,
    #[schemars(description = "get時: 取得開始行 (0始まり、省略時は0)")]
    pub offset: Option<u32>,
    #[schemars(description = "get時: 取得する行数 (省略時は全行)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListRepositoryParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListWorkgroupsParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetWorktreeStatusParams {
    #[schemars(description = "絞り込みキーワード（name / branchName / description の部分一致、大文字小文字は区別しない）。省略時は全件")]
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct InspectWorktreeParams {
    #[schemars(description = "調べるワークツリーの名前")]
    pub worktree_name: Option<String>,
    #[schemars(description = "ワークツリーID（同名ワークツリーが複数ある場合に指定）")]
    pub worktree_id: Option<String>,
    #[schemars(description = "マージ済み判定の対象ブランチ（省略時はリポジトリの既定ブランチを自動判定）")]
    pub base_branch: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetAppOptionsParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTaskParams {
    #[schemars(description = "タスクのプロンプト (AIに実行させたい作業の説明)")]
    pub prompt: String,
    #[schemars(description = "リモート実行するかどうか (省略時は false)")]
    pub remote_exec: Option<bool>,
    #[schemars(description = "追加先ワークグループのID (oretachi_list_workgroups の id)。省略時はUIで現在選択中のワークグループに入る (isDefault のグループとは限らない)")]
    pub workgroup_id: Option<String>,
    #[schemars(description = "追加先ワークグループの表示名 (oretachi_list_workgroups の name)。workgroup_id 指定時は無視される")]
    pub workgroup_name: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct AddTaskEvent {
    prompt: String,
    remote_exec: bool,
    /// 追加先ワークグループID。None ならフロントに委ね、UI で現在選択中の WG に入る。
    workgroup_id: Option<String>,
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
    #[schemars(description = "未マージでもブランチを強制削除（git branch -D）するか。省略時は delete_branch と同じ値（削除するなら強制削除。UI の手動削除と同じ挙動）。false を明示するとマージ済み確認つき（git branch -d）になり、未マージのブランチではクローズが失敗する")]
    pub force_branch: Option<bool>,
}

#[derive(Debug, Serialize, Clone)]
struct CloseWorktreeEvent {
    request_id: String,
    worktree_id: String,
    worktree_name: String,
    merge_to: String,
    delete_branch: bool,
    force_branch: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SpawnTerminalParams {
    #[schemars(description = "対象ワークツリーの名前")]
    pub worktree_name: String,
    #[schemars(description = "ワークツリーID（同名ワークツリーが複数ある場合に指定）")]
    pub worktree_id: Option<String>,
    #[schemars(description = "新規ターミナルで実行するコマンド（末尾改行は自動付与）")]
    pub command: String,
    #[schemars(description = "ターミナルタブのタイトル（省略時はデフォルト）")]
    pub title: Option<String>,
    #[schemars(description = "起動理由のメモ（ログ用、省略可）")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct SpawnTerminalEvent {
    worktree_id: String,
    command: String,
    title: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImportWorktreeParams {
    #[schemars(description = "取り込むワークツリーの絶対パス。省略すると候補の列挙のみ行う")]
    pub path: Option<String>,
    #[schemars(description = "対象リポジトリ名で絞り込む（省略時は登録済み全リポジトリを走査）")]
    pub repository_name: Option<String>,
    #[schemars(description = "true なら path を指定していても登録せず候補列挙だけ行う（デフォルト false）")]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Serialize, Clone)]
struct ImportWorktreeEvent {
    request_id: String,
    repository_id: String,
    repository_name: String,
    path: String,
    name: String,
    branch_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShowWorktreeParams {
    #[schemars(description = "フォーカスするワークツリーの名前")]
    pub worktree_name: Option<String>,
    #[schemars(description = "ワークツリーID（同名ワークツリーが複数ある場合に指定）")]
    pub worktree_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct ShowWorktreeEvent {
    worktree_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTerminalsParams {
    #[schemars(description = "絞り込みするワークツリー名（省略時は全ワークツリー横断）")]
    pub worktree_name: Option<String>,
    #[schemars(description = "ワークツリーID（同名ワークツリーが複数ある場合に指定）")]
    pub worktree_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct KillTerminalParams {
    #[schemars(description = "停止する PTY セッションID（oretachi_list_terminals で取得）")]
    pub session_id: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadTerminalParams {
    #[schemars(description = "PTY セッションID（oretachi_list_terminals で取得）")]
    pub session_id: u32,
    #[schemars(description = "1 回の呼び出しで返す最大バイト数（デフォルト 8192）")]
    pub max_bytes: Option<usize>,
    #[schemars(description = "前回呼び出しで返された cursor を渡すと、それ以降の新規出力だけを返す（差分読み）。省略時はバッファ末尾から max_bytes")]
    pub from_cursor: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteTerminalParams {
    #[schemars(description = "PTY セッションID（oretachi_list_terminals で取得）")]
    pub session_id: u32,
    #[schemars(description = "送信するテキスト")]
    pub text: String,
    #[schemars(description = "true なら改行を \\r 正規化＋末尾 \\r 保証してから送信（デフォルト true）。vitest の単一キー入力など改行不要時は false")]
    pub submit: Option<bool>,
}

// ─── 購読 / inbox 系ツールのパラメータ (issue #123) ───────────────────────────

// 各ツールの terminal_id / project_dir パラメータの説明文は schemars が文字列リテラルしか
// 受け付けないため各構造体に直接書いている（定数に切り出せない）。
// エージェントは自分の terminal_id を SessionStart の additionalContext から知る。

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SubscribeWorktreeParams {
    #[schemars(description = "購読対象。ワークツリー名 / ID のほか、ワイルドカードとして \"*\"(全ワークツリー) / \"workgroup:<ID または名前>\" / \"repo:<リポジトリ名>\" を指定できる。**これから作成されるワークツリーの worktree.created を購読したい場合はワイルドカードを使う**（ID 固定では表現できない）")]
    pub target: String,
    #[schemars(description = "購読するイベント種別の配列。\"worktree.closed\"(クローズ) / \"worktree.created\"(作成) / \"worktree.message\"(他ワークツリーのエージェントが notify_worktree で送る自由文)。省略時は [\"worktree.closed\"]")]
    pub event_kinds: Option<Vec<String>>,
    #[schemars(description = "配送戦略: \"turn_end\"(既定。待機中なら PTY へ押し込み、走行中はターン境界を待つ) / \"interrupt\"(走行中でも即 PTY へ割り込む) / \"passive\"(押し込まない。oretachi_poll_inbox で自分から取りに来る)。どの戦略でもセッション開始時の回収と oretachi_poll_inbox は使える")]
    pub delivery: Option<String>,
    #[schemars(description = "自分のターミナルが閉じていた場合に新しいターミナルを起動して通知するか（既定 false）。true にすると未読が溜まった時点で oretachi が自動でタブを立ててエージェントを起動する")]
    pub spawn_if_closed: Option<bool>,
    #[schemars(description = "購読の有効期間（秒）。省略時は無期限。ターミナルが閉じても購読は引き継ぎ待ちとして7日間保持され、同じワークツリーで**同じ AI セッション**（--resume で再開した会話）が立ち上がったときに引き継がれる")]
    pub expires_in: Option<i64>,
    #[schemars(description = "自分が動いているターミナルの terminal_id。セッション開始時に oretachi から注入されている値をそのまま渡す。省略時は project_dir から AI エージェント端末が1つだけのワークツリーとして推測する")]
    pub terminal_id: Option<String>,
    #[schemars(description = "自分の作業ディレクトリ絶対パス。terminal_id 省略時の推測に使う")]
    pub project_dir: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UnsubscribeWorktreeParams {
    #[schemars(description = "解除する購読 ID（oretachi_list_subscriptions で取得）")]
    pub subscription_id: Option<String>,
    #[schemars(description = "解除する購読対象のワークツリー名または ID（subscription_id を指定しない場合）")]
    pub target: Option<String>,
    #[schemars(description = "自分が動いているターミナルの terminal_id。セッション開始時に oretachi から注入されている値をそのまま渡す。省略時は project_dir から AI エージェント端末が1つだけのワークツリーとして推測する")]
    pub terminal_id: Option<String>,
    #[schemars(description = "自分の作業ディレクトリ絶対パス。terminal_id 省略時の推測に使う")]
    pub project_dir: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSubscriptionsParams {
    #[schemars(description = "自分が動いているターミナルの terminal_id。セッション開始時に oretachi から注入されている値をそのまま渡す。省略時は project_dir から AI エージェント端末が1つだけのワークツリーとして推測する")]
    pub terminal_id: Option<String>,
    #[schemars(description = "自分の作業ディレクトリ絶対パス。terminal_id 省略時の推測に使う")]
    pub project_dir: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PollInboxParams {
    #[schemars(description = "自分が動いているターミナルの terminal_id。セッション開始時に oretachi から注入されている値をそのまま渡す。省略時は project_dir から AI エージェント端末が1つだけのワークツリーとして推測する")]
    pub terminal_id: Option<String>,
    #[schemars(description = "自分の作業ディレクトリ絶対パス。terminal_id 省略時の推測に使う")]
    pub project_dir: Option<String>,
    #[schemars(description = "true なら ack 済みも含めて返す（既定 false = 未 ack のみ）")]
    pub include_acked: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AckMessageParams {
    #[schemars(description = "ack する inbox メッセージ ID。複数まとめて渡せる")]
    pub ids: Vec<String>,
    #[schemars(description = "自分が動いているターミナルの terminal_id。セッション開始時に oretachi から注入されている値をそのまま渡す。省略時は project_dir から AI エージェント端末が1つだけのワークツリーとして推測する")]
    pub terminal_id: Option<String>,
    #[schemars(description = "自分の作業ディレクトリ絶対パス。terminal_id 省略時の推測に使う")]
    pub project_dir: Option<String>,
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

    #[tool(description = "アーティファクトを操作する。create: 新規作成, update: 差分更新(old_str→new_str), rewrite: 全置換, get: 1件取得(offset/limitで行範囲指定可)。保存先は project_dir(現在の作業ディレクトリ)で指定するのが最も確実。HOMEタブやリポジトリルートで作業している場合は repository/branch では特定できないため project_dir が必須", annotations(read_only_hint = true))]
    async fn artifact(
        &self,
        Parameters(ArtifactParams {
            command,
            id,
            project_dir,
            worktree_id: target_worktree_id,
            repository,
            branch,
            content_type,
            title,
            content,
            language,
            old_str,
            new_str,
            offset,
            limit,
        }): Parameters<ArtifactParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let wt = resolve_artifact_worktree(
            &settings,
            target_worktree_id.as_deref(),
            project_dir.as_deref(),
            repository.as_deref(),
            branch.as_deref(),
        )?;
        let worktree_id = wt.id.clone();

        // artifact ID のパストラバーサル防止
        if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
            return Err(McpError::invalid_params("不正なアーティファクトIDです".to_string(), None));
        }

        // 書き込み系コマンドは read-modify-write の競合を避けるため直列化する
        let _write_guard = if matches!(command.as_str(), "get" | "outline") {
            None
        } else {
            Some(ARTIFACT_WRITE_LOCK.lock().await)
        };

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
            let mut data: ArtifactData = serde_json::from_str(&raw)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            let total_lines = data.content.lines().count();
            let off = offset.unwrap_or(0) as usize;
            let had_trailing_newline = data.content.ends_with('\n');
            if let Some(lim) = limit {
                let at_end = off + lim as usize >= total_lines;
                let sliced = data.content.lines().skip(off).take(lim as usize).collect::<Vec<_>>().join("\n");
                data.content = if had_trailing_newline && at_end { sliced + "\n" } else { sliced };
            } else if off > 0 {
                let sliced = data.content.lines().skip(off).collect::<Vec<_>>().join("\n");
                data.content = if had_trailing_newline { sliced + "\n" } else { sliced };
            }
            let mut json_val = serde_json::to_value(&data)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            if offset.is_some() || limit.is_some() {
                json_val["content_total_lines"] = serde_json::Value::Number(total_lines.into());
                json_val["content_offset"] = serde_json::Value::Number(off.into());
            }
            let json = serde_json::to_string_pretty(&json_val)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            log::info!("[mcp] artifact command=get id={} offset={:?} limit={:?} worktree_id={}", id, offset, limit, worktree_id);
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        if command == "outline" {
            let raw = tokio_fs::read_to_string(&artifact_path).await
                .map_err(|_| McpError::invalid_params(format!("アーティファクト '{}' が存在しません", id), None))?;
            let data: ArtifactData = serde_json::from_str(&raw)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            fn extract_exports(src: &str) -> Vec<String> {
                let mut names = Vec::new();
                for line in src.lines() {
                    let trimmed = line.trim();
                    // export default function/class Name
                    if trimmed.starts_with("export default function ") || trimmed.starts_with("export default class ") {
                        let rest = trimmed.trim_start_matches("export default function ").trim_start_matches("export default class ");
                        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        if !name.is_empty() { names.push(name); }
                    }
                    // export function/const/class Name
                    else if trimmed.starts_with("export function ") || trimmed.starts_with("export const ") || trimmed.starts_with("export class ") {
                        let rest = trimmed
                            .trim_start_matches("export function ")
                            .trim_start_matches("export const ")
                            .trim_start_matches("export class ");
                        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        if !name.is_empty() { names.push(name); }
                    }
                }
                names.dedup();
                names
            }

            let entry_lines = data.content.lines().count();
            let entry_exports = extract_exports(&data.content);
            let modules_outline: serde_json::Value = data.modules.iter().map(|(name, src)| {
                (name.clone(), serde_json::json!({
                    "lines": src.lines().count(),
                    "exports": extract_exports(src),
                }))
            }).collect::<serde_json::Map<_, _>>().into();

            let outline = serde_json::json!({
                "id": data.id,
                "title": data.title,
                "type": data.content_type,
                "entry_lines": entry_lines,
                "entry_exports": entry_exports,
                "modules": modules_outline,
            });
            let json = serde_json::to_string_pretty(&outline)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            log::info!("[mcp] artifact command=outline id={} worktree_id={}", id, worktree_id);
            return Ok(CallToolResult::success(vec![Content::text(json)]));
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
                ArtifactData { id: id.clone(), content_type, title, content, language, modules: HashMap::new(), created_at: now, updated_at: now }
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
                format!("不明なコマンド '{}'. create / update / rewrite / get / outline のいずれかを指定してください", other),
                None,
            )),
        };

        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        write_artifact_atomic(&artifact_path, &json).await?;

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

    #[tool(description = "Reactアーティファクトのモジュールを操作する。大規模アーティファクトをファイル単位で管理するために使用。list: モジュール一覧(行数のみ), get: 1モジュール取得(offset/limitで行範囲指定可), create: 追加, update: 差分更新, rewrite: 全置換, delete: 削除。対象は project_dir(現在の作業ディレクトリ)で指定するのが最も確実。HOMEタブやリポジトリルートで作業している場合は project_dir が必須", annotations(read_only_hint = true))]
    async fn artifact_module(
        &self,
        Parameters(ArtifactModuleParams {
            command, id,
            project_dir, worktree_id: target_worktree_id,
            repository, branch,
            module_name, content, old_str, new_str,
            offset, limit,
        }): Parameters<ArtifactModuleParams>,
    ) -> Result<CallToolResult, McpError> {
        // モジュール名バリデーション
        fn validate_module_name(name: &str) -> Result<(), McpError> {
            if name.contains("..") || name.starts_with('/') || name.contains('\\') || name.contains('\0') {
                return Err(McpError::invalid_params(
                    format!("無効なモジュール名: '{}'", name), None,
                ));
            }
            Ok(())
        }

        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let wt = resolve_artifact_worktree(
            &settings,
            target_worktree_id.as_deref(),
            project_dir.as_deref(),
            repository.as_deref(),
            branch.as_deref(),
        )?;
        let worktree_id = wt.id.clone();

        // artifact ID のパストラバーサル防止
        if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
            return Err(McpError::invalid_params("不正なアーティファクトIDです".to_string(), None));
        }

        // 書き込み系コマンドは read-modify-write の競合を避けるため直列化する
        let _write_guard = if matches!(command.as_str(), "list" | "get") {
            None
        } else {
            Some(ARTIFACT_WRITE_LOCK.lock().await)
        };

        let artifacts_dir = self.app_handle.path().app_data_dir()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .join("artifacts")
            .join(&worktree_id);
        let artifact_path = artifacts_dir.join(format!("{}.json", id));

        let raw = tokio_fs::read_to_string(&artifact_path).await
            .map_err(|_| McpError::invalid_params(format!("アーティファクト '{}' が存在しません", id), None))?;
        let mut data: ArtifactData = serde_json::from_str(&raw)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        // モジュール操作は application/vnd.ant.react のみ対象
        if data.content_type != "application/vnd.ant.react" {
            return Err(McpError::invalid_params(
                format!("artifact_module は application/vnd.ant.react のみ対象です (現在: {})", data.content_type),
                None,
            ));
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

        let result_json: String = match command.as_str() {
            "list" => {
                let mut list: Vec<serde_json::Value> = data.modules.iter().map(|(name, src)| {
                    serde_json::json!({ "module_name": name, "lines": src.lines().count() })
                }).collect();
                list.sort_by(|a, b| {
                    a["module_name"].as_str().unwrap_or("").cmp(b["module_name"].as_str().unwrap_or(""))
                });
                serde_json::to_string_pretty(&serde_json::json!({
                    "id": id,
                    "modules": list,
                })).map_err(|e| McpError::internal_error(e.to_string(), None))?
            }
            "get" => {
                let name = module_name.as_deref()
                    .ok_or_else(|| McpError::invalid_params("module_name が必要です", None))?;
                validate_module_name(name)?;
                let src = data.modules.get(name)
                    .ok_or_else(|| McpError::invalid_params(format!("モジュール '{}' が存在しません", name), None))?;
                let total_lines = src.lines().count();
                let off = offset.unwrap_or(0) as usize;
                let had_trailing_newline = src.ends_with('\n');
                let sliced = if let Some(lim) = limit {
                    let at_end = off + lim as usize >= total_lines;
                    let s = src.lines().skip(off).take(lim as usize).collect::<Vec<_>>().join("\n");
                    if had_trailing_newline && at_end { s + "\n" } else { s }
                } else if off > 0 {
                    let s = src.lines().skip(off).collect::<Vec<_>>().join("\n");
                    if had_trailing_newline { s + "\n" } else { s }
                } else {
                    src.clone()
                };
                let mut val = serde_json::json!({ "id": id, "module_name": name, "content": sliced });
                if offset.is_some() || limit.is_some() {
                    val["content_total_lines"] = serde_json::Value::Number(total_lines.into());
                    val["content_offset"] = serde_json::Value::Number(off.into());
                }
                serde_json::to_string_pretty(&val).map_err(|e| McpError::internal_error(e.to_string(), None))?
            }
            "create" => {
                let name = module_name.as_deref()
                    .ok_or_else(|| McpError::invalid_params("module_name が必要です", None))?;
                validate_module_name(name)?;
                if data.modules.contains_key(name) {
                    return Err(McpError::invalid_params(
                        format!("モジュール '{}' は既に存在します。上書きするには rewrite を使用してください", name), None,
                    ));
                }
                let src = content
                    .ok_or_else(|| McpError::invalid_params("content が必要です", None))?;
                data.modules.insert(name.to_string(), src);
                data.updated_at = now;
                let json = serde_json::to_string_pretty(&data)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                write_artifact_atomic(&artifact_path, &json).await?;
                if let Err(e) = self.app_handle.emit("artifact-changed", serde_json::json!({
                    "worktreeId": worktree_id, "artifactId": id, "command": "create",
                })) { log::warn!("Failed to emit artifact-changed: {}", e); }
                json
            }
            "rewrite" => {
                let name = module_name.as_deref()
                    .ok_or_else(|| McpError::invalid_params("module_name が必要です", None))?;
                validate_module_name(name)?;
                let src = content
                    .ok_or_else(|| McpError::invalid_params("content が必要です", None))?;
                data.modules.insert(name.to_string(), src);
                data.updated_at = now;
                let json = serde_json::to_string_pretty(&data)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                write_artifact_atomic(&artifact_path, &json).await?;
                if let Err(e) = self.app_handle.emit("artifact-changed", serde_json::json!({
                    "worktreeId": worktree_id, "artifactId": id, "command": "rewrite",
                })) { log::warn!("Failed to emit artifact-changed: {}", e); }
                json
            }
            "update" => {
                let name = module_name.as_deref()
                    .ok_or_else(|| McpError::invalid_params("module_name が必要です", None))?;
                validate_module_name(name)?;
                let old = old_str.ok_or_else(|| McpError::invalid_params("old_str が必要です", None))?;
                let new = new_str.ok_or_else(|| McpError::invalid_params("new_str が必要です", None))?;
                let src = data.modules.get_mut(name)
                    .ok_or_else(|| McpError::invalid_params(format!("モジュール '{}' が存在しません", name), None))?;
                let count = src.matches(old.as_str()).count();
                if count == 0 {
                    return Err(McpError::invalid_params(format!("old_str がモジュール '{}' 内に見つかりません", name), None));
                }
                if count > 1 {
                    return Err(McpError::invalid_params(format!("old_str がモジュール '{}' 内に{}箇所あります。1箇所だけにしてください", name, count), None));
                }
                *src = src.replacen(old.as_str(), new.as_str(), 1);
                data.updated_at = now;
                let json = serde_json::to_string_pretty(&data)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                write_artifact_atomic(&artifact_path, &json).await?;
                if let Err(e) = self.app_handle.emit("artifact-changed", serde_json::json!({
                    "worktreeId": worktree_id, "artifactId": id, "command": "update",
                })) { log::warn!("Failed to emit artifact-changed: {}", e); }
                json
            }
            "delete" => {
                let name = module_name.as_deref()
                    .ok_or_else(|| McpError::invalid_params("module_name が必要です", None))?;
                validate_module_name(name)?;
                if data.modules.remove(name).is_none() {
                    return Err(McpError::invalid_params(format!("モジュール '{}' が存在しません", name), None));
                }
                data.updated_at = now;
                let json = serde_json::to_string_pretty(&data)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                write_artifact_atomic(&artifact_path, &json).await?;
                if let Err(e) = self.app_handle.emit("artifact-changed", serde_json::json!({
                    "worktreeId": worktree_id, "artifactId": id, "command": "delete_module",
                })) { log::warn!("Failed to emit artifact-changed: {}", e); }
                json
            }
            other => return Err(McpError::invalid_params(
                format!("不明なコマンド '{}'. list/get/create/update/rewrite/delete のいずれかを指定してください", other),
                None,
            )),
        };

        log::info!("[mcp] artifact_module command={} id={} module={:?}", command, id, module_name);
        Ok(CallToolResult::success(vec![Content::text(result_json)]))
    }

    #[tool(description = "ワークツリーに通知を送信する。event_kind に \"worktree.message\" を指定すると、通知に加えて自分のワークツリーを購読している他のワークツリーへ body を自由文メッセージとして配送する（購読方式なので送信側は宛先を指定しない）。受信側には本文ではなく「届いている」ことだけが提示され、本文は受信側が oretachi_poll_inbox で取得する")]
    async fn notify_worktree(
        &self,
        Parameters(NotifyWorktreeParams { worktree_name, kind, body, event_kind, terminal_id, project_dir }): Parameters<NotifyWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        // 購読イベントの発行は通知トーストとは**独立した第三の経路**（event_db → 配送
        // ワーカー）に載せる。`hook_tx`（broadcast channel）も WebView IPC もトースト用の
        // 経路で、下の `should_send_notify` による debounce（hook 3s / approval 1s）が
        // かかる。購読配送を debounce に載せるとイベントが黙って落ちる（#120 §1）ので、
        // 判定より前にここで発行しきる。
        //
        // **失敗しても `?` で早期 return しない。** イベント発行のエラー（DB 未初期化、
        // terminal_id の解決失敗、body 空など）でトーストまで巻き添えにすると、
        // 「`kind=approval` に `event_kind` を添えただけで承認待ちトーストが出ない」という
        // 独立経路の設計と真逆の挙動になる。トーストは必ず送り、失敗はレスポンスで返す。
        let event_result = match event_kind.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(k) => Some(
                publish_worktree_message(
                    &self.app_handle,
                    k,
                    body.as_deref(),
                    terminal_id.as_deref(),
                    project_dir.as_deref(),
                )
                .await,
            ),
            None => None,
        };

        let event = NotifyWorktreeEvent {
            worktree_name: worktree_name.clone(),
            kind: kind.unwrap_or_else(|| "general".to_string()),
            body,
            agent: None,
            // `notify_worktree` ツール（kind 明示の意図的な通知）は trayNotification に
            // かかわらず常にトレイへ出す。オフでもユーザー判断を仰げる唯一の経路。
            tray: true,
        };
        // 通知トーストを送ったかどうかにかかわらず、イベント発行の結果は必ず返す
        // （debounce で落ちても購読配送は成立しているため、"ok" だけ返すと嘘になる）。
        // イベント発行が失敗した場合はツール結果を isError にして呼び出し元に気付かせる
        // ——トーストは既に送っているので、エラーにしても通知は失われない。
        let reply = |notified: bool| {
            let result = match &event_result {
                Some(Ok(v)) => CallToolResult::success(vec![Content::text(
                    serde_json::json!({ "ok": true, "notified": notified, "event": v }).to_string(),
                )]),
                Some(Err(e)) => CallToolResult::error(vec![Content::text(
                    serde_json::json!({
                        "ok": false,
                        "notified": notified,
                        "error": e.message,
                        "hint": "通知トースト自体は送信済みです。購読イベントの発行だけが失敗しました。",
                    })
                    .to_string(),
                )]),
                None => CallToolResult::success(vec![Content::text("ok")]),
            };
            Ok(result)
        };

        let hook_tx = {
            let manager = self.app_handle.state::<McpServerManager>();
            // HTTP 経路と同じ debounce ポリシー (hook=3s, approval=1s, 他は対象外) を適用
            if !should_send_notify(&manager.notify_last_sent, &event.worktree_name, &event.kind) {
                return reply(false);
            }
            manager.hook_tx.clone()
        };
        if event.kind == "hook" {
            // hook イベントは broadcast channel 経由で MCP ピアに直接送信する（WebView IPC をバイパス）
            let _ = hook_tx.send(event);
            reply(true)
        } else {
            // トーストの emit に失敗しても `?` で返さない。イベント発行は既に済んでいる
            // ので、ここで Err にすると成功した配送結果まで呼び出し元から見えなくなる
            // （上のイベント発行側と同じ理由。2経路は互いを巻き添えにしない）。
            if let Err(e) = self.app_handle.emit("notify-worktree", &event) {
                log::warn!("[mcp] notify_worktree の emit に失敗: {}", e);
                // 報告すべきイベント結果が無い（＝従来どおりの呼び出し）なら、
                // 従来どおりエラーで返す。`"ok"` を返すと debounce と区別が付かない。
                if event_result.is_none() {
                    return Err(McpError::internal_error(e.to_string(), None));
                }
                return reply(false);
            }
            log::info!("[mcp] notify_worktree: {} kind={}", worktree_name, event.kind);
            reply(true)
        }
    }

    #[tool(description = "ワークツリーの1行説明(description)を直接セットする。description はこのワークツリーで進めている作業全体の目的を表す1行。更新するのは (a) 未設定のとき (b) 全く別の作業・目的に切り替わったとき (c) 説明が実態と大きくずれているとき、のみ。同一プラン内のサブタスク進行・レビュー対応・細部の修正では更新しないこと")]
    fn oretachi_set_description(
        &self,
        Parameters(SetWorktreeDescriptionParams { description, project_dir, worktree_name, worktree_id }): Parameters<SetWorktreeDescriptionParams>,
    ) -> Result<CallToolResult, McpError> {
        // 改行は空白に潰して1行に正規化。長すぎる場合は切り詰める。
        let description: String = description
            .replace(['\r', '\n'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if description.is_empty() {
            return Err(McpError::invalid_params("description must not be empty", None));
        }
        let description: String = description.chars().take(200).collect();

        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();

        // 解決優先順位: worktree_id > worktree_name > project_dir 逆引き
        let wt = resolve_worktree(
            &settings,
            worktree_id.as_deref(),
            worktree_name.as_deref(),
            project_dir.as_deref(),
            "specify one of project_dir / worktree_name / worktree_id",
        )?;

        let previous = wt.description.clone();
        let event = SetWorktreeDescriptionEvent {
            worktree: wt.name.clone(),
            plan: None,
            description: Some(description.clone()),
        };
        self.app_handle
            .emit("set-worktree-description", &event)
            .map_err(|e: tauri::Error| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_set_description: worktree={} desc={}", wt.name, description);

        let json = serde_json::json!({
            "ok": true,
            "worktree": wt.name,
            "previous": previous,
            "new": description,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&json).map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    #[tool(description = "ワークツリーのトレイ通知（フック由来の承認待ち・作業完了通知）のオン/オフを切り替える。enabled=true で通知する / enabled=false で通知しない / **enabled を省略すると「未設定」に戻り、所属ワークグループの既定値（無ければ true）へフォールバックする**。オフにしてもツール `notify_worktree` による明示通知は常にトレイへ出るため、ユーザーの判断を仰ぐ経路は残る。teamwork-parent のような進行管理セッションが自分自身のノイズを止める用途を想定している。他人のワークツリーを勝手にオフにしないこと")]
    fn oretachi_set_tray_notification(
        &self,
        Parameters(SetTrayNotificationParams { enabled, project_dir, worktree_name, worktree_id }): Parameters<SetTrayNotificationParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();

        // 解決優先順位: worktree_id > worktree_name > project_dir 逆引き（set_description と同じ）
        let wt = resolve_worktree(
            &settings,
            worktree_id.as_deref(),
            worktree_name.as_deref(),
            project_dir.as_deref(),
            "specify one of project_dir / worktree_name / worktree_id",
        )?;

        let previous = wt.tray_notification;
        let previous_effective = resolve_tray_notification(&settings, wt);
        // 変更後の実効値は「値をコピーせず毎回解決する」規則を壊さないよう、
        // 新しい値を載せたエントリを resolve_tray_notification に通して求める。
        let new_effective = {
            let mut probe = wt.clone();
            probe.tray_notification = enabled;
            resolve_tray_notification(&settings, &probe)
        };

        // 永続化と UI 反映はフロント（App.vue）に任せる。Rust 側の SettingsManager を
        // 直接書き換えると、フロントが持つ settings と食い違って次の save で巻き戻る。
        let event = SetTrayNotificationEvent {
            worktree: wt.name.clone(),
            worktree_id: wt.id.clone(),
            tray_notification: enabled,
        };
        self.app_handle
            .emit("set-worktree-tray-notification", &event)
            .map_err(|e: tauri::Error| McpError::internal_error(e.to_string(), None))?;
        log::info!(
            "[mcp] oretachi_set_tray_notification: worktree={} {:?} -> {:?} (effective {} -> {})",
            wt.name, previous, enabled, previous_effective, new_effective
        );

        let json = serde_json::json!({
            "ok": true,
            "worktree": wt.name,
            "previous": previous,
            "previousEffective": previous_effective,
            "new": enabled,
            "newEffective": new_effective,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&json).map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    #[tool(description = "登録済みワークツリーのステータス一覧を取得する。各エントリはルートパス(path)・1行説明(description)・ブランチ名・所属ワークグループ(workgroupId / workgroupName)・isHome・isRepository を含む。isHome / isRepository が true のものは git ワークツリーではない擬似エントリなので、作業割り当てや削除の候補からは外すこと。query で name / branchName / description の部分一致検索ができる", annotations(read_only_hint = true))]
    fn oretachi_get_worktree_status(
        &self,
        Parameters(GetWorktreeStatusParams { query }): Parameters<GetWorktreeStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let detached: std::collections::HashSet<&str> =
            settings.detached_worktree_ids.iter().map(|s| s.as_str()).collect();

        // query は name / branchName / description のいずれかに部分一致すれば通す（大文字小文字を無視）
        let needle = query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(|q| q.to_lowercase());

        let results: Vec<serde_json::Value> = settings
            .worktrees
            .iter()
            .filter(|wt| match needle.as_deref() {
                None => true,
                Some(q) => {
                    wt.name.to_lowercase().contains(q)
                        || wt.branch_name.to_lowercase().contains(q)
                        || wt
                            .description
                            .as_deref()
                            .map_or(false, |d| d.to_lowercase().contains(q))
                }
            })
            .map(|wt| {
                // 所属ワークグループ。未設定なら先頭グループへフォールバックする（UI の表示と同じ解決）。
                // グループ自体が未定義なら workgroupId / workgroupName ともに null。
                let group = resolve_workgroup(&settings, wt);
                serde_json::json!({
                    "id": wt.id,
                    "name": wt.name,
                    "path": wt.path,
                    "description": wt.description,
                    "repositoryName": wt.repository_name,
                    "branchName": wt.branch_name,
                    "workgroupId": group.map(|g| g.id.as_str()),
                    "workgroupName": group.map(|g| workgroup_display_name(&settings, g)),
                    "isHome": wt.is_home,
                    "isRepository": wt.is_repository,
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

    #[tool(description = "指定ワークツリーの git 状態を返す。未コミット変更のあるファイル数(dirtyCount)・ベースブランチへのマージ済み判定(mergedInto)・最終コミット日時(lastCommitAt)・ahead/behind を含む。不要ワークツリーの判定根拠に使う", annotations(read_only_hint = true))]
    fn oretachi_inspect_worktree(
        &self,
        Parameters(InspectWorktreeParams { worktree_name, worktree_id, base_branch }): Parameters<InspectWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();

        let wt = resolve_worktree(
            &settings,
            worktree_id.as_deref(),
            worktree_name.as_deref(),
            None,
            "specify one of worktree_name / worktree_id",
        )?;

        // ホーム / リポジトリは git ワークツリーではないので、ワークツリーとしての git 状態を持たない
        // （リポジトリ root で git は動くが branchName が空の擬似エントリなので結果が噛み合わない）
        if wt.is_home || wt.is_repository {
            let kind = if wt.is_home { "home worktree" } else { "repository" };
            return Err(McpError::invalid_params(
                format!("worktree '{}' is the {} and has no worktree git state", wt.name, kind),
                None,
            ));
        }

        let inspection = crate::git_worktree::inspect_worktree(&wt.path, base_branch.as_deref())
            .map_err(|e| McpError::internal_error(e, None))?;

        let mut json = serde_json::to_value(&inspection)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        if let Some(obj) = json.as_object_mut() {
            obj.insert("id".to_string(), serde_json::json!(wt.id));
            obj.insert("name".to_string(), serde_json::json!(wt.name));
            obj.insert("path".to_string(), serde_json::json!(wt.path));
        }

        log::info!(
            "[mcp] oretachi_inspect_worktree: name={} dirty={} merged={:?}",
            wt.name, inspection.dirty_count, inspection.merged_into
        );
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&json)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    #[tool(description = "AI agent が参照するグローバル app options (background terminal の起動先トグル等) を取得する", annotations(read_only_hint = true))]
    fn oretachi_get_app_options(
        &self,
        Parameters(_params): Parameters<GetAppOptionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let json = serde_json::to_string(&serde_json::json!({
            "useOretachiTerminalForBackground": settings.use_oretachi_terminal_for_background,
        }))
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!(
            "[mcp] oretachi_get_app_options: useOretachiTerminalForBackground={}",
            settings.use_oretachi_terminal_for_background
        );
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all registered repositories with their names and git remote URLs", annotations(read_only_hint = true))]
    async fn oretachi_list_repository(
        &self,
        Parameters(_params): Parameters<ListRepositoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let paths: Vec<(String, String, Option<String>)> = settings
            .repositories
            .iter()
            .map(|repo| {
                let pattern = repo
                    .branch_name_pattern
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string);
                (repo.name.clone(), repo.path.clone(), pattern)
            })
            .collect();
        let repos: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
            paths
                .iter()
                .map(|(name, path, pattern)| {
                    let remotes = get_git_remotes(path);
                    serde_json::json!({ "name": name, "remotes": remotes, "branchNamePattern": pattern })
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

    #[tool(description = "登録済みワークグループの一覧を返す。各エントリは id・表示名(name)・色(color)・タスク実行エージェント(taskAddAgent)・isDefault を含む。oretachi_add_task で追加先ワークグループを指定する前に、利用可能なワークグループを確認するために使う。isDefault は「ワークグループ未設定のワークツリーが表示上フォールバックする先頭グループ」を意味し、oretachi_add_task で指定を省略したときの追加先ではない (省略時はUIで現在選択中のワークグループに入る)", annotations(read_only_hint = true))]
    fn oretachi_list_workgroups(
        &self,
        Parameters(_params): Parameters<ListWorkgroupsParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        // 先頭グループが既定（未所属ワークツリーのフォールバック先）。
        // resolve_workgroup / フロントの useWorkgroups.resolvedGroupId と同じ規則。
        let groups: Vec<serde_json::Value> = settings
            .workgroups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                serde_json::json!({
                    "id": g.id,
                    "name": workgroup_display_name(&settings, g),
                    "color": g.color,
                    "taskAddAgent": g.task_add_agent,
                    "isDefault": i == 0,
                })
            })
            .collect();
        let json = serde_json::to_string_pretty(&groups)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_list_workgroups: {} groups", groups.len());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "アーティファクトを検索する。queryを省略すると全件返却。title/content/type/languageを対象に部分一致検索。結果はcontentを除いたメタデータのみ。検索対象は project_dir(現在の作業ディレクトリ)で指定するのが最も確実。HOMEタブやリポジトリルートで作業している場合は project_dir が必須", annotations(read_only_hint = true))]
    async fn search_artifact(
        &self,
        Parameters(SearchArtifactParams {
            project_dir,
            worktree_id: target_worktree_id,
            repository,
            branch,
            query,
        }): Parameters<SearchArtifactParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();
        let wt = resolve_artifact_worktree(
            &settings,
            target_worktree_id.as_deref(),
            project_dir.as_deref(),
            repository.as_deref(),
            branch.as_deref(),
        )?;
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

    #[tool(description = "タスク追加リクエストを送信する。AIがタスクコードを生成し、ワークツリー作成やエージェント実行を非同期で行う。追加先ワークグループを指定したい場合は oretachi_list_workgroups で一覧を取得してから workgroup_id / workgroup_name を渡す")]
    fn oretachi_add_task(
        &self,
        Parameters(AddTaskParams { prompt, remote_exec, workgroup_id, workgroup_name }): Parameters<AddTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(McpError::invalid_params("prompt must not be empty", None));
        }
        let resolved_workgroup_id = {
            let settings_manager = self.app_handle.state::<SettingsManager>();
            let settings = settings_manager.get();
            resolve_workgroup_target(&settings, workgroup_id.as_deref(), workgroup_name.as_deref())
                .map_err(|e| McpError::invalid_params(e, None))?
        };
        let remote = remote_exec.unwrap_or(false);
        let event = AddTaskEvent {
            prompt: prompt.clone(),
            remote_exec: remote,
            workgroup_id: resolved_workgroup_id.clone(),
        };
        self.app_handle
            .emit("mcp-add-task", &event)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        log::info!(
            "[mcp] oretachi_add_task: prompt={} remote_exec={} workgroup_id={}",
            prompt,
            remote,
            resolved_workgroup_id.as_deref().unwrap_or("(default)")
        );
        Ok(CallToolResult::success(vec![Content::text(
            "タスク追加リクエストを送信しました。タスクの生成・実行は非同期に行われます。",
        )]))
    }

    #[tool(description = "ワークツリーをアーカイブ（クローズ）する。アーカイブDBに記録してgitワークツリーを削除する", annotations(destructive_hint = true))]
    async fn oretachi_close_worktree(
        &self,
        Parameters(CloseWorktreeParams { worktree_name, worktree_id, merge_to, delete_branch, force_branch }): Parameters<CloseWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_name = worktree_name.trim().to_string();
        if worktree_name.is_empty() {
            return Err(McpError::invalid_params("worktree_name must not be empty", None));
        }

        // await をまたいで State / settings の参照を保持しないよう、ここで所有権のある値へ確定させる
        let (target_id, target_name) = {
            let settings_manager = self.app_handle.state::<SettingsManager>();
            let settings = settings_manager.get();

            // worktree_name は上で空チェック済みなので missing_hint には到達しない
            let wt = resolve_worktree(
                &settings,
                worktree_id.as_deref(),
                Some(worktree_name.as_str()),
                None,
                "specify one of worktree_name / worktree_id",
            )?;

            // ホーム / リポジトリは git ワークツリーではなく、path がワークツリー追加先ディレクトリ
            // またはリポジトリのルートそのもの。削除すると親ごと壊すため、要求は必ず拒否する。
            if wt.is_home || wt.is_repository {
                let kind = if wt.is_home { "home worktree" } else { "repository" };
                return Err(McpError::invalid_params(
                    format!("worktree '{}' is the {} and cannot be closed", wt.name, kind),
                    None,
                ));
            }

            (wt.id.clone(), wt.name.clone())
        };

        // フロント側の処理結果を受け取るための oneshot を先に登録してから emit する
        let request_id = format!(
            "close-{}",
            CLOSE_WORKTREE_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let rx = {
            let registry = self.app_handle.state::<CloseWorktreeAckRegistry>();
            registry.register(request_id.clone())
        };

        // force_branch 省略時は delete_branch と同じ値にする（UI の手動削除と同じ意味論）。
        // git branch -d は「現在の HEAD またはその upstream にマージ済みか」しか見ないため、
        // 未マージブランチはもちろん、merge_to を指定して別ワークツリーでマージした場合でも
        // メインリポジトリの HEAD 基準では未マージ判定になり失敗する。
        // ここで Option を潰すため、明示的な false は尊重される（-d でのマージ済み確認）。
        let delete_branch = delete_branch.unwrap_or(false);
        let force_branch = force_branch.unwrap_or(delete_branch);

        let event = CloseWorktreeEvent {
            request_id: request_id.clone(),
            worktree_id: target_id,
            worktree_name: target_name.clone(),
            merge_to: merge_to.unwrap_or_default(),
            delete_branch,
            force_branch,
        };
        if let Err(e) = self.app_handle.emit("mcp-close-worktree", &event) {
            self.app_handle.state::<CloseWorktreeAckRegistry>().take(&request_id);
            return Err(McpError::internal_error(e.to_string(), None));
        }
        log::info!("[mcp] oretachi_close_worktree: name={} request_id={}", worktree_name, request_id);

        match tokio::time::timeout(
            std::time::Duration::from_secs(CLOSE_WORKTREE_ACK_TIMEOUT_SECS),
            rx,
        )
        .await
        {
            Ok(Ok(CloseWorktreeOutcome::Closed)) => Ok(CallToolResult::success(vec![Content::text(format!(
                "ワークツリー '{}' をクローズしました。",
                target_name
            ))])),
            Ok(Ok(CloseWorktreeOutcome::Cancelled)) => Ok(CallToolResult::success(vec![Content::text(format!(
                "ワークツリー '{}' のクローズはユーザー操作によりキャンセルされました。ワークツリーはそのまま残っています。",
                target_name
            ))])),
            Ok(Ok(CloseWorktreeOutcome::Busy)) => Ok(CallToolResult::success(vec![Content::text(format!(
                "ワークツリー '{}' は既にクローズ処理中です。再実行せず、oretachi_get_worktree_status で完了を確認してください。",
                target_name
            ))])),
            Ok(Ok(CloseWorktreeOutcome::Failed(msg))) => {
                log::warn!("[mcp] oretachi_close_worktree failed: name={} error={}", worktree_name, msg);
                Err(McpError::internal_error(
                    format!("ワークツリー '{}' のクローズに失敗しました: {}", target_name, msg),
                    None,
                ))
            }
            // 送信側が take 済みで drop された（通常発生しない）
            Ok(Err(_)) => Ok(CallToolResult::success(vec![Content::text(
                "ワークツリーのクローズリクエストを送信しましたが、結果を受け取れませんでした。oretachi_get_worktree_status で確認してください。",
            )])),
            Err(_) => {
                self.app_handle.state::<CloseWorktreeAckRegistry>().take(&request_id);
                Ok(CallToolResult::success(vec![Content::text(
                    "ワークツリーのクローズリクエストを送信しましたが、時間内に完了しませんでした。削除リトライ中（UI からキャンセル可能）か、メインウィンドウがまだ処理を受け付けていない可能性があります。このツールを再実行せず、oretachi_get_worktree_status で状態を確認してください（再実行しても処理中のクローズには影響しません）。",
                )]))
            }
        }
    }

    #[tool(description = "他ワークツリーのイベント（クローズ / 作成 / エージェントからの自由文メッセージ）を購読する。別ワークツリーで進めている関連作業の完了や開始を自分のセッションで検知したいときに使う。target には \"*\" / \"workgroup:<ID>\" / \"repo:<名前>\" のワイルドカードも指定でき、まだ存在しないワークツリーの作成も購読できる。クローズ / 作成は本文ごと提示される。自由文メッセージは**本文を運ばず「届いている」ことと件数だけ**が提示されるので、本文は oretachi_poll_inbox で取りに来ること。提示のタイミングは待機中なら随時、走行中はターン境界、およびセッション開始時。ターミナルを閉じたりアプリを再起動したりしても購読は引き継ぎ待ちとして保持され、同じワークツリーで**同じ AI セッション**（--resume で再開した会話）が立ち上がったときに自動で引き継がれる。別のセッションへ渡す場合は oretachi の購読パネルから人間が引き継ぎ先を選ぶ（無関係なタスクのセッションが黙って他ワークツリーのイベントを拾わないようにするため）")]
    async fn oretachi_subscribe_worktree(
        &self,
        Parameters(SubscribeWorktreeParams {
            target,
            event_kinds,
            delivery,
            spawn_if_closed,
            expires_in,
            terminal_id,
            project_dir,
        }): Parameters<SubscribeWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let target = target.trim().to_string();
        if target.is_empty() {
            return Err(McpError::invalid_params("target must not be empty", None));
        }

        // 未対応種別を黙って受け付けると「購読したのに来ない」という分かりにくい失敗に
        // なるので明示的に弾く。既定が `worktree.closed` だけなのは後方互換のため
        // （`*` の主用途は `worktree.created` なので description で明示している）。
        let kinds = event_kinds
            .unwrap_or_else(|| vec![crate::event_db::KIND_WORKTREE_CLOSED.to_string()]);
        if kinds.is_empty() {
            return Err(McpError::invalid_params(
                "event_kinds must not be empty",
                None,
            ));
        }
        for k in &kinds {
            if !crate::event_db::SUPPORTED_EVENT_KINDS.contains(&k.as_str()) {
                return Err(McpError::invalid_params(
                    format!(
                        "event_kind '{}' は未対応です。対応種別: [{}]",
                        k,
                        crate::event_db::SUPPORTED_EVENT_KINDS.join(", ")
                    ),
                    None,
                ));
            }
        }

        let delivery = delivery
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| crate::event_db::DELIVERY_TURN_END.to_string());
        if !crate::event_db::SUPPORTED_DELIVERIES.contains(&delivery.as_str()) {
            return Err(McpError::invalid_params(
                format!(
                    "delivery '{}' は未対応です。対応値: [{}]",
                    delivery,
                    crate::event_db::SUPPORTED_DELIVERIES.join(", ")
                ),
                None,
            ));
        }

        if let Some(secs) = expires_in {
            if secs <= 0 {
                return Err(McpError::invalid_params(
                    "expires_in must be a positive number of seconds (省略すると無期限)",
                    None,
                ));
            }
        }

        // await をまたいで State / settings の参照を保持しないよう、ここで所有権のある値へ確定させる
        let (subscriber, resolved) = {
            let subscriber = resolve_subscriber(
                &self.app_handle,
                terminal_id.as_deref(),
                project_dir.as_deref(),
            )?;
            let settings = self.app_handle.state::<SettingsManager>().get();
            let resolved = resolve_subscription_target(&settings, target.as_str())?;
            (subscriber, resolved)
        };
        let target_id = resolved.stored.clone();
        let target_name = resolved.label.clone();

        // 逆引きできないと自己エコー抑止も購読者クローズ時の掃除も効かないため受け付けない
        let Some(subscriber_worktree_id) = subscriber.worktree_id.clone() else {
            return Err(McpError::invalid_params(
                "呼び出し元ターミナルの作業ディレクトリから oretachi 管理下のワークツリーを特定できませんでした。oretachi が管理しているワークツリー内で実行してください",
                None,
            ));
        };
        // 厳密一致のときだけ「自分自身は購読できない」を課す。ワイルドカードでは
        // 自ワークツリーのイベントも必ず target にマッチするが、そちらは `is_self_echo` が
        // 配送段階で落とすので購読自体は成立する（他のワークツリーのぶんが届く）。
        if resolved.worktree_id.as_deref() == Some(subscriber_worktree_id.as_str()) {
            return Err(McpError::invalid_params(
                format!(
                    "自分自身が居る{}のクローズは購読できません（クローズされた時点でこのセッションも消えるため通知先がありません）",
                    target_name
                ),
                None,
            ));
        }

        let pool = event_pool(&self.app_handle)?;
        let now = crate::event_db::now_ms();
        let sub = crate::event_db::SubscriptionRow {
            id: uuid::Uuid::new_v4().to_string(),
            subscriber_terminal_id: subscriber.terminal_id.clone(),
            subscriber_worktree_id: Some(subscriber_worktree_id),
            subscriber_agent_session: subscriber.agent_session.clone(),
            target: target_id.clone(),
            event_kinds: serde_json::to_string(&kinds)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            delivery: delivery.clone(),
            spawn_if_closed: i64::from(spawn_if_closed.unwrap_or(false)),
            created_at: now,
            // 巨大な値を渡されても panic させない（debug ビルドのオーバーフロー検査対策）
            expires_at: expires_in.map(|secs| now.saturating_add(secs.saturating_mul(1000))),
            state: crate::event_db::STATE_ACTIVE.to_string(),
            // 呼び出し元のタブは生存しているので引き継ぎ待ちではない。再購読で orphaned な
            // 行を上書きしたときも、ここで active + None に戻るのが正しい。
            orphaned_at: None,
        };
        // 既存の購読を更新した場合は既存の id が返る（再購読で解除手段を失わせない）
        let subscription_id = crate::event_db::upsert_subscription(&pool, &sub)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        // カードの購読バッジ（#137）は常時見えているので、エージェントが購読した瞬間に
        // 反映されないと嘘の状態を見せ続ける。イベント名は「未読が変わった」だが、
        // フロントの `loadSubscriptions` は購読一覧も一緒に読み直すのでこれで足りる。
        let _ = self.app_handle.emit("event-inbox-changed", ());

        log::info!(
            "[mcp] oretachi_subscribe_worktree: target={} ({}) terminal={} kinds={:?} delivery={} expires_at={:?} subscription_id={}",
            target_name, target_id, subscriber.terminal_id, kinds, delivery, sub.expires_at, subscription_id
        );
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({
                "subscriptionId": subscription_id,
                "target": target_name,
                "targetWorktreeId": target_id,
                "eventKinds": kinds,
                "delivery": delivery,
                "expiresAt": sub.expires_at,
                "subscriberTerminalId": subscriber.terminal_id,
                "message": format!(
                    "{}の {} を購読しました。イベントが発生すると次のセッション開始時に通知が提示されます（oretachi_poll_inbox でも取得できます）。{}{}",
                    target_name,
                    kinds.join(" / "),
                    // `*` は将来立つワークツリーも含めて全部に反応する。自動 spawn と
                    // 組み合わせると意図せずタブが立つので、そこだけは明示的に警告する
                    // （端末数上限 / クールダウンで爆発はしないが、驚きは残る）。
                    if resolved.worktree_id.is_none() && sub.spawn_if_closed != 0 {
                        " 注意: ワイルドカード購読と spawn_if_closed を併用しているため、未読が溜まると自動でタブが立ちます（生存端末数の上限と再試行のクールダウンで制限されます）。"
                    } else {
                        ""
                    },
                    // 自動引き継ぎは同一 AI セッション限定なので、セッションを名乗れない
                    // エージェント（gemini / codex / cline は session UUID を持たない。
                    // Claude Code でも状態ファイルを辿れない構成では取れない）では
                    // **タブを閉じた時点で自動復帰の手段が無くなる**。黙って失われるより、
                    // 購読した本人に伝えて ack を急がせるか手動引き継ぎへ誘導する。
                    if subscriber.agent_session.is_none() {
                        " 注意: このターミナルの AI セッションを特定できないため、タブを閉じるとこの購読は自動では引き継がれません（oretachi の購読パネルから手動で引き継いでください）。"
                    } else {
                        ""
                    }
                ),
            })
            .to_string(),
        )]))
    }

    #[tool(description = "ワークツリーのクローズ購読を解除する。subscription_id か target のどちらかを指定する", annotations(idempotent_hint = true))]
    async fn oretachi_unsubscribe_worktree(
        &self,
        Parameters(UnsubscribeWorktreeParams { subscription_id, target, terminal_id, project_dir }): Parameters<UnsubscribeWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let subscription_id = subscription_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let target = target.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if subscription_id.is_none() && target.is_none() {
            return Err(McpError::invalid_params(
                "subscription_id または target を指定してください",
                None,
            ));
        }

        let (subscriber, target_id) = {
            let subscriber = resolve_subscriber(
                &self.app_handle,
                terminal_id.as_deref(),
                project_dir.as_deref(),
            )?;
            // 購読対象が既にクローズ済みで settings から消えている場合もあるため、
            // 解決できなければ正規化した文字列をそのまま保存値として扱う（#126 の
            // ワイルドカードも `resolve_subscription_target` が同じ正規化を通す）
            let target_id = target.as_ref().map(|t| {
                let settings = self.app_handle.state::<SettingsManager>().get();
                resolve_subscription_target(&settings, t.as_str())
                    .map(|r| r.stored)
                    .unwrap_or_else(|_| crate::event_db::normalize_target(t))
            });
            (subscriber, target_id)
        };

        await_rebind(&self.app_handle, &subscriber).await;
        let pool = event_pool(&self.app_handle)?;
        let deleted = if let Some(id) = subscription_id.as_deref() {
            crate::event_db::delete_subscription(&pool, id, &subscriber.terminal_id).await
        } else {
            crate::event_db::delete_subscription_by_target(
                &pool,
                &subscriber.terminal_id,
                target_id.as_deref().unwrap_or_default(),
            )
            .await
        }
        .map_err(|e| McpError::internal_error(e, None))?;
        // 購読が減ったこともカードのバッジへ即座に伝える（#137）
        if deleted > 0 {
            let _ = self.app_handle.emit("event-inbox-changed", ());
        }

        log::info!(
            "[mcp] oretachi_unsubscribe_worktree: terminal={} subscription_id={:?} target={:?} deleted={}",
            subscriber.terminal_id, subscription_id, target_id, deleted
        );
        Ok(CallToolResult::success(vec![Content::text(if deleted > 0 {
            format!("購読を解除しました（{} 件）。", deleted)
        } else {
            "該当する購読はありませんでした（既に解除済み、または別のターミナルの購読です）。oretachi_list_subscriptions で確認してください。".to_string()
        })]))
    }

    #[tool(description = "自分のターミナルが登録しているワークツリー購読の一覧と、未確認メッセージ件数を返す。自分が何を購読中か分からなくなったときに使う", annotations(read_only_hint = true))]
    async fn oretachi_list_subscriptions(
        &self,
        Parameters(ListSubscriptionsParams { terminal_id, project_dir }): Parameters<ListSubscriptionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let subscriber = resolve_subscriber(
            &self.app_handle,
            terminal_id.as_deref(),
            project_dir.as_deref(),
        )?;
        await_rebind(&self.app_handle, &subscriber).await;
        let pool = event_pool(&self.app_handle)?;
        let now = crate::event_db::now_ms();
        let subs = crate::event_db::list_subscriptions(&pool, &subscriber.terminal_id, now)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        let (unacked, undelivered) = crate::event_db::count_unacked(&pool, &subscriber.terminal_id)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;

        // 名前の付与は表示目的のみ。厳密一致 target が既にクローズ済みなら ID だけになる。
        let settings = self.app_handle.state::<SettingsManager>().get();
        let items: Vec<serde_json::Value> = subs
            .iter()
            .map(|s| {
                let (target_kind, target_label) = describe_target(&settings, &s.target);
                let name = if target_kind == "worktree" { target_label.clone() } else { None };
                serde_json::json!({
                    "subscriptionId": s.id,
                    "target": s.target,
                    "targetKind": target_kind,
                    "targetLabel": target_label,
                    "targetWorktreeId": s.target,
                    "targetWorktreeName": name,
                    "eventKinds": serde_json::from_str::<serde_json::Value>(&s.event_kinds).unwrap_or(serde_json::Value::Null),
                    "delivery": s.delivery,
                    "spawnIfClosed": s.spawn_if_closed != 0,
                    "createdAt": s.created_at,
                    "expiresAt": s.expires_at,
                    "state": s.state,
                    "orphanedAt": s.orphaned_at,
                })
            })
            .collect();

        log::info!(
            "[mcp] oretachi_list_subscriptions: terminal={} count={} unacked={}",
            subscriber.terminal_id, items.len(), unacked
        );
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({
                "terminalId": subscriber.terminal_id,
                "subscriptions": items,
                "unackedMessages": unacked,
                "undeliveredMessages": undelivered,
            })
            .to_string(),
        )]))
    }

    // `read_only_hint = true` は artifact 系ツール（本ファイル冒頭のコメント参照）と同じ理由で
    // 意図的に付けている: plan モードで一律 ask になるのを避けるため。実際には
    // `delivered_at` を UPDATE するが、`WHERE delivered_at IS NULL` ガード付きの冪等更新なので
    // 並列実行されても破損しない。
    #[tool(description = "購読していたワークツリーイベントの未確認メッセージを取得する。セッション開始時の自動提示を取りこぼした場合や、走行中に届いた分を自分で取りに行く場合に使う。読んだら oretachi_ack_message で ack すること", annotations(read_only_hint = true))]
    async fn oretachi_poll_inbox(
        &self,
        Parameters(PollInboxParams { terminal_id, project_dir, include_acked }): Parameters<PollInboxParams>,
    ) -> Result<CallToolResult, McpError> {
        let subscriber = resolve_subscriber(
            &self.app_handle,
            terminal_id.as_deref(),
            project_dir.as_deref(),
        )?;
        await_rebind(&self.app_handle, &subscriber).await;
        let pool = event_pool(&self.app_handle)?;
        // 明示的な pull なので未 ack 全件を返す（自動注入を取りこぼしてもここで必ず回収できる）
        let filter = if include_acked.unwrap_or(false) {
            crate::event_db::InboxFilter::All
        } else {
            crate::event_db::InboxFilter::Unacked
        };
        let mut items = crate::event_db::list_inbox(&pool, &subscriber.terminal_id, filter)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;

        // 取得した時点で「送った」印を打つ。ack とは分けているので、ack されない限り
        // 行は残り、UI や再 poll から人間が気づける（#120 §5.2）。
        let now = crate::event_db::now_ms();
        let ids: Vec<String> = items
            .iter()
            .filter(|i| i.delivered_at.is_none())
            .map(|i| i.id.clone())
            .collect();
        if let Err(e) = crate::event_db::mark_delivered(&pool, &ids, now).await {
            log::warn!("[mcp] oretachi_poll_inbox: mark_delivered failed: {}", e);
        } else {
            // 打刻した値をレスポンスにも反映する（DB と表示のズレを残さない）
            for item in items.iter_mut().filter(|i| i.delivered_at.is_none()) {
                item.delivered_at = Some(now);
            }
        }

        log::info!(
            "[mcp] oretachi_poll_inbox: terminal={} count={}",
            subscriber.terminal_id,
            items.len()
        );
        let messages: Vec<serde_json::Value> = items
            .iter()
            .map(|i| {
                serde_json::json!({
                    "id": i.id,
                    "kind": i.kind,
                    "sourceWorktreeId": i.source_worktree_id,
                    "body": serde_json::from_str::<serde_json::Value>(&i.body).unwrap_or(serde_json::Value::Null),
                    "actor": i.actor,
                    "createdAt": i.created_at,
                    "deliveredAt": i.delivered_at,
                    // セッション開始時に告知だけした時刻（ターンは開始していない）。
                    // `deliveredAt` と分けているので、どちらの経路で目にしたか区別できる
                    "notifiedAt": i.notified_at,
                    "ackedAt": i.acked_at,
                    "text": crate::event_db::format_inbox_line(i),
                })
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({
                "terminalId": subscriber.terminal_id,
                "messages": messages,
                "hint": if messages.is_empty() {
                    "未確認メッセージはありません。".to_string()
                } else {
                    "内容を確認したら oretachi_ack_message に id を渡して ack してください。ack しない限りこの一覧に残り続けます（セッション開始時の自動提示は一度だけなので、以後はこのツールで取り直します）。".to_string()
                },
            })
            .to_string(),
        )]))
    }

    #[tool(description = "受け取ったワークツリーイベントのメッセージを確認済み（ack）にする。ack しない限りセッション開始ごとに再掲される。既に ack 済みの ID を渡してもエラーにならない", annotations(idempotent_hint = true))]
    async fn oretachi_ack_message(
        &self,
        Parameters(AckMessageParams { ids, terminal_id, project_dir }): Parameters<AckMessageParams>,
    ) -> Result<CallToolResult, McpError> {
        let ids: Vec<String> = ids
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            return Err(McpError::invalid_params("ids must not be empty", None));
        }
        let subscriber = resolve_subscriber(
            &self.app_handle,
            terminal_id.as_deref(),
            project_dir.as_deref(),
        )?;
        await_rebind(&self.app_handle, &subscriber).await;
        let pool = event_pool(&self.app_handle)?;
        let acked = crate::event_db::ack(
            &pool,
            &ids,
            &subscriber.terminal_id,
            crate::event_db::now_ms(),
        )
        .await
        .map_err(|e| McpError::internal_error(e, None))?;

        log::info!(
            "[mcp] oretachi_ack_message: terminal={} requested={} acked={}",
            subscriber.terminal_id,
            ids.len(),
            acked
        );
        // 未読件数が減ったことを UI に伝える。これが無いと正常系（押し込み → エージェントが
        // ack）でタブの未読バッジが消えず、人間がホームの購読パネルを開き直すまで残る。
        if acked > 0 {
            let _ = self.app_handle.emit("event-inbox-changed", ());
        }
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} 件を確認済みにしました（要求 {} 件。差分は既に ack 済みか、別のターミナル宛のメッセージです）。",
            acked,
            ids.len()
        ))]))
    }

    #[tool(description = "指定ワークツリーに新しいターミナルタブを追加し、与えられたコマンドを流し込む。pnpm dev / tauri dev / vite / next dev など長時間常駐するバックグラウンドコマンドを oretachi UI 上で起動するために使う")]
    fn oretachi_spawn_terminal(
        &self,
        Parameters(SpawnTerminalParams { worktree_name, worktree_id, command, title, reason }): Parameters<SpawnTerminalParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_name = worktree_name.trim().to_string();
        if worktree_name.is_empty() {
            return Err(McpError::invalid_params("worktree_name must not be empty", None));
        }
        if command.trim().is_empty() {
            return Err(McpError::invalid_params("command must not be empty", None));
        }

        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();

        // worktree_name は上で空チェック済みなので missing_hint には到達しない
        let wt = resolve_worktree(
            &settings,
            worktree_id.as_deref(),
            Some(worktree_name.as_str()),
            None,
            "specify one of worktree_name / worktree_id",
        )?;

        // detached（サブウィンドウ化済み）ワークツリーへの spawn はフロント側で
        // handleDetachedMcpSpawn 経由でサブウィンドウへ pendingCommand 付きでルーティングされる。
        let event = SpawnTerminalEvent {
            worktree_id: wt.id.clone(),
            command: command.clone(),
            title: title.clone(),
        };
        self.app_handle
            .emit("mcp-spawn-terminal", &event)
            .map_err(|e: tauri::Error| McpError::internal_error(e.to_string(), None))?;
        log::info!(
            "[mcp] oretachi_spawn_terminal: worktree={} command={:?} title={:?} reason={:?}",
            worktree_name, command, title, reason
        );
        Ok(CallToolResult::success(vec![Content::text(
            "新規ターミナルの追加リクエストを送信しました。oretachi UI に新しいタブが追加され、コマンドが流し込まれます。",
        )]))
    }

    #[tool(description = "指定ワークツリーを oretachi UI 上でフォーカスする。メインウィンドウにあればタブを切り替え（所属ワークグループが非アクティブなら併せて切り替え）、サブウィンドウへ分離済みならそのウィンドウを前面に出す。ユーザーに特定ワークツリーの様子を見せたいときに使う", annotations(read_only_hint = true))]
    fn oretachi_show_worktree(
        &self,
        Parameters(ShowWorktreeParams { worktree_name, worktree_id }): Parameters<ShowWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();

        let wt = resolve_worktree(
            &settings,
            worktree_id.as_deref(),
            worktree_name.as_deref(),
            None,
            "specify one of worktree_name / worktree_id",
        )?;

        // detached かどうかの判定はフロント側に任せる。ここで分岐すると
        // サブウィンドウの生成/破棄との競合で古い情報を見ることになる。
        let event = ShowWorktreeEvent { worktree_id: wt.id.clone() };
        self.app_handle
            .emit("mcp-show-worktree", &event)
            .map_err(|e: tauri::Error| McpError::internal_error(e.to_string(), None))?;
        log::info!("[mcp] oretachi_show_worktree: name={} id={}", wt.name, wt.id);

        Ok(CallToolResult::success(vec![Content::text(format!(
            "ワークツリー '{}' を表示しました。",
            wt.name
        ))]))
    }

    #[tool(description = "git リポジトリには存在するが oretachi に未登録のワークツリーを取り込む。path 省略または dry_run=true で候補を列挙し、path を指定するとその1件を登録する。ワークツリーの追加先ディレクトリの外にあるものも検出できる")]
    async fn oretachi_import_worktree(
        &self,
        Parameters(ImportWorktreeParams { path, repository_name, dry_run }): Parameters<ImportWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        // await をまたいで State / settings の参照を保持しないよう、必要な値をここで所有権付きに確定させる
        let (repos, registered): (Vec<(String, String, String)>, std::collections::HashSet<String>) = {
            let settings_manager = self.app_handle.state::<SettingsManager>();
            let settings = settings_manager.get();

            let wanted = repository_name.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let repos: Vec<(String, String, String)> = settings
                .repositories
                .iter()
                .filter(|r| wanted.map_or(true, |n| r.name == n))
                .map(|r| (r.id.clone(), r.name.clone(), r.path.clone()))
                .collect();
            if repos.is_empty() {
                let names: Vec<&str> = settings.repositories.iter().map(|r| r.name.as_str()).collect();
                return Err(McpError::invalid_params(
                    match wanted {
                        Some(n) => format!("repository '{}' not found. available: [{}]", n, names.join(", ")),
                        None => "no repository is registered in oretachi".to_string(),
                    },
                    None,
                ));
            }

            // 登録済み判定にはリポジトリ擬似エントリ・ホームも含める（同一パスの二重登録を防ぐ）
            let mut registered: std::collections::HashSet<String> = settings
                .worktrees
                .iter()
                .map(|w| normalize_path_for_match(&w.path))
                .collect();
            // リポジトリ root 自体は「ワークツリー」として取り込む対象ではない
            registered.extend(repos.iter().map(|(_, _, p)| normalize_path_for_match(p)));
            (repos, registered)
        };

        // git worktree list はブロッキング I/O なのでワーカースレッドへ逃がす
        let scan_repos = repos.clone();
        let candidates: Vec<(String, String, crate::git_worktree::GitWorktreeInfo)> =
            tokio::task::spawn_blocking(move || {
                let mut out = Vec::new();
                for (repo_id, repo_name, repo_path) in &scan_repos {
                    match crate::git_worktree::list_worktrees(repo_path) {
                        Ok(list) => {
                            for info in list {
                                // bare リポジトリ本体は作業ディレクトリを持たない。
                                // メインワークツリーは git worktree remove できず、誤登録して削除すると
                                // リポジトリ本体を巻き込む（リポジトリをサブディレクトリで登録していると
                                // registered による除外をすり抜けるため、フラグで確実に外す）。
                                // prunable は実体ディレクトリが既に無く、登録しても壊れたカードになるだけ。
                                if info.bare || info.is_main || info.prunable {
                                    continue;
                                }
                                if registered.contains(&normalize_path_for_match(&info.path)) {
                                    continue;
                                }
                                out.push((repo_id.clone(), repo_name.clone(), info));
                            }
                        }
                        // 1 リポジトリの失敗で全体を落とさない（移動済み・削除済みのリポジトリがありうる）
                        Err(e) => log::warn!(
                            "[mcp] oretachi_import_worktree: git worktree list failed for {}: {}",
                            repo_path, e
                        ),
                    }
                }
                out
            })
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let target_path = path.as_deref().map(str::trim).filter(|s| !s.is_empty());

        // path 未指定 or dry_run なら候補の列挙だけ
        if target_path.is_none() || dry_run.unwrap_or(false) {
            let json: Vec<serde_json::Value> = candidates
                .iter()
                .map(|(_, repo_name, info)| {
                    serde_json::json!({
                        "path": info.path,
                        "name": worktree_name_from_path(&info.path),
                        "branch": info.branch,
                        "detached": info.detached,
                        "repositoryName": repo_name,
                    })
                })
                .collect();
            log::info!("[mcp] oretachi_import_worktree: {} candidate(s)", json.len());
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&json)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            )]));
        }

        let wanted_path = normalize_path_for_match(target_path.expect("checked above"));
        let Some((repo_id, repo_name, info)) = candidates
            .into_iter()
            .find(|(_, _, i)| normalize_path_for_match(&i.path) == wanted_path)
        else {
            return Err(McpError::invalid_params(
                format!(
                    "'{}' is not an unregistered worktree of any registered repository. call this tool without path to list candidates",
                    target_path.unwrap_or_default()
                ),
                None,
            ));
        };

        // フロント側の登録結果を受け取るための oneshot を先に登録してから emit する
        let request_id = format!(
            "import-{}",
            IMPORT_WORKTREE_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let rx = {
            let registry = self.app_handle.state::<ImportWorktreeAckRegistry>();
            registry.register(request_id.clone())
        };

        let name = worktree_name_from_path(&info.path);
        let event = ImportWorktreeEvent {
            request_id: request_id.clone(),
            repository_id: repo_id,
            repository_name: repo_name,
            path: info.path.clone(),
            name: name.clone(),
            // detached HEAD のワークツリーはブランチ名を持たない。空文字で登録する
            branch_name: info.branch.clone().unwrap_or_default(),
        };
        if let Err(e) = self.app_handle.emit("mcp-import-worktree", &event) {
            self.app_handle.state::<ImportWorktreeAckRegistry>().take(&request_id);
            return Err(McpError::internal_error(e.to_string(), None));
        }
        log::info!("[mcp] oretachi_import_worktree: path={} request_id={}", info.path, request_id);

        match tokio::time::timeout(
            std::time::Duration::from_secs(IMPORT_WORKTREE_ACK_TIMEOUT_SECS),
            rx,
        )
        .await
        {
            Ok(Ok(ImportWorktreeOutcome::Imported(id))) => Ok(CallToolResult::success(vec![Content::text(
                format!("ワークツリー '{}' ({}) を登録しました。id={}", name, info.path, id),
            )])),
            Ok(Ok(ImportWorktreeOutcome::AlreadyRegistered)) => Ok(CallToolResult::success(vec![Content::text(
                format!("'{}' は既に登録済みでした。", info.path),
            )])),
            Ok(Ok(ImportWorktreeOutcome::Failed(e))) => {
                Err(McpError::internal_error(format!("取り込みに失敗しました: {}", e), None))
            }
            // 送信側が drop された（ウィンドウが閉じた等）/ タイムアウト
            Ok(Err(_)) | Err(_) => {
                self.app_handle.state::<ImportWorktreeAckRegistry>().take(&request_id);
                Err(McpError::internal_error(
                    format!(
                        "'{}' の取り込み結果を {} 秒以内に受け取れませんでした。oretachi_get_worktree_status で登録状況を確認してください。",
                        info.path, IMPORT_WORKTREE_ACK_TIMEOUT_SECS
                    ),
                    None,
                ))
            }
        }
    }

    #[tool(description = "現在の PTY セッション一覧を返す。sessionId, terminalId, cwd, isAiAgent, agentName, agentSessionId, ワークツリー名/ID を含む。terminalId は oretachi がタブ毎に発番する UUID で、SessionStart 時に自分の terminal_id が伝えられているので、それと突合すれば自分自身のターミナルを同定できる。oretachi_kill_terminal を呼ぶ前の確認に使う", annotations(read_only_hint = true))]
    fn oretachi_list_terminals(
        &self,
        Parameters(ListTerminalsParams { worktree_name, worktree_id }): Parameters<ListTerminalsParams>,
    ) -> Result<CallToolResult, McpError> {
        let pty = self.app_handle.state::<PtyManager>();
        let raw = pty.list_sessions();

        let settings_manager = self.app_handle.state::<SettingsManager>();
        let settings = settings_manager.get();

        // どちらも未指定なら絞り込みなし（全 PTY を返す）
        let has_filter = worktree_id.as_deref().map_or(false, |s| !s.trim().is_empty())
            || worktree_name.as_deref().map_or(false, |s| !s.trim().is_empty());
        let filter_id: Option<String> = if has_filter {
            Some(
                resolve_worktree(
                    &settings,
                    worktree_id.as_deref(),
                    worktree_name.as_deref(),
                    None,
                    "specify one of worktree_name / worktree_id",
                )?
                .id
                .clone(),
            )
        } else {
            None
        };

        let mut items: Vec<serde_json::Value> = Vec::new();
        for info in raw {
            let matched_wt = info
                .cwd
                .as_deref()
                .and_then(|c| resolve_worktree_by_cwd(&settings, c));
            let matched_wt_id = matched_wt.map(|w| w.id.clone());
            if let Some(ref fid) = filter_id {
                if matched_wt_id.as_deref() != Some(fid.as_str()) {
                    continue;
                }
            }
            let status = if info.exit_code.is_some() { "exited" } else { "running" };
            items.push(serde_json::json!({
                "sessionId": info.session_id,
                "terminalId": info.terminal_id,
                "cwd": info.cwd,
                "isAiAgent": info.is_ai_agent,
                "agentName": info.agent_name,
                "agentSessionId": info.agent_session_id,
                "worktreeId": matched_wt_id,
                "worktreeName": matched_wt.map(|w| w.name.clone()),
                "status": status,
                "exitCode": info.exit_code,
                "lastCommandExitCode": info.last_command_exit_code,
            }));
        }

        let json = serde_json::to_string(&items)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "指定 PTY セッションを停止する。oretachi_list_terminals で取得した session_id を渡す。UI のタブは pty-exit イベント経由で自動的に消える", annotations(destructive_hint = true))]
    async fn oretachi_kill_terminal(
        &self,
        Parameters(KillTerminalParams { session_id }): Parameters<KillTerminalParams>,
    ) -> Result<CallToolResult, McpError> {
        let pty = self.app_handle.state::<PtyManager>();
        if !pty.list_sessions().iter().any(|s| s.session_id == session_id) {
            return Err(McpError::invalid_params(
                format!("session_id {} not found", session_id),
                None,
            ));
        }
        // taskkill 最大10秒 + watcher join を含むため tokio ワーカーを塞がないよう spawn_blocking
        let manager = pty.inner().clone();
        tauri::async_runtime::spawn_blocking(move || manager.kill(session_id, "mcp-kill-terminal"))
            .await
            .map_err(|e| McpError::internal_error(format!("spawn_blocking join error: {}", e), None))?
            .map_err(|e| McpError::internal_error(e, None))?;
        log::info!("[mcp] oretachi_kill_terminal: session_id={}", session_id);
        Ok(CallToolResult::success(vec![Content::text("killed")]))
    }

    #[tool(description = "指定 PTY セッションの最近の出力履歴を返す。レスポンスは JSON: { text, cursor, lostBytes }。text は ANSI 除去済み UTF-8。連続ポーリングで重複を避けるには、次回呼び出しの from_cursor に前回の cursor を渡す（差分読み）。lostBytes>0 はリングバッファ溢れで先頭が欠落したことを意味する。先に oretachi_list_terminals で session_id を取得すること", annotations(read_only_hint = true))]
    fn oretachi_read_terminal(
        &self,
        Parameters(ReadTerminalParams { session_id, max_bytes, from_cursor }): Parameters<ReadTerminalParams>,
    ) -> Result<CallToolResult, McpError> {
        let pty = self.app_handle.state::<PtyManager>();
        let result = pty
            .read_output_history(session_id, Some(max_bytes.unwrap_or(8192)), from_cursor)
            .map_err(|e| McpError::invalid_params(e, None))?;
        let text = crate::pty_manager::strip_ansi(&result.data);
        log::info!(
            "[mcp] oretachi_read_terminal: session_id={} bytes={} cursor={} lost_bytes={}",
            session_id,
            result.data.len(),
            result.cursor,
            result.lost_bytes
        );
        let json = serde_json::json!({
            "text": text,
            "cursor": result.cursor,
            "lostBytes": result.lost_bytes,
        })
        .to_string();
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "指定 PTY セッションへテキストを送信する。submit=true（デフォルト）なら改行を PowerShell/conpty 互換の \\r へ正規化し末尾にも保証して、コマンド送信扱いにする。submit=false なら raw のまま送る（vitest の単一キー入力など）")]
    fn oretachi_write_terminal(
        &self,
        Parameters(WriteTerminalParams { session_id, text, submit }): Parameters<WriteTerminalParams>,
    ) -> Result<CallToolResult, McpError> {
        let pty = self.app_handle.state::<PtyManager>();
        if !pty.list_sessions().iter().any(|s| s.session_id == session_id) {
            return Err(McpError::invalid_params(
                format!("session_id {} not found", session_id),
                None,
            ));
        }
        let payload = if submit.unwrap_or(true) {
            let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
            if normalized.ends_with('\r') {
                normalized
            } else {
                normalized + "\r"
            }
        } else {
            text
        };
        let bytes_len = payload.len();
        pty.write(session_id, payload.into_bytes())
            .map_err(|e| McpError::internal_error(e, None))?;
        log::info!(
            "[mcp] oretachi_write_terminal: session_id={} bytes={} submit={:?}",
            session_id,
            bytes_len,
            submit
        );
        Ok(CallToolResult::success(vec![Content::text("written")]))
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
            instructions: Some(
                "ワークツリーへの通知と description(1行説明) を管理します。作業内容が決まったら oretachi_set_description で作業全体の目的を1行でセットし、作業の目的そのものが変わったら更新してください（サブタスクの進行では更新不要）".to_string(),
            ),
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

// ─── worktree / kind の解決（userConfig 非依存 hook 用） ──────────────────────

/// パスを比較用に正規化する（`\`→`/`、末尾 `/` 除去、Windows は小文字化）。
fn normalize_path_for_match(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s
}

/// project_dir（${CLAUDE_PROJECT_DIR}）に一致するワークツリーエントリを返す。
fn resolve_worktree_by_dir<'a>(settings: &'a AppSettings, dir: &str) -> Option<&'a WorktreeEntry> {
    let target = normalize_path_for_match(dir);
    settings
        .worktrees
        .iter()
        .find(|w| normalize_path_for_match(&w.path) == target)
}

/// ターミナルの cwd からワークツリーを逆引きする（**前方一致＋最長一致**）。
///
/// `resolve_worktree_by_dir` の完全一致と違い、ワークツリーのサブディレクトリで
/// エージェントを起動した場合も解決できる。ホームの path はワークツリー追加先
/// ディレクトリ = 全ワークツリーの祖先なので、先頭一致だけで拾うと全ターミナルが
/// ホーム所属になってしまう。最長一致を採用する。
pub fn resolve_worktree_by_cwd<'a>(
    settings: &'a AppSettings,
    cwd: &str,
) -> Option<&'a WorktreeEntry> {
    let cp = std::path::Path::new(cwd);
    settings
        .worktrees
        .iter()
        .filter(|w| cp.starts_with(std::path::Path::new(&w.path)))
        .max_by_key(|w| std::path::Path::new(&w.path).components().count())
}

/// ワークツリーのパスから表示名（末尾ディレクトリ名）を取り出す。
/// フロントのリポジトリ名導出（`path.split(/[/\\]/).pop()`）と同じ規則。
fn worktree_name_from_path(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// 登録済みワークツリー名を列挙する（エラーメッセージ用）。
fn available_worktree_names(settings: &AppSettings) -> String {
    settings
        .worktrees
        .iter()
        .map(|w| w.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// worktree_id / worktree_name / project_dir のいずれかからワークツリーを1件に確定する。
/// 優先順位は id > name > project_dir。同名が複数ある場合は ID を列挙したエラーを返し、
/// 呼び出し元（AI agent）に worktree_id での指定をやり直させる。
/// `missing_hint` はどれも指定されなかったときのエラー文（ツールごとに受け付ける引数が違うため呼び出し側が渡す）。
fn resolve_worktree<'a>(
    settings: &'a AppSettings,
    worktree_id: Option<&str>,
    worktree_name: Option<&str>,
    project_dir: Option<&str>,
    missing_hint: &str,
) -> Result<&'a WorktreeEntry, McpError> {
    if let Some(id) = worktree_id.map(str::trim).filter(|s| !s.is_empty()) {
        return settings.worktrees.iter().find(|w| w.id == id).ok_or_else(|| {
            McpError::invalid_params(format!("worktree id '{}' not found", id), None)
        });
    }

    if let Some(name) = worktree_name.map(str::trim).filter(|s| !s.is_empty()) {
        let matches: Vec<_> = settings.worktrees.iter().filter(|w| w.name == name).collect();
        return match matches.len() {
            0 => Err(McpError::invalid_params(
                format!(
                    "worktree '{}' not found. available: [{}]",
                    name,
                    available_worktree_names(settings)
                ),
                None,
            )),
            1 => Ok(matches[0]),
            _ => {
                let ids: Vec<&str> = matches.iter().map(|w| w.id.as_str()).collect();
                Err(McpError::invalid_params(
                    format!(
                        "multiple worktrees named '{}'. specify worktree_id to disambiguate: [{}]",
                        name,
                        ids.join(", ")
                    ),
                    None,
                ))
            }
        };
    }

    if let Some(dir) = project_dir.map(str::trim).filter(|s| !s.is_empty()) {
        return resolve_worktree_by_dir(settings, dir).ok_or_else(|| {
            McpError::invalid_params(
                format!(
                    "no worktree matches project_dir '{}'. available: [{}]",
                    dir,
                    available_worktree_names(settings)
                ),
                None,
            )
        });
    }

    Err(McpError::invalid_params(missing_hint.to_string(), None))
}

/// 保存済みの購読 `target` を `(種別, 表示名)` へ分解する（#126）。
///
/// 種別は `worktree` | `all` | `workgroup` | `repo`。**UI / ツール応答はこの種別を見て
/// 「クローズ済み」の判定をすること。** 名前が引けないことだけを根拠にすると、
/// ワイルドカード購読がすべて「対象がクローズ済み」と誤表示される。
///
/// `*` は表示名を持たない（表示側でローカライズする）。`workgroup:` は表示名を解決する
/// （未リネームのグループは name が None なので、生の ID を出すと人間が読めない）。
pub fn describe_target(settings: &AppSettings, target: &str) -> (String, Option<String>) {
    if target == crate::event_db::TARGET_ALL {
        return ("all".to_string(), None);
    }
    if let Some(gid) = target.strip_prefix(crate::event_db::TARGET_WORKGROUP_PREFIX) {
        let label = settings
            .workgroups
            .iter()
            .find(|g| g.id == gid)
            .map(|g| workgroup_display_name(settings, g))
            .unwrap_or_else(|| gid.to_string());
        return ("workgroup".to_string(), Some(label));
    }
    if let Some(repo) = target.strip_prefix(crate::event_db::TARGET_REPO_PREFIX) {
        // 照合用に小文字化して保存しているので、元の表記が settings にあればそちらを出す。
        // 比較は `normalize_target` と同じ Unicode の `to_lowercase()` で行う。
        // `eq_ignore_ascii_case` だと非 ASCII を含む名前で保存値と突き合わなくなる。
        let label = settings
            .repositories
            .iter()
            .map(|r| r.name.as_str())
            .chain(settings.worktrees.iter().map(|w| w.repository_name.as_str()))
            .find(|name| name.to_lowercase() == repo)
            .unwrap_or(repo)
            .to_string();
        return ("repo".to_string(), Some(label));
    }
    (
        "worktree".to_string(),
        settings
            .worktrees
            .iter()
            .find(|w| w.id == target)
            .map(|w| w.name.clone()),
    )
}

/// 購読の `target` の解決結果（#126）。
struct ResolvedTarget {
    /// DB に保存する正規化済みの target 文字列
    stored: String,
    /// 人間 / エージェントへ返す表示名
    label: String,
    /// 厳密一致（単一ワークツリー）の場合のみ Some。ワイルドカードでは None
    worktree_id: Option<String>,
}

/// 購読の `target` を解決する。ワークツリー ID / 名前のほか、`*` / `workgroup:<id|名前>` /
/// `repo:<名前>` のワイルドカードを受ける（#126）。
///
/// **まだ存在しないワークツリーの `worktree.created` を購読したい**という要求は ID 固定の
/// target では表現できないため、ワイルドカードは `worktree.created` とセットで必要になる。
fn resolve_subscription_target(
    settings: &AppSettings,
    raw: &str,
) -> Result<ResolvedTarget, McpError> {
    let trimmed = raw.trim();
    if trimmed == crate::event_db::TARGET_ALL {
        return Ok(ResolvedTarget {
            stored: crate::event_db::TARGET_ALL.to_string(),
            label: "全ワークツリー".to_string(),
            worktree_id: None,
        });
    }

    if let Some(value) = trimmed.strip_prefix(crate::event_db::TARGET_WORKGROUP_PREFIX) {
        let value = value.trim();
        if value.is_empty() {
            return Err(McpError::invalid_params(
                "workgroup: の後にワークグループ ID または名前を指定してください",
                None,
            ));
        }
        // ID 優先、次に表示名。どちらでも解決できなければ利用可能な一覧を添えて返す
        // （エージェントが自己修復できるようにするのが既存 target 解決の流儀）。
        let gid = resolve_workgroup_target(settings, Some(value), None)
            .or_else(|_| resolve_workgroup_target(settings, None, Some(value)))
            .map_err(|e| McpError::invalid_params(e, None))?
            .ok_or_else(|| {
                McpError::invalid_params("ワークグループを解決できませんでした", None)
            })?;
        let label = settings
            .workgroups
            .iter()
            .find(|g| g.id == gid)
            .map(|g| workgroup_display_name(settings, g))
            .unwrap_or_else(|| gid.clone());
        return Ok(ResolvedTarget {
            stored: crate::event_db::normalize_target(&format!(
                "{}{}",
                crate::event_db::TARGET_WORKGROUP_PREFIX,
                gid
            )),
            label: format!("ワークグループ '{}'", label),
            worktree_id: None,
        });
    }

    if let Some(value) = trimmed.strip_prefix(crate::event_db::TARGET_REPO_PREFIX) {
        let value = value.trim();
        if value.is_empty() {
            return Err(McpError::invalid_params(
                "repo: の後にリポジトリ名を指定してください",
                None,
            ));
        }
        // 登録済みリポジトリ名と、既存ワークツリーが持つリポジトリ名の両方を突合する
        // （リポジトリ登録を消してもワークツリーだけ残っているケースがあるため）。
        // 大小の吸収は `normalize_target` と同じ Unicode の `to_lowercase()` で行う
        // （`eq_ignore_ascii_case` は非 ASCII を畳まないので、日本語混じりの名前を
        //   大小違いで打つと「見つかりません」になる）。
        let needle = value.to_lowercase();
        let matched = settings
            .repositories
            .iter()
            .map(|r| r.name.as_str())
            .chain(settings.worktrees.iter().map(|w| w.repository_name.as_str()))
            .find(|name| name.to_lowercase() == needle);
        let Some(name) = matched else {
            let available: Vec<&str> = settings.repositories.iter().map(|r| r.name.as_str()).collect();
            return Err(McpError::invalid_params(
                format!(
                    "リポジトリ '{}' が見つかりません。利用可能: [{}]",
                    value,
                    available.join(", ")
                ),
                None,
            ));
        };
        return Ok(ResolvedTarget {
            stored: crate::event_db::normalize_target(&format!(
                "{}{}",
                crate::event_db::TARGET_REPO_PREFIX,
                name
            )),
            label: format!("リポジトリ '{}'", name),
            worktree_id: None,
        });
    }

    let wt = resolve_worktree_by_name_or_id(settings, trimmed)?;
    // ホーム / リポジトリ擬似ワークツリーは削除自体が禁止されているので worktree.closed が
    // 構造的に永久に発火しない。**この制約は厳密一致 target のときだけ**で、`*` を弾く
    // 理由にはならない（`*` は他のワークツリーのイベントで成立する）。
    if wt.is_home || wt.is_repository {
        let kind = if wt.is_home { "ホーム" } else { "リポジトリ" };
        return Err(McpError::invalid_params(
            format!(
                "'{}' は{}擬似ワークツリーでクローズされることがないため購読できません",
                wt.name, kind
            ),
            None,
        ));
    }
    Ok(ResolvedTarget {
        stored: wt.id.clone(),
        label: format!("ワークツリー '{}'", wt.name),
        worktree_id: Some(wt.id.clone()),
    })
}

/// `notify_worktree` の `event_kind` 指定時に購読イベントを発行する（#126）。
///
/// **発信元は呼び出し元のワークツリー**であって、`notify_worktree` の `worktree_name`
/// （トーストの宛先）ではない。#120 は「送信側が宛先を知っている前提を置くと破綻する」
/// ため購読方式を採っており、宛先を決めるのは購読者側。
///
/// `depth` はエージェントに申告させず、そのタブが直近に受け取ったイベントから自動計算する。
/// 申告制にすると「返信時に depth を足す」という約束を破るだけで `MAX_EVENT_DEPTH` の
/// 暴走防止ガードを無効化できてしまう（A↔B の往復が止まらなくなる）。
///
/// 既知の限界（#126）: `depth` が止めるのは**連鎖**であって連打ではない。同じタブから
/// 立て続けに送れば他ワークツリーの inbox には積まれる。押し込み側は宛先ごとの
/// 最小間隔（30秒）と保持期限で有界なので、実害は「未読が増える」までに留まる。
async fn publish_worktree_message(
    app_handle: &AppHandle,
    event_kind: &str,
    body: Option<&str>,
    terminal_id: Option<&str>,
    project_dir: Option<&str>,
) -> Result<serde_json::Value, McpError> {
    if event_kind != crate::event_db::KIND_WORKTREE_MESSAGE {
        return Err(McpError::invalid_params(
            format!(
                "event_kind '{}' は notify_worktree からは発行できません。指定できるのは '{}' のみです（'{}' / '{}' は oretachi がワークツリーの追加・削除時に自動で発行します）",
                event_kind,
                crate::event_db::KIND_WORKTREE_MESSAGE,
                crate::event_db::KIND_WORKTREE_CREATED,
                crate::event_db::KIND_WORKTREE_CLOSED,
            ),
            None,
        ));
    }
    let text = body
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            McpError::invalid_params(
                "event_kind を指定する場合は body（購読者へ届けるメッセージ本文）が必要です",
                None,
            )
        })?
        .to_string();
    // 自由文は他ワークツリーのエージェントのコンテキストへそのまま注入される。
    // 上限が無いと相手のセッションを本文で埋められるので入口で弾く（切り詰めではなく
    // エラーにするのは、勝手に削って「送れた」と誤解させないため）。
    let text_len = text.chars().count();
    if text_len > crate::event_db::MESSAGE_TEXT_MAX_CHARS {
        return Err(McpError::invalid_params(
            format!(
                "body が長すぎます（{} 文字 / 上限 {} 文字）。要点を絞って送るか、詳細は共有ファイルや issue に置いて参照を送ってください",
                text_len,
                crate::event_db::MESSAGE_TEXT_MAX_CHARS
            ),
            None,
        ));
    }

    // await をまたいで State / settings の参照を持たないよう、ここで所有権のある値へ確定させる
    let (source_terminal_id, source_worktree_id, source_worktree_name, repository_name, workgroup_id) = {
        let subscriber = resolve_subscriber(app_handle, terminal_id, project_dir)?;
        let Some(worktree_id) = subscriber.worktree_id.clone() else {
            return Err(McpError::invalid_params(
                "呼び出し元ターミナルの作業ディレクトリから oretachi 管理下のワークツリーを特定できませんでした。oretachi が管理しているワークツリー内で実行してください",
                None,
            ));
        };
        let settings = app_handle.state::<SettingsManager>().get();
        let wt = settings.worktrees.iter().find(|w| w.id == worktree_id);
        let group = wt
            .and_then(|w| resolve_workgroup(&settings, w))
            .map(|g| g.id.clone());
        (
            subscriber.terminal_id,
            worktree_id,
            wt.map(|w| w.name.clone()),
            wt.map(|w| w.repository_name.clone()),
            group,
        )
    };

    let pool = event_pool(app_handle)?;
    let now = crate::event_db::now_ms();
    // 受信した最大 depth + 1。受信が無ければ 0（連鎖の起点）。
    let depth = crate::event_db::max_inbound_depth(
        &pool,
        &source_terminal_id,
        now,
        crate::event_db::CHAIN_WINDOW_MS,
    )
    .await
    .map_err(|e| McpError::internal_error(e, None))?
    .map_or(0, |d| d + 1);

    let body_json = serde_json::json!({
        "text": text,
        "sourceWorktreeName": source_worktree_name,
    })
    .to_string();
    let event = crate::event_db::EventRow {
        id: uuid::Uuid::new_v4().to_string(),
        source_worktree_id: source_worktree_id.clone(),
        // 自己エコー抑止（同じタブへ配り返さない）と depth 伝播の起点になる
        source_terminal_id: Some(source_terminal_id.clone()),
        kind: crate::event_db::KIND_WORKTREE_MESSAGE.to_string(),
        body: body_json,
        actor: Some("mcp".to_string()),
        created_at: now,
        depth,
        origin: Some(format!("mcp-notify:{}", source_terminal_id)),
    };
    // 閾値超過でも events には残す（監査とループ解析用）。配送は `fanout` が落とす。
    crate::event_db::insert_event(&pool, &event)
        .await
        .map_err(|e| McpError::internal_error(e, None))?;

    let targets = crate::event_db::matching_targets(
        &source_worktree_id,
        workgroup_id.as_deref(),
        repository_name.as_deref(),
    );
    let delivered = crate::event_db::fanout(&pool, &event, &targets, now)
        .await
        .map_err(|e| McpError::internal_error(e, None))?;
    if delivered > 0 {
        crate::event_delivery::notify_event_queued(app_handle);
    }
    log::info!(
        "[mcp] notify_worktree event_kind={} source={} terminal={} depth={} targets={:?} delivered={}",
        event_kind,
        source_worktree_id,
        source_terminal_id,
        depth,
        targets,
        delivered
    );

    // 閾値超過は静かに捨てず呼び出し元へ返す。返さないとエージェントは「送れた」と
    // 誤解したまま相手の応答を待ち続ける。
    let message = if depth > crate::event_db::MAX_EVENT_DEPTH {
        format!(
            "連鎖の深さが {} に達したためメッセージは配送されませんでした（上限 {}）。ワークツリー間の自動往復を止めるための制限です。続ける場合は人間に確認してください。",
            depth,
            crate::event_db::MAX_EVENT_DEPTH
        )
    } else if delivered == 0 {
        "メッセージを発行しましたが、このワークツリーを購読しているセッションがありませんでした。".to_string()
    } else {
        format!("{} 件の購読者へメッセージを配送しました。", delivered)
    };
    Ok(serde_json::json!({
        "eventId": event.id,
        "eventKind": event.kind,
        "sourceWorktreeId": source_worktree_id,
        "sourceTerminalId": source_terminal_id,
        "depth": depth,
        "maxDepth": crate::event_db::MAX_EVENT_DEPTH,
        "delivered": delivered,
        "message": message,
    }))
}

/// 購読系ツールを呼んでいるタブの同定結果（issue #123）。
struct SubscriberIdentity {
    /// 購読の主キー。PTY spawn 時に発番された UUID
    terminal_id: String,
    /// タブの cwd からの逆引き。管理外ディレクトリなら None
    worktree_id: Option<String>,
    /// 最後に見た Claude Code の session UUID（監査用）
    agent_session: Option<String>,
}

/// 購読系ツールの呼び出し元タブを同定する。
///
/// MCP ツール呼び出しからは呼び出し元セッションを特定できず、`project_dir` (cwd) は
/// 同一ワークツリーの全タブで同じなので cwd では絞れない。そのため
/// `ORETACHI_TERMINAL_ID`（SessionStart の additionalContext で本人に伝えている）を
/// 渡してもらうのが本筋で、省略時のみ「そのワークツリーで走行中の AI エージェント端末が
/// 1つだけ」という条件下で推測する。
fn resolve_subscriber(
    app_handle: &AppHandle,
    terminal_id: Option<&str>,
    project_dir: Option<&str>,
) -> Result<SubscriberIdentity, McpError> {
    let pty = app_handle.state::<crate::pty_manager::PtyManager>();
    let sessions = pty.list_sessions();
    let settings = app_handle.state::<SettingsManager>().get();

    let resolve = |info: &crate::pty_manager::SessionInfo| SubscriberIdentity {
        terminal_id: info.terminal_id.clone(),
        worktree_id: info
            .cwd
            .as_deref()
            .and_then(|c| resolve_worktree_by_cwd(&settings, c))
            .map(|w| w.id.clone()),
        agent_session: info.agent_session_id.clone(),
    };

    // 注意: ここで検証できるのは「その terminal_id が実在するか」だけで、呼び出し元本人か
    // どうかは分からない（MCP ツール呼び出しから発信セッションを特定する手段が無い）。
    // つまり terminal_id を差し替えれば他タブの inbox を読み・ack し、購読を解除できる。
    // ローカルの信頼境界内なので許容しているが、`event_db` 側の
    // 「自分のタブの購読しか消せない」は SQL のスコープ制約であって本人性の保証ではない。
    if let Some(id) = terminal_id.map(str::trim).filter(|s| !s.is_empty()) {
        // 購読系ツールを叩けている＝このタブでエージェントが走っている。ついでに
        // 引き継ぎ待ちを引き取らせる（非ブロッキング。SessionStart を取り逃した場合の保険）。
        //
        // **引き継いだ結果を読むツールはこれだけでは足りない**（#137）。ここは応答を
        // 待たないので、直後に `subscriber_terminal_id` で SELECT すると行がまだ死んだ
        // タブの ID を向いていて 0 件に見える。そういうツールは `resolve_subscriber` の
        // あとで `rebind_and_wait` を await すること（`await_rebind` ヘルパ）。
        crate::event_delivery::request_rebind(app_handle, id.to_string());
        return sessions
            .iter()
            .find(|s| s.terminal_id == id)
            .map(resolve)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "terminal_id '{}' に一致するターミナルがありません。セッション開始時に oretachi から注入された terminal_id をそのまま渡すか、oretachi_list_terminals で確認してください",
                        id
                    ),
                    None,
                )
            });
    }

    // terminal_id 省略時のフォールバック: project_dir のワークツリーで走行中の AI 端末が
    // 1つだけならそれを採用する。複数タブがある場合は誤配送に直結するので推測しない。
    let Some(dir) = project_dir.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(McpError::invalid_params(
            "terminal_id または project_dir を指定してください。terminal_id はセッション開始時に oretachi から注入されています",
            None,
        ));
    };
    let Some(wt) = resolve_worktree_by_cwd(&settings, dir) else {
        return Err(McpError::invalid_params(
            format!(
                "project_dir '{}' に一致するワークツリーがありません。available: [{}]",
                dir,
                available_worktree_names(&settings)
            ),
            None,
        ));
    };
    let candidates: Vec<&crate::pty_manager::SessionInfo> = sessions
        .iter()
        .filter(|s| s.exit_code.is_none() && s.is_ai_agent)
        .filter(|s| {
            s.cwd
                .as_deref()
                .and_then(|c| resolve_worktree_by_cwd(&settings, c))
                .map(|w| w.id == wt.id)
                .unwrap_or(false)
        })
        .collect();
    match candidates.len() {
        1 => Ok(resolve(candidates[0])),
        0 => Err(McpError::invalid_params(
            format!(
                "ワークツリー '{}' に走行中の AI エージェント端末が見つかりません。terminal_id を明示してください（セッション開始時に oretachi から注入されています）",
                wt.name
            ),
            None,
        )),
        n => Err(McpError::invalid_params(
            format!(
                "ワークツリー '{}' に AI エージェント端末が {} 個あるため呼び出し元を特定できません。terminal_id を明示してください（セッション開始時に oretachi から注入されています）",
                wt.name, n
            ),
            None,
        )),
    }
}

/// 引き継ぎ待ちの回収を待ってから DB を読むための一手（#137）。
///
/// 購読 / inbox の行は `subscriber_terminal_id` で引くので、タブを立て直した直後は
/// **引き継ぎが終わるまで自分の購読も未読も 0 件に見える**。`collect_digest` が
/// `list_inbox` の前に `try_rebind_once` を await しているのと同じ手当てを、MCP
/// ツール側にも入れる。`resolve_subscriber` は同期関数なので分けている。
async fn await_rebind(app_handle: &AppHandle, subscriber: &SubscriberIdentity) {
    crate::event_delivery::rebind_and_wait(app_handle, &subscriber.terminal_id).await;
}

/// 1つの文字列をワークツリー ID としても名前としても解決する（購読の `target` 用）。
/// エージェントに「ID か名前か」を意識させないため両方試す。ID を優先する。
fn resolve_worktree_by_name_or_id<'a>(
    settings: &'a AppSettings,
    value: &str,
) -> Result<&'a WorktreeEntry, McpError> {
    if let Some(wt) = settings.worktrees.iter().find(|w| w.id == value) {
        return Ok(wt);
    }
    resolve_worktree(settings, None, Some(value), None, "specify a worktree name or id")
}

/// event_db のプールを取り出す。未初期化なら AI に伝わるエラーにする。
fn event_pool(app_handle: &AppHandle) -> Result<sqlx::SqlitePool, McpError> {
    app_handle
        .try_state::<crate::event_db::EventPool>()
        .map(|p| p.0.clone())
        .ok_or_else(|| {
            McpError::internal_error(
                "イベント DB が初期化されていないため購読機能を利用できません（oretachi のログを確認してください）",
                None,
            )
        })
}

/// artifact 系ツール（artifact / artifact_module / search_artifact）の保存先ワークツリーを解決する。
/// 優先順位: worktree_id > project_dir 逆引き > repository + branch。
/// HOME / リポジトリ擬似ワークツリーは repository_name / branch_name が空なので
/// repository+branch では当たらない。project_dir(= エージェントの cwd) で解決させる。
fn resolve_artifact_worktree<'a>(
    settings: &'a AppSettings,
    worktree_id: Option<&str>,
    project_dir: Option<&str>,
    repository: Option<&str>,
    branch: Option<&str>,
) -> Result<&'a WorktreeEntry, McpError> {
    if let Some(id) = worktree_id.map(str::trim).filter(|s| !s.is_empty()) {
        return settings.worktrees.iter().find(|w| w.id == id).ok_or_else(|| {
            McpError::invalid_params(format!("worktree id '{}' が存在しません", id), None)
        });
    }

    if let Some(dir) = project_dir.map(str::trim).filter(|s| !s.is_empty()) {
        return resolve_worktree_by_dir(settings, dir).ok_or_else(|| {
            McpError::invalid_params(
                format!(
                    "project_dir '{}' に一致するワークツリーが存在しません。available: [{}]",
                    dir,
                    available_worktree_names(settings)
                ),
                None,
            )
        });
    }

    let repository = repository.map(str::trim).filter(|s| !s.is_empty());
    let branch = branch.map(str::trim).filter(|s| !s.is_empty());
    match (repository, branch) {
        (Some(repository), Some(branch)) => settings
            .worktrees
            .iter()
            .find(|wt| wt.repository_name == repository && wt.branch_name == branch)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "repository='{}', branch='{}' に一致するワークツリーが存在しません。HOMEタブやリポジトリルートで作業している場合は project_dir を指定してください",
                        repository, branch
                    ),
                    None,
                )
            }),
        (None, None) => Err(McpError::invalid_params(
            format!(
                "project_dir / worktree_id / repository+branch のいずれかを指定してください。available: [{}]",
                available_worktree_names(settings)
            ),
            None,
        )),
        _ => Err(McpError::invalid_params(
            "repository と branch は両方セットで指定してください（片方のみでは解決できません）。作業ディレクトリが分かる場合は project_dir を指定してください",
            None,
        )),
    }
}

/// ワークツリーの所属ワークグループを解決する。workgroup_id が未設定/不明な場合は
/// 先頭グループにフォールバック（フロントの resolvedGroupId と同仕様）。
fn resolve_workgroup<'a>(settings: &'a AppSettings, worktree: &WorktreeEntry) -> Option<&'a Workgroup> {
    resolve_workgroup_by_id(settings, worktree.workgroup_id.as_deref())
}

/// `resolve_workgroup` の ID 版。ワークツリーの実体が既に settings から消えた後でも
/// 所属グループを解決したい経路（`worktree.closed` の target 照合）で使う（#126）。
pub fn resolve_workgroup_by_id<'a>(
    settings: &'a AppSettings,
    workgroup_id: Option<&str>,
) -> Option<&'a Workgroup> {
    workgroup_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|id| settings.workgroups.iter().find(|g| g.id == id))
        .or_else(|| settings.workgroups.first())
}

/// ワークグループの表示名。UI と同じ規則で、name 未設定なら並び順から自動生成する
/// （フロントの useWorkgroups.displayName / i18n `workgroup.autoName` と一致させる）。
/// 生の name をそのまま返すと、リネームしていない既定グループが全部 null になり
/// レポート側で「グループが出てこない」状態になるため、ここで解決しておく。
pub fn workgroup_display_name(settings: &AppSettings, group: &Workgroup) -> String {
    if let Some(name) = group.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    let n = settings
        .workgroups
        .iter()
        .position(|g| g.id == group.id)
        .map_or(1, |i| i + 1);
    match settings.locale.as_deref() {
        Some("en") => format!("Group ({})", n),
        _ => format!("グループ({})", n),
    }
}

/// MCP から指定された workgroup_id / workgroup_name を settings 上のワークグループIDに解決する。
/// どちらも未指定なら `Ok(None)` を返し、追加先はフロントに委ねる
/// （UI で現在選択中の WG = useWorkgroups.activeWorkgroupId に入る）。
/// 解決できない・曖昧な場合は先頭 WG へ暗黙にフォールバックせずエラーにする
/// （意図しないワークグループへタスクが入るのを防ぐため）。
fn resolve_workgroup_target(
    settings: &AppSettings,
    workgroup_id: Option<&str>,
    workgroup_name: Option<&str>,
) -> Result<Option<String>, String> {
    fn non_empty(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|v| !v.is_empty())
    }
    let id = non_empty(workgroup_id);
    let name = non_empty(workgroup_name);

    // エージェントが自己修復できるよう、エラー時は利用可能な WG を列挙する。
    let available = || {
        if settings.workgroups.is_empty() {
            return "(ワークグループが未定義です)".to_string();
        }
        settings
            .workgroups
            .iter()
            .map(|g| format!("{} ({})", workgroup_display_name(settings, g), g.id))
            .collect::<Vec<_>>()
            .join(", ")
    };

    if let Some(id) = id {
        return match settings.workgroups.iter().find(|g| g.id == id) {
            Some(g) => Ok(Some(g.id.clone())),
            None => Err(format!(
                "workgroup_id '{}' に一致するワークグループがありません。利用可能: {}",
                id,
                available()
            )),
        };
    }

    let Some(name) = name else {
        return Ok(None);
    };
    let matched: Vec<&Workgroup> = settings
        .workgroups
        .iter()
        .filter(|g| workgroup_display_name(settings, g).trim().eq_ignore_ascii_case(name))
        .collect();
    match matched.as_slice() {
        [g] => Ok(Some(g.id.clone())),
        [] => Err(format!(
            "workgroup_name '{}' に一致するワークグループがありません。利用可能: {}",
            name,
            available()
        )),
        _ => Err(format!(
            "workgroup_name '{}' に一致するワークグループが複数あります。workgroup_id で指定してください。利用可能: {}",
            name,
            available()
        )),
    }
}

/// リポジトリに通知フックが1件以上設定されているか。
fn repo_has_notification_hooks(settings: &AppSettings, worktree: &WorktreeEntry) -> bool {
    settings
        .repositories
        .iter()
        .find(|r| r.id == worktree.repository_id)
        .and_then(|r| r.notification_hooks.as_ref())
        .map_or(false, |h| !h.is_empty())
}

/// イベント名の既定 kind。ユーザー設定 (repo.notification_hooks) が無い場合のフォールバック。
fn default_kind_for_event(event: &str) -> &'static str {
    match event {
        "Stop" => "completed",
        "PermissionRequest" => "approval",
        _ => "hook",
    }
}

/// ワークツリーの所属リポジトリの notification_hooks から event に対応する kind を解決する。
/// 設定が無ければ default_kind_for_event にフォールバック。
fn resolve_kind_for_event(settings: &AppSettings, worktree: &WorktreeEntry, event: &str) -> String {
    settings
        .repositories
        .iter()
        .find(|r| r.id == worktree.repository_id)
        .and_then(|r| r.notification_hooks.as_ref())
        .and_then(|hooks| hooks.iter().find(|h| h.event == event))
        .map(|h| h.kind.clone())
        .unwrap_or_else(|| default_kind_for_event(event).to_string())
}

/// hook body の JSON にサブエージェント（Task tool）内部発火の目印 `agent_id` があるか判定する（#141）。
/// メインエージェント発火の hook JSON には存在しない。パースできない body は
/// 「サブエージェントではない」に倒す（疑わしきは通知する＝安全側）。
fn hook_body_has_agent_id(body: Option<&str>) -> bool {
    body.and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())
        .map_or(false, |v| v.get("agent_id").is_some())
}

/// サブエージェント（Task tool）内部発火由来の通知を抑制すべきか判定する（#141）。
/// - kind 明示指定なし（旧形式/MCP 経由の意図的な通知は対象外）
/// - agent == "cc"（サイドカーが --agent cc で送る。claude_plugin.rs 参照）
/// - hook body に agent_id がある（サブエージェント内部発火）
/// の3条件がすべて揃った場合のみ true。
fn should_skip_subagent_notify(agent: Option<&str>, kind: Option<&str>, body: Option<&str>) -> bool {
    kind.is_none() && agent == Some("cc") && hook_body_has_agent_id(body)
}

// ─── Simple REST endpoint (/notify) ──────────────────────────────────────────

async fn notify_handler(
    State(app_handle): State<AppHandle>,
    Json(payload): Json<NotifyPayload>,
) -> StatusCode {
    let settings = app_handle.state::<SettingsManager>().get();

    // ワークツリー: project_dir から逆引き。見つからなければ後方互換の worktree 名で解決。
    let worktree = payload
        .project_dir
        .as_deref()
        .and_then(|d| resolve_worktree_by_dir(&settings, d));
    let worktree_name = match worktree {
        Some(w) => w.name.clone(),
        None => match payload.worktree.clone() {
            Some(name) => name,
            None => {
                log::warn!(
                    "[notify] could not resolve worktree (projectDir={:?}); dropping",
                    payload.project_dir
                );
                return StatusCode::OK;
            }
        },
    };

    // URL アーティファクトの自動登録。通知フックの設定有無とは独立に動かしたいので、
    // 通知フック未設定リポジトリの早期 return より前に処理する。
    // ツール呼び出しごとに走るため "http" を含まない body は即スキップし、
    // 実処理は spawn へ逃がして通知パス（サイドカーの read timeout 500ms）を塞がない。
    if let (Some(w), Some(ev), Some(body)) = (worktree, payload.event.as_deref(), payload.body.as_deref())
    {
        if matches!(ev, "PreToolUse" | "PostToolUse") && body.contains("http") {
            let hook_wt = crate::artifact_url::HookWorktree {
                id: w.id.clone(),
                repository_name: w.repository_name.clone(),
                branch_name: w.branch_name.clone(),
                path: w.path.clone(),
            };
            let handle = app_handle.clone();
            let event_name = ev.to_string();
            let body_owned = body.to_string();
            tokio::spawn(async move {
                crate::artifact_url::handle_tool_hook(handle, hook_wt, event_name, body_owned).await;
            });
        }
    }

    // サブエージェント（Task tool）内部発火の通知を抑制する（#141）。hook JSON の agent_id を
    // 見て判定し、kind 明示指定や artifact URL 自動登録（上のブロック）には影響させない。
    if should_skip_subagent_notify(payload.agent.as_deref(), payload.kind.as_deref(), payload.body.as_deref()) {
        log::debug!(
            "[notify] skip subagent-internal notify: worktree={} event={:?}",
            worktree_name,
            payload.event
        );
        return StatusCode::OK;
    }

    // トレイ通知の可否（#153）。フック由来（event 指定・kind 明示なし）かつ
    // `resolve_tray_notification == false` のときだけ `tray: false` を載せる。
    // **イベント自体は drop しない** —— `useAppAutoApproval.ts` / `SubWindowApp.vue` の
    // 自動承認が `notify-worktree` をトリガにしているため、ここで落とすと
    // トレイ通知をオフにしたワークツリーで自動承認が止まる。
    let tray = if payload.kind.is_none() && payload.event.is_some() {
        match worktree {
            Some(w) => resolve_tray_notification(&settings, w),
            None => true,
        }
    } else {
        true
    };

    // ライフサイクルフック由来（event 指定・kind 明示なし）の通知は、通知フックが1件も
    // 設定されていないリポジトリでは破棄する。プラグインは全ワークツリーで無条件有効化される
    // （SessionStart 注入用）ため、未設定リポジトリの通知挙動を従来（プラグイン無効=通知なし）
    // と一致させる。kind 明示指定（旧形式/MCP 経由）は意図的な通知なので対象外。
    if payload.kind.is_none() && payload.event.is_some() {
        if let Some(w) = worktree {
            if !repo_has_notification_hooks(&settings, w) {
                return StatusCode::OK;
            }
        }
    }

    // kind: 明示指定(旧形式/MCP) > event からの解決 > "general"
    let kind = if let Some(k) = payload.kind.clone() {
        k
    } else if let Some(ev) = payload.event.as_deref() {
        match worktree {
            Some(w) => resolve_kind_for_event(&settings, w, ev),
            None => default_kind_for_event(ev).to_string(),
        }
    } else {
        "general".to_string()
    };

    let event = NotifyWorktreeEvent {
        worktree_name,
        kind,
        body: payload.body,
        agent: payload.agent,
        tray,
    };
    // terminal_id は現状ログのみ（発火元タブの同定に使う）。購読機構 (#123 以降) で消費する。
    log::info!(
        "[notify] worktree={} kind={} tray={} terminal={:?}",
        event.worktree_name,
        event.kind,
        event.tray,
        payload.terminal_id
    );

    let manager = app_handle.state::<McpServerManager>();
    // hook: 3秒 / approval: 1秒 で (worktree, kind) 単位に送信制限。
    // それ以外の kind (general/completed/任意) は debounce 対象外で常に通す。
    let should_send = should_send_notify(&manager.notify_last_sent, &event.worktree_name, &event.kind);
    if !should_send {
        return StatusCode::OK;
    }

    if event.kind == "hook" {
        // hook 通知は WebView IPC を経由せず broadcast channel で直接 MCP ピアへ配信。
        // app_handle.emit() は WebView UIスレッドを経由するため、高頻度の hook 通知では
        // UIスレッドへの負荷が累積しフリーズの原因になる。
        let _ = manager.hook_tx.send(event);
        StatusCode::OK
    } else {
        match app_handle.emit("notify-worktree", &event) {
            Ok(_) => StatusCode::OK,
            Err(e) => {
                // webview の状態情報を詳細ログに記録してハング診断に役立てる
                let window_info: Vec<String> = app_handle
                    .webview_windows()
                    .iter()
                    .map(|(label, w)| {
                        format!(
                            "{}(visible={:?} focused={:?})",
                            label,
                            w.is_visible().unwrap_or(false),
                            w.is_focused().unwrap_or(false)
                        )
                    })
                    .collect();
                log::error!(
                    "[emit-failed] event=notify-worktree error={} windows=[{}]",
                    e,
                    window_info.join(", ")
                );
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

// ─── Simple REST endpoint (/set-description) ─────────────────────────────────

/// ExitPlanMode フックの hook JSON からプラン本文を抽出する。
/// tool_response.plan → tool_input.plan の順で探し、無ければ filePath を読み込む。
fn extract_plan_from_hook_json(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;

    // インラインのプラン本文を優先
    for path in [["tool_response", "plan"], ["tool_input", "plan"]] {
        if let Some(s) = v
            .get(path[0])
            .and_then(|o| o.get(path[1]))
            .and_then(|p| p.as_str())
        {
            if !s.trim().is_empty() {
                return Some(s.to_string());
            }
        }
    }

    // フォールバック: filePath からプランファイルを読み込む
    for path in [["tool_response", "filePath"], ["tool_input", "filePath"]] {
        if let Some(fp) = v
            .get(path[0])
            .and_then(|o| o.get(path[1]))
            .and_then(|p| p.as_str())
        {
            if let Ok(content) = std::fs::read_to_string(fp) {
                if !content.trim().is_empty() {
                    return Some(content);
                }
            }
        }
    }
    None
}

async fn set_description_handler(
    State(app_handle): State<AppHandle>,
    Json(payload): Json<SetDescriptionPayload>,
) -> StatusCode {
    let settings = app_handle.state::<SettingsManager>().get();

    // ワークツリー: project_dir から逆引き。無ければ後方互換の worktree 名。
    let worktree_name = payload
        .project_dir
        .as_deref()
        .and_then(|d| resolve_worktree_by_dir(&settings, d))
        .map(|w| w.name.clone())
        .or_else(|| payload.worktree.clone());
    let Some(worktree_name) = worktree_name else {
        log::warn!(
            "[set-description] could not resolve worktree (projectDir={:?}); skipping",
            payload.project_dir
        );
        return StatusCode::OK;
    };

    let plan = payload
        .hook_json
        .as_deref()
        .and_then(extract_plan_from_hook_json);

    let Some(plan) = plan else {
        // プランが取れない場合はベストエフォートで握りつぶす（フックをブロックしない）
        log::info!("[set-description] worktree={} no plan extracted; skipping", worktree_name);
        return StatusCode::OK;
    };

    log::info!("[set-description] worktree={} plan_len={}", worktree_name, plan.len());

    let event = SetWorktreeDescriptionEvent {
        worktree: worktree_name,
        plan: Some(plan),
        description: None,
    };
    match app_handle.emit("set-worktree-description", &event) {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            log::error!("[emit-failed] event=set-worktree-description error={}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// ─── Simple REST endpoint (/session-context) ─────────────────────────────────

/// SessionStart の inbox 取得に許す時間。サイドカーの読み取りタイムアウト（2 秒）より
/// 十分に短くして、DB が詰まっても systemPrompt の注入まで巻き込まないようにする。
const INBOX_DIGEST_BUDGET_MS: u64 = 1200;

/// SessionStart フックから呼ばれ、ワークツリー所属グループの systemPrompt を返す。
/// 解決を毎回ここで行うため、グループ設定の変更は次のセッション開始から自動反映される。
/// 未解決（管理外ディレクトリ・プロンプト未設定）は prompt: null を返し、注入は行われない。
async fn session_context_handler(
    State(app_handle): State<AppHandle>,
    Json(payload): Json<SessionContextPayload>,
) -> Json<serde_json::Value> {
    let settings = app_handle.state::<SettingsManager>().get();
    let prompt = payload
        .project_dir
        .as_deref()
        .and_then(|d| resolve_worktree_by_dir(&settings, d))
        .and_then(|w| {
            if w.is_home {
                // ホームで起動したセッションは常にワークツリー管理エージェントとして振る舞わせる。
                // 通常の開発向けであろうグループの systemPrompt は用途が違うので使わない。
                Some(crate::home_skills::resolve_home_agent_prompt(&settings))
            } else if w.is_repository {
                // リポジトリ root は擬似ワークツリー導入前は resolve_worktree_by_dir が解決できず
                // None が返っていた場所。開発ワークツリー向けのグループ systemPrompt は用途が違うので
                // 注入しない（将来 repositoryAgentPrompt を足すならここを分岐させる）。
                None
            } else {
                resolve_workgroup(&settings, w).and_then(|g| g.system_prompt.clone())
            }
        })
        .filter(|s| !s.trim().is_empty());
    // terminal_id はサイドカーが env から拾って送ってくる。自己 ID そのものの
    // additionalContext 注入はサイドカー側で行うが、購読メッセージの回収はここで行う。
    // アプリ停止中に溜まった分もセッション開始のこのタイミングで回収できる（issue #123）。
    // サイドカーの読み取りタイムアウトは 2 秒。それを超えるとレスポンス自体が捨てられ
    // グループ systemPrompt の注入まで失う（`notify/src/main.rs` の Err → default 経路）。
    // DB が詰まっている場合は inbox を諦めて prompt だけでも返す。諦めた分は打刻していない
    // ので次のセッション開始で再度提示される。
    let inbox = match tokio::time::timeout(
        std::time::Duration::from_millis(INBOX_DIGEST_BUDGET_MS),
        collect_inbox_digest(
            &app_handle,
            payload.terminal_id.as_deref(),
            crate::event_delivery::DigestReason::SessionStart,
        ),
    )
    .await
    {
        Ok(inbox) => inbox,
        Err(_) => {
            log::warn!(
                "[session-context] inbox の取得が {}ms を超えたため今回は注入を見送る (terminal={:?})",
                INBOX_DIGEST_BUDGET_MS,
                payload.terminal_id
            );
            None
        }
    };
    log::info!(
        "[session-context] projectDir={:?} terminal={:?} prompt_len={:?} inbox_len={:?}",
        payload.project_dir,
        payload.terminal_id,
        prompt.as_ref().map(|p| p.len()),
        inbox.as_ref().map(|s| s.len())
    );
    Json(serde_json::json!({ "prompt": prompt, "inbox": inbox }))
}

/// 指定タブ宛の inbox を hook 注入用テキストにまとめ、本文を出した分に `delivered_at` を打つ。
///
/// terminal_id が無い（oretachi 管理外のターミナルから起動されたエージェント）場合や
/// イベント DB が未初期化の場合は None を返し、既存の挙動を一切変えない。
///
/// 実体は `event_delivery` の**単一ワーカー**が持つ（#124）。押し込みと同じキューを通すことで
/// 「未配送を SELECT → 注入 → 打刻」の隙間に押し込みが割り込んで二重配送になる経路を潰す。
/// ワーカーが詰まっていれば None が返り「今回は注入しない」に劣化するだけで、上位の
/// `INBOX_DIGEST_BUDGET_MS` タイムアウトを食い潰さない。
async fn collect_inbox_digest(
    app_handle: &AppHandle,
    terminal_id: Option<&str>,
    reason: crate::event_delivery::DigestReason,
) -> Option<String> {
    let terminal_id = terminal_id.map(str::trim).filter(|s| !s.is_empty())?;
    // DB 未初期化なら `DeliveryHandle` も manage されていないので、ここで弾かなくても
    // no-op になる。早期 return は無駄なチャネル往復を避けるためだけのもの。
    app_handle.try_state::<crate::event_db::EventPool>()?;
    crate::event_delivery::collect_digest_and_wait(app_handle, terminal_id, reason).await
}

// ─── Simple REST endpoint (/turn-context) ────────────────────────────────────

/// Stop フックの stdin JSON から `(prompt_id, stop_hook_active)` を読む（#124）。
///
/// Phase 0 (#121) の実測で CC 2.1.227 の Stop payload には両方が実在することが確認済み。
/// `stop_hook_active` は初回発火が `false`、`additionalContext` による継続後の発火はすべて
/// `true` になる。パースできない場合は「継続ターンではない」に倒す（配送側は
/// `prompt_id` 単位の上限と `delivered_at` でも守られているため、ここで止める必要はない）。
fn parse_stop_hook_fields(hook_json: Option<&str>) -> (Option<String>, bool) {
    let Some(v) = hook_json.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()) else {
        return (None, false);
    };
    let prompt_id = v
        .get("prompt_id")
        .and_then(|p| p.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let stop_hook_active = v
        .get("stop_hook_active")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    (prompt_id, stop_hook_active)
}

/// Stop フック (--turn-context) 用。そのタブ宛の未読を `additionalContext` 本文として返す。
///
/// **`Stop` の `additionalContext` は会話を継続させる**ので、無条件に返すと未読が残る限り
/// 永久に回る。防波堤は3枚（#120 §5.1・Phase 0 の訂正コメント）:
///
/// 1. `stop_hook_active == true`（= この Stop 自体が hook 由来の継続ターンの終わり）なら返さない
/// 2. `prompt_id` 単位で1ターン1回（`event_delivery` のワーカーが持つ）
/// 3. `delivered_at` による二重注入防止（`inbox` の UNIQUE 制約と `mark_delivered`）
///
/// CC 側にも継続9回で自然停止する上限があるが、**打ち切りが呼び出し側から判別できない**
/// （警告が出ず headless の結果 JSON も `is_error: false`）ため依存しない。
async fn turn_context_handler(
    State(app_handle): State<AppHandle>,
    Json(payload): Json<TurnContextPayload>,
) -> Json<serde_json::Value> {
    let (prompt_id, stop_hook_active) = parse_stop_hook_fields(payload.hook_json.as_deref());
    if stop_hook_active {
        log::debug!(
            "[turn-context] stop_hook_active=true のため配送しない terminal={:?} prompt_id={:?}",
            payload.terminal_id,
            prompt_id
        );
        return Json(serde_json::json!({ "inbox": serde_json::Value::Null }));
    }
    // サイドカーの読み取りタイムアウトは 2 秒。それを超えるとレスポンス自体が捨てられる。
    // DB が詰まっている場合は諦める。諦めた分は打刻していないので次の機会に再度出る。
    let inbox = match tokio::time::timeout(
        std::time::Duration::from_millis(INBOX_DIGEST_BUDGET_MS),
        collect_inbox_digest(
            &app_handle,
            payload.terminal_id.as_deref(),
            crate::event_delivery::DigestReason::TurnEnd {
                prompt_id: prompt_id.clone(),
            },
        ),
    )
    .await
    {
        Ok(inbox) => inbox,
        Err(_) => {
            log::warn!(
                "[turn-context] inbox の取得が {}ms を超えたため今回は注入を見送る (terminal={:?})",
                INBOX_DIGEST_BUDGET_MS,
                payload.terminal_id
            );
            None
        }
    };
    match &inbox {
        Some(_) => log::info!(
            "[turn-context] ターン境界で未読を注入する terminal={:?} prompt_id={:?}",
            payload.terminal_id,
            prompt_id
        ),
        // 注入しなかったことも残す。Stop はターンごとに必ず来るので、ここが無いと
        // 「hook が届いていない」のか「未読が無かった」のかログから切り分けられない。
        None => log::debug!(
            "[turn-context] 注入なし terminal={:?} prompt_id={:?}",
            payload.terminal_id,
            prompt_id
        ),
    }
    Json(serde_json::json!({ "inbox": inbox }))
}

// ─── Simple REST endpoint (/prompt-context) ──────────────────────────────────

/// UserPromptSubmit フック (--prompt-context) 用。現在の description と未読を返す。
///
/// description はワークツリー単位でスロットルし、期間内の再送・未解決時は `skip: true` を
/// 返す（サイドカーは description を出力しない）。
///
/// **イベント配送はこのスロットルの対象外**（#120 §5.3）。スロットルは description の再注入
/// 頻度を抑えるためのもので、600 秒に1回しか未読を渡せないのでは Stop の取りこぼし回収に
/// ならない。よって `skip` は description 側だけを支配し、`inbox` は毎回計算する。
const PROMPT_CONTEXT_THROTTLE_SECS: u64 = 600;

async fn prompt_context_handler(
    State(app_handle): State<AppHandle>,
    Json(payload): Json<PromptContextPayload>,
) -> Json<serde_json::Value> {
    // 未読の回収はワークツリー解決にもスロットルにも依存させない。鍵は terminal_id だけ。
    // ここで諦めても打刻していないので次のプロンプト送信で再度出る。
    let inbox = match tokio::time::timeout(
        std::time::Duration::from_millis(INBOX_DIGEST_BUDGET_MS),
        collect_inbox_digest(
            &app_handle,
            payload.terminal_id.as_deref(),
            crate::event_delivery::DigestReason::PromptSubmit,
        ),
    )
    .await
    {
        Ok(inbox) => inbox,
        Err(_) => {
            log::warn!(
                "[prompt-context] inbox の取得が {}ms を超えたため今回は注入を見送る (terminal={:?})",
                INBOX_DIGEST_BUDGET_MS,
                payload.terminal_id
            );
            None
        }
    };

    let settings = app_handle.state::<SettingsManager>().get();
    let Some(wt) = payload
        .project_dir
        .as_deref()
        .and_then(|d| resolve_worktree_by_dir(&settings, d))
    else {
        log::debug!(
            "[prompt-context] could not resolve worktree (projectDir={:?}); skipping description",
            payload.project_dir
        );
        return Json(serde_json::json!({ "skip": true, "inbox": inbox }));
    };

    let manager = app_handle.state::<McpServerManager>();
    let now = std::time::Instant::now();
    {
        let mut map = manager
            .prompt_context_last_sent
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = map.get(&wt.id) {
            if now.duration_since(*prev).as_secs() < PROMPT_CONTEXT_THROTTLE_SECS {
                return Json(serde_json::json!({ "skip": true, "inbox": inbox }));
            }
        }
        map.insert(wt.id.clone(), now);
    }

    log::info!(
        "[prompt-context] worktree={} terminal={:?} description={:?} inbox_len={:?}",
        wt.name,
        payload.terminal_id,
        wt.description,
        inbox.as_ref().map(|s| s.len())
    );
    Json(serde_json::json!({
        "worktreeName": wt.name,
        "description": wt.description,
        "inbox": inbox,
    }))
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
        // 再発時の切り分け用に最小限の差分情報を出す（秘密値そのものは出さない）。
        let reason = match auth_header {
            None => "authorization header missing".to_string(),
            Some(h) if !h.starts_with("Bearer ") => "authorization header is not a Bearer token".to_string(),
            Some(h) => format!(
                "key mismatch (provided len={}, expected len={})",
                h.len().saturating_sub(7),
                expected_key.len()
            ),
        };
        log::warn!("[mcp] unauthorized request: {}", reason);
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

async fn broadcast_notify_worktree(peer_registry: &PeerMap, timeout_counts: &PeerTimeoutCounts, event: &NotifyWorktreeEvent) {
    let params = LoggingMessageNotificationParam {
        level: LoggingLevel::Info,
        logger: Some("oretachi".to_string()),
        data: serde_json::json!({
            "event": "notify_worktree",
            "worktreeName": event.worktree_name,
            "kind": event.kind,
            "body": event.body,
            "agent": event.agent,
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
    // ポート上書き禁止かつ既存ファイルがある場合: ポートは既存値を使い、APIキーのみ更新
    let effective_port = if !overwrite_port {
        server_info_file_path(app_handle)
            .and_then(|p| {
                if p.exists() {
                    fs::read_to_string(&p)
                        .ok()
                        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                        .and_then(|v| v["port"].as_u64())
                        .map(|n| n as u16)
                } else {
                    None
                }
            })
            .unwrap_or(port)
    } else {
        port
    };
    if let Some(path) = server_info_file_path(app_handle) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let info = serde_json::json!({ "port": effective_port, "apiKey": api_key });
        if let Err(e) = fs::write(&path, serde_json::to_string_pretty(&info).unwrap_or_default()) {
            log::warn!("Failed to write server info file: {}", e);
        }
    }

    // プラグインの .mcp.json も同じ値で更新（app_data_dir 取得失敗時は plugin_dir.exists() で早期 return）
    // ORETACHI_PLUGIN_OVERWRITE=false の場合はグローバルプラグインを汚染しないようスキップ。
    // （mcp-server.json 自体は MCP 接続に必要なため上で常に書き込み済み）
    if crate::claude_plugin::overwrite_enabled() {
        if let Err(e) = crate::claude_plugin::update_mcp_config(app_handle, effective_port, api_key)
        {
            log::warn!("[ClaudePlugin] Failed to update .mcp.json: {}", e);
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

/// `.env` 由来のポート上書きを解決する。
///
/// **空文字は「未設定」と同じ扱いにする。** `.env` には3つのポート上書きを空値で
/// 並べてあり（既定値のドキュメントを兼ねる）、`dotenvy` はそれを空文字として
/// プロセス環境へ載せるため、ここで弾かないと既定値へ落ちない。
pub fn parse_port_override(raw: Option<&str>, default: u16) -> u16 {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(default)
}

/// env からポート上書きを読む（未設定・空・数値でない場合は `default`）。
pub fn env_port_override(name: &str, default: u16) -> u16 {
    parse_port_override(std::env::var(name).ok().as_deref(), default)
}

pub fn start_mcp_server(app_handle: AppHandle, port: u16, remote_access: bool) {
    // `settings.json` は本番と全 dev インスタンスで共有されるため、別ワークツリーで
    // dev ビルドを立ち上げると `mcpPort` を奪い合って bind に失敗する。env で退避できる
    // ようにしておく（`MCP_PORT_OVERWRITE=false` と併用すれば mcp-server.json も奪わない）。
    let port = env_port_override("ORETACHI_MCP_PORT", port);
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
    if let Ok(mut guard) = manager.notify_listener_id.lock() {
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

    // notify-worktree イベント受信時に全 MCP クライアントへブロードキャスト
    let peer_map_for_notify_listener = peer_map.clone();
    let timeout_counts_for_notify_listener = timeout_counts.clone();
    let notify_listener_id = app_handle.listen("notify-worktree", move |event: tauri::Event| {
        let registry = peer_map_for_notify_listener.clone();
        let tc = timeout_counts_for_notify_listener.clone();
        if let Ok(payload) = serde_json::from_str::<NotifyWorktreeEvent>(event.payload()) {
            tauri::async_runtime::spawn(async move {
                broadcast_notify_worktree(&registry, &tc, &payload).await;
            });
        }
    });

    let manager = app_handle.state::<McpServerManager>();
    if let Ok(mut guard) = manager.notify_listener_id.lock() {
        *guard = Some(notify_listener_id);
    }
    drop(manager);

    // hook 通知は broadcast channel 経由で MCP ピアにのみ配信（WebView IPC を完全バイパス）
    {
        let peer_map_for_hook = peer_map.clone();
        let timeout_counts_for_hook = timeout_counts.clone();
        let mut hook_rx = app_handle.state::<McpServerManager>().hook_tx.subscribe();
        let mut shutdown_rx_for_hook = shutdown_rx.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    result = hook_rx.recv() => {
                        match result {
                            Ok(payload) => {
                                broadcast_notify_worktree(&peer_map_for_hook, &timeout_counts_for_hook, &payload).await;
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!("[mcp] hook broadcast lagged, {} messages dropped", n);
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = shutdown_rx_for_hook.changed() => {
                        if *shutdown_rx_for_hook.borrow() { break; }
                    }
                }
            }
        });
    }

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
            .route("/set-description", post(set_description_handler))
            .route("/session-context", post(session_context_handler))
            .route("/prompt-context", post(prompt_context_handler))
            .route("/turn-context", post(turn_context_handler))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase 0 (#121) で実測した Stop payload そのもの（CC 2.1.227 / Windows 10）。
    /// #120 本文は「`stop_hook_active` 相当のフラグは現行ドキュメントに見当たらない」と
    /// していたが、実在することが確認され訂正されている。ここに生データを残しておく。
    const STOP_PAYLOAD: &str = r#"{
        "session_id": "bcbd95af-3066-4bc9-9b6b-98fabdd3ef8b",
        "transcript_path": "X:/t.jsonl",
        "cwd": "X:/wt/foo",
        "prompt_id": "46513e5d-9375-47f3-a6c0-c3a4452eb2fa",
        "permission_mode": "default",
        "effort": { "level": "medium" },
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": "2",
        "background_tasks": [],
        "session_crons": []
    }"#;

    /// `.env` にはポート上書き3本を**空値**で並べてある（既定値のドキュメントを兼ねる）。
    /// `dotenvy` は空値もプロセス環境へ載せるので、「空 = 未設定」がここで崩れると
    /// 本番の MCP ポートが 0 になったり tauri-mcp が起動に失敗したりする。
    #[test]
    fn test_parse_port_override_treats_blank_as_unset() {
        assert_eq!(parse_port_override(None, 4000), 4000);
        assert_eq!(parse_port_override(Some(""), 4000), 4000);
        assert_eq!(parse_port_override(Some("   "), 4000), 4000);
        // 数値でない値も既定へ落とす（起動を止めない）
        assert_eq!(parse_port_override(Some("abc"), 4000), 4000);
        assert_eq!(parse_port_override(Some("70000"), 4000), 4000);
        // 明示された値は使う
        assert_eq!(parse_port_override(Some("9163"), 4000), 9163);
        assert_eq!(parse_port_override(Some(" 9163 "), 4000), 9163);
    }

    #[test]
    fn test_parse_stop_hook_fields_initial_firing() {
        let (prompt_id, active) = parse_stop_hook_fields(Some(STOP_PAYLOAD));
        assert_eq!(prompt_id.as_deref(), Some("46513e5d-9375-47f3-a6c0-c3a4452eb2fa"));
        assert!(!active, "初回発火は stop_hook_active=false");
    }

    #[test]
    fn test_parse_stop_hook_fields_continuation() {
        let json = STOP_PAYLOAD.replace("\"stop_hook_active\": false", "\"stop_hook_active\": true");
        let (prompt_id, active) = parse_stop_hook_fields(Some(&json));
        // 継続ターンでも prompt_id は不変（1発話に対する9回の発火すべてで同一値だった）
        assert_eq!(prompt_id.as_deref(), Some("46513e5d-9375-47f3-a6c0-c3a4452eb2fa"));
        assert!(active);
    }

    #[test]
    fn test_parse_stop_hook_fields_missing_or_broken() {
        assert_eq!(parse_stop_hook_fields(None), (None, false));
        assert_eq!(parse_stop_hook_fields(Some("not json")), (None, false));
        assert_eq!(parse_stop_hook_fields(Some("{}")), (None, false));
        // 空文字の prompt_id は「取れなかった」と同じ扱い（1ターン1回の鍵にできない）
        assert_eq!(
            parse_stop_hook_fields(Some(r#"{"prompt_id":"  ","stop_hook_active":true}"#)),
            (None, true)
        );
    }

    /// 公式ドキュメント記載のサブエージェント（Task tool）内発火 hook JSON の想定形（#141）。
    /// リポジトリ内に実測サンプルが無いため STOP_PAYLOAD をベースに agent_id/agent_type を
    /// トップレベルへ追加した形で構成する。
    const SUBAGENT_PERMISSION_REQUEST_PAYLOAD: &str = r#"{
        "session_id": "bcbd95af-3066-4bc9-9b6b-98fabdd3ef8b",
        "transcript_path": "X:/t.jsonl",
        "cwd": "X:/wt/foo",
        "hook_event_name": "PermissionRequest",
        "agent_id": "agent-01",
        "agent_type": "general-purpose",
        "tool_name": "Bash",
        "tool_input": { "command": "ls" }
    }"#;

    #[test]
    fn test_hook_body_has_agent_id_present() {
        assert!(hook_body_has_agent_id(Some(SUBAGENT_PERMISSION_REQUEST_PAYLOAD)));
    }

    #[test]
    fn test_hook_body_has_agent_id_absent() {
        assert!(!hook_body_has_agent_id(Some(STOP_PAYLOAD)));
    }

    #[test]
    fn test_hook_body_has_agent_id_missing_or_broken() {
        assert!(!hook_body_has_agent_id(None));
        assert!(!hook_body_has_agent_id(Some("not json")));
        assert!(!hook_body_has_agent_id(Some("{}")));
    }

    #[test]
    fn test_should_skip_subagent_notify_true_when_cc_and_agent_id_and_kind_none() {
        assert!(should_skip_subagent_notify(
            Some("cc"),
            None,
            Some(SUBAGENT_PERMISSION_REQUEST_PAYLOAD)
        ));
    }

    #[test]
    fn test_should_skip_subagent_notify_false_when_agent_not_cc() {
        assert!(!should_skip_subagent_notify(
            Some("gemini"),
            None,
            Some(SUBAGENT_PERMISSION_REQUEST_PAYLOAD)
        ));
        assert!(!should_skip_subagent_notify(
            None,
            None,
            Some(SUBAGENT_PERMISSION_REQUEST_PAYLOAD)
        ));
    }

    #[test]
    fn test_should_skip_subagent_notify_false_when_kind_explicit() {
        assert!(!should_skip_subagent_notify(
            Some("cc"),
            Some("hook"),
            Some(SUBAGENT_PERMISSION_REQUEST_PAYLOAD)
        ));
    }

    #[test]
    fn test_should_skip_subagent_notify_false_when_no_agent_id() {
        assert!(!should_skip_subagent_notify(Some("cc"), None, Some(STOP_PAYLOAD)));
    }

    #[test]
    fn test_should_skip_subagent_notify_false_when_body_none() {
        assert!(!should_skip_subagent_notify(Some("cc"), None, None));
    }

    /// MCP ブロードキャスト経路（`listen("notify-worktree")` → `from_str`）は `tray` を
    /// 持たない旧ペイロードも受け取る。default で落とすと全通知が抑制扱いになる (#153)。
    #[test]
    fn test_notify_worktree_event_tray_defaults_to_true() {
        let legacy = r#"{"worktree_name":"wt","kind":"hook","body":null,"agent":null}"#;
        let event: NotifyWorktreeEvent = serde_json::from_str(legacy).unwrap();
        assert!(event.tray);
    }

    #[test]
    fn test_notify_worktree_event_tray_round_trip() {
        let event = NotifyWorktreeEvent {
            worktree_name: "wt".into(),
            kind: "hook".into(),
            body: None,
            agent: None,
            tray: false,
        };
        let restored: NotifyWorktreeEvent =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert!(!restored.tray);
    }

    /// name 未設定のグループは UI 側 (useWorkgroups.displayName / i18n workgroup.autoName) が
    /// 並び順から「グループ(N)」を自動生成して表示する。MCP が生の name をそのまま返すと
    /// リネームしていない既定グループが null になり、レポートにグループが出せなくなる。
    #[test]
    fn workgroup_display_name_matches_ui_fallback() {
        let mut settings = AppSettings::default();
        settings.workgroups = vec![
            Workgroup { id: "wg-1".into(), name: None, ..Default::default() },
            Workgroup { id: "wg-2".into(), name: Some("  ".into()), ..Default::default() },
            Workgroup { id: "wg-3".into(), name: Some("リリース準備".into()), ..Default::default() },
        ];

        // 既定ロケール (ja) は 1 始まりの並び順で自動生成
        assert_eq!(workgroup_display_name(&settings, &settings.workgroups[0]), "グループ(1)");
        // 空白のみの name も未設定扱い（UI の trim と揃える）
        assert_eq!(workgroup_display_name(&settings, &settings.workgroups[1]), "グループ(2)");
        // 明示的な name はそのまま
        assert_eq!(workgroup_display_name(&settings, &settings.workgroups[2]), "リリース準備");

        settings.locale = Some("en".into());
        assert_eq!(workgroup_display_name(&settings, &settings.workgroups[0]), "Group (1)");
    }

    fn wg_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.workgroups = vec![
            Workgroup { id: "wg-1".into(), name: None, ..Default::default() },
            Workgroup { id: "wg-2".into(), name: Some("リリース準備".into()), ..Default::default() },
            Workgroup { id: "wg-3".into(), name: Some("Bug Fix".into()), ..Default::default() },
        ];
        settings
    }

    /// 未指定なら追加先はフロント既定の WG に委ねる（従来挙動の維持）。
    #[test]
    fn resolve_workgroup_target_defaults_to_none() {
        let settings = wg_settings();
        assert_eq!(resolve_workgroup_target(&settings, None, None), Ok(None));
        // 空文字・空白のみは未指定扱い
        assert_eq!(resolve_workgroup_target(&settings, Some(""), Some("  ")), Ok(None));
    }

    #[test]
    fn resolve_workgroup_target_matches_id_and_name() {
        let settings = wg_settings();
        // id 優先（name が別グループを指していても id が勝つ）
        assert_eq!(
            resolve_workgroup_target(&settings, Some("wg-2"), Some("Bug Fix")),
            Ok(Some("wg-2".into()))
        );
        // 表示名の完全一致
        assert_eq!(
            resolve_workgroup_target(&settings, None, Some("リリース準備")),
            Ok(Some("wg-2".into()))
        );
        // 前後空白と大文字小文字は無視
        assert_eq!(
            resolve_workgroup_target(&settings, None, Some("  bug fix  ")),
            Ok(Some("wg-3".into()))
        );
        // 自動生成された表示名でも指定できる
        assert_eq!(
            resolve_workgroup_target(&settings, None, Some("グループ(1)")),
            Ok(Some("wg-1".into()))
        );
    }

    /// 解決できない指定は先頭 WG へ暗黙フォールバックせずエラーにし、
    /// エージェントが言い直せるよう利用可能な WG を列挙する。
    #[test]
    fn resolve_workgroup_target_errors_instead_of_falling_back() {
        let settings = wg_settings();

        let err = resolve_workgroup_target(&settings, Some("wg-404"), None).unwrap_err();
        assert!(err.contains("wg-404"), "{}", err);
        assert!(err.contains("リリース準備 (wg-2)"), "{}", err);

        let err = resolve_workgroup_target(&settings, None, Some("存在しない")).unwrap_err();
        assert!(err.contains("存在しない"), "{}", err);
        assert!(err.contains("グループ(1) (wg-1)"), "{}", err);
    }

    /// 同名 WG が複数ある場合は、どちらに入るか予測できないのでエラーにする。
    #[test]
    fn resolve_workgroup_target_rejects_ambiguous_name() {
        let mut settings = wg_settings();
        settings.workgroups.push(Workgroup {
            id: "wg-4".into(),
            name: Some("リリース準備".into()),
            ..Default::default()
        });
        let err = resolve_workgroup_target(&settings, None, Some("リリース準備")).unwrap_err();
        assert!(err.contains("複数"), "{}", err);
        assert!(err.contains("workgroup_id"), "{}", err);
    }

    // ─── #126: 購読 target のワイルドカード ──────────────────────────────────

    fn target_settings() -> AppSettings {
        let mut settings = wg_settings();
        settings.repositories = vec![crate::settings::Repository {
            id: "r-1".into(),
            name: "OreTachi".into(),
            path: "X:/repo".into(),
            exec_script: None,
            copy_targets: None,
            package_manager: None,
            package_manager_args: None,
            notification_hooks: None,
            pull_before_add: None,
            branch_name_pattern: None,
        }];
        settings.worktrees = vec![WorktreeEntry {
            id: "wt-1".into(),
            name: "oretachi-abcd".into(),
            repository_id: "r-1".into(),
            repository_name: "OreTachi".into(),
            path: "X:/wt".into(),
            branch_name: "feature/x".into(),
            hotkey_char: None,
            auto_approval: None,
            auto_approval_prompt: None,
            description: None,
            description_open: None,
            workgroup_id: Some("wg-2".into()),
            tray_notification: None,
            is_home: false,
            is_repository: false,
        }];
        settings
    }

    #[test]
    fn resolve_subscription_target_accepts_wildcards() {
        let settings = target_settings();

        let all = resolve_subscription_target(&settings, " * ").unwrap();
        assert_eq!(all.stored, "*");
        assert!(all.worktree_id.is_none());

        // ワークグループは ID でも表示名でも解決でき、保存は ID に正規化される
        for raw in ["workgroup:wg-2", "workgroup: リリース準備 "] {
            let g = resolve_subscription_target(&settings, raw).unwrap();
            assert_eq!(g.stored, "workgroup:wg-2", "{}", raw);
            assert!(g.label.contains("リリース準備"), "{}", g.label);
            assert!(g.worktree_id.is_none());
        }

        // リポジトリ名は大小を問わず、保存は小文字へ正規化される（照合側と一致）
        let r = resolve_subscription_target(&settings, "repo:oreTACHI").unwrap();
        assert_eq!(r.stored, "repo:oretachi");
        assert!(r.label.contains("OreTachi"), "{}", r.label);
        assert!(r.worktree_id.is_none());

        // 厳密一致は従来どおりワークツリー ID を保存する
        let w = resolve_subscription_target(&settings, "oretachi-abcd").unwrap();
        assert_eq!(w.stored, "wt-1");
        assert_eq!(w.worktree_id.as_deref(), Some("wt-1"));
    }

    /// 非 ASCII を含むリポジトリ名でも、大小を変えて入力できて表示名も元表記に戻る。
    /// `eq_ignore_ascii_case` は非 ASCII を畳まないので、保存側の `to_lowercase()` と
    /// 突合方法がずれていると「見つかりません」やラベル崩れになる。
    #[test]
    fn resolve_subscription_target_handles_non_ascii_repo_name() {
        let mut settings = target_settings();
        settings.repositories[0].name = "テストRepo".into();
        settings.worktrees[0].repository_name = "テストRepo".into();

        let r = resolve_subscription_target(&settings, "repo:テストREPO").unwrap();
        assert_eq!(r.stored, "repo:テストrepo");
        assert!(r.label.contains("テストRepo"), "{}", r.label);
        assert_eq!(
            describe_target(&settings, &r.stored),
            ("repo".to_string(), Some("テストRepo".to_string()))
        );
        // 照合側の候補集合とも噛み合う
        assert!(crate::event_db::matching_targets("wt-1", None, Some("テストRepo"))
            .contains(&r.stored));
    }

    /// 保存値と `matching_targets` の突合が実際に噛み合うこと。ここがずれると
    /// 「購読は成功するのに一生届かない」という一番分かりにくい失敗になる。
    #[test]
    fn resolve_subscription_target_agrees_with_matching_targets() {
        let settings = target_settings();
        let wt = &settings.worktrees[0];
        let group = resolve_workgroup(&settings, wt).map(|g| g.id.clone());
        let targets = crate::event_db::matching_targets(
            &wt.id,
            group.as_deref(),
            Some(&wt.repository_name),
        );
        for raw in ["*", "workgroup:wg-2", "repo:OreTachi", "oretachi-abcd"] {
            let resolved = resolve_subscription_target(&settings, raw).unwrap();
            assert!(
                targets.contains(&resolved.stored),
                "target '{}' -> '{}' が {:?} に含まれない",
                raw,
                resolved.stored,
                targets
            );
        }
    }

    #[test]
    fn resolve_subscription_target_rejects_unknown_wildcards() {
        let settings = target_settings();
        assert!(resolve_subscription_target(&settings, "repo:").is_err());
        assert!(resolve_subscription_target(&settings, "workgroup:").is_err());
        let err = resolve_subscription_target(&settings, "repo:unknown")
            .err()
            .expect("未知のリポジトリはエラー");
        assert!(format!("{:?}", err).contains("OreTachi"), "利用可能な候補を添える");
    }

    /// UI / ツール応答は種別で判断する。名前が引けないことだけを根拠にすると、
    /// ワイルドカード購読がすべて「対象がクローズ済み」と誤表示される。
    #[test]
    fn describe_target_distinguishes_wildcards_from_closed_worktree() {
        let settings = target_settings();
        assert_eq!(describe_target(&settings, "*"), ("all".to_string(), None));
        let (kind, label) = describe_target(&settings, "workgroup:wg-1");
        assert_eq!(kind, "workgroup");
        // 未リネームのグループでも生の ID ではなく表示名を返す
        assert_eq!(label.as_deref(), Some("グループ(1)"));
        assert_eq!(
            describe_target(&settings, "repo:oretachi"),
            ("repo".to_string(), Some("OreTachi".to_string()))
        );
        assert_eq!(
            describe_target(&settings, "wt-1"),
            ("worktree".to_string(), Some("oretachi-abcd".to_string()))
        );
        // クローズ済みの厳密一致だけが「名前なしの worktree」になる
        assert_eq!(
            describe_target(&settings, "wt-gone"),
            ("worktree".to_string(), None)
        );
    }

    /// 「未設定へ戻す」は `trayNotification: null` としてフロントへ渡る。
    /// キーごと落とすと App.vue 側で「変更なし」と区別が付かないため、
    /// `skip_serializing_if` を足さないことをテストで固定する。
    #[test]
    fn set_tray_notification_event_serializes_inherit_as_null() {
        let ev = SetTrayNotificationEvent {
            worktree: "wt".into(),
            worktree_id: "id".into(),
            tray_notification: None,
        };
        let v: serde_json::Value = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["trayNotification"], serde_json::Value::Null);
        assert_eq!(v["worktreeId"], "id");

        let ev = SetTrayNotificationEvent { tray_notification: Some(false), ..ev };
        let v: serde_json::Value = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["trayNotification"], serde_json::Value::Bool(false));
    }

    /// 変更後の実効値は「値をコピーせず毎回解決する」規則を守り、
    /// 新しい値を載せた probe を `resolve_tray_notification` に通して求める。
    #[test]
    fn set_tray_notification_new_effective_falls_back_to_workgroup() {
        let mut settings = AppSettings::default();
        settings.workgroups.push(Workgroup {
            id: "g".into(),
            tray_notification: Some(false),
            ..Default::default()
        });
        let wt = WorktreeEntry {
            id: "wt-1".into(),
            name: "oretachi-abcd".into(),
            repository_id: "r-1".into(),
            repository_name: "OreTachi".into(),
            path: "X:/wt".into(),
            branch_name: "feature/x".into(),
            hotkey_char: None,
            auto_approval: None,
            auto_approval_prompt: None,
            description: None,
            description_open: None,
            workgroup_id: Some("g".into()),
            tray_notification: Some(true),
            is_home: false,
            is_repository: false,
        };
        assert!(resolve_tray_notification(&settings, &wt));

        // enabled 省略 (= None) で未設定へ戻すと、ワークグループ既定値へ落ちる
        let mut probe = wt.clone();
        probe.tray_notification = None;
        assert!(!resolve_tray_notification(&settings, &probe));
    }

    /// Claude Code は plan モードで `readOnlyHint` が立っていない MCP ツールを
    /// permissions.allow に関わらず一律 ask にする。ここで advertise 内容を固定しておく。
    #[test]
    fn read_only_tools_advertise_read_only_hint() {
        const READ_ONLY: &[&str] = &[
            "artifact",
            "artifact_module",
            "search_artifact",
            "oretachi_get_worktree_status",
            "oretachi_inspect_worktree",
            "oretachi_get_app_options",
            "oretachi_list_repository",
            "oretachi_list_workgroups",
            "oretachi_list_terminals",
            "oretachi_read_terminal",
            "oretachi_show_worktree",
        ];
        const NOT_READ_ONLY: &[&str] = &[
            "notify_worktree",
            "oretachi_set_description",
            "oretachi_set_tray_notification",
            "oretachi_add_task",
            "oretachi_close_worktree",
            "oretachi_spawn_terminal",
            "oretachi_kill_terminal",
            "oretachi_write_terminal",
            "oretachi_import_worktree",
        ];

        let tools = NotifyService::tool_router().list_all();
        let hint = |name: &str| -> Option<bool> {
            tools
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("tool '{}' が存在しません", name))
                .annotations
                .as_ref()
                .and_then(|a| a.read_only_hint)
        };

        for name in READ_ONLY {
            assert_eq!(hint(name), Some(true), "{} は read_only_hint = true であるべき", name);
        }
        for name in NOT_READ_ONLY {
            assert_ne!(hint(name), Some(true), "{} が誤って read-only 宣言されている", name);
        }
    }

    #[test]
    fn destructive_tools_advertise_destructive_hint() {
        let tools = NotifyService::tool_router().list_all();
        for name in ["oretachi_close_worktree", "oretachi_kill_terminal"] {
            let t = tools.iter().find(|t| t.name == name).expect("tool が存在しません");
            assert_eq!(
                t.annotations.as_ref().and_then(|a| a.destructive_hint),
                Some(true),
                "{} は destructive_hint = true であるべき",
                name
            );
        }
    }
}
