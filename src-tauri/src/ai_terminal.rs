//! インスタンス専用 AI ターミナルの所有レジストリ（#372）。
//!
//! アプリ型アーティファクトの常駐インスタンスが、ワークツリーに「自分専用の AI セッション端末」を
//! 開いて使うための**所有の単一の真実**。`is_ai_agent` は PTY の監視で自動判定されるので専用端末にも
//! 立つ。そのため「所有している」という印は別にここで持ち、各所の除外判定はこのレジストリを引く。
//!
//! # 流れ
//!
//! 1. host が `terminal_id`（UUID）を採番して [`Inner::reserve`] で予約する（PTY 起動の**前**）。
//!    SessionStart フックは `claude` の起動直後に走るので、起動後に登録すると最初のフックに
//!    グループ向けの内容が注入されてしまう。
//! 2. `pty_spawn` が予約を一度だけ消費し（[`Inner::take_reservation`]）、spawn が返した
//!    session_id をその場で紐付ける（[`Inner::bind`]）。**紐付けはフロントの申告に頼らない。**
//!    偽の session_id を申告されると、メインの端末を所有端末として操作できてしまうため。
//! 3. 以降、すべての操作は [`Inner::resolve_owned`] を通す。自分が開いていない端末は拒否する。
//!
//! レジストリはメモリのみ。アプリの再起動で空になる（専用端末は復元しない）。

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// インスタンス Key = (worktree_id, artifact_id)。`app_instance` の `Key` と同じ。
pub type Key = (String, String);

/// 全インスタンスの合計上限。既知の webview ハングは端末数 15 以上と強く相関するので、
/// 専用端末だけで低めに頭打ちにする。
pub const GLOBAL_MAX: usize = 8;

/// 予約したまま PTY が起動しなかったエントリを掃除するまでの時間。
const RESERVATION_TTL: Duration = Duration::from_secs(30);

pub const MAX_TITLE_CHARS: usize = 100;
pub const MAX_PROMPT_CHARS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// 予約済み。`pty_spawn` がまだ消費していない。
    Reserved,
    /// `pty_spawn` が消費済みで、PTY を起動中。
    Spawning,
    Bound(u32),
}

#[derive(Debug, Clone)]
struct Entry {
    key: Key,
    app_name: String,
    title: String,
    system_prompt: String,
    /// host が解決済みの作業ディレクトリ。予約 ID での `pty_spawn` はフロントの申告ではなくこれを使う。
    cwd: String,
    phase: Phase,
    created: Instant,
}

/// 端末の所有者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub worktree_id: String,
    pub artifact_id: String,
    pub app_name: String,
}

/// フロントへ渡す、所有端末 1 件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedTerminal {
    pub terminal_id: String,
    pub session_id: u32,
    pub worktree_id: String,
    pub artifact_id: String,
    pub app_name: String,
    pub title: String,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
}

impl Inner {
    /// 生きていない端末と、古い予約を落とす。
    fn prune(&mut self, alive: &HashSet<u32>, now: Instant) {
        self.entries.retain(|_, e| match e.phase {
            Phase::Reserved | Phase::Spawning => now.duration_since(e.created) < RESERVATION_TTL,
            Phase::Bound(sid) => alive.contains(&sid),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn reserve(
        &mut self,
        key: &Key,
        app_name: &str,
        title: &str,
        system_prompt: &str,
        cwd: &str,
        max_concurrent: u32,
        alive: &HashSet<u32>,
        now: Instant,
    ) -> Result<String, String> {
        self.prune(alive, now);
        let mine = self.entries.values().filter(|e| &e.key == key).count();
        if mine >= max_concurrent as usize {
            return Err(format!(
                "このアプリが同時に開ける AI ターミナルは {} 個までです（承認済みの maxConcurrent）",
                max_concurrent
            ));
        }
        if self.entries.len() >= GLOBAL_MAX {
            return Err(format!(
                "インスタンス専用の AI ターミナルは全体で {} 個までです。不要な端末を閉じてください",
                GLOBAL_MAX
            ));
        }
        let terminal_id = uuid::Uuid::new_v4().to_string();
        self.entries.insert(
            terminal_id.clone(),
            Entry {
                key: key.clone(),
                app_name: app_name.to_string(),
                title: title.to_string(),
                system_prompt: system_prompt.to_string(),
                cwd: cwd.to_string(),
                phase: Phase::Reserved,
                created: now,
            },
        );
        Ok(terminal_id)
    }

    /// 予約済み・未消費の ID だけを受理して、消費済みにする。一回限り。戻り値は host が解決した cwd。
    fn take_reservation(&mut self, terminal_id: &str) -> Result<String, String> {
        match self.entries.get_mut(terminal_id) {
            Some(e) if e.phase == Phase::Reserved => {
                e.phase = Phase::Spawning;
                Ok(e.cwd.clone())
            }
            _ => Err("予約されていない terminal_id です".to_string()),
        }
    }

    fn bind(&mut self, terminal_id: &str, session_id: u32) -> bool {
        match self.entries.get_mut(terminal_id) {
            Some(e) if e.phase == Phase::Spawning => {
                e.phase = Phase::Bound(session_id);
                true
            }
            _ => false,
        }
    }

    /// エントリを外す。紐付け済みだった session_id を返す。
    fn cancel(&mut self, terminal_id: &str) -> Option<u32> {
        match self.entries.remove(terminal_id)?.phase {
            Phase::Bound(sid) => Some(sid),
            _ => None,
        }
    }

    fn owner_of(&self, terminal_id: &str) -> Option<Owner> {
        self.entries.get(terminal_id).map(|e| Owner {
            worktree_id: e.key.0.clone(),
            artifact_id: e.key.1.clone(),
            app_name: e.app_name.clone(),
        })
    }

    fn system_prompt(&self, terminal_id: &str) -> Option<String> {
        self.entries.get(terminal_id).map(|e| e.system_prompt.clone())
    }

    /// 自分（`key`）が開いた、紐付け済みの端末だけを解決する。他インスタンス・メインの端末は拒否。
    fn resolve_owned(&self, key: &Key, terminal_id: &str) -> Result<u32, String> {
        match self.entries.get(terminal_id) {
            Some(e) if &e.key == key => match e.phase {
                Phase::Bound(sid) => Ok(sid),
                _ => Err("この端末はまだ起動中です".to_string()),
            },
            // 存在しない・他人の端末は区別せず同じ文言にする（他インスタンスの端末の有無を漏らさない）
            _ => Err("この terminal_id は、このアプリが開いた AI ターミナルではありません".to_string()),
        }
    }

    fn list_for(&self, key: &Key) -> Vec<OwnedTerminal> {
        let mut v: Vec<OwnedTerminal> = self
            .entries
            .iter()
            .filter(|(_, e)| &e.key == key)
            .filter_map(|(tid, e)| Self::owned_view(tid, e))
            .collect();
        v.sort_by_key(|t| t.session_id);
        v
    }

    fn list_all(&self) -> Vec<OwnedTerminal> {
        let mut v: Vec<OwnedTerminal> =
            self.entries.iter().filter_map(|(tid, e)| Self::owned_view(tid, e)).collect();
        v.sort_by_key(|t| t.session_id);
        v
    }

    fn owned_view(tid: &str, e: &Entry) -> Option<OwnedTerminal> {
        match e.phase {
            Phase::Bound(sid) => Some(OwnedTerminal {
                terminal_id: tid.to_string(),
                session_id: sid,
                worktree_id: e.key.0.clone(),
                artifact_id: e.key.1.clone(),
                app_name: e.app_name.clone(),
                title: e.title.clone(),
            }),
            _ => None,
        }
    }

    /// インスタンスの全エントリを外す。紐付け済みだった session_id を返す。
    fn remove_for_instance(&mut self, key: &Key) -> Vec<u32> {
        let mut sids = Vec::new();
        self.entries.retain(|_, e| {
            if &e.key != key {
                return true;
            }
            if let Phase::Bound(sid) = e.phase {
                sids.push(sid);
            }
            false
        });
        sids
    }
}

pub struct AiTerminalRegistry(Mutex<Inner>);

impl Default for AiTerminalRegistry {
    fn default() -> Self {
        Self(Mutex::new(Inner::default()))
    }
}

/// 生きている PTY セッションの session_id 集合。
fn alive_sessions(app: &AppHandle) -> HashSet<u32> {
    app.state::<crate::pty_manager::PtyManager>()
        .list_sessions()
        .into_iter()
        .filter(|s| s.exit_code.is_none())
        .map(|s| s.session_id)
        .collect()
}

fn emit_owned_changed(app: &AppHandle) {
    let _ = app.emit("ai-terminal-owned-changed", ());
}

/// 予約する（PTY 起動の前）。戻り値は採番した `terminal_id`。
pub fn reserve(
    app: &AppHandle,
    key: &Key,
    app_name: &str,
    title: &str,
    system_prompt: &str,
    cwd: &str,
    max_concurrent: u32,
) -> Result<String, String> {
    let alive = alive_sessions(app);
    let reg = app.state::<AiTerminalRegistry>();
    let mut inner = reg.0.lock().map_err(|e| format!("lock error: {}", e))?;
    inner.reserve(key, app_name, title, system_prompt, cwd, max_concurrent, &alive, Instant::now())
}

/// `pty_spawn` から: 予約を一度だけ消費する。戻り値は host が解決した cwd。
pub fn take_reservation(app: &AppHandle, terminal_id: &str) -> Result<String, String> {
    let reg = app.state::<AiTerminalRegistry>();
    let mut inner = reg.0.lock().map_err(|e| format!("lock error: {}", e))?;
    inner.take_reservation(terminal_id)
}

/// `pty_spawn` から: spawn が返した session_id を紐付ける。
pub fn bind(app: &AppHandle, terminal_id: &str, session_id: u32) -> bool {
    let ok = {
        let reg = app.state::<AiTerminalRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return false };
        inner.bind(terminal_id, session_id)
    };
    if ok {
        emit_owned_changed(app);
    }
    ok
}

/// 予約・紐付けを取り消す。紐付け済みだった session_id を返す（呼び出し側で PTY を kill する）。
pub fn cancel(app: &AppHandle, terminal_id: &str) -> Option<u32> {
    let sid = {
        let reg = app.state::<AiTerminalRegistry>();
        let mut inner = reg.0.lock().ok()?;
        inner.cancel(terminal_id)
    };
    emit_owned_changed(app);
    sid
}

/// 所有端末か（**どの phase でも**真）。フックの除外判定が使う。
pub fn is_owned(app: &AppHandle, terminal_id: &str) -> bool {
    owner_of(app, terminal_id).is_some()
}

pub fn owner_of(app: &AppHandle, terminal_id: &str) -> Option<Owner> {
    let reg = app.try_state::<AiTerminalRegistry>()?;
    let inner = reg.0.lock().ok()?;
    inner.owner_of(terminal_id)
}

/// SessionStart で注入する、インスタンス指定のプロンプト。
pub fn system_prompt(app: &AppHandle, terminal_id: &str) -> Option<String> {
    let reg = app.try_state::<AiTerminalRegistry>()?;
    let inner = reg.0.lock().ok()?;
    inner.system_prompt(terminal_id)
}

pub fn resolve_owned(app: &AppHandle, key: &Key, terminal_id: &str) -> Result<u32, String> {
    let reg = app.state::<AiTerminalRegistry>();
    let inner = reg.0.lock().map_err(|e| format!("lock error: {}", e))?;
    inner.resolve_owned(key, terminal_id)
}

/// 自インスタンスの生きている端末。
pub fn list_for(app: &AppHandle, key: &Key) -> Vec<OwnedTerminal> {
    let alive = alive_sessions(app);
    let reg = app.state::<AiTerminalRegistry>();
    let Ok(mut inner) = reg.0.lock() else { return Vec::new() };
    inner.prune(&alive, Instant::now());
    inner.list_for(key)
}

/// すべての所有端末（フロントのタブ表示・除外用）。
pub fn list_all(app: &AppHandle) -> Vec<OwnedTerminal> {
    let alive = alive_sessions(app);
    let reg = app.state::<AiTerminalRegistry>();
    let Ok(mut inner) = reg.0.lock() else { return Vec::new() };
    inner.prune(&alive, Instant::now());
    inner.list_all()
}

/// 所有端末の session_id 集合。`pty_manager` の rebind 抑止など、`SessionInfo` から判定する側が使う。
pub fn owned_terminal_ids(app: &AppHandle) -> HashSet<String> {
    let Some(reg) = app.try_state::<AiTerminalRegistry>() else { return HashSet::new() };
    let Ok(inner) = reg.0.lock() else { return HashSet::new() };
    inner.entries.keys().cloned().collect()
}

/// インスタンス停止: 開いた端末をすべて閉じる。`app_instance::release` から呼ぶ。
pub fn close_all_for_instance(app: &AppHandle, key: &Key) {
    let sids = {
        let reg = app.state::<AiTerminalRegistry>();
        let Ok(mut inner) = reg.0.lock() else { return };
        inner.remove_for_instance(key)
    };
    emit_owned_changed(app);
    kill_sessions(app, sids, "app-instance-stop");
}

/// PTY の kill は子プロセスの待ち合わせを含むので、呼び出し元（async コマンド等）を塞がない。
pub fn kill_sessions(app: &AppHandle, sids: Vec<u32>, source: &'static str) {
    if sids.is_empty() {
        return;
    }
    let manager = app.state::<crate::pty_manager::PtyManager>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        for sid in sids {
            if let Err(e) = manager.kill(sid, source) {
                log::debug!("[ai-terminal] kill session={} failed: {}", sid, e);
            }
        }
    });
}

/// 走行中の `open` が待っている、フロントからの「PTY を起動した／失敗した」の合図。
#[derive(Default)]
pub struct SpawnWaiters(Mutex<HashMap<String, tokio::sync::oneshot::Sender<Result<(), String>>>>);

impl SpawnWaiters {
    pub fn register(&self, request_id: &str) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Ok(mut m) = self.0.lock() {
            m.insert(request_id.to_string(), tx);
        }
        rx
    }

    pub fn remove(&self, request_id: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(request_id);
        }
    }

    pub fn resolve(&self, request_id: &str, result: Result<(), String>) -> bool {
        let tx = self.0.lock().ok().and_then(|mut m| m.remove(request_id));
        match tx {
            Some(tx) => tx.send(result).is_ok(),
            None => false,
        }
    }
}

// ─── アプリ（`appinst-*`）向けコマンド ────────────────────────────────────

use crate::app_grant::AppGrantStore;
use crate::app_instance::{self, add_log, InstanceContext};
use crate::app_manifest::AiTerminalPermission;
use serde::Deserialize;
use serde_json::{json, Value};

/// open の待ち時間。フロントが PTY を起動して返す（ready は待たない）までの上限。
const OPEN_TIMEOUT: Duration = Duration::from_secs(8);

fn instance_ctx(app: &AppHandle, window: &tauri::Window) -> Result<InstanceContext, String> {
    app_instance::require_instance_window(window)?;
    app_instance::context_for_label(app, window.label())
}

/// 保存済みの grant（host 側）から `aiTerminal` 権限を引く。アーティファクト JSON の manifest は見ない。
fn granted(app: &AppHandle, ctx: &InstanceContext) -> Result<AiTerminalPermission, String> {
    app.state::<AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .and_then(|g| g.resolved.ai_terminal)
        .ok_or_else(|| "このアプリには aiTerminal 権限が承認されていません".to_string())
}

fn key_of(ctx: &InstanceContext) -> Key {
    (ctx.worktree_id.clone(), ctx.artifact_id.clone())
}

/// ツールの結果文字列。JSON として読めれば構造化して返す。
fn to_value(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn short(tid: &str) -> &str {
    tid.get(..8).unwrap_or(tid)
}

/// 許可・拒否をデバッグログに残して結果を返す（`app_call_mcp_tool` と同じ書式）。
fn logged<T>(app: &AppHandle, ctx: &InstanceContext, what: &str, r: Result<T, String>) -> Result<T, String> {
    match &r {
        Ok(_) => add_log(app, &ctx.worktree_id, &ctx.artifact_id, "info", format!("許可: aiTerminal.{}", what)),
        Err(e) => {
            let note = if ctx.reapproval_pending {
                "（再承認待ちのため、新しい権限はまだ使えません）"
            } else {
                ""
            };
            add_log(app, &ctx.worktree_id, &ctx.artifact_id, "warn", format!("拒否: aiTerminal.{} — {}{}", what, e, note));
        }
    }
    r
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenParams {
    pub title: Option<String>,
    pub system_prompt: Option<String>,
    pub initial_prompt: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
}

fn check_text(what: &str, s: &str) -> Result<(), String> {
    if s.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!("{} は {} 文字以内にしてください", what, MAX_PROMPT_CHARS));
    }
    if s.contains('\0') {
        return Err(format!("{} に NUL は使えません", what));
    }
    Ok(())
}

async fn open_inner(app: &AppHandle, ctx: &InstanceContext, p: OpenParams) -> Result<Value, String> {
    let perm = granted(app, ctx)?;
    let system_prompt = p.system_prompt.unwrap_or_default();
    let initial_prompt = p.initial_prompt.unwrap_or_default();
    check_text("systemPrompt", &system_prompt)?;
    check_text("initialPrompt", &initial_prompt)?;
    if let Some(m) = p.model.as_deref() {
        validate_model(m)?;
    }
    let app_name = app_instance::instance_app_name(app, ctx);
    let title: String = p
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} AI", app_name))
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    let worktree_path = app
        .state::<crate::settings::SettingsManager>()
        .get()
        .worktrees
        .iter()
        .find(|w| w.id == ctx.worktree_id)
        .map(|w| w.path.clone())
        .ok_or_else(|| format!("ワークツリー '{}' が見つかりません", ctx.worktree_id))?;
    let cwd = resolve_cwd(&worktree_path, p.cwd.as_deref())?;

    let key = key_of(ctx);
    let terminal_id = reserve(app, &key, &app_name, &title, &system_prompt, &cwd, perm.max_concurrent)?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let waiters = app.state::<SpawnWaiters>();
    let rx = waiters.register(&request_id);
    let _ = app.emit(
        "ai-terminal-spawn",
        json!({
            "requestId": request_id,
            "worktreeId": ctx.worktree_id,
            "terminalId": terminal_id,
            "appName": app_name,
            "title": title,
            "cwd": cwd,
            "model": p.model,
            "initialPrompt": initial_prompt,
        }),
    );
    let outcome = tokio::time::timeout(OPEN_TIMEOUT, rx).await;
    waiters.remove(&request_id);
    let failure = match outcome {
        Ok(Ok(Ok(()))) => None,
        Ok(Ok(Err(e))) => Some(e),
        Ok(Err(_)) => Some("起動要求が取り消されました".to_string()),
        Err(_) => Some("端末の起動がタイムアウトしました".to_string()),
    };
    // フロントの成否通知だけを信用せず、紐付けが Rust 側で済んでいることを確かめる
    let session_id = match failure {
        None => resolve_owned(app, &key, &terminal_id),
        Some(e) => Err(e),
    };
    match session_id {
        Ok(sid) => Ok(json!({ "terminalId": terminal_id, "sessionId": sid })),
        Err(e) => {
            if let Some(sid) = cancel(app, &terminal_id) {
                kill_sessions(app, vec![sid], "ai-terminal-open-failed");
            }
            Err(format!("AI ターミナルを開けませんでした: {}", e))
        }
    }
}

#[tauri::command]
pub async fn app_ai_terminal_open(window: tauri::Window, app: AppHandle, params: OpenParams) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let r = open_inner(&app, &ctx, params).await;
    logged(&app, &ctx, "open", r)
}

/// 所有を確かめて、既存の MCP 端末ツールへ流す共通処理。
async fn call_owned(
    app: &AppHandle,
    ctx: &InstanceContext,
    terminal_id: &str,
    tool: &str,
    mut args: serde_json::Map<String, Value>,
) -> Result<Value, String> {
    granted(app, ctx)?;
    let sid = resolve_owned(app, &key_of(ctx), terminal_id)?;
    args.insert("session_id".to_string(), json!(sid));
    let text = crate::mcp_server::call_owned_terminal_tool(app, &ctx.worktree_id, &ctx.artifact_id, tool, Value::Object(args)).await?;
    Ok(to_value(text))
}

fn args(pairs: &[(&str, Value)]) -> serde_json::Map<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[tauri::command]
pub async fn app_ai_terminal_write(
    window: tauri::Window,
    app: AppHandle,
    terminal_id: String,
    text: String,
    submit: Option<bool>,
) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let mut a = args(&[("text", json!(text))]);
    if let Some(s) = submit {
        a.insert("submit".to_string(), json!(s));
    }
    let r = call_owned(&app, &ctx, &terminal_id, "oretachi_write_terminal", a).await;
    logged(&app, &ctx, &format!("write({})", short(&terminal_id)), r)
}

/// `allow_confirm` は渡せない（Enter / Esc を含められない）。ダイアログの確定は `answerPrompt` に寄せる。
#[tauri::command]
pub async fn app_ai_terminal_send_keys(
    window: tauri::Window,
    app: AppHandle,
    terminal_id: String,
    keys: Vec<String>,
) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let a = args(&[("keys", json!(keys)), ("allow_confirm", json!(false))]);
    let r = call_owned(&app, &ctx, &terminal_id, "oretachi_send_keys", a).await;
    logged(&app, &ctx, &format!("sendKeys({})", short(&terminal_id)), r)
}

#[tauri::command]
pub async fn app_ai_terminal_read(
    window: tauri::Window,
    app: AppHandle,
    terminal_id: String,
    max_bytes: Option<usize>,
    from_cursor: Option<u64>,
) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let mut a = serde_json::Map::new();
    if let Some(n) = max_bytes {
        a.insert("max_bytes".to_string(), json!(n));
    }
    if let Some(c) = from_cursor {
        a.insert("from_cursor".to_string(), json!(c));
    }
    let r = call_owned(&app, &ctx, &terminal_id, "oretachi_read_terminal", a).await;
    logged(&app, &ctx, &format!("read({})", short(&terminal_id)), r)
}

#[tauri::command]
pub async fn app_ai_terminal_inspect_prompt(
    window: tauri::Window,
    app: AppHandle,
    terminal_id: String,
) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let r = call_owned(&app, &ctx, &terminal_id, "oretachi_inspect_prompt", serde_json::Map::new()).await;
    logged(&app, &ctx, &format!("inspectPrompt({})", short(&terminal_id)), r)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnswerArgs {
    pub expect_fingerprint: String,
    pub kind: String,
    pub option_index: Option<u32>,
    pub option_indices: Option<Vec<u32>>,
    pub option_texts: Option<Vec<String>>,
    pub text: Option<String>,
    pub value: Option<String>,
}

/// ダイアログへの回答。指紋検査は `oretachi_answer_prompt` のものをそのまま使う。
#[tauri::command]
pub async fn app_ai_terminal_answer_prompt(
    window: tauri::Window,
    app: AppHandle,
    terminal_id: String,
    answer: AnswerArgs,
) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let mut a = args(&[
        ("expect_fingerprint", json!(answer.expect_fingerprint)),
        ("kind", json!(answer.kind)),
    ]);
    for (k, v) in [
        ("option_index", answer.option_index.map(|v| json!(v))),
        ("option_indices", answer.option_indices.map(|v| json!(v))),
        ("option_texts", answer.option_texts.map(|v| json!(v))),
        ("text", answer.text.map(|v| json!(v))),
        ("value", answer.value.map(|v| json!(v))),
    ] {
        if let Some(v) = v {
            a.insert(k.to_string(), v);
        }
    }
    let r = call_owned(&app, &ctx, &terminal_id, "oretachi_answer_prompt", a).await;
    logged(&app, &ctx, &format!("answerPrompt({})", short(&terminal_id)), r)
}

#[tauri::command]
pub async fn app_ai_terminal_close(window: tauri::Window, app: AppHandle, terminal_id: String) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let r: Result<Value, String> = (|| {
        granted(&app, &ctx)?;
        let sid = resolve_owned(&app, &key_of(&ctx), &terminal_id)?;
        cancel(&app, &terminal_id);
        kill_sessions(&app, vec![sid], "app-ai-terminal-close");
        Ok(json!({ "closed": true }))
    })();
    logged(&app, &ctx, &format!("close({})", short(&terminal_id)), r)
}

#[tauri::command]
pub async fn app_ai_terminal_list(window: tauri::Window, app: AppHandle) -> Result<Value, String> {
    let ctx = instance_ctx(&app, &window)?;
    let r = granted(&app, &ctx).map(|_| {
        let items: Vec<Value> = list_for(&app, &key_of(&ctx))
            .into_iter()
            .map(|t| json!({ "terminalId": t.terminal_id, "sessionId": t.session_id, "title": t.title }))
            .collect();
        Value::Array(items)
    });
    logged(&app, &ctx, "list", r)
}

// ─── ホスト（main / sub / tray）向けコマンド ───────────────────────────────

/// アプリ（`appinst-*`）と通常のビューア（`artifact-*`）からは呼べない。
fn require_host_window(window: &tauri::Window) -> Result<(), String> {
    let l = window.label();
    if l == "main" || l.starts_with("sub-") || l.starts_with("tray-") {
        Ok(())
    } else {
        Err(format!("このコマンドは oretachi 本体のウィンドウからだけ呼べます（呼び出し元: {}）", l))
    }
}

/// `open` が待っている起動要求への、フロントからの成否通知。**所有の根拠にはしない**
/// （紐付けは `pty_spawn` が Rust 側で済ませる）。
#[tauri::command]
pub fn ai_terminal_spawn_result(
    window: tauri::Window,
    app: AppHandle,
    request_id: String,
    ok: bool,
    error: Option<String>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("このコマンドはメインウィンドウからだけ呼べます".to_string());
    }
    let result = if ok { Ok(()) } else { Err(error.unwrap_or_else(|| "端末を起動できませんでした".to_string())) };
    app.state::<SpawnWaiters>().resolve(&request_id, result);
    Ok(())
}

/// 全所有端末（タブのアイコン・自動承認と復元の除外に使う）。
#[tauri::command]
pub fn list_owned_ai_terminals(window: tauri::Window, app: AppHandle) -> Result<Vec<OwnedTerminal>, String> {
    require_host_window(&window)?;
    Ok(list_all(&app))
}

/// フックのイベント（Stop / Notification / PermissionRequest）を、所有インスタンスのデバッグログと
/// iframe へ回す。ワークツリーのトレイ通知・バッジには積まない（呼び出し側でここへ分岐する）。
pub fn forward_event(app: &AppHandle, terminal_id: &str, event: &str, kind: &str, body: Option<&str>) {
    let Some(owner) = owner_of(app, terminal_id) else { return };
    let text = body.map(|b| b.chars().take(500).collect::<String>());
    add_log(
        app,
        &owner.worktree_id,
        &owner.artifact_id,
        "info",
        format!("AI 端末 {} のイベント: {}{}", short(terminal_id), event, text.as_deref().map(|t| format!(" — {}", t)).unwrap_or_default()),
    );
    if let Some(label) = app_instance::label_for_key(app, &owner.worktree_id, &owner.artifact_id) {
        let _ = app.emit_to(
            label,
            "app-ai-terminal-event",
            json!({ "terminalId": terminal_id, "event": event, "kind": kind, "body": text }),
        );
    }
}

/// モデル名: `claude --model <m>` にそのままシェルへ渡るので、安全な文字種だけ受理する。
pub fn validate_model(model: &str) -> Result<(), String> {
    let ok = !model.is_empty()
        && model.chars().count() <= 100
        && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '[' | ']'));
    if ok {
        Ok(())
    } else {
        Err("model には英数字と . _ - [ ] だけを使えます（100 文字まで）".to_string())
    }
}

/// `worktree://` 相対の作業ディレクトリを、ワークツリー配下の絶対パスへ解決する。
///
/// - 省略・空・`worktree://` だけならワークツリーのルート。
/// - 絶対パス・ドライブ指定・`..`・NUL は拒否する。区切りは `/` と `\` の両方を受ける。
/// - 結果は `worktree_path` に**コンポーネントを順に足した**形で返す（フックの `projectDir` 照合
///   `resolve_worktree_by_cwd` が正規化なしの前方一致なので、`canonicalize` した形は使わない）。
/// - 実在するディレクトリで、シンボリックリンクを辿ってもワークツリーの外へ出ないことを確かめる。
pub fn resolve_cwd(worktree_path: &str, requested: Option<&str>) -> Result<String, String> {
    let rel = match requested.map(str::trim).filter(|s| !s.is_empty()) {
        None => "",
        Some(s) => s.strip_prefix("worktree://").ok_or_else(|| {
            "cwd は worktree:// で始まるワークツリー内のパスで指定してください".to_string()
        })?,
    };
    let mut path = std::path::PathBuf::from(worktree_path);
    for comp in rel.split(['/', '\\']) {
        match comp {
            "" | "." => {}
            ".." => return Err("cwd に .. は使えません".to_string()),
            c if c.contains('\0') || c.contains(':') => {
                return Err("cwd に使えない文字が含まれています".to_string())
            }
            c => path.push(c),
        }
    }
    let base = std::fs::canonicalize(worktree_path)
        .map_err(|e| format!("ワークツリーのパスを解決できません: {}", e))?;
    let real = std::fs::canonicalize(&path)
        .map_err(|_| format!("cwd のディレクトリが見つかりません: {}", rel))?;
    if !real.is_dir() {
        return Err(format!("cwd はディレクトリではありません: {}", rel));
    }
    if !real.starts_with(&base) {
        return Err("cwd がワークツリーの外を指しています".to_string());
    }
    path.to_str().map(str::to_string).ok_or_else(|| "cwd が UTF-8 ではありません".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(a: &str) -> Key {
        ("wt".to_string(), a.to_string())
    }

    fn alive(ids: &[u32]) -> HashSet<u32> {
        ids.iter().copied().collect()
    }

    fn reserve(inner: &mut Inner, k: &Key, max: u32, alive: &HashSet<u32>) -> Result<String, String> {
        inner.reserve(k, "App", "t", "sys", "/wt", max, alive, Instant::now())
    }

    #[test]
    fn reservation_is_consumed_once() {
        let mut inner = Inner::default();
        let tid = reserve(&mut inner, &key("a"), 2, &alive(&[])).unwrap();
        // 消費すると host が解決した cwd が返る（フロントの申告は使わない）
        assert_eq!(inner.take_reservation(&tid).as_deref(), Ok("/wt"));
        // 二度目・未予約の ID は受理しない（任意 ID の注入は不可）
        assert!(inner.take_reservation(&tid).is_err());
        assert!(inner.take_reservation("not-reserved").is_err());
    }

    #[test]
    fn bind_requires_consumed_reservation() {
        let mut inner = Inner::default();
        let tid = reserve(&mut inner, &key("a"), 2, &alive(&[])).unwrap();
        assert!(!inner.bind(&tid, 7), "未消費の予約には紐付けない");
        inner.take_reservation(&tid).unwrap();
        assert!(inner.bind(&tid, 7));
        assert!(!inner.bind(&tid, 8), "二重 bind は不可");
        assert_eq!(inner.resolve_owned(&key("a"), &tid), Ok(7));
    }

    #[test]
    fn other_instances_and_unknown_terminals_are_rejected() {
        let mut inner = Inner::default();
        let tid = reserve(&mut inner, &key("a"), 2, &alive(&[])).unwrap();
        inner.take_reservation(&tid).unwrap();
        inner.bind(&tid, 7);
        assert!(inner.resolve_owned(&key("b"), &tid).is_err());
        assert!(inner.resolve_owned(&key("a"), "main-terminal-id").is_err());
        // 他人の端末と存在しない端末は同じ文言（存在を漏らさない）
        assert_eq!(
            inner.resolve_owned(&key("b"), &tid),
            inner.resolve_owned(&key("b"), "main-terminal-id")
        );
    }

    #[test]
    fn unbound_terminal_cannot_be_operated_but_is_owned() {
        let mut inner = Inner::default();
        let tid = reserve(&mut inner, &key("a"), 2, &alive(&[])).unwrap();
        assert!(inner.resolve_owned(&key("a"), &tid).is_err());
        // 予約の時点でフックの除外対象になる
        assert!(inner.owner_of(&tid).is_some());
        assert_eq!(inner.system_prompt(&tid).as_deref(), Some("sys"));
    }

    #[test]
    fn per_instance_and_global_limits() {
        let mut inner = Inner::default();
        for _ in 0..2 {
            reserve(&mut inner, &key("a"), 2, &alive(&[])).unwrap();
        }
        assert!(reserve(&mut inner, &key("a"), 2, &alive(&[])).is_err());
        // 別インスタンスは別カウント
        for i in 0..(GLOBAL_MAX - 2) {
            reserve(&mut inner, &key(&format!("o{i}")), 1, &alive(&[])).unwrap();
        }
        assert!(reserve(&mut inner, &key("zz"), 1, &alive(&[])).is_err());
    }

    #[test]
    fn dead_sessions_and_stale_reservations_are_pruned() {
        let mut inner = Inner::default();
        let tid = reserve(&mut inner, &key("a"), 1, &alive(&[])).unwrap();
        inner.take_reservation(&tid).unwrap();
        inner.bind(&tid, 7);
        // 生きている間は枠を使う
        assert!(reserve(&mut inner, &key("a"), 1, &alive(&[7])).is_err());
        // PTY が死ねば枠が空く
        assert!(reserve(&mut inner, &key("a"), 1, &alive(&[])).is_ok());
        // 古い予約は掃除される
        let mut inner = Inner::default();
        let t0 = Instant::now();
        inner.reserve(&key("a"), "App", "t", "s", "/wt", 1, &alive(&[]), t0).unwrap();
        let later = t0 + RESERVATION_TTL + Duration::from_secs(1);
        assert!(inner.reserve(&key("a"), "App", "t", "s", "/wt", 1, &alive(&[]), later).is_ok());
    }

    #[test]
    fn remove_for_instance_returns_bound_sessions_only() {
        let mut inner = Inner::default();
        let a1 = reserve(&mut inner, &key("a"), 3, &alive(&[])).unwrap();
        let _a2 = reserve(&mut inner, &key("a"), 3, &alive(&[])).unwrap();
        let b1 = reserve(&mut inner, &key("b"), 3, &alive(&[])).unwrap();
        inner.take_reservation(&a1).unwrap();
        inner.bind(&a1, 5);
        inner.take_reservation(&b1).unwrap();
        inner.bind(&b1, 6);
        assert_eq!(inner.remove_for_instance(&key("a")), vec![5]);
        assert!(inner.owner_of(&a1).is_none());
        assert!(inner.owner_of(&b1).is_some());
    }

    #[test]
    fn cancel_reports_bound_session() {
        let mut inner = Inner::default();
        let tid = reserve(&mut inner, &key("a"), 1, &alive(&[])).unwrap();
        assert_eq!(inner.cancel(&tid), None);
        let tid = reserve(&mut inner, &key("a"), 1, &alive(&[])).unwrap();
        inner.take_reservation(&tid).unwrap();
        inner.bind(&tid, 9);
        assert_eq!(inner.cancel(&tid), Some(9));
    }

    #[test]
    fn model_charset() {
        assert!(validate_model("claude-opus-4-1").is_ok());
        assert!(validate_model("sonnet[1m]").is_ok());
        for bad in ["", "a b", "a;b", "$(x)", "a\"b", "a`b", "a|b", "a&b", "a\nb"] {
            assert!(validate_model(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn cwd_resolution() {
        let base = std::env::temp_dir().join(format!("oretachi-aiterm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("sub/dir")).unwrap();
        std::fs::write(base.join("file.txt"), "x").unwrap();
        let bs = base.to_str().unwrap();

        assert_eq!(resolve_cwd(bs, None).unwrap(), bs);
        assert_eq!(resolve_cwd(bs, Some("worktree://")).unwrap(), bs);
        let sub = resolve_cwd(bs, Some("worktree://sub/dir")).unwrap();
        assert!(sub.ends_with("dir") && std::path::Path::new(&sub).starts_with(&base));
        assert!(resolve_cwd(bs, Some("worktree://sub\\dir")).is_ok());

        for bad in [
            "sub",                    // worktree:// なし
            "/etc",
            "C:/Windows",
            "worktree://..",
            "worktree://sub/../..",
            "worktree://C:/Windows",
            "worktree://missing",
            "worktree://file.txt",    // ディレクトリでない
        ] {
            assert!(resolve_cwd(bs, Some(bad)).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
