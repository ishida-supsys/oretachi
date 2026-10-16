//! アプリ形式アーティファクトの起動レジストリと常駐ウィンドウ（`appinst-*`）。
//!
//! - インスタンスごとに常駐ウィンドウを 1 つ作る。アプリのコードはそのウィンドウの中の
//!   `sandbox="allow-scripts"` iframe でだけ動く（シェルは oretachi 自身のコード）。
//! - 呼び出し元インスタンスは、呼び出した webview の**ウィンドウラベルから**このレジストリを引いて
//!   決める。リクエストのパラメータからは決めない。
//! - 起動は 1 インスタンス 1 つ。別アーティファクトなら同時に起動できる（上限 [`MAX_INSTANCES`]）。
//! - 再起動をまたいで復活しない（レジストリは in-memory のみ）。
//! - `artifact_lock`（表示中ロック）は流用しない。あれは「ビューアで表示中の 1 件」を守る粒度で、
//!   非表示で常駐するインスタンスとは別物。

use crate::app_grant::AppGrantStore;
use crate::app_manifest::{self, AppGrant, AppManifest, RequestedPermissions, ResolvedPermissions};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::sync::oneshot;

pub const LABEL_PREFIX: &str = "appinst-";

/// 同時に起動できるインスタンス数（承認待ちの Starting を含む）。
/// webview の数は既知のハング（端末 15 個以上と強相関）を悪化させうるので小さく始める。
pub const MAX_INSTANCES: usize = 4;

/// インスタンスごとのデバッグログの保持件数（リングバッファ）。
const LOG_CAPACITY: usize = 500;

/// ログを保持するキーの上限。停止後もログは見られるようにするが、無制限には持たない。
const MAX_LOG_KEYS: usize = 64;

/// 起動時の承認ダイアログを待つ上限。超えたら起動を中止する（インスタンスは作っていない）。
const START_APPROVAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

const REACT_TYPE: &str = "application/vnd.ant.react";

type Key = (String, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstanceState {
    /// 承認ダイアログの待ち。ウィンドウはまだ無く、API は呼べない。
    Starting,
    Running,
}

/// 承認ダイアログの結果。`Superseded` / `Cancelled` は `Rejected` と**区別する**
/// （拒否は再承認ではインスタンスの停止を意味するが、差し替え・取り消しは何も変えない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Approved,
    Rejected,
    /// より新しい manifest の要求に置き換えられた
    Superseded,
    /// インスタンスの停止・アーティファクトの削除などで取り下げられた（oneshot の drop も含む）
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// UNIX ミリ秒
    pub ts: u64,
    /// "info" | "warn" | "error"
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    request_id: String,
    requested: RequestedPermissions,
}

#[derive(Debug)]
struct Instance {
    worktree_id: String,
    artifact_id: String,
    state: InstanceState,
    visible: bool,
    pending: Option<PendingApproval>,
    /// コード更新・再承認でリロードした回数と最後の時刻（UNIX ミリ秒）。起動画面に出す
    reload_count: u64,
    last_reload_at: Option<u64>,
    /// 直近に測ったメモリーのバイト数（シリアライズした JSON）。未測定は `None`
    memory_bytes: Option<u64>,
}

struct Waiter {
    tx: oneshot::Sender<ApprovalOutcome>,
    key: Key,
    /// ダイアログへ送った要求そのもの。main の webview が再読込されてキューを失っても、
    /// `list_pending_app_grants` で取り戻せるように持っておく
    payload: Option<serde_json::Value>,
}

#[derive(Default)]
struct Inner {
    by_label: HashMap<String, Instance>,
    by_key: HashMap<Key, String>,
    waiters: HashMap<String, Waiter>,
    logs: HashMap<Key, VecDeque<LogEntry>>,
    label_seq: u64,
    request_seq: u64,
}

/// 認可の入口で使う、呼び出し元インスタンスの身元。
#[derive(Debug, Clone)]
pub struct InstanceContext {
    /// 常駐ウィンドウ（`appinst-<n>`）のラベル。サブウィンドウから呼ばれても**インスタンスの**ラベル。
    /// DB 接続・実行中の通信など、インスタンスごとに持つ資源のキーに使う
    pub instance_label: String,
    pub worktree_id: String,
    pub artifact_id: String,
    pub reapproval_pending: bool,
}

#[derive(Default)]
pub struct AppInstanceRegistry(Mutex<Inner>);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─── レジストリ本体（純粋な状態遷移。AppHandle を取らないのでテストできる） ───────────

impl Inner {
    /// 起動枠を確保する。同一インスタンスの多重起動と上限超過はここで拒否する。
    fn reserve(&mut self, worktree_id: &str, artifact_id: &str) -> Result<String, String> {
        let key: Key = (worktree_id.to_string(), artifact_id.to_string());
        if self.by_key.contains_key(&key) {
            return Err("このアプリは既に起動中です（同一インスタンスの多重起動はできません）".to_string());
        }
        if self.by_label.len() >= MAX_INSTANCES {
            return Err(format!(
                "同時に起動できるアプリは {} 個までです。どれかを停止してから起動してください",
                MAX_INSTANCES
            ));
        }
        self.label_seq += 1;
        let label = format!("{}{}", LABEL_PREFIX, self.label_seq);
        self.by_label.insert(
            label.clone(),
            Instance {
                worktree_id: key.0.clone(),
                artifact_id: key.1.clone(),
                state: InstanceState::Starting,
                visible: false,
                pending: None,
                reload_count: 0,
                last_reload_at: None,
                memory_bytes: None,
            },
        );
        self.by_key.insert(key, label.clone());
        Ok(label)
    }

    /// インスタンスを取り除き、待っている承認はすべて `Cancelled` にする。
    /// 取り下げたリクエスト ID を返す（ダイアログを閉じさせるため）。
    fn remove(&mut self, label: &str) -> Option<(Instance, Vec<String>)> {
        let inst = self.by_label.remove(label)?;
        self.by_key.remove(&(inst.worktree_id.clone(), inst.artifact_id.clone()));
        let key: Key = (inst.worktree_id.clone(), inst.artifact_id.clone());
        let ids: Vec<String> = self
            .waiters
            .iter()
            .filter(|(_, w)| w.key == key)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            if let Some(w) = self.waiters.remove(id) {
                let _ = w.tx.send(ApprovalOutcome::Cancelled);
            }
        }
        Some((inst, ids))
    }

    /// 承認要求を登録する。同じインスタンスに保留があれば `Superseded` で置き換える（最新が勝つ）。
    /// 戻り値は (新しいリクエスト ID, 受信側, 置き換えられた旧リクエスト ID)。
    fn new_request(
        &mut self,
        label: &str,
        requested: RequestedPermissions,
    ) -> Option<(String, oneshot::Receiver<ApprovalOutcome>, Option<String>)> {
        let key = {
            let inst = self.by_label.get(label)?;
            (inst.worktree_id.clone(), inst.artifact_id.clone())
        };
        self.request_seq += 1;
        let request_id = format!("appgrant-{}", self.request_seq);
        let old = self.by_label.get(label).and_then(|i| i.pending.as_ref()).map(|p| p.request_id.clone());
        if let Some(old_id) = &old {
            if let Some(w) = self.waiters.remove(old_id) {
                let _ = w.tx.send(ApprovalOutcome::Superseded);
            }
        }
        let (tx, rx) = oneshot::channel();
        self.waiters.insert(request_id.clone(), Waiter { tx, key, payload: None });
        if let Some(inst) = self.by_label.get_mut(label) {
            inst.pending = Some(PendingApproval { request_id: request_id.clone(), requested });
        }
        Some((request_id, rx, old))
    }

    /// 保留中の要求を取り下げる（manifest が保存済みの権限に戻ったときなど）。
    fn cancel_pending(&mut self, label: &str) -> Option<String> {
        let pending = self.by_label.get_mut(label)?.pending.take()?;
        if let Some(w) = self.waiters.remove(&pending.request_id) {
            let _ = w.tx.send(ApprovalOutcome::Cancelled);
        }
        Some(pending.request_id)
    }

    /// ダイアログからの応答を受け取る。すでに差し替え・取り下げ済みなら `None`。
    fn answer(&mut self, request_id: &str, outcome: ApprovalOutcome) -> Option<Key> {
        let w = self.waiters.remove(request_id)?;
        let key = w.key.clone();
        let _ = w.tx.send(outcome);
        Some(key)
    }

    fn push_log(&mut self, key: &Key, level: &str, message: String) -> LogEntry {
        if !self.logs.contains_key(key) && self.logs.len() >= MAX_LOG_KEYS {
            // 起動中でないキーを 1 つ捨てる
            let victim = self
                .logs
                .keys()
                .find(|k| !self.by_key.contains_key(*k))
                .cloned();
            if let Some(v) = victim {
                self.logs.remove(&v);
            }
        }
        let entry = LogEntry { ts: now_ms(), level: level.to_string(), message };
        let buf = self.logs.entry(key.clone()).or_default();
        if buf.len() >= LOG_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(entry.clone());
        entry
    }
}

// ─── イベント・状態の通知 ──────────────────────────────────────────────

fn status_value(
    inner: &Inner,
    store: &AppGrantStore,
    worktree_id: &str,
    artifact_id: &str,
) -> serde_json::Value {
    let key: Key = (worktree_id.to_string(), artifact_id.to_string());
    let inst = inner.by_key.get(&key).and_then(|l| inner.by_label.get(l));
    let grant = store.get(worktree_id, artifact_id);
    let memory_cap_of = grant.as_ref().and_then(|g| g.resolved.memory).map(|m| m.max_bytes);
    let memory_bytes = inst.and_then(|i| i.memory_bytes);
    serde_json::json!({
        "worktreeId": worktree_id,
        "artifactId": artifact_id,
        "running": inst.map(|i| i.state == InstanceState::Running).unwrap_or(false),
        "state": inst.map(|i| i.state),
        "visible": inst.map(|i| i.visible).unwrap_or(false),
        // 起動中の権限変更だけ。初回起動の承認待ち（Starting）は state で表す
        "reapprovalPending": inst
            .map(|i| i.state == InstanceState::Running && i.pending.is_some())
            .unwrap_or(false),
        "granted": grant.is_some(),
        "grantedAt": grant.as_ref().map(|g| g.granted_at),
        "maxInstances": MAX_INSTANCES,
        "reloadCount": inst.map(|i| i.reload_count).unwrap_or(0),
        "lastReloadAt": inst.and_then(|i| i.last_reload_at),
        // 承認済みの最大サイズと、直近に測った使用量。上限を下げた結果、既存データが超えている間は
        // `memoryOverCap`（書き込みだけ拒否・読み取りは可）。データは消さない
        "memoryMaxBytes": memory_cap_of,
        "memoryBytes": memory_bytes,
        "memoryOverCap": matches!((memory_bytes, memory_cap_of), (Some(b), Some(c)) if b > c),
    })
}

fn emit_changed(app: &AppHandle, worktree_id: &str, artifact_id: &str) {
    let payload = {
        let reg = app.state::<AppInstanceRegistry>();
        let store = app.state::<AppGrantStore>();
        let Ok(inner) = reg.0.lock() else { return };
        status_value(&inner, &store, worktree_id, artifact_id)
    };
    let _ = app.emit("app-instance-changed", payload);
}

/// デバッグログへ 1 行足し、ビューアへ通知する。
pub fn add_log(app: &AppHandle, worktree_id: &str, artifact_id: &str, level: &str, message: impl Into<String>) {
    let message = message.into();
    let key: Key = (worktree_id.to_string(), artifact_id.to_string());
    let entry = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return };
        inner.push_log(&key, level, message.clone())
    };
    match level {
        "error" => log::warn!("[app-instance] {}/{}: {}", worktree_id, artifact_id, message),
        _ => log::debug!("[app-instance] {}/{}: {}", worktree_id, artifact_id, message),
    }
    let _ = app.emit(
        "app-instance-log",
        serde_json::json!({ "worktreeId": worktree_id, "artifactId": artifact_id, "entry": entry }),
    );
}

fn emit_grant_cancelled(app: &AppHandle, request_ids: &[String]) {
    for id in request_ids {
        let _ = app.emit("app-grant-cancelled", serde_json::json!({ "requestId": id }));
    }
}

// ─── 呼び出し元の検査 ─────────────────────────────────────────────────

/// 起動画面を持つビューア（`artifact-*`）からだけ受け付ける。`appinst-*` から呼べてしまうと
/// アプリ自身が自分の起動・停止・承認取り消しをできてしまう。
fn require_viewer(window: &tauri::Window) -> Result<(), String> {
    if window.label().starts_with("artifact-") {
        Ok(())
    } else {
        Err(format!("このコマンドはアーティファクトのビューアからだけ呼べます（呼び出し元: {}）", window.label()))
    }
}

fn require_viewer_or_main(window: &tauri::Window) -> Result<(), String> {
    if window.label() == "main" {
        return Ok(());
    }
    require_viewer(window)
}

pub(crate) fn require_instance_window(window: &tauri::Window) -> Result<(), String> {
    if window.label().starts_with(LABEL_PREFIX) {
        Ok(())
    } else {
        Err(format!("このコマンドはアプリの実行ウィンドウからだけ呼べます（呼び出し元: {}）", window.label()))
    }
}

fn validate_ids(worktree_id: &str, artifact_id: &str) -> Result<(), String> {
    crate::validate_path_component(worktree_id)?;
    crate::validate_path_component(artifact_id)?;
    Ok(())
}

/// ラベルから呼び出し元インスタンスを引く。**Running のものだけ**を返す（Starting は拒否）。
pub fn context_for_label(app: &AppHandle, label: &str) -> Result<InstanceContext, String> {
    // サブウィンドウ（`appinst-<n>-w<k>`）は、所有するインスタンスとして扱う。同じ grant・同じ関門を通る
    let label = crate::app_subwindow::parent_label(app, label).unwrap_or_else(|| label.to_string());
    let label = label.as_str();
    let reg = app.state::<AppInstanceRegistry>();
    let inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
    let inst = inner
        .by_label
        .get(label)
        .ok_or_else(|| "このウィンドウは起動中のアプリとして登録されていません".to_string())?;
    if inst.state != InstanceState::Running {
        return Err("このアプリはまだ起動処理中です".to_string());
    }
    Ok(InstanceContext {
        instance_label: label.to_string(),
        worktree_id: inst.worktree_id.clone(),
        artifact_id: inst.artifact_id.clone(),
        reapproval_pending: inst.pending.is_some(),
    })
}

// ─── アーティファクトの読み出し ───────────────────────────────────────

struct AppArtifact {
    raw: serde_json::Value,
    title: String,
    manifest: AppManifest,
}

fn read_app_artifact(app: &AppHandle, worktree_id: &str, artifact_id: &str) -> Result<AppArtifact, String> {
    let dir = crate::artifacts_dir_in(
        &app.path().app_data_dir().map_err(|e| e.to_string())?,
        worktree_id,
    )?;
    let raw_text = crate::read_artifact_in_dir(&dir, artifact_id)
        .map_err(|_| format!("アーティファクト '{}' が存在しません", artifact_id))?;
    let raw: serde_json::Value =
        serde_json::from_str(&raw_text).map_err(|e| format!("アーティファクトの JSON を解析できません: {}", e))?;
    if raw.get("type").and_then(|v| v.as_str()) != Some(REACT_TYPE) {
        return Err("アプリにできるのは application/vnd.ant.react のアーティファクトだけです".to_string());
    }
    let app_value = raw
        .get("app")
        .filter(|v| !v.is_null())
        .ok_or_else(|| "このアーティファクトはアプリ（manifest の app）を持っていません".to_string())?;
    let manifest = app_manifest::parse_manifest(app_value)?;
    let title = raw.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    Ok(AppArtifact { raw, title, manifest })
}

// ─── 承認ダイアログ ───────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GrantRequestPayload<'a> {
    request_id: &'a str,
    /// "start"（初回・manifest 変更後の起動）| "change"（起動中の権限変更）
    kind: &'a str,
    worktree_id: &'a str,
    worktree_name: &'a str,
    artifact_id: &'a str,
    artifact_title: &'a str,
    app_name: &'a str,
    app_version: Option<&'a str>,
    app_description: Option<&'a str>,
    source: Option<&'a app_manifest::AppSource>,
    requested: &'a RequestedPermissions,
    resolved: &'a ResolvedPermissions,
    /// 再承認のとき、現在有効な権限（差分表示用）
    previous: Option<&'a ResolvedPermissions>,
    dangerous_tools: &'a [&'a str],
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 承認ダイアログを main に出す。**解決はここへ来る前に済ませてある**（承認時に再解決しない。
/// 表示したものと保存するものを一致させるため）。
fn emit_grant_request(
    app: &AppHandle,
    request_id: &str,
    kind: &str,
    worktree_id: &str,
    artifact_id: &str,
    art: &AppArtifact,
    resolved: &ResolvedPermissions,
    previous: Option<&ResolvedPermissions>,
) {
    let worktree_name = app
        .state::<crate::settings::SettingsManager>()
        .get()
        .worktrees
        .iter()
        .find(|w| w.id == worktree_id)
        .map(|w| w.name.clone())
        .unwrap_or_else(|| worktree_id.to_string());
    let payload = GrantRequestPayload {
        request_id,
        kind,
        worktree_id,
        worktree_name: &worktree_name,
        artifact_id,
        artifact_title: &art.title,
        app_name: &art.manifest.name,
        app_version: art.manifest.version.as_deref(),
        app_description: art.manifest.description.as_deref(),
        source: art.manifest.source.as_ref(),
        requested: &art.manifest.permissions,
        resolved,
        previous,
        dangerous_tools: app_manifest::DANGEROUS_TOOLS,
    };
    let value = match serde_json::to_value(&payload) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[app-instance] 承認要求を直列化できません: {}", e);
            return;
        }
    };
    if let Ok(mut inner) = app.state::<AppInstanceRegistry>().0.lock() {
        if let Some(w) = inner.waiters.get_mut(request_id) {
            w.payload = Some(value.clone());
        }
    }
    show_main_window(app);
    if let Err(e) = app.emit("app-grant-request", &value) {
        log::warn!("[app-instance] app-grant-request の送出に失敗: {}", e);
    }
}

/// 応答待ちの承認要求の一覧（古い順）。main の webview が再読込・再マウントされると、イベントで
/// 届いた要求のキューが消えて、起動が 10 分ハングする / 再承認の要求が二度と出なくなる。
/// main はマウント時にこれを呼んでキューを復元する。
#[tauri::command]
pub async fn list_pending_app_grants(
    window: tauri::Window,
    app: AppHandle,
) -> Result<Vec<serde_json::Value>, String> {
    if window.label() != "main" {
        return Err("承認要求の一覧は main ウィンドウからだけ取得できます".to_string());
    }
    let reg = app.state::<AppInstanceRegistry>();
    let inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
    let mut items: Vec<(u64, serde_json::Value)> = inner
        .waiters
        .iter()
        .filter_map(|(id, w)| {
            let n = id.strip_prefix("appgrant-")?.parse::<u64>().ok()?;
            Some((n, w.payload.clone()?))
        })
        .collect();
    items.sort_by_key(|(n, _)| *n);
    Ok(items.into_iter().map(|(_, v)| v).collect())
}

/// ダイアログの応答（main だけが呼べる）。
#[tauri::command]
pub async fn app_grant_respond(
    window: tauri::Window,
    app: AppHandle,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("承認の応答は main ウィンドウからだけ受け付けます".to_string());
    }
    let outcome = if approved { ApprovalOutcome::Approved } else { ApprovalOutcome::Rejected };
    let answered = {
        let reg = app.state::<AppInstanceRegistry>();
        let mut inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
        inner.answer(&request_id, outcome)
    };
    if answered.is_none() {
        // 差し替え・取り下げ済み。ダイアログはすでに閉じられているはずなので無害
        log::debug!("[app-instance] 応答先のない承認要求: {}", request_id);
    }
    Ok(())
}

// ─── 起動 ───────────────────────────────────────────────────────────

/// 承認時の解決。MCP の越境先（ワークツリー名 → ID）と、FS のスコープ（絶対パス）をここで確定する。
/// ダイアログに出した集合をそのまま grant へ保存するので、解決はこの 1 回だけ。
fn build_resolved(
    app: &AppHandle,
    worktree_id: &str,
    artifact_id: &str,
    requested: &RequestedPermissions,
) -> Result<ResolvedPermissions, String> {
    let settings = app.state::<crate::settings::SettingsManager>().get();
    let mut resolved = crate::mcp_server::resolve_app_permissions(&settings, requested)?;
    resolved.fs = build_resolved_scopes(app, worktree_id, artifact_id, requested.fs.as_ref())?;
    resolved.db = build_resolved_scopes(app, worktree_id, artifact_id, requested.db.as_ref())?;
    Ok(resolved)
}

/// FS / DB のスコープだけを解決する（絶対パスはワークツリーの移動などで変わりうるので、起動のたびに比べる）。
fn build_resolved_scopes(
    app: &AppHandle,
    worktree_id: &str,
    artifact_id: &str,
    requested: Option<&app_manifest::RequestedFs>,
) -> Result<Option<app_manifest::ResolvedFs>, String> {
    let Some(fs) = requested else { return Ok(None) };
    let settings = app.state::<crate::settings::SettingsManager>().get();
    // ホーム / リポジトリ擬似ワークツリーは全ワークツリーの祖先・メインクローンになりうるので、
    // worktree:// の根にしない（None を渡して拒否させる）
    let worktree_path = settings
        .worktrees
        .iter()
        .find(|w| w.id == worktree_id)
        .filter(|w| !w.is_home && !w.is_repository)
        .map(|w| std::path::PathBuf::from(&w.path));
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let artifact_dir = crate::app_fs::artifact_files_dir(&data_dir, worktree_id, artifact_id)?;
    crate::app_fs::resolve_requested_fs(fs, worktree_path.as_deref(), &artifact_dir).map(Some)
}

#[tauri::command]
pub async fn app_instance_start(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<(), String> {
    // ホームのアプリ一覧（main）からも起動できる。承認は start_inner の承認ダイアログを必ず通る
    require_viewer_or_main(&window)?;
    validate_ids(&worktree_id, &artifact_id)?;

    let label = {
        let reg = app.state::<AppInstanceRegistry>();
        let mut inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
        inner.reserve(&worktree_id, &artifact_id)?
    };
    emit_changed(&app, &worktree_id, &artifact_id);

    match start_inner(&app, &label, &worktree_id, &artifact_id).await {
        Ok(()) => Ok(()),
        Err(e) => {
            release(&app, &label);
            add_log(&app, &worktree_id, &artifact_id, "error", format!("起動に失敗: {}", e));
            emit_changed(&app, &worktree_id, &artifact_id);
            Err(e)
        }
    }
}

async fn start_inner(
    app: &AppHandle,
    label: &str,
    worktree_id: &str,
    artifact_id: &str,
) -> Result<(), String> {
    let art = read_app_artifact(app, worktree_id, artifact_id)?;
    let requested = art.manifest.permissions.clone();
    let saved = app.state::<AppGrantStore>().get(worktree_id, artifact_id);

    let needs_approval = match &saved {
        Some(g) => {
            app_manifest::requested_changed(&g.requested, &requested)
                // manifest が同じでも、解決済みの絶対パスが変わっていたら（ワークツリーの移動・
                // worktree_base_dir の変更・リンク先の変更）再承認する。保存済みのパスでは
                // 呼び出しのたびに拒否され、manifest を変えない限り再承認の導線が無くなる
                || (requested.fs.is_some()
                    && build_resolved_scopes(app, worktree_id, artifact_id, requested.fs.as_ref())? != g.resolved.fs)
                || (requested.db.is_some()
                    && build_resolved_scopes(app, worktree_id, artifact_id, requested.db.as_ref())? != g.resolved.db)
        }
        None => true,
    };

    if needs_approval {
        // 解決は 1 回だけ。ダイアログに出した集合をそのまま保存する
        let resolved = build_resolved(app, worktree_id, artifact_id, &requested)?;
        let (request_id, rx) = {
            let reg = app.state::<AppInstanceRegistry>();
            let mut inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
            let (id, rx, _) = inner
                .new_request(label, requested.clone())
                .ok_or_else(|| "起動枠が失われました".to_string())?;
            (id, rx)
        };
        add_log(app, worktree_id, artifact_id, "info", "権限の承認を待っています");
        emit_grant_request(
            app,
            &request_id,
            "start",
            worktree_id,
            artifact_id,
            &art,
            &resolved,
            saved.as_ref().map(|g| &g.resolved),
        );
        let outcome = match tokio::time::timeout(START_APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(o)) => o,
            Ok(Err(_)) => ApprovalOutcome::Cancelled,
            Err(_) => {
                let reg = app.state::<AppInstanceRegistry>();
                if let Ok(mut inner) = reg.0.lock() {
                    inner.waiters.remove(&request_id);
                }
                emit_grant_cancelled(app, &[request_id.clone()]);
                return Err("承認の待ち時間が上限（10 分）を超えたため、起動を中止しました".to_string());
            }
        };
        match outcome {
            ApprovalOutcome::Approved => {}
            ApprovalOutcome::Rejected => {
                return Err("権限が承認されなかったため、起動しませんでした".to_string());
            }
            ApprovalOutcome::Superseded | ApprovalOutcome::Cancelled => {
                return Err("承認の要求が取り下げられたため、起動しませんでした".to_string());
            }
        }
        let grant = AppGrant {
            granted_at: now_ms() / 1000,
            requested,
            resolved,
        };
        {
            // 起動枠が残っているときだけ、**ロックを持ったまま**保存する。承認と削除が競ったとき、
            // 削除側（`on_artifact_deleted`）が消した grant をここで書き戻して孤児にしないため
            // （同じ id・同じ権限で作り直した別物が、承認なしで起動できてしまう）
            let reg = app.state::<AppInstanceRegistry>();
            let mut inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
            let inst = inner
                .by_label
                .get_mut(label)
                .ok_or_else(|| "起動が取り消されました".to_string())?;
            inst.pending = None;
            app.state::<AppGrantStore>().put(worktree_id, artifact_id, grant)?;
        }
        add_log(app, worktree_id, artifact_id, "info", "権限を承認しました");
    }

    // 承認待ちの間にアーティファクトが消えていないか（レジストリの枠も取り下げられていないか）
    {
        let reg = app.state::<AppInstanceRegistry>();
        let inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
        if !inner.by_label.contains_key(label) {
            return Err("起動が取り消されました".to_string());
        }
    }

    create_window(app, label, &art.manifest.name)?;

    {
        let reg = app.state::<AppInstanceRegistry>();
        let mut inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
        match inner.by_label.get_mut(label) {
            Some(i) => i.state = InstanceState::Running,
            None => {
                drop(inner);
                destroy_window(app, label);
                return Err("起動が取り消されました".to_string());
            }
        }
    }
    add_log(app, worktree_id, artifact_id, "info", format!("起動しました（{}）", label));
    emit_changed(app, worktree_id, artifact_id);
    refresh_memory_bytes(app, worktree_id, artifact_id, true).await;

    // 承認待ちの間に manifest が書き換わっていた場合に備え、もう一度だけ差分を見る
    check_permission_change(app, worktree_id, artifact_id).await;
    Ok(())
}

fn create_window(app: &AppHandle, label: &str, app_name: &str) -> Result<(), String> {
    let win = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .title(format!("{} — oretachi app", app_name))
        .inner_size(960.0, 720.0)
        // 最小化させない。実機検証で、WebView2 は**最小化**したウィンドウの visibility を hidden にし、
        // タイマーを約 10 秒に 1 回まで間引いた（hide() しただけなら visible のまま間引かれない）。
        // 隠すのは「閉じる = hide」と起動画面の切り替えだけにして、常駐アプリのタイマーを守る
        .minimizable(false)
        .visible(false)
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| format!("実行ウィンドウを作れませんでした: {}", e))?;

    // 閉じる = 隠す。インスタンスは継続し、停止ボタンかアプリ終了で止まる
    let handle = app.clone();
    let label_owned = label.to_string();
    let win_for_events = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = win_for_events.hide();
            set_visible(&handle, &label_owned, false);
        }
    });
    Ok(())
}

fn destroy_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        // destroy は CloseRequested を経ない（hide にすり替えられない）
        if let Err(e) = w.destroy() {
            log::warn!("[app-instance] {} の破棄に失敗: {}", label, e);
        }
    }
}

fn set_visible(app: &AppHandle, label: &str, visible: bool) {
    let key = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return };
        let Some(inst) = inner.by_label.get_mut(label) else { return };
        inst.visible = visible;
        (inst.worktree_id.clone(), inst.artifact_id.clone())
    };
    emit_changed(app, &key.0, &key.1);
}

/// レジストリから外し、待っている承認ダイアログを閉じさせる。ウィンドウは壊さない。
fn release(app: &AppHandle, label: &str) -> Option<(String, String)> {
    let removed = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return None };
        inner.remove(label)
    };
    removed.map(|(inst, ids)| {
        emit_grant_cancelled(app, &ids);
        // DB の接続も閉じる（前提 10: インスタンスの停止ですべて閉じる）。閉じは非同期
        crate::app_db::on_instance_released(app, label);
        // 実行中のネットワーク通信は中断し、以後は出せなくする
        crate::app_net::on_instance_released(app, label);
        // インスタンスが開いたサブウィンドウもすべて閉じる（再起動での復元はしない）
        crate::app_subwindow::on_instance_released(app, label);
        // インスタンスが開いた専用 AI ターミナルも、停止とともにすべて閉じる（再起動での復元はしない）。
        // ホットリロードは `release` を通らないので、リロードをまたいで維持される。
        crate::ai_terminal::close_all_for_instance(app, &(inst.worktree_id.clone(), inst.artifact_id.clone()));
        (inst.worktree_id, inst.artifact_id)
    })
}

/// 表示用のアプリ名（manifest の `name`）。読めなければ artifact_id。
pub(crate) fn instance_app_name(app: &AppHandle, ctx: &InstanceContext) -> String {
    read_app_artifact(app, &ctx.worktree_id, &ctx.artifact_id)
        .map(|a| a.manifest.name)
        .unwrap_or_else(|_| ctx.artifact_id.clone())
}

// ─── 停止 ───────────────────────────────────────────────────────────

pub(crate) fn label_for_key(app: &AppHandle, worktree_id: &str, artifact_id: &str) -> Option<String> {
    let reg = app.state::<AppInstanceRegistry>();
    let inner = reg.0.lock().ok()?;
    inner.by_key.get(&(worktree_id.to_string(), artifact_id.to_string())).cloned()
}

/// インスタンスを停止する。ウィンドウを破棄し、レジストリから外す。止めたものがあれば `true`。
pub fn stop_instance(app: &AppHandle, worktree_id: &str, artifact_id: &str, reason: &str) -> bool {
    let Some(label) = label_for_key(app, worktree_id, artifact_id) else { return false };
    let released = release(app, &label);
    destroy_window(app, &label);
    if released.is_some() {
        add_log(app, worktree_id, artifact_id, "info", format!("停止しました（{}）", reason));
        emit_changed(app, worktree_id, artifact_id);
    }
    released.is_some()
}

#[tauri::command]
pub async fn app_instance_stop(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<bool, String> {
    require_viewer_or_main(&window)?;
    validate_ids(&worktree_id, &artifact_id)?;
    Ok(stop_instance(&app, &worktree_id, &artifact_id, "停止ボタン"))
}

/// `WindowEvent::Destroyed`（`lib.rs` の RunEvent）から呼ぶ。ウィンドウが外から壊れても
/// レジストリに幽霊を残さない。
pub fn on_window_destroyed(app: &AppHandle, label: &str) {
    if !label.starts_with(LABEL_PREFIX) {
        return;
    }
    // サブウィンドウが閉じても、インスタンスは止めない（所有者の管理から外して、アプリへ知らせるだけ）
    if crate::app_subwindow::on_window_destroyed(app, label) {
        return;
    }
    if let Some((wt, art)) = release(app, label) {
        add_log(app, &wt, &art, "info", "実行ウィンドウが閉じられたため停止しました");
        emit_changed(app, &wt, &art);
    }
}

/// アーティファクトの削除時（MCP `artifact delete`・UI の削除）。起動中なら止め、承認も消す。
/// **承認を残すと、同じ id で作り直した別物が同じ権限セットで承認なしに起動してしまう。**
pub async fn on_artifact_deleted(app: &AppHandle, worktree_id: &str, artifact_id: &str) {
    stop_instance(app, worktree_id, artifact_id, "アーティファクトが削除されました");
    // DB の接続が閉じるのを待ってからフォルダを消す（Windows は開いたままのファイルを消せない）
    app.state::<crate::app_db::AppDbRegistry>().drain_closes().await;
    if let Err(e) = app.state::<AppGrantStore>().remove(worktree_id, artifact_id) {
        log::warn!("[app-instance] grant の削除に失敗: {}", e);
    }
    // artifact:// の同梱領域も一緒に消す（停止の後。開いているハンドルが無くなってから）。
    // 残すと、同じ id で作り直した別物が前のアプリのファイルを引き継ぐ
    match app.path().app_data_dir() {
        Ok(dir) => {
            if let Err(e) = crate::app_fs::remove_artifact_files(&dir, worktree_id, artifact_id) {
                log::warn!("[app-instance] 同梱領域の削除に失敗: {}", e);
            }
        }
        Err(e) => log::warn!("[app-instance] app_data_dir を取得できません: {}", e),
    }
}

/// ワークツリーのアーティファクトを丸ごと消す経路（`delete_artifacts`）。
pub async fn on_worktree_artifacts_deleted(app: &AppHandle, worktree_id: &str) {
    let victims: Vec<String> = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(inner) = reg.0.lock() else { return };
        inner
            .by_label
            .values()
            .filter(|i| i.worktree_id == worktree_id)
            .map(|i| i.artifact_id.clone())
            .collect()
    };
    for art in victims {
        stop_instance(app, worktree_id, &art, "ワークツリーのアーティファクトが削除されました");
    }
    app.state::<crate::app_db::AppDbRegistry>().drain_closes().await;
    if let Err(e) = app.state::<AppGrantStore>().remove_worktree(worktree_id) {
        log::warn!("[app-instance] grant の一括削除に失敗: {}", e);
    }
    match app.path().app_data_dir() {
        Ok(dir) => {
            if let Err(e) = crate::app_fs::remove_worktree_artifact_files(&dir, worktree_id) {
                log::warn!("[app-instance] 同梱領域の一括削除に失敗: {}", e);
            }
        }
        Err(e) => log::warn!("[app-instance] app_data_dir を取得できません: {}", e),
    }
}

// ─── 権限の変更への対応（前提 1） ─────────────────────────────────────

/// `artifact` の create / rewrite が書かれた後に呼ぶ。起動中のインスタンスの manifest の権限が
/// 保存済みの承認と違えば、直ちに host 側で再承認ダイアログを出す。**コードの更新だけでは出ない**
/// （比較するのは権限セットだけ）。
///
/// 保留中も旧セットのまま動き続ける（関門は保存済みの grant だけを見る）。
/// 承認で入れ替えてリロード、拒否で停止。保留は 1 インスタンスにつき 1 件で最新が勝つ。
pub async fn check_permission_change(app: &AppHandle, worktree_id: &str, artifact_id: &str) {
    let Some(label) = label_for_key(app, worktree_id, artifact_id) else { return };
    let running = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(inner) = reg.0.lock() else { return };
        inner.by_label.get(&label).map(|i| i.state == InstanceState::Running).unwrap_or(false)
    };
    if !running {
        // Starting は start_inner の最後にもう一度見る
        return;
    }

    let art = match read_app_artifact(app, worktree_id, artifact_id) {
        Ok(a) => a,
        Err(e) => {
            add_log(app, worktree_id, artifact_id, "error", format!("manifest を読めないため権限の変更を検査できません: {}", e));
            return;
        }
    };
    let requested = art.manifest.permissions.clone();
    let Some(saved) = app.state::<AppGrantStore>().get(worktree_id, artifact_id) else {
        add_log(app, worktree_id, artifact_id, "error", "承認が見つからないため停止します");
        stop_instance(app, worktree_id, artifact_id, "承認の欠落");
        return;
    };

    if !app_manifest::requested_changed(&saved.requested, &requested) {
        // 保存済みの権限に戻った。保留があれば取り下げる
        let cancelled = {
            let reg = app.state::<AppInstanceRegistry>();
            let Ok(mut inner) = reg.0.lock() else { return };
            inner.cancel_pending(&label)
        };
        if let Some(id) = cancelled {
            emit_grant_cancelled(app, &[id]);
            add_log(app, worktree_id, artifact_id, "info", "権限が承認済みの内容に戻ったため、再承認の要求を取り下げました");
            emit_changed(app, worktree_id, artifact_id);
        }
        return;
    }

    // すでに同じ内容の要求が保留中なら、ダイアログを重ねない
    {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(inner) = reg.0.lock() else { return };
        let same = inner
            .by_label
            .get(&label)
            .and_then(|i| i.pending.as_ref())
            .map(|p| p.requested == requested)
            .unwrap_or(false);
        if same {
            return;
        }
    }

    let resolved = match build_resolved(app, worktree_id, artifact_id, &requested) {
        Ok(r) => r,
        Err(e) => {
            add_log(app, worktree_id, artifact_id, "error", format!("新しい権限を解決できません（旧セットのまま継続）: {}", e));
            return;
        }
    };
    let (request_id, rx, superseded) = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return };
        match inner.new_request(&label, requested.clone()) {
            Some(t) => t,
            None => return,
        }
    };
    if let Some(old) = superseded {
        emit_grant_cancelled(app, &[old]);
    }
    add_log(app, worktree_id, artifact_id, "warn", "manifest の権限が変わりました。再承認を待っています（承認までは旧セットで動作し、新しい権限が要る呼び出しは拒否されます）");
    emit_grant_request(app, &request_id, "change", worktree_id, artifact_id, &art, &resolved, Some(&saved.resolved));
    emit_changed(app, worktree_id, artifact_id);

    let app2 = app.clone();
    let wt = worktree_id.to_string();
    let aid = artifact_id.to_string();
    let rid = request_id.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = rx.await.unwrap_or(ApprovalOutcome::Cancelled);
        match outcome {
            ApprovalOutcome::Approved => {
                // 保留が自分のものかを確かめ、**同じロックの中で**保存する（確認から保存までの間に
                // 削除が入ると、削除側が消した grant を書き戻してしまうため）
                let saved: Option<Result<(), String>> = {
                    let reg = app2.state::<AppInstanceRegistry>();
                    let Ok(mut inner) = reg.0.lock() else { return };
                    match inner.by_label.get_mut(&label) {
                        Some(i) if i.pending.as_ref().map(|p| p.request_id == rid).unwrap_or(false) => {
                            i.pending = None;
                            let grant = AppGrant { granted_at: now_ms() / 1000, requested, resolved };
                            Some(app2.state::<AppGrantStore>().put(&wt, &aid, grant))
                        }
                        _ => None,
                    }
                };
                match saved {
                    None => return,
                    Some(Err(e)) => {
                        add_log(&app2, &wt, &aid, "error", format!("承認の保存に失敗したため停止します: {}", e));
                        stop_instance(&app2, &wt, &aid, "承認の保存失敗");
                        return;
                    }
                    Some(Ok(())) => {}
                }
                add_log(&app2, &wt, &aid, "info", "再承認されました");
                // 開いている DB は閉じる。次の呼び出しが新しい権限セットで開き直す
                crate::app_db::on_grant_replaced(&app2, &label);
                // 権限が狭まって収まらなくなったサブウィンドウは閉じる（残りは維持）
                crate::app_subwindow::on_grant_replaced(&app2, &label);
                // aiTerminal 権限が外れたなら、開いている専用 AI ターミナルを閉じる。権限が無いと
                // 以後は操作できず、持ち主の居ない端末が残ってしまうため（上限が下がっただけなら
                // 既存の端末は残し、新しく開く分から効く）
                let ai_revoked = app2
                    .state::<AppGrantStore>()
                    .get(&wt, &aid)
                    .map(|g| g.resolved.ai_terminal.is_none())
                    .unwrap_or(true);
                if ai_revoked {
                    crate::ai_terminal::close_all_for_instance(&app2, &(wt.clone(), aid.clone()));
                }
                // 上限が下がっていれば、ここで超過を知らせる（データは消さない）
                refresh_memory_bytes(&app2, &wt, &aid, true).await;
                request_reload(&app2, &label, "再承認");
            }
            ApprovalOutcome::Rejected => {
                add_log(&app2, &wt, &aid, "warn", "再承認が拒否されました");
                stop_instance(&app2, &wt, &aid, "再承認の拒否");
            }
            // 差し替え・取り下げは何も変えない（新しい要求 / 停止処理が引き継ぐ）
            ApprovalOutcome::Superseded | ApprovalOutcome::Cancelled => {}
        }
    });
}

/// 常駐ウィンドウのシェルへ iframe のリロードを依頼する。リロードの実体（旧 iframe への flush 要求・
/// 新しい nonce の採番・メモリーの読み直し）はシェルが行う。ウィンドウ自体は作り直さない
/// （承認・サブウィンドウなど host 側の資源はそのまま維持される）。
fn request_reload(app: &AppHandle, label: &str, reason: &str) {
    let key = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return };
        let Some(inst) = inner.by_label.get_mut(label) else { return };
        if inst.state != InstanceState::Running {
            return;
        }
        inst.reload_count += 1;
        inst.last_reload_at = Some(now_ms());
        (inst.worktree_id.clone(), inst.artifact_id.clone())
    };
    add_log(
        app,
        &key.0,
        &key.1,
        "info",
        format!("リロードします（{}）。承認とメモリーは維持されます", reason),
    );
    emit_changed(app, &key.0, &key.1);
    // 通信中のネットワークリクエストは中断する（前提 10。DB・端末・メモリは維持）。再承認の承認後も
    // ここを通る。古い iframe への応答は nonce で捨てられるので、中断は資源を空けるためのもの
    crate::app_net::on_reload(app, label);
    // 宛先はこのインスタンスのウィンドウだけ（常駐ウィンドウと、所有するサブウィンドウ）。
    // サブウィンドウは作り直さず、中の iframe だけが再接続する（前提 10）。他のインスタンスへ漏らさない
    crate::app_subwindow::emit_to_instance(app, label, "app-instance-reload", reason);
}

/// 起動中インスタンスの更新を、どの書き込み経路でも同じに扱うための直列化。
/// 同じインスタンスに対する `check_permission_change` が並走して、再承認ダイアログを二重に出さないため
static CHANGE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// アーティファクトが書き換わった（`artifact-changed`）ときの、起動中インスタンスへの反映。
///
/// 1. 権限セットが変わっていれば再承認を求める（`check_permission_change`）。保留中は旧セットで
///    動き続け、**リロードもしない**（新しいコードが新しい権限を前提にしているため。承認でセットを
///    入れ替えてからリロード、拒否で停止）。
/// 2. 権限が変わっていなければ、承認を保ったままリロードする。コード（content / modules）の更新は
///    どの経路で誰が書いても再承認を求めない（関門が見るのは保存済みの権限セットだけ）。
pub async fn on_artifact_changed(app: &AppHandle, worktree_id: &str, artifact_id: &str) {
    if label_for_key(app, worktree_id, artifact_id).is_none() {
        return;
    }
    let _guard = CHANGE_LOCK.lock().await;
    check_permission_change(app, worktree_id, artifact_id).await;

    let Some(label) = label_for_key(app, worktree_id, artifact_id) else { return };
    let (running, pending) = {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(inner) = reg.0.lock() else { return };
        match inner.by_label.get(&label) {
            Some(i) => (i.state == InstanceState::Running, i.pending.is_some()),
            None => return,
        }
    };
    if !running {
        // Starting。ウィンドウの読み込み時に最新を読むのでリロードは要らない
        return;
    }
    if pending {
        add_log(
            app,
            worktree_id,
            artifact_id,
            "info",
            "再承認の待ち中なので、コードの更新は承認後にリロードで反映します",
        );
        return;
    }
    // 権限が変わっているのに再承認の保留が立っていない（権限の解決に失敗した等）なら、リロードしない。
    // 旧権限のまま新しいコードを動かさないため（関門は保存済みの grant だけを見るので迂回にはならない）
    let unchanged = match (
        read_app_artifact(app, worktree_id, artifact_id),
        app.state::<AppGrantStore>().get(worktree_id, artifact_id),
    ) {
        (Ok(art), Some(saved)) => !app_manifest::requested_changed(&saved.requested, &art.manifest.permissions),
        _ => false,
    };
    if !unchanged {
        add_log(
            app,
            worktree_id,
            artifact_id,
            "warn",
            "権限の再承認を要求できなかったか manifest を読めないため、リロードしません（旧版のまま継続）",
        );
        return;
    }
    request_reload(app, &label, "コードの更新");
}

/// `artifact-changed` を拾って [`on_artifact_changed`] を呼ぶリスナーを張る。アプリ起動時に 1 回だけ。
/// 削除は `on_artifact_deleted`（停止と grant の削除）が別経路で扱う。
pub fn install_listeners(app: &AppHandle) {
    let handle = app.clone();
    app.listen("artifact-changed", move |event: tauri::Event| {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) else { return };
        let (Some(wt), Some(art)) = (payload["worktreeId"].as_str(), payload["artifactId"].as_str()) else {
            return;
        };
        if payload["command"].as_str() == Some("delete") {
            return;
        }
        if validate_ids(wt, art).is_err() {
            return;
        }
        let (h, wt, art) = (handle.clone(), wt.to_string(), art.to_string());
        tauri::async_runtime::spawn(async move { on_artifact_changed(&h, &wt, &art).await });
    });
}

// ─── メモリー（書き込みはここと MCP の artifact_store だけ。どちらも承認済みの maxBytes を強制する） ───

fn artifact_dir(app: &AppHandle, worktree_id: &str) -> Result<std::path::PathBuf, String> {
    crate::artifacts_dir_in(&app.path().app_data_dir().map_err(|e| e.to_string())?, worktree_id)
}

/// 承認済みの `maxBytes`。**保存済みの grant だけ**を見る（manifest は見ない）。
/// 未承認（memory を宣言していない・承認されていない）なら `Err`。
fn granted_memory_cap(app: &AppHandle, ctx: &InstanceContext) -> Result<u64, String> {
    let grant = app
        .state::<AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .ok_or_else(|| "このアプリには承認がありません".to_string())?;
    grant
        .resolved
        .memory
        .map(|m| m.max_bytes)
        .filter(|&b| b > 0)
        .ok_or_else(|| {
            "このアプリには memory 権限が承認されていません（manifest で permissions.memory.maxBytes を宣言し、承認ダイアログで許可されたアプリだけがメモリーを使えます）".to_string()
        })
}

fn read_memory_blocking(dir: std::path::PathBuf, artifact_id: String) -> serde_json::Value {
    crate::read_artifact_store(&dir, &artifact_id)
        .0
        .unwrap_or_else(|| serde_json::json!({}))
}

/// メモリーの使用量を測ってレジストリへ反映し、起動画面へ通知する。起動中でなければ何もしない。
/// `warn_if_over` のときは、承認済みの上限を超えていればデバッグログに残す
/// （上限を下げる再承認のあと。データは消さず、収まらない書き込みだけが拒否される）。
pub async fn refresh_memory_bytes(app: &AppHandle, worktree_id: &str, artifact_id: &str, warn_if_over: bool) {
    let Some(label) = label_for_key(app, worktree_id, artifact_id) else { return };
    let Ok(dir) = artifact_dir(app, worktree_id) else { return };
    let id = artifact_id.to_string();
    let Ok(memory) = tokio::task::spawn_blocking(move || read_memory_blocking(dir, id)).await else {
        return;
    };
    let bytes = serde_json::to_string(&memory).map(|s| s.len() as u64).unwrap_or(0);
    {
        let reg = app.state::<AppInstanceRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return };
        match inner.by_label.get_mut(&label) {
            Some(i) => i.memory_bytes = Some(bytes),
            None => return,
        }
    }
    if warn_if_over {
        let cap = app
            .state::<AppGrantStore>()
            .get(worktree_id, artifact_id)
            .and_then(|g| g.resolved.memory)
            .map(|m| m.max_bytes);
        if let Some(cap) = cap.filter(|&c| bytes > c) {
            add_log(
                app,
                worktree_id,
                artifact_id,
                "warn",
                format!(
                    "保存済みのメモリー（{} バイト）が承認済みの最大サイズ（{} バイト）を超えています。データは消しません。サイズが収まるまで書き込みは拒否され、読み取りはできます",
                    bytes, cap
                ),
            );
        }
    }
    emit_changed(app, worktree_id, artifact_id);
}

async fn write_app_memory(
    app: &AppHandle,
    ctx: &InstanceContext,
    memory: serde_json::Value,
) -> Result<(), String> {
    let cap = granted_memory_cap(app, ctx)?;
    // null は「リセット」（キーごと削除）。オブジェクト以外と host の絶対上限はここで弾かれる
    let validated = crate::validate_artifact_memory(Some(memory))?;
    if let Some(v) = validated.as_ref() {
        app_manifest::check_memory_size(v, cap)?;
    }
    let dir = artifact_dir(app, &ctx.worktree_id)?;
    crate::write_artifact_memory(dir, &ctx.artifact_id, validated, None).await.map(|_| ())
}

/// アプリ（iframe）のメモリー保存。呼び出し元はウィンドウラベルから決める。
/// 拒否はデバッグログへ残し、iframe にもエラーで返す。
#[tauri::command]
pub async fn app_set_memory(
    window: tauri::Window,
    app: AppHandle,
    memory: serde_json::Value,
) -> Result<(), String> {
    require_instance_window(&window)?;
    let ctx = context_for_label(&app, window.label())?;
    match write_app_memory(&app, &ctx, memory).await {
        Ok(()) => {
            refresh_memory_bytes(&app, &ctx.worktree_id, &ctx.artifact_id, false).await;
            Ok(())
        }
        Err(e) => {
            add_log(&app, &ctx.worktree_id, &ctx.artifact_id, "warn", format!("メモリーの書き込みを拒否: {}", e));
            Err(e)
        }
    }
}

/// 外から（MCP の `artifact_store`）書き換わったメモリーを、シェルが iframe へ押し込むために読む。
#[tauri::command]
pub async fn app_get_memory(window: tauri::Window, app: AppHandle) -> Result<serde_json::Value, String> {
    require_instance_window(&window)?;
    let ctx = context_for_label(&app, window.label())?;
    granted_memory_cap(&app, &ctx)?;
    let dir = artifact_dir(&app, &ctx.worktree_id)?;
    let id = ctx.artifact_id.clone();
    tokio::task::spawn_blocking(move || read_memory_blocking(dir, id))
        .await
        .map_err(|e| format!("task join error: {}", e))
}

// ─── ビューア（起動画面）向けコマンド ─────────────────────────────────

#[tauri::command]
pub async fn app_instance_toggle_visible(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<bool, String> {
    require_viewer(&window)?;
    validate_ids(&worktree_id, &artifact_id)?;
    let label = label_for_key(&app, &worktree_id, &artifact_id)
        .ok_or_else(|| "このアプリは起動していません".to_string())?;
    let w = app
        .get_webview_window(&label)
        .ok_or_else(|| "実行ウィンドウが見つかりません".to_string())?;
    let visible = w.is_visible().unwrap_or(false);
    if visible {
        let _ = w.hide();
    } else {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    set_visible(&app, &label, !visible);
    Ok(!visible)
}

#[tauri::command]
pub async fn app_instance_status(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<serde_json::Value, String> {
    require_viewer(&window)?;
    validate_ids(&worktree_id, &artifact_id)?;
    let reg = app.state::<AppInstanceRegistry>();
    let store = app.state::<AppGrantStore>();
    let inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
    Ok(status_value(&inner, &store, &worktree_id, &artifact_id))
}

#[tauri::command]
pub async fn app_instance_log(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<Vec<LogEntry>, String> {
    require_viewer(&window)?;
    validate_ids(&worktree_id, &artifact_id)?;
    let reg = app.state::<AppInstanceRegistry>();
    let inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
    Ok(inner
        .logs
        .get(&(worktree_id, artifact_id))
        .map(|b| b.iter().cloned().collect())
        .unwrap_or_default())
}

/// 起動画面が出す「承認済みの権限」の要約（表示専用）。
#[tauri::command]
pub async fn app_instance_grant(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<Option<AppGrant>, String> {
    require_viewer(&window)?;
    validate_ids(&worktree_id, &artifact_id)?;
    Ok(app.state::<AppGrantStore>().get(&worktree_id, &artifact_id))
}

/// ホームの「アプリ一覧」の 1 行。アプリ形式のアーティファクト（承認の有無を問わない）と、
/// アーティファクトが消えたあとも残っている承認（取り消せるように）の両方を載せる。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInstanceListItem {
    worktree_id: String,
    worktree_name: String,
    artifact_id: String,
    artifact_title: String,
    /// manifest の表示名。アーティファクトが消えて承認だけ残っているときは空
    app_name: String,
    granted: bool,
    granted_at: Option<u64>,
    resolved: Option<ResolvedPermissions>,
    /// 起動処理中（承認ダイアログ待ち）か起動済みか。停止中は null
    state: Option<InstanceState>,
    running: bool,
    /// 起動中の権限変更が再承認待ち
    reapproval_pending: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInstanceList {
    items: Vec<AppInstanceListItem>,
    /// 承認ダイアログと同じく「ユーザーと同等の権限」として強調するツール
    dangerous_tools: &'static [&'static str],
}

/// アプリ一覧（ホーム画面）。**表示専用**で、承認や起動の権限は与えない。
#[tauri::command]
pub async fn list_app_instances(window: tauri::Window, app: AppHandle) -> Result<AppInstanceList, String> {
    require_viewer_or_main(&window)?;
    let settings = app.state::<crate::settings::SettingsManager>().get();
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let grants = app.state::<AppGrantStore>().list();
    // (state, reapproval_pending) をレジストリから引く
    let live: HashMap<Key, (InstanceState, bool)> = {
        let reg = app.state::<AppInstanceRegistry>();
        let inner = reg.0.lock().map_err(|_| "レジストリのロックが壊れています".to_string())?;
        inner
            .by_key
            .iter()
            .filter_map(|(k, label)| {
                inner
                    .by_label
                    .get(label)
                    .map(|i| (k.clone(), (i.state, i.state == InstanceState::Running && i.pending.is_some())))
            })
            .collect()
    };
    let worktrees: Vec<(String, String)> = settings.worktrees.iter().map(|w| (w.id.clone(), w.name.clone())).collect();

    let items = tokio::task::spawn_blocking(move || {
        let name_of = |id: &str| {
            worktrees
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let mut grant_map: HashMap<Key, AppGrant> =
            grants.into_iter().map(|(w, a, g)| ((w, a), g)).collect();
        let mut out: Vec<AppInstanceListItem> = Vec::new();
        let mut push = |wt: &str, art: &str, title: String, app_name: String, grant: Option<AppGrant>| {
            let key: Key = (wt.to_string(), art.to_string());
            let st = live.get(&key).copied();
            out.push(AppInstanceListItem {
                worktree_id: wt.to_string(),
                worktree_name: name_of(wt),
                artifact_id: art.to_string(),
                artifact_title: title,
                app_name,
                granted: grant.is_some(),
                granted_at: grant.as_ref().map(|g| g.granted_at),
                resolved: grant.map(|g| g.resolved),
                state: st.map(|s| s.0),
                running: st.map(|s| s.0 == InstanceState::Running).unwrap_or(false),
                reapproval_pending: st.map(|s| s.1).unwrap_or(false),
            });
        };
        for (wt_id, _) in &worktrees {
            let Ok(dir) = crate::artifacts_dir_in(&data_dir, wt_id) else { continue };
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(art_id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
                if crate::validate_path_component(&art_id).is_err() {
                    continue;
                }
                // 1 件が壊れていても他を出す（一覧は表示専用）
                let Ok(raw) = std::fs::read_to_string(&path) else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
                if v.get("type").and_then(|t| t.as_str()) != Some(REACT_TYPE) {
                    continue;
                }
                let Some(app_value) = v.get("app").filter(|a| !a.is_null()) else { continue };
                let Ok(manifest) = app_manifest::parse_manifest(app_value) else { continue };
                let title = v.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let grant = grant_map.remove(&(wt_id.clone(), art_id.clone()));
                push(wt_id, &art_id, title, manifest.name, grant);
            }
        }
        // アーティファクトが無くなっても残っている承認は、取り消せるように載せる
        let mut orphans: Vec<(Key, AppGrant)> = grant_map.into_iter().collect();
        orphans.sort_by(|a, b| a.0.cmp(&b.0));
        for ((wt, art), g) in orphans {
            push(&wt, &art, String::new(), String::new(), Some(g));
        }
        out.sort_by(|a, b| {
            (&a.worktree_name, &a.artifact_title, &a.artifact_id).cmp(&(&b.worktree_name, &b.artifact_title, &b.artifact_id))
        });
        out
    })
    .await
    .map_err(|e| format!("task join error: {}", e))?;
    Ok(AppInstanceList { items, dangerous_tools: app_manifest::DANGEROUS_TOOLS })
}

/// 承認を取り消す。起動中なら止める。
#[tauri::command]
pub async fn revoke_app_grant(
    window: tauri::Window,
    app: AppHandle,
    worktree_id: String,
    artifact_id: String,
) -> Result<bool, String> {
    require_viewer_or_main(&window)?;
    validate_ids(&worktree_id, &artifact_id)?;
    stop_instance(&app, &worktree_id, &artifact_id, "承認の取り消し");
    let removed = app.state::<AppGrantStore>().remove(&worktree_id, &artifact_id)?;
    add_log(&app, &worktree_id, &artifact_id, "info", "承認を取り消しました");
    emit_changed(&app, &worktree_id, &artifact_id);
    Ok(removed)
}

// ─── 常駐ウィンドウ（シェル）向けコマンド。引数でインスタンスを指定させない ──────

/// シェルが描画に使う材料。**ラベルから**インスタンスを引くので、他のアプリの中身は読めない。
#[tauri::command]
pub async fn app_instance_bootstrap(window: tauri::Window, app: AppHandle) -> Result<serde_json::Value, String> {
    require_instance_window(&window)?;
    let ctx = context_for_label(&app, window.label())?;
    let art = read_app_artifact(&app, &ctx.worktree_id, &ctx.artifact_id)?;
    // memory 権限が承認されていないアプリには、保存済みのデータがあっても渡さない（空で始める）
    let memory = if granted_memory_cap(&app, &ctx).is_ok() {
        let dir = artifact_dir(&app, &ctx.worktree_id)?;
        let id = ctx.artifact_id.clone();
        tokio::task::spawn_blocking(move || read_memory_blocking(dir, id))
            .await
            .map_err(|e| format!("task join error: {}", e))?
    } else {
        serde_json::json!({})
    };
    Ok(serde_json::json!({
        "memory": memory,
        "worktreeId": ctx.worktree_id,
        "artifactId": ctx.artifact_id,
        "title": art.title,
        "appName": art.manifest.name,
        "content": art.raw.get("content").cloned().unwrap_or(serde_json::Value::String(String::new())),
        "modules": art.raw.get("modules").cloned().unwrap_or_else(|| serde_json::json!({})),
        // サブウィンドウのシェルだけに入る（常駐ウィンドウは null）。アプリは `oretachi.subWindow.info` で読む
        "subWindow": crate::app_subwindow::info_for_label(&app, window.label()),
    }))
}

/// アプリからの MCP ツール呼び出し。関門は `mcp_server::call_tool_for_app_instance`。
/// 拒否も許可もデバッグログに残す。
#[tauri::command]
pub async fn app_call_mcp_tool(
    window: tauri::Window,
    app: AppHandle,
    tool: String,
    params: serde_json::Value,
) -> Result<String, String> {
    require_instance_window(&window)?;
    let ctx = context_for_label(&app, window.label())?;
    let Some(grant) = app.state::<AppGrantStore>().get(&ctx.worktree_id, &ctx.artifact_id) else {
        add_log(&app, &ctx.worktree_id, &ctx.artifact_id, "error", format!("拒否: {}（承認が見つかりません）", tool));
        return Err("このアプリには承認がありません".to_string());
    };
    let result = crate::mcp_server::call_tool_for_app_instance(
        &app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        &grant.resolved,
        &tool,
        params,
    )
    .await;
    match &result {
        Ok(_) => add_log(&app, &ctx.worktree_id, &ctx.artifact_id, "info", format!("許可: {}", tool)),
        Err(e) => {
            // 再承認待ちの間に新しい権限が要る呼び出しをしたときは、理由が分かるよう付記する
            let reapproval = if ctx.reapproval_pending {
                "（再承認待ちのため、新しい権限はまだ使えません）"
            } else {
                ""
            };
            add_log(&app, &ctx.worktree_id, &ctx.artifact_id, "warn", format!("拒否: {} — {}{}", tool, e, reapproval));
        }
    }
    result
}

/// iframe 内の未処理エラーなどをデバッグログへ。
#[tauri::command]
pub async fn app_instance_report(
    window: tauri::Window,
    app: AppHandle,
    level: String,
    message: String,
) -> Result<(), String> {
    require_instance_window(&window)?;
    let ctx = context_for_label(&app, window.label())?;
    let level = match level.as_str() {
        "error" | "warn" => level.as_str(),
        _ => "info",
    };
    let clipped: String = message.chars().take(2000).collect();
    add_log(&app, &ctx.worktree_id, &ctx.artifact_id, level, clipped);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_manifest::{MemoryPermission, RequestedPermissions};

    fn perms(max: u64) -> RequestedPermissions {
        RequestedPermissions { mcp: None, memory: Some(MemoryPermission { max_bytes: max }), ai_terminal: None, fs: None, db: None, network: None, sub_window: None }
    }

    #[test]
    fn rejects_double_start_of_same_instance() {
        let mut inner = Inner::default();
        assert!(inner.reserve("wt", "a").is_ok());
        assert!(inner.reserve("wt", "a").unwrap_err().contains("既に起動中"));
    }

    #[test]
    fn different_artifacts_can_run_together() {
        let mut inner = Inner::default();
        let a = inner.reserve("wt", "a").unwrap();
        let b = inner.reserve("wt", "b").unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(LABEL_PREFIX));
    }

    #[test]
    fn enforces_instance_cap_and_frees_slot_on_remove() {
        let mut inner = Inner::default();
        let mut labels = Vec::new();
        for i in 0..MAX_INSTANCES {
            labels.push(inner.reserve("wt", &format!("a{}", i)).unwrap());
        }
        assert!(inner.reserve("wt", "extra").unwrap_err().contains("まで"));
        inner.remove(&labels[0]);
        assert!(inner.reserve("wt", "extra").is_ok());
    }

    #[test]
    fn labels_are_never_reused() {
        let mut inner = Inner::default();
        let a = inner.reserve("wt", "a").unwrap();
        inner.remove(&a);
        let b = inner.reserve("wt", "a").unwrap();
        assert_ne!(a, b, "古いウィンドウのラベルが新しいインスタンスに引き継がれてはいけない");
    }

    #[test]
    fn newer_request_supersedes_older_one() {
        let mut inner = Inner::default();
        let label = inner.reserve("wt", "a").unwrap();
        let (id1, mut rx1, old1) = inner.new_request(&label, perms(1)).unwrap();
        assert!(old1.is_none());
        let (id2, _rx2, old2) = inner.new_request(&label, perms(2)).unwrap();
        assert_eq!(old2.as_deref(), Some(id1.as_str()));
        assert_eq!(rx1.try_recv().unwrap(), ApprovalOutcome::Superseded);
        // 古い要求への応答は無効
        assert!(inner.answer(&id1, ApprovalOutcome::Approved).is_none());
        assert!(inner.answer(&id2, ApprovalOutcome::Approved).is_some());
    }

    #[test]
    fn remove_cancels_pending_request_instead_of_rejecting() {
        let mut inner = Inner::default();
        let label = inner.reserve("wt", "a").unwrap();
        let (id, mut rx, _) = inner.new_request(&label, perms(1)).unwrap();
        let (_, cancelled) = inner.remove(&label).unwrap();
        assert_eq!(cancelled, vec![id]);
        assert_eq!(rx.try_recv().unwrap(), ApprovalOutcome::Cancelled);
    }

    #[test]
    fn dropped_sender_is_not_a_rejection() {
        // start_inner / 再承認タスクは oneshot の Err を Cancelled として扱う
        let mut inner = Inner::default();
        let label = inner.reserve("wt", "a").unwrap();
        let (id, rx, _) = inner.new_request(&label, perms(1)).unwrap();
        inner.waiters.remove(&id);
        assert!(rx.blocking_recv().is_err());
    }

    #[test]
    fn cancel_pending_clears_state() {
        let mut inner = Inner::default();
        let label = inner.reserve("wt", "a").unwrap();
        let (id, mut rx, _) = inner.new_request(&label, perms(1)).unwrap();
        assert_eq!(inner.cancel_pending(&label), Some(id));
        assert_eq!(rx.try_recv().unwrap(), ApprovalOutcome::Cancelled);
        assert!(inner.cancel_pending(&label).is_none());
    }

    #[test]
    fn log_buffer_is_a_ring() {
        let mut inner = Inner::default();
        let key: Key = ("wt".into(), "a".into());
        for i in 0..(LOG_CAPACITY + 10) {
            inner.push_log(&key, "info", format!("m{}", i));
        }
        let buf = inner.logs.get(&key).unwrap();
        assert_eq!(buf.len(), LOG_CAPACITY);
        assert_eq!(buf.front().unwrap().message, "m10");
    }
}
