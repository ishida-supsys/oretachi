//! アプリ形式アーティファクト向けの SQLite API（`window.__oretachi.db`）。
//!
//! # 関門
//!
//! DB ファイルの置き場所は、FS と同じ `worktree://` / `artifact://` のスコープ（grant の `resolved.db`）で
//! 決まる。呼び出しのたびに [`crate::app_fs::resolve_db_path`] を通すので、キャッシュ済みの接続でも
//! 範囲外になったパスへは届かない。**読み書きの区別は API のメソッドではなく、開くときのフラグ**で付ける
//! （write のスコープなら読み書き・作成可、read のスコープなら読み取り専用。`query` でも
//! `INSERT … RETURNING` は書けるため、メソッドは境界にならない）。
//!
//! # 自由な SQL と FS 権限
//!
//! SQL をそのまま通すと、SQL 自身がファイルを触って範囲外へ出られる。接続ごとに SQLite の
//! authorizer を張って塞ぐ（[`harden`]）:
//!
//! - `ATTACH DATABASE`: ファイル名が空文字列（一時 DB）のものだけ許す。
//! - `VACUUM INTO`: 内部で `ATTACH '<出力先>' AS vacuum_db` を実行するので、上の規則で止まる。
//! - `load_extension()`: 関数として拒否する（拡張のロードは既定でも無効）。
//! - `PRAGMA temp_store_directory` / `data_store_directory`: 任意のディレクトリへファイルを作れる。
//!
//! - `PRAGMA max_page_count` / `page_size` の変更: DB のサイズ上限（下記）を外せてしまう。
//!
//! 加えて `SQLITE_LIMIT_ATTACHED` を 1 にし（VACUUM が使う 1 本だけ）、`SQLITE_LIMIT_LENGTH` を絞り、
//! `DEFENSIVE` を立てる。
//!
//! # 資源
//!
//! 接続はインスタンス（ウィンドウラベル）× 実パスごとに 1 本、host 側で持つ。iframe のリロードをまたいで
//! 維持し（前提 10）、インスタンスの停止で閉じる。1 本だけなので、`BEGIN` と `COMMIT` を別の呼び出しに
//! 分けても成り立つ。実行時間・結果の行数とバイト数・接続数には上限がある。

use crate::app_manifest::ResolvedFs;
use base64::Engine;
use futures_util::TryStreamExt;
use serde::Serialize;
use serde_json::Value as Json;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqliteRow};
use sqlx::{Column, ConnectOptions, Connection, Row, TypeInfo, Value, ValueRef};
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// 1 回の実行の上限時間。iframe ブリッジの待ち時間（10 秒）より短くしておく。
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
/// 1 回の `query` が返せる行数の上限。
pub const MAX_ROWS: usize = 10_000;
/// 1 回の `query` が返せる結果の概算サイズ（FS の 1 回の読み書きの上限と同じ）。
pub const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;
/// 書き込み可で開く DB のファイルサイズの上限（`max_page_count` × 既定のページサイズ 4096）。
/// 超える書き込みは `database or disk is full` で失敗する。既存の DB がこれより大きくても読み取りはできる。
pub const MAX_DB_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_PAGE_SIZE: u64 = 4096;
/// 1 つの文字列 / BLOB / SQL 文の最大長（`zeroblob(1e9)` のような一括確保を防ぐ）。
const MAX_VALUE_BYTES: c_int = 16 * 1024 * 1024;
/// 1 インスタンスが同時に開ける DB の数。
pub const MAX_CONNECTIONS_PER_INSTANCE: usize = 8;
const MAX_SQL_BYTES: usize = 1024 * 1024;
const MAX_PARAMS: usize = 1000;
const BUSY_TIMEOUT: Duration = Duration::from_secs(3);
/// ログに残す SQL の長さ（文字数）。
const LOG_SQL_CHARS: usize = 200;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub max_rows: usize,
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { timeout: QUERY_TIMEOUT, max_rows: MAX_ROWS, max_bytes: MAX_RESULT_BYTES }
    }
}

// ─── authorizer ──────────────────────────────────────────────────────

/// 任意のディレクトリへファイルを作れる PRAGMA。
const DENIED_PRAGMAS: &[&str] = &["temp_store_directory", "data_store_directory"];
/// 読むのは構わないが、値を設定するとサイズの上限が外れる PRAGMA。
const READ_ONLY_PRAGMAS: &[&str] = &["max_page_count", "page_size"];

/// SQLite の authorizer。状態は持たない。`SQLITE_OK` で許可、`SQLITE_DENY` で準備（prepare）を失敗させる。
unsafe extern "C" fn authorizer(
    _user: *mut c_void,
    action: c_int,
    arg1: *const c_char,
    arg2: *const c_char,
    _arg3: *const c_char,
    _arg4: *const c_char,
) -> c_int {
    use libsqlite3_sys as ffi;
    // SAFETY: SQLite は NUL 終端の文字列か NULL を渡す
    let text = |p: *const c_char| -> Option<String> {
        if p.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        }
    };
    match action {
        // 一時 DB（空文字列）だけ許す。NULL（式で指定された）は通さない
        ffi::SQLITE_ATTACH => match text(arg1) {
            Some(name) if name.is_empty() => ffi::SQLITE_OK,
            _ => ffi::SQLITE_DENY,
        },
        ffi::SQLITE_FUNCTION => match text(arg2) {
            Some(name) if name.eq_ignore_ascii_case("load_extension") => ffi::SQLITE_DENY,
            _ => ffi::SQLITE_OK,
        },
        ffi::SQLITE_PRAGMA => match text(arg1) {
            Some(name) if DENIED_PRAGMAS.iter().any(|p| p.eq_ignore_ascii_case(&name)) => ffi::SQLITE_DENY,
            // arg2 は PRAGMA に渡された値（読むだけなら NULL）
            Some(name)
                if READ_ONLY_PRAGMAS.iter().any(|p| p.eq_ignore_ascii_case(&name)) && !arg2.is_null() =>
            {
                ffi::SQLITE_DENY
            }
            _ => ffi::SQLITE_OK,
        },
        _ => ffi::SQLITE_OK,
    }
}

/// 接続に authorizer などを張る。開いた直後（ユーザーの SQL を通す前）に必ず呼ぶ。
async fn harden(conn: &mut SqliteConnection) -> Result<(), String> {
    use libsqlite3_sys as ffi;
    let mut handle = conn.lock_handle().await.map_err(|e| format!("接続を設定できません: {}", e))?;
    let db = handle.as_raw_handle().as_ptr();
    // SAFETY: `lock_handle` の間は sqlx のバックグラウンドスレッドが同じハンドルを触らない
    unsafe {
        let rc = ffi::sqlite3_set_authorizer(db, Some(authorizer), std::ptr::null_mut());
        if rc != ffi::SQLITE_OK {
            return Err(format!("authorizer を設定できません（SQLite のエラーコード {}）", rc));
        }
        ffi::sqlite3_limit(db, ffi::SQLITE_LIMIT_ATTACHED, 1);
        ffi::sqlite3_limit(db, ffi::SQLITE_LIMIT_LENGTH, MAX_VALUE_BYTES);
        // 失敗しても致命的ではない（writable_schema などの防御が効かないだけ）。戻り値は見ない
        let mut _out: c_int = 0;
        ffi::sqlite3_db_config(db, ffi::SQLITE_DBCONFIG_DEFENSIVE, 1 as c_int, &mut _out as *mut c_int);
    }
    Ok(())
}

/// 実行時間の上限を張る。上限を超えると SQLite が実行を中断する（`interrupted`）。
/// 呼ばれるのは VM の命令の間だけなので、ロック待ち（`BUSY_TIMEOUT`）の間は止まらない。
/// 実際の最大は上限 + `BUSY_TIMEOUT`（iframe ブリッジの 10 秒には収まる）。
async fn arm_deadline(conn: &mut SqliteConnection, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut handle = conn.lock_handle().await.map_err(|e| format!("接続を設定できません: {}", e))?;
    handle.set_progress_handler(1000, move || Instant::now() < deadline);
    Ok(())
}

// ─── 開く ────────────────────────────────────────────────────────────

/// DB を開く。`path` は [`crate::app_fs::resolve_db_path`] が返した実パス。
///
/// **URL 文字列は組み立てない。** 実パスは Windows では `\\?\C:\…` で、`sqlite:<path>?mode=rwc` の形に
/// すると `\\?\` の `?` がクエリの開始として解釈されて壊れる。
pub async fn open_connection(path: &Path, writable: bool) -> Result<SqliteConnection, String> {
    if writable {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("ディレクトリを作れません: {}", e))?;
        }
    }
    let mut options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(!writable)
        .create_if_missing(writable)
        .busy_timeout(BUSY_TIMEOUT);
    // journal_mode は指定しない（sqlx は求められない限り設定しない）。既存の DB のモードを黙って変えず、
    // WAL の DB を読み取り専用で開けなくなることも避ける。サイズの上限は authorizer を張る前に設定する
    // （張った後は max_page_count を変えられない）
    if writable {
        options = options.pragma("max_page_count", (MAX_DB_BYTES / DEFAULT_PAGE_SIZE).to_string());
    }
    let mut conn = options.connect().await.map_err(|e| format!("DB を開けません: {}", e))?;
    if let Err(e) = harden(&mut conn).await {
        let _ = conn.close().await;
        return Err(e);
    }
    Ok(conn)
}

// ─── 値の変換 ────────────────────────────────────────────────────────

fn bind_json<'q>(
    q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    v: &Json,
) -> Result<sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>, String> {
    Ok(match v {
        Json::Null => q.bind(None::<i64>),
        Json::Bool(b) => q.bind(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                return Err(format!("数値 {} を扱えません", n));
            }
        }
        Json::String(s) => q.bind(s.clone()),
        Json::Object(o) => match (o.len(), o.get("$blob").and_then(|b| b.as_str())) {
            (1, Some(b64)) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| format!("$blob を base64 として解釈できません: {}", e))?;
                q.bind(bytes)
            }
            _ => return Err("パラメータにできるオブジェクトは { \"$blob\": \"<base64>\" } だけです".to_string()),
        },
        Json::Array(_) => return Err("パラメータに配列は使えません".to_string()),
    })
}

/// 1 セルを JSON にする。戻りの `usize` は結果サイズへの概算の加算分。
fn cell_to_json(row: &SqliteRow, i: usize) -> Result<(Json, usize), String> {
    let raw = row.try_get_raw(i).map_err(|e| format!("列 {} を読めません: {}", i, e))?;
    if raw.is_null() {
        return Ok((Json::Null, 4));
    }
    let owned = raw.to_owned();
    let decode_err = |e: sqlx::Error| format!("列 {} を読めません: {}", i, e);
    let type_name = owned.type_info().name().to_string();
    match type_name.as_str() {
        "INTEGER" => {
            let n: i64 = owned.try_decode().map_err(decode_err)?;
            Ok((Json::from(n), 8))
        }
        "REAL" => {
            let f: f64 = owned.try_decode().map_err(decode_err)?;
            // NaN / Infinity は JSON にできない
            Ok((serde_json::Number::from_f64(f).map(Json::Number).unwrap_or(Json::Null), 8))
        }
        "BLOB" => {
            let bytes: Vec<u8> = owned.try_decode().map_err(decode_err)?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let size = b64.len() + 16;
            Ok((serde_json::json!({ "$blob": b64 }), size))
        }
        _ => {
            let s: String = owned.try_decode().map_err(decode_err)?;
            let size = s.len();
            Ok((Json::String(s), size))
        }
    }
}

// ─── 実行 ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    /// 結果が 0 行のときは空（行が無いと列の情報が得られない）
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Json>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub rows_affected: u64,
    pub last_insert_rowid: i64,
}

fn check_request(sql: &str, params: &[Json]) -> Result<(), String> {
    if sql.trim().is_empty() {
        return Err("SQL が空です".to_string());
    }
    if sql.len() > MAX_SQL_BYTES {
        return Err(format!("SQL が長すぎます（{} バイトまで）", MAX_SQL_BYTES));
    }
    if params.len() > MAX_PARAMS {
        return Err(format!("パラメータが多すぎます（{} 個まで）", MAX_PARAMS));
    }
    Ok(())
}

/// SQLite のエラーを、アプリ側に分かる文面にする。
fn describe_error(e: sqlx::Error) -> String {
    let msg = e.to_string();
    // authorizer の拒否は SQLITE_AUTH（"authorization denied"）、それ以外の権限エラーは "not authorized"
    if msg.contains("authorization denied") || msg.contains("not authorized") {
        format!(
            "この SQL は許可されていません（ATTACH / VACUUM INTO / load_extension / 一部の PRAGMA は使えません）: {}",
            msg
        )
    } else if msg.contains("interrupted") {
        format!("実行時間の上限を超えたため中断しました: {}", msg)
    } else {
        msg
    }
}

pub async fn run_query(
    conn: &mut SqliteConnection,
    sql: &str,
    params: &[Json],
    limits: &Limits,
) -> Result<QueryResult, String> {
    check_request(sql, params)?;
    arm_deadline(conn, limits.timeout).await?;
    let mut q = sqlx::query(sql).persistent(false);
    for p in params {
        q = bind_json(q, p)?;
    }
    let mut stream = q.fetch(&mut *conn);
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<Json>> = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = stream.try_next().await.map_err(describe_error)? {
        if rows.len() >= limits.max_rows {
            return Err(format!("結果の行数が上限（{} 行）を超えました。LIMIT で絞ってください", limits.max_rows));
        }
        if columns.is_empty() {
            columns = row.columns().iter().map(|c| c.name().to_string()).collect();
            bytes += columns.iter().map(|c| c.len() + 4).sum::<usize>();
        }
        let mut out = Vec::with_capacity(row.len());
        for i in 0..row.len() {
            let (v, size) = cell_to_json(&row, i)?;
            bytes += size + 2;
            out.push(v);
        }
        if bytes > limits.max_bytes {
            return Err(format!(
                "結果のサイズが上限（{} バイト）を超えました。列や行を絞ってください",
                limits.max_bytes
            ));
        }
        rows.push(out);
    }
    Ok(QueryResult { columns, rows })
}

pub async fn run_execute(
    conn: &mut SqliteConnection,
    sql: &str,
    params: &[Json],
    limits: &Limits,
) -> Result<ExecResult, String> {
    check_request(sql, params)?;
    arm_deadline(conn, limits.timeout).await?;
    let mut q = sqlx::query(sql).persistent(false);
    for p in params {
        q = bind_json(q, p)?;
    }
    let r = q.execute(&mut *conn).await.map_err(describe_error)?;
    Ok(ExecResult { rows_affected: r.rows_affected(), last_insert_rowid: r.last_insert_rowid() })
}

// ─── 接続の台帳（host 側） ────────────────────────────────────────────

/// `None` は閉じ済み。実行中の呼び出しがロックを持つので、閉じるときは終わるまで待つ。
type Shared = Arc<tokio::sync::Mutex<Option<SqliteConnection>>>;

#[derive(Default)]
struct Inner {
    /// ウィンドウラベル → (実パス, 書き込み可) → 接続
    conns: HashMap<String, HashMap<(PathBuf, bool), Shared>>,
    /// 停止したインスタンスのラベル（ラベルは再利用されない）。開いている最中の接続が、停止の後に
    /// 台帳へ入ってしまわないようにする
    dead: HashSet<String>,
    /// 実行中の「閉じるタスク」の数。0 になると `closed` を通知する
    closing: usize,
}

#[derive(Default)]
pub struct AppDbRegistry {
    inner: Arc<std::sync::Mutex<Inner>>,
    /// 閉じるタスクが 1 つ終わるたびに通知する（`drain_closes` が複数同時に待てる）
    closed: Arc<tokio::sync::Notify>,
}

enum InsertOutcome {
    Inserted(Shared),
    /// 同時に別の呼び出しが先に入れていた。自分の接続は閉じて、こちらを使う
    Existing(Shared),
}

impl AppDbRegistry {
    fn is_dead(&self, label: &str) -> bool {
        self.inner.lock().map(|i| i.dead.contains(label)).unwrap_or(true)
    }

    fn get(&self, label: &str, key: &(PathBuf, bool)) -> Option<Shared> {
        let inner = self.inner.lock().ok()?;
        inner.conns.get(label)?.get(key).cloned()
    }

    fn insert(&self, label: &str, key: (PathBuf, bool), conn: SqliteConnection) -> Result<InsertOutcome, (String, SqliteConnection)> {
        let Ok(mut inner) = self.inner.lock() else {
            return Err(("DB 台帳のロックが壊れています".to_string(), conn));
        };
        if inner.dead.contains(label) {
            return Err(("アプリが停止しました".to_string(), conn));
        }
        let map = inner.conns.entry(label.to_string()).or_default();
        if let Some(existing) = map.get(&key) {
            drop(conn);
            return Ok(InsertOutcome::Existing(existing.clone()));
        }
        if map.len() >= MAX_CONNECTIONS_PER_INSTANCE {
            return Err((
                format!("同時に開ける DB は {} 個までです。使い終わったものは db.close で閉じてください", MAX_CONNECTIONS_PER_INSTANCE),
                conn,
            ));
        }
        let shared: Shared = Arc::new(tokio::sync::Mutex::new(Some(conn)));
        map.insert(key, shared.clone());
        Ok(InsertOutcome::Inserted(shared))
    }

    fn remove(&self, label: &str, key: &(PathBuf, bool)) -> Option<Shared> {
        let mut inner = self.inner.lock().ok()?;
        inner.conns.get_mut(label)?.remove(key)
    }

    /// そのインスタンスの接続を台帳から外して返す（閉じはしない）。`stopped` なら以後は開けない。
    fn take_instance(&self, label: &str, stopped: bool) -> Vec<Shared> {
        let Ok(mut inner) = self.inner.lock() else { return Vec::new() };
        if stopped {
            inner.dead.insert(label.to_string());
        }
        inner.conns.remove(label).map(|m| m.into_values().collect()).unwrap_or_default()
    }

    /// 閉じるタスクを起こす。完了は [`AppDbRegistry::drain_closes`] で待てる。
    fn spawn_close(&self, conns: Vec<Shared>) {
        if conns.is_empty() {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.closing += 1;
        }
        let inner = self.inner.clone();
        let notify = self.closed.clone();
        tauri::async_runtime::spawn(async move {
            close_all(conns).await;
            if let Ok(mut i) = inner.lock() {
                i.closing = i.closing.saturating_sub(1);
            }
            notify.notify_waiters();
        });
    }

    /// これまでに起こした「閉じるタスク」がすべて終わるまで待つ。複数の呼び出しが同時に待てる。
    pub async fn drain_closes(&self) {
        loop {
            // 通知を取りこぼさないよう、数を見る前に待ちを登録する
            let notified = self.closed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let pending = self.inner.lock().map(|i| i.closing).unwrap_or(0);
            if pending == 0 {
                return;
            }
            notified.await;
        }
    }
}

async fn close_all(conns: Vec<Shared>) {
    for shared in conns {
        // 実行中の呼び出しがあれば、その終了（上限時間まで）を待つ
        let mut guard = shared.lock().await;
        if let Some(conn) = guard.take() {
            let _ = conn.close().await;
        }
    }
}

/// インスタンスの停止時（`app_instance::release`）。接続を外し、以後は開けないようにして閉じる。
/// 閉じは非同期。ファイルを消す前に待ちたいときは [`AppDbRegistry::drain_closes`]。
pub fn on_instance_released(app: &AppHandle, label: &str) {
    let reg = app.state::<AppDbRegistry>();
    let conns = reg.take_instance(label, true);
    reg.spawn_close(conns);
}

/// 権限セットが入れ替わったとき。開いている接続を閉じる（次の呼び出しが新しい grant で開き直す）。
pub fn on_grant_replaced(app: &AppHandle, label: &str) {
    let reg = app.state::<AppDbRegistry>();
    let conns = reg.take_instance(label, false);
    reg.spawn_close(conns);
}

// ─── Tauri コマンド（appinst-* ウィンドウからだけ） ────────────────────

struct Call {
    ctx: crate::app_instance::InstanceContext,
    shared: Shared,
}

fn caller_db(window: &tauri::Window, app: &AppHandle) -> Result<(crate::app_instance::InstanceContext, ResolvedFs), String> {
    crate::app_instance::require_instance_window(window)?;
    let ctx = crate::app_instance::context_for_label(app, window.label())?;
    let grant = app
        .state::<crate::app_grant::AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .ok_or_else(|| "このアプリには承認がありません".to_string())?;
    let db = grant
        .resolved
        .db
        .ok_or_else(|| "このアプリにはデータベースの権限が承認されていません".to_string())?;
    Ok((ctx, db))
}

fn truncate_sql(sql: &str) -> String {
    let mut s: String = sql.chars().take(LOG_SQL_CHARS).collect();
    if sql.chars().count() > LOG_SQL_CHARS {
        s.push('…');
    }
    s.replace(['\r', '\n'], " ")
}

/// 拒否・失敗はデバッグログ（アプリのログとアプリのログファイル）へ残す。成功は残さない。
fn log_failure(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, op: &str, path: &str, sql: &str, error: &str) {
    let note = if ctx.reapproval_pending { "（再承認待ちのため、新しい権限はまだ使えません）" } else { "" };
    log::debug!("[app-db] 拒否 {} {}/{} {} — {} — {}", op, ctx.worktree_id, ctx.artifact_id, path, sql, error);
    let detail = if sql.is_empty() { String::new() } else { format!(" [{}]", sql) };
    crate::app_instance::add_log(
        app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        "warn",
        format!("db.{} を拒否/失敗: {}{} — {}{}", op, path, detail, error, note),
    );
}

/// 呼び出し元の検査 → パス解決 → 接続の取得（無ければ開く）。
async fn prepare(window: &tauri::Window, app: &AppHandle, path: &str) -> Result<Call, String> {
    let (ctx, db) = caller_db(window, app)?;
    let p = path.to_string();
    let (real, writable) = tokio::task::spawn_blocking(move || crate::app_fs::resolve_db_path(&db, &p))
        .await
        .map_err(|e| format!("task join error: {}", e))
        .and_then(|r| r)
        .map_err(|e| {
            log_failure(app, &ctx, "open", path, "", &e);
            e
        })?;

    let reg = app.state::<AppDbRegistry>();
    let key = (real.clone(), writable);
    // 停止済みのインスタンスには、既存の接続も新しい接続も渡さない
    if reg.is_dead(&ctx.instance_label) {
        let e = "アプリが停止しました".to_string();
        log_failure(app, &ctx, "open", path, "", &e);
        return Err(e);
    }
    if let Some(shared) = reg.get(&ctx.instance_label, &key) {
        return Ok(Call { ctx, shared });
    }
    // 開くと（write なら）ファイルとディレクトリを作る。停止・削除と競って、消した後の同梱領域を
    // 作り直してしまった場合に片付けられるよう、作る前の状態を覚えておく
    let existed = real.exists();
    let conn = open_connection(&real, writable).await.map_err(|e| {
        log_failure(app, &ctx, "open", path, "", &e);
        e
    })?;
    match reg.insert(&ctx.instance_label, key, conn) {
        Ok(InsertOutcome::Inserted(shared)) | Ok(InsertOutcome::Existing(shared)) => Ok(Call { ctx, shared }),
        Err((e, conn)) => {
            let _ = conn.close().await;
            if writable && !existed {
                // 停止との競合で作ってしまった空の DB を残さない（同じ id で作り直した別物に引き継がれる）
                let _ = std::fs::remove_file(&real);
                if let Some(parent) = real.parent() {
                    let _ = std::fs::remove_dir(parent); // 空のときだけ消える
                }
            }
            log_failure(app, &ctx, "open", path, "", &e);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn app_db_query(
    window: tauri::Window,
    app: AppHandle,
    path: String,
    sql: String,
    params: Option<Vec<Json>>,
) -> Result<QueryResult, String> {
    let call = prepare(&window, &app, &path).await?;
    let params = params.unwrap_or_default();
    let result = {
        let mut guard = call.shared.lock().await;
        match guard.as_mut() {
            Some(conn) => run_query(conn, &sql, &params, &Limits::default()).await,
            None => Err("接続は閉じられました".to_string()),
        }
    };
    if let Err(e) = &result {
        log_failure(&app, &call.ctx, "query", &path, &truncate_sql(&sql), e);
    }
    result
}

#[tauri::command]
pub async fn app_db_execute(
    window: tauri::Window,
    app: AppHandle,
    path: String,
    sql: String,
    params: Option<Vec<Json>>,
) -> Result<ExecResult, String> {
    let call = prepare(&window, &app, &path).await?;
    let params = params.unwrap_or_default();
    let result = {
        let mut guard = call.shared.lock().await;
        match guard.as_mut() {
            Some(conn) => run_execute(conn, &sql, &params, &Limits::default()).await,
            None => Err("接続は閉じられました".to_string()),
        }
    };
    if let Err(e) = &result {
        log_failure(&app, &call.ctx, "execute", &path, &truncate_sql(&sql), e);
    }
    result
}

/// 接続を明示的に閉じる。閉じたら `true`、開いていなければ `false`。
#[tauri::command]
pub async fn app_db_close(window: tauri::Window, app: AppHandle, path: String) -> Result<bool, String> {
    let (ctx, db) = caller_db(&window, &app)?;
    let p = path.clone();
    let (real, writable) = tokio::task::spawn_blocking(move || crate::app_fs::resolve_db_path(&db, &p))
        .await
        .map_err(|e| format!("task join error: {}", e))
        .and_then(|r| r)
        .map_err(|e| {
            log_failure(&app, &ctx, "close", &path, "", &e);
            e
        })?;
    let reg = app.state::<AppDbRegistry>();
    match reg.remove(&ctx.instance_label, &(real, writable)) {
        Some(shared) => {
            close_all(vec![shared]).await;
            Ok(true)
        }
        None => Ok(false),
    }
}

// ─── テスト ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_manifest::RequestedFs;
    use serde_json::json;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oretachi-appdb-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn requested(read: &[&str], write: &[&str], allow: bool) -> RequestedFs {
        RequestedFs {
            read: read.iter().map(|s| s.to_string()).collect(),
            write: write.iter().map(|s| s.to_string()).collect(),
            allow_sensitive_paths: allow,
        }
    }

    /// `<base>/wt`（worktree://）と `<base>/art`（artifact://）を用意して grant の db を解決する
    fn setup(tag: &str, read: &[&str], write: &[&str], allow: bool) -> (PathBuf, ResolvedFs) {
        let base = temp_dir(tag);
        let wt = base.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let fs = crate::app_fs::resolve_requested_fs(&requested(read, write, allow), Some(&wt), &base.join("art"))
            .expect("resolve");
        (base, fs)
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(f)
    }

    async fn open_uri(fs: &ResolvedFs, uri: &str) -> Result<SqliteConnection, String> {
        let (path, writable) = crate::app_fs::resolve_db_path(fs, uri)?;
        open_connection(&path, writable).await
    }

    async fn q(conn: &mut SqliteConnection, sql: &str) -> Result<QueryResult, String> {
        run_query(conn, sql, &[], &Limits::default()).await
    }

    async fn x(conn: &mut SqliteConnection, sql: &str) -> Result<ExecResult, String> {
        run_execute(conn, sql, &[], &Limits::default()).await
    }

    fn expect_err<T: std::fmt::Debug>(r: Result<T, String>, needle: &str) {
        let e = r.expect_err("denied");
        assert!(e.contains(needle), "expected '{}' in '{}'", needle, e);
    }

    #[test]
    fn create_write_and_read_inside_scope() {
        let (base, fs) = setup("rw", &[], &["worktree://data/", "artifact://"], false);
        run(async {
            for uri in ["worktree://data/app.db", "artifact://app.db"] {
                // resolve_db_path が返す実パス（Windows では `\\?\` 付き）で開く
                let mut c = open_uri(&fs, uri).await.expect(uri);
                x(&mut c, "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL, raw BLOB)").await.unwrap();
                let r = run_execute(
                    &mut c,
                    "INSERT INTO t (name, score, raw) VALUES (?, ?, ?)",
                    &[json!("あ"), json!(1.5), json!({ "$blob": "AQID" })],
                    &Limits::default(),
                )
                .await
                .unwrap();
                assert_eq!((r.rows_affected, r.last_insert_rowid), (1, 1));
                let r = q(&mut c, "SELECT id, name, score, raw, NULL AS n FROM t").await.unwrap();
                assert_eq!(r.columns, vec!["id", "name", "score", "raw", "n"]);
                assert_eq!(r.rows, vec![vec![json!(1), json!("あ"), json!(1.5), json!({ "$blob": "AQID" }), Json::Null]]);
                c.close().await.unwrap();
            }
        });
        assert!(base.join("wt").join("data").join("app.db").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn data_persists_across_connections() {
        let (base, fs) = setup("persist", &[], &["artifact://"], false);
        run(async {
            let mut c = open_uri(&fs, "artifact://p.db").await.unwrap();
            x(&mut c, "CREATE TABLE t (v INTEGER)").await.unwrap();
            x(&mut c, "INSERT INTO t VALUES (7)").await.unwrap();
            c.close().await.unwrap();
            let mut c = open_uri(&fs, "artifact://p.db").await.unwrap();
            assert_eq!(q(&mut c, "SELECT v FROM t").await.unwrap().rows, vec![vec![json!(7)]]);
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn out_of_scope_and_sensitive_paths_cannot_be_opened() {
        let (base, fs) = setup("scope", &["worktree://ro/"], &["worktree://data/"], false);
        // 範囲外
        expect_err(crate::app_fs::resolve_db_path(&fs, "worktree://other/x.db"), "範囲の外");
        expect_err(crate::app_fs::resolve_db_path(&fs, "worktree://x.db"), "範囲の外");
        // 範囲内でも artifact:// は承認されていない
        expect_err(crate::app_fs::resolve_db_path(&fs, "artifact://x.db"), "承認されていません");
        // 生の絶対パス・相対移動
        expect_err(crate::app_fs::resolve_db_path(&fs, "C:/x.db"), "worktree://");
        expect_err(crate::app_fs::resolve_db_path(&fs, "worktree://data/../x.db"), "不正");
        let _ = std::fs::remove_dir_all(&base);

        // 既定拒否パスは、manifest で read / write に挙げても DB には使えない（allowSensitivePaths なし）
        let (base, fs) = setup("sens", &["worktree://.git/"], &["worktree://"], false);
        expect_err(crate::app_fs::resolve_db_path(&fs, "worktree://.git/x.db"), "拒否");
        expect_err(crate::app_fs::resolve_db_path(&fs, "worktree://sub/.claude/x.db"), "拒否");
        assert!(crate::app_fs::resolve_db_path(&fs, "worktree://sub/ok.db").is_ok());
        let _ = std::fs::remove_dir_all(&base);

        // allowSensitivePaths を承認していれば開ける
        let (base, fs) = setup("sens2", &[], &["worktree://"], true);
        assert!(crate::app_fs::resolve_db_path(&fs, "worktree://.git/x.db").is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn read_scope_is_read_only_even_through_query() {
        let (base, fs) = setup("ro", &["worktree://ro/"], &["worktree://rw/"], false);
        // write のスコープで DB を用意し、同じファイルを read のスコープ経由で開く
        std::fs::create_dir_all(base.join("wt").join("ro")).unwrap();
        run(async {
            // read のスコープだけの場所には、DB を新しく作れない（作成も書き込みもしない）
            let (path, writable) = crate::app_fs::resolve_db_path(&fs, "worktree://ro/new.db").unwrap();
            assert!(!writable);
            expect_err(open_connection(&path, writable).await.map(|_| ()), "DB を開けません");
            assert!(!base.join("wt").join("ro").join("new.db").exists());

            // 既存の DB は読める。書き込みは、query 経由の INSERT … RETURNING でも拒否される
            let seed = base.join("wt").join("ro").join("seed.db");
            let mut c = open_connection(&seed, true).await.unwrap();
            x(&mut c, "CREATE TABLE t (v INTEGER)").await.unwrap();
            x(&mut c, "INSERT INTO t VALUES (1)").await.unwrap();
            c.close().await.unwrap();

            let mut c = open_uri(&fs, "worktree://ro/seed.db").await.unwrap();
            assert_eq!(q(&mut c, "SELECT v FROM t").await.unwrap().rows, vec![vec![json!(1)]]);
            expect_err(q(&mut c, "INSERT INTO t VALUES (2) RETURNING v").await, "readonly");
            expect_err(x(&mut c, "INSERT INTO t VALUES (2)").await, "readonly");
            expect_err(x(&mut c, "CREATE TABLE u (v)").await, "readonly");
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn attach_vacuum_into_and_load_extension_are_denied() {
        let (base, fs) = setup("deny", &[], &["worktree://data/"], false);
        let outside = base.join("wt").join("outside.db");
        let outside_sql = outside.to_string_lossy().replace('\'', "''");
        run(async {
            let mut c = open_uri(&fs, "worktree://data/a.db").await.unwrap();
            x(&mut c, "CREATE TABLE t (v INTEGER)").await.unwrap();
            x(&mut c, "INSERT INTO t VALUES (1)").await.unwrap();

            // ATTACH（リテラル・バインド値・式）
            expect_err(x(&mut c, &format!("ATTACH DATABASE '{}' AS o", outside_sql)).await, "許可されていません");
            expect_err(
                run_execute(&mut c, "ATTACH DATABASE ? AS o", &[json!(outside.to_string_lossy())], &Limits::default())
                    .await,
                "許可されていません",
            );
            expect_err(x(&mut c, "ATTACH DATABASE 'a' || 'b' AS o").await, "許可されていません");
            // VACUUM INTO（リテラル・バインド値・スキーマ名つき）
            expect_err(x(&mut c, &format!("VACUUM INTO '{}'", outside_sql)).await, "許可されていません");
            expect_err(
                run_execute(&mut c, "VACUUM INTO ?", &[json!(outside.to_string_lossy())], &Limits::default()).await,
                "許可されていません",
            );
            expect_err(x(&mut c, &format!("VACUUM main INTO '{}'", outside_sql)).await, "許可されていません");
            // load_extension
            expect_err(q(&mut c, "SELECT load_extension('x')").await, "許可されていません");
            // 任意のディレクトリへ一時ファイルを作れる PRAGMA
            expect_err(x(&mut c, &format!("PRAGMA temp_store_directory = '{}'", outside_sql)).await, "許可されていません");
            expect_err(x(&mut c, "PRAGMA data_store_directory = 'x'").await, "許可されていません");

            // 拒否のあとも接続は使える。範囲外のファイルは作られていない
            assert_eq!(q(&mut c, "SELECT v FROM t").await.unwrap().rows, vec![vec![json!(1)]]);
            assert!(!outside.exists(), "outside.db must not be created");
            assert!(!base.join("wt").join("data").join("a.db-journal").exists());
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn plain_vacuum_and_temp_attach_still_work() {
        let (base, fs) = setup("vac", &[], &["worktree://data/"], false);
        run(async {
            let mut c = open_uri(&fs, "worktree://data/a.db").await.unwrap();
            x(&mut c, "CREATE TABLE t (v INTEGER)").await.unwrap();
            x(&mut c, "INSERT INTO t VALUES (1)").await.unwrap();
            x(&mut c, "VACUUM").await.unwrap();
            x(&mut c, "ATTACH DATABASE '' AS scratch").await.unwrap();
            x(&mut c, "CREATE TABLE scratch.s (v INTEGER)").await.unwrap();
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn runaway_query_is_interrupted_by_the_time_limit() {
        let (base, fs) = setup("time", &[], &["artifact://"], false);
        run(async {
            let mut c = open_uri(&fs, "artifact://t.db").await.unwrap();
            let limits = Limits { timeout: Duration::from_millis(300), ..Limits::default() };
            let started = Instant::now();
            let r = run_query(
                &mut c,
                "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
                &[],
                &limits,
            )
            .await;
            expect_err(r, "上限");
            assert!(started.elapsed() < Duration::from_secs(5), "must stop promptly");
            // 中断のあとも使える
            assert_eq!(q(&mut c, "SELECT 1").await.unwrap().rows, vec![vec![json!(1)]]);
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn result_size_is_limited() {
        let (base, fs) = setup("size", &[], &["artifact://"], false);
        run(async {
            let mut c = open_uri(&fs, "artifact://s.db").await.unwrap();
            let sql = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 100) SELECT x FROM c";
            let rows = Limits { max_rows: 10, ..Limits::default() };
            expect_err(run_query(&mut c, sql, &[], &rows).await, "行数");
            let bytes = Limits { max_bytes: 100, ..Limits::default() };
            expect_err(run_query(&mut c, sql, &[], &bytes).await, "サイズ");
            assert_eq!(run_query(&mut c, sql, &[], &Limits::default()).await.unwrap().rows.len(), 100);
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn bad_parameters_are_rejected() {
        let (base, fs) = setup("param", &[], &["artifact://"], false);
        run(async {
            let mut c = open_uri(&fs, "artifact://p.db").await.unwrap();
            let l = Limits::default();
            expect_err(run_query(&mut c, "SELECT ?", &[json!([1])], &l).await, "配列");
            expect_err(run_query(&mut c, "SELECT ?", &[json!({ "a": 1 })], &l).await, "$blob");
            expect_err(run_query(&mut c, "SELECT ?", &[json!({ "$blob": "!!!" })], &l).await, "base64");
            expect_err(run_query(&mut c, "  ", &[], &l).await, "空");
            // 値の型の往復
            let r = run_query(&mut c, "SELECT ?, ?, ?, ?", &[json!(true), json!(2), json!(0.5), json!("s")], &l)
                .await
                .unwrap();
            assert_eq!(r.rows, vec![vec![json!(1), json!(2), json!(0.5), json!("s")]]);
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn size_limits_cannot_be_lifted_from_sql() {
        let (base, fs) = setup("quota", &[], &["artifact://"], false);
        run(async {
            let mut c = open_uri(&fs, "artifact://q.db").await.unwrap();
            // 読むのは構わない。設定は拒否される
            let r = q(&mut c, "PRAGMA max_page_count").await.unwrap();
            assert_eq!(r.rows, vec![vec![json!(MAX_DB_BYTES / DEFAULT_PAGE_SIZE)]]);
            expect_err(x(&mut c, "PRAGMA max_page_count = 2147483646").await, "許可されていません");
            expect_err(x(&mut c, "PRAGMA page_size = 65536").await, "許可されていません");
            // 1 つの値の最大長（zeroblob での一括確保）
            expect_err(q(&mut c, "SELECT zeroblob(1000000000)").await, "too big");
            // 上限に達したら書き込みが失敗する（max_page_count を小さくして再現する代わりに、現在値で検算）
            x(&mut c, "CREATE TABLE t (v BLOB)").await.unwrap();
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn existing_wal_database_opens_read_only_and_keeps_its_journal_mode() {
        let (base, fs) = setup("wal", &["worktree://ro/"], &["worktree://rw/"], false);
        std::fs::create_dir_all(base.join("wt").join("ro")).unwrap();
        run(async {
            // ユーザーが WAL で作った DB を用意する
            let seed = base.join("wt").join("ro").join("wal.db");
            let mut c = open_connection(&seed, true).await.unwrap();
            q(&mut c, "PRAGMA journal_mode = WAL").await.unwrap();
            x(&mut c, "CREATE TABLE t (v INTEGER)").await.unwrap();
            x(&mut c, "INSERT INTO t VALUES (1)").await.unwrap();
            c.close().await.unwrap();

            // read スコープ（読み取り専用）で開いて読める
            let mut c = open_uri(&fs, "worktree://ro/wal.db").await.expect("open read-only");
            assert_eq!(q(&mut c, "SELECT v FROM t").await.unwrap().rows, vec![vec![json!(1)]]);
            c.close().await.unwrap();

            // write スコープで開いても、journal_mode を黙って変えない
            let mut c = open_connection(&seed, true).await.unwrap();
            assert_eq!(q(&mut c, "PRAGMA journal_mode").await.unwrap().rows, vec![vec![json!("wal")]]);
            c.close().await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn several_waiters_can_drain_closes_at_once() {
        let (base, fs) = setup("drain", &[], &["artifact://"], false);
        run(async {
            let reg = Arc::new(AppDbRegistry::default());
            let (path, writable) = crate::app_fs::resolve_db_path(&fs, "artifact://d.db").unwrap();
            let c = open_connection(&path, writable).await.unwrap();
            let shared = match reg.insert("appinst-1", (path.clone(), writable), c) {
                Ok(InsertOutcome::Inserted(s)) => s,
                _ => panic!("insert"),
            };
            // 接続を掴んだまま停止し、閉じが保留になっている間に複数の待ち手が同時に待つ
            let guard = shared.lock().await;
            reg.spawn_close(reg.take_instance("appinst-1", true));
            let waiters: Vec<_> = (0..3)
                .map(|_| {
                    let r = reg.clone();
                    tauri::async_runtime::spawn(async move { r.drain_closes().await })
                })
                .collect();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(waiters.iter().all(|w| !w.inner().is_finished()), "must still be waiting for the close");
            drop(guard);
            for w in waiters {
                tokio::time::timeout(Duration::from_secs(5), w).await.expect("drain finished").unwrap();
            }
            assert!(shared.lock().await.is_none(), "closed before drain returned");
            // 閉じてあるので、ファイルを消せる
            std::fs::remove_file(&path).expect("no open handle left");
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn registry_refuses_connections_after_the_instance_stopped() {
        let (base, fs) = setup("reg", &[], &["artifact://"], false);
        run(async {
            let reg = AppDbRegistry::default();
            let (path, writable) = crate::app_fs::resolve_db_path(&fs, "artifact://r.db").unwrap();
            let key = (path.clone(), writable);

            // 通常の登録と、同じキーへの二重登録（後から来た接続は台帳に入らない）
            let c1 = open_connection(&path, writable).await.unwrap();
            let shared = match reg.insert("appinst-1", key.clone(), c1) {
                Ok(InsertOutcome::Inserted(s)) => s,
                _ => panic!("first insert"),
            };
            let c2 = open_connection(&path, writable).await.unwrap();
            assert!(matches!(reg.insert("appinst-1", key.clone(), c2), Ok(InsertOutcome::Existing(_))));
            assert!(reg.get("appinst-1", &key).is_some());

            // 停止: 台帳から外れ、閉じるまで待てる。以後の登録は拒否される（開いている最中の接続の取りこぼし防止）
            let taken = reg.take_instance("appinst-1", true);
            assert_eq!(taken.len(), 1);
            close_all(taken).await;
            assert!(shared.lock().await.is_none(), "connection must be closed");
            let c3 = open_connection(&path, writable).await.unwrap();
            match reg.insert("appinst-1", key.clone(), c3) {
                Err((e, c)) => {
                    assert!(e.contains("停止"));
                    c.close().await.unwrap();
                }
                Ok(_) => panic!("must refuse after stop"),
            }

            // 権限の入れ替えでは「停止」扱いにしない（開き直せる）
            let c4 = open_connection(&path, writable).await.unwrap();
            assert!(matches!(reg.insert("appinst-2", key.clone(), c4), Ok(InsertOutcome::Inserted(_))));
            close_all(reg.take_instance("appinst-2", false)).await;
            let c5 = open_connection(&path, writable).await.unwrap();
            assert!(matches!(reg.insert("appinst-2", key.clone(), c5), Ok(InsertOutcome::Inserted(_))));
            close_all(reg.take_instance("appinst-2", true)).await;

            // 接続数の上限
            for i in 0..MAX_CONNECTIONS_PER_INSTANCE {
                let p = base.join("art").join(format!("n{}.db", i));
                let c = open_connection(&p, true).await.unwrap();
                assert!(matches!(reg.insert("appinst-3", (p, true), c), Ok(InsertOutcome::Inserted(_))));
            }
            let p = base.join("art").join("over.db");
            let c = open_connection(&p, true).await.unwrap();
            match reg.insert("appinst-3", (p, true), c) {
                Err((e, c)) => {
                    assert!(e.contains("同時に開ける"));
                    c.close().await.unwrap();
                }
                Ok(_) => panic!("must refuse over the limit"),
            }
            close_all(reg.take_instance("appinst-3", true)).await;
        });
        // 閉じてあるので、フォルダごと消せる（Windows で開いたままのファイルは消せない）
        std::fs::remove_dir_all(&base).expect("no open handles left");
    }
}
