//! ワークツリー間イベントの発行・購読・蓄積を担う DB レイヤ（issue #120 / #123）。
//!
//! 「あるワークツリーがクローズされたことを知りたいのは、そのクローズを実行した側が
//! 知らない第三者」であることが多いため、送信側が宛先を指定する push 方式ではなく
//! **知りたい側が subscribe する方式**を採る。
//!
//! ```text
//! 発行(events) → 照合(subscriptions) → 蓄積(inbox) → 配送(SessionStart / poll)
//! ```
//!
//! 購読の主キーは `terminal_id`（PTY spawn 時に oretachi が発番する UUID。issue #122）。
//! ワークツリー単位にすると同一ワークツリーの複数タブを区別できず、A タブ宛のメッセージを
//! B タブが抜き取る誤配送が発生する。
//!
//! settings.json には置かない: ワークツリー削除で settings が書き換わる最中に `closed`
//! イベントが飛ぶのでレースする。加えてワークツリーは settings（カウント用）と runtime
//! （表示用）の2配列で管理されており、この二重管理を増やすと分裂事故を招く。
//!
//! `sqlx` に `macros` / `migrate` feature が無いため `query!` マクロと `sqlx::migrate!` は
//! 使えない。`CREATE TABLE IF NOT EXISTS` を毎起動流す方式（task_db / archive_db と同じ）。

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::Manager;

/// `manage()` で複数の SqlitePool を区別するためのnewtypeラッパー
pub struct EventPool(pub SqlitePool);

/// イベント種別: ワークツリーがクローズ（アーカイブ / 削除）された。
pub const KIND_WORKTREE_CLOSED: &str = "worktree.closed";

/// Phase 1 で受け付けるイベント種別。`worktree.created` / `worktree.message` は #126。
pub const SUPPORTED_EVENT_KINDS: &[&str] = &[KIND_WORKTREE_CLOSED];

/// 配送戦略。`interrupt`（即 PTY 押し込み）は #125 で追加する。
pub const DELIVERY_TURN_END: &str = "turn_end";
pub const DELIVERY_PASSIVE: &str = "passive";
pub const SUPPORTED_DELIVERIES: &[&str] = &[DELIVERY_TURN_END, DELIVERY_PASSIVE];

pub const STATE_ACTIVE: &str = "active";
pub const STATE_PENDING: &str = "pending";
pub const STATE_ACKED: &str = "acked";

/// inbox の保持期限。積みっぱなしで肥大するのを防ぐ（#120 §5.6）。
/// ack 済み・未 ack を問わず `created_at` からこの期間で削除する。
pub const INBOX_RETENTION_DAYS: i64 = 30;
pub const INBOX_RETENTION_MS: i64 = INBOX_RETENTION_DAYS * 24 * 60 * 60 * 1000;

/// sqlite の書き込みロック待ち上限。SessionStart フックのリクエストパスから触るため、
/// サイドカーの読み取りタイムアウト（2 秒）より短くする。
const BUSY_TIMEOUT_MS: u64 = 800;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct SubscriptionRow {
    pub id: String,
    /// 購読主体。PTY タブと 1:1 の UUID（`ORETACHI_TERMINAL_ID`）
    pub subscriber_terminal_id: String,
    /// タブが死んだ後の復活先の解決用（不変）。逆引き不能なら None
    pub subscriber_worktree_id: Option<String>,
    /// 最後に見た Claude Code の session UUID（監査・resume 追跡用）
    pub subscriber_agent_session: Option<String>,
    /// 購読対象。Phase 1 はワークツリー ID 固定（`*` / `workgroup:` / `repo:` は #126）
    pub target: String,
    /// JSON string: string[]
    pub event_kinds: String,
    pub delivery: String,
    /// 0 / 1。Phase 1 では保存するだけで消費しない（#125 で spawn する）
    pub spawn_if_closed: i64,
    pub created_at: i64,
    /// None は無期限
    pub expires_at: Option<i64>,
    /// active | orphaned（orphaned への遷移は #125）
    pub state: String,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct EventRow {
    pub id: String,
    pub source_worktree_id: String,
    /// 発火元タブ。`worktree.closed` は close 経路が terminal_id を運んでいないため None
    pub source_terminal_id: Option<String>,
    pub kind: String,
    /// JSON string: 種別ごとの本体
    pub body: String,
    pub actor: Option<String>,
    pub created_at: i64,
}

/// inbox 1件と、それが指すイベントを結合した配送単位。
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct InboxItem {
    pub id: String,
    pub subscriber_terminal_id: String,
    /// 積んだ時点の購読者ワークツリー。購読が先に消えても掃除できるよう inbox 側にも持つ
    pub subscriber_worktree_id: Option<String>,
    pub event_id: String,
    pub state: String,
    pub created_at: i64,
    pub delivered_at: Option<i64>,
    pub acked_at: Option<i64>,
    // events からの結合分
    pub kind: String,
    pub body: String,
    pub source_worktree_id: String,
    pub actor: Option<String>,
}

/// UNIX epoch からのミリ秒。task/archive DB と同じ `i64` 流儀で時刻を持つ。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub async fn init_event_db(
    app: &tauri::AppHandle,
) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;
    let db_path = app_data_dir.join("events.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    // この DB は SessionStart フックのリクエストパス（`/session-context`）から読み書きされる。
    // サイドカーの読み取りタイムアウトは 2 秒なので、書き込みロック競合で sqlx 既定の
    // 5 秒待つとレスポンスが届かない。それより短く切ってエラーで返し、
    // 「配送済みにしたのに注入されなかった」状態を作らないようにする。
    let options = db_url
        .parse::<sqlx::sqlite::SqliteConnectOptions>()?
        .busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS));
    let pool = SqlitePool::connect_with(options).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS subscriptions (
            id                       TEXT PRIMARY KEY,
            subscriber_terminal_id   TEXT NOT NULL,
            subscriber_worktree_id   TEXT,
            subscriber_agent_session TEXT,
            target                   TEXT NOT NULL,
            event_kinds              TEXT NOT NULL DEFAULT '[]',
            delivery                 TEXT NOT NULL,
            spawn_if_closed          INTEGER NOT NULL DEFAULT 0,
            created_at               INTEGER NOT NULL,
            expires_at               INTEGER,
            state                    TEXT NOT NULL DEFAULT 'active'
        )"#,
    )
    .execute(pool)
    .await?;
    // 同じタブが同じ対象を二重購読しないようにする（再 subscribe は上書き）
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_subs_terminal_target ON subscriptions(subscriber_terminal_id, target)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_subs_target ON subscriptions(target, state)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_subs_worktree ON subscriptions(subscriber_worktree_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS events (
            id                 TEXT PRIMARY KEY,
            source_worktree_id TEXT NOT NULL,
            source_terminal_id TEXT,
            kind               TEXT NOT NULL,
            body               TEXT NOT NULL DEFAULT '{}',
            actor              TEXT,
            created_at         INTEGER NOT NULL
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_created_at ON events(created_at DESC)")
        .execute(pool)
        .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS inbox (
            id                     TEXT PRIMARY KEY,
            subscriber_terminal_id TEXT NOT NULL,
            subscriber_worktree_id TEXT,
            event_id               TEXT NOT NULL,
            state                  TEXT NOT NULL DEFAULT 'pending',
            created_at             INTEGER NOT NULL,
            delivered_at           INTEGER,
            acked_at               INTEGER,
            UNIQUE(subscriber_terminal_id, event_id)
        )"#,
    )
    .execute(pool)
    .await?;
    // 既存 DB 向けマイグレーション: 購読者ワークツリー（購読行が先に消えても掃除できるように
    // inbox 側にも持たせる）。既にあればエラーを握りつぶす。
    let _ = sqlx::query("ALTER TABLE inbox ADD COLUMN subscriber_worktree_id TEXT")
        .execute(pool)
        .await;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_inbox_terminal ON inbox(subscriber_terminal_id, state)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_inbox_worktree ON inbox(subscriber_worktree_id)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_inbox_created_at ON inbox(created_at)")
        .execute(pool)
        .await?;
    Ok(())
}

// ─── 購読 ─────────────────────────────────────────────────────────────────────

/// 購読を登録する。`(subscriber_terminal_id, target)` が既にあれば内容を更新する。
///
/// `INSERT OR REPLACE` ではなく `ON CONFLICT DO UPDATE` を使う。前者は既存行を
/// 「削除して挿入」するため id が変わり、エージェントが握っていた `subscription_id` で
/// 解除できなくなる。id を保つことで再購読しても解除手段が失われない。
///
/// 戻り値は**実際に DB に入っている購読 ID**。既存行を更新した場合は `sub.id` ではなく
/// 既存の id が返るので、呼び出し元はこれをエージェントへ返すこと。
pub async fn upsert_subscription(pool: &SqlitePool, sub: &SubscriptionRow) -> Result<String, String> {
    sqlx::query(
        "INSERT INTO subscriptions (id, subscriber_terminal_id, subscriber_worktree_id, subscriber_agent_session, target, event_kinds, delivery, spawn_if_closed, created_at, expires_at, state) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(subscriber_terminal_id, target) DO UPDATE SET \
           subscriber_worktree_id = excluded.subscriber_worktree_id, \
           subscriber_agent_session = excluded.subscriber_agent_session, \
           event_kinds = excluded.event_kinds, \
           delivery = excluded.delivery, \
           spawn_if_closed = excluded.spawn_if_closed, \
           expires_at = excluded.expires_at, \
           state = excluded.state",
    )
    .bind(&sub.id)
    .bind(&sub.subscriber_terminal_id)
    .bind(&sub.subscriber_worktree_id)
    .bind(&sub.subscriber_agent_session)
    .bind(&sub.target)
    .bind(&sub.event_kinds)
    .bind(&sub.delivery)
    .bind(sub.spawn_if_closed)
    .bind(sub.created_at)
    .bind(sub.expires_at)
    .bind(&sub.state)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let stored: (String,) = sqlx::query_as(
        "SELECT id FROM subscriptions WHERE subscriber_terminal_id = ? AND target = ?",
    )
    .bind(&sub.subscriber_terminal_id)
    .bind(&sub.target)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(stored.0)
}

/// 購読 ID で解除する。渡された terminal_id の購読だけを対象にする（誤って他タブの購読を
/// 消さないためのスコープ制約。呼び出し元の本人性は MCP 経路では保証できない）。
/// 戻り値は削除件数。
pub async fn delete_subscription(
    pool: &SqlitePool,
    id: &str,
    terminal_id: &str,
) -> Result<u64, String> {
    let result =
        sqlx::query("DELETE FROM subscriptions WHERE id = ? AND subscriber_terminal_id = ?")
            .bind(id)
            .bind(terminal_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(result.rows_affected())
}

/// 購読対象で解除する。戻り値は削除件数。
pub async fn delete_subscription_by_target(
    pool: &SqlitePool,
    terminal_id: &str,
    target: &str,
) -> Result<u64, String> {
    let result =
        sqlx::query("DELETE FROM subscriptions WHERE subscriber_terminal_id = ? AND target = ?")
            .bind(terminal_id)
            .bind(target)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(result.rows_affected())
}

/// 指定タブの有効な購読を返す（失効分は除外。行の削除は `purge_expired` に任せる）。
pub async fn list_subscriptions(
    pool: &SqlitePool,
    terminal_id: &str,
    now: i64,
) -> Result<Vec<SubscriptionRow>, String> {
    sqlx::query_as::<_, SubscriptionRow>(
        "SELECT * FROM subscriptions WHERE subscriber_terminal_id = ? AND (expires_at IS NULL OR expires_at > ?) ORDER BY created_at DESC",
    )
    .bind(terminal_id)
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

// ─── イベント発行と照合 ───────────────────────────────────────────────────────

pub async fn insert_event(pool: &SqlitePool, event: &EventRow) -> Result<(), String> {
    sqlx::query(
        "INSERT OR REPLACE INTO events (id, source_worktree_id, source_terminal_id, kind, body, actor, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&event.id)
    .bind(&event.source_worktree_id)
    .bind(&event.source_terminal_id)
    .bind(&event.kind)
    .bind(&event.body)
    .bind(&event.actor)
    .bind(event.created_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// `event_kinds`（JSON 配列文字列）に `kind` が含まれるか。
/// 空配列 / パース失敗は「絞り込み無し」とはみなさず false（意図しない全配送を防ぐ）。
pub fn event_kinds_match(event_kinds_json: &str, kind: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(event_kinds_json) else {
        return false;
    };
    let Some(arr) = parsed.as_array() else {
        return false;
    };
    arr.iter().any(|v| v.as_str() == Some(kind))
}

/// 自己エコー抑止（#120 §5.4）。発火元へ配り返すと往復ループの起点になる。
///
/// - 同じタブが発火元なら配送しない
/// - 発火元ワークツリーに属するタブへも配送しない。`worktree.closed` は close 経路が
///   terminal_id を運んでいないので `source_terminal_id` が None であり、実質こちらが効く
///   （そもそも閉じたワークツリーのタブは既に存在しないので配送先として無意味）
pub fn is_self_echo(sub: &SubscriptionRow, event: &EventRow) -> bool {
    if event.source_terminal_id.as_deref() == Some(sub.subscriber_terminal_id.as_str()) {
        return true;
    }
    sub.subscriber_worktree_id.as_deref() == Some(event.source_worktree_id.as_str())
}

/// イベントを購読と照合し、該当する購読者の inbox へ積む。戻り値は積んだ件数。
///
/// SQL では target / state / 失効までを絞り、`event_kinds` の突合と自己エコー抑止は
/// Rust 側の純粋関数で行う（JSON カラムは SQL で扱いづらい。task_db の
/// `steps_match_worktree` と同じ流儀）。
///
/// 複数の購読が同じイベントを拾ったときの重複は `inbox` の
/// `UNIQUE(subscriber_terminal_id, event_id)` + `INSERT OR IGNORE` でマージされる。
pub async fn fanout(pool: &SqlitePool, event: &EventRow, now: i64) -> Result<usize, String> {
    let candidates = sqlx::query_as::<_, SubscriptionRow>(
        "SELECT * FROM subscriptions WHERE target = ? AND state = ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(&event.source_worktree_id)
    .bind(STATE_ACTIVE)
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut delivered = 0usize;
    for sub in candidates {
        if !event_kinds_match(&sub.event_kinds, &event.kind) {
            continue;
        }
        if is_self_echo(&sub, event) {
            log::debug!(
                "[event-db] self-echo suppressed: sub={} terminal={} event={}",
                sub.id,
                sub.subscriber_terminal_id,
                event.id
            );
            continue;
        }
        let result = sqlx::query(
            "INSERT OR IGNORE INTO inbox (id, subscriber_terminal_id, subscriber_worktree_id, event_id, state, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&sub.subscriber_terminal_id)
        .bind(&sub.subscriber_worktree_id)
        .bind(&event.id)
        .bind(STATE_PENDING)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        delivered += result.rows_affected() as usize;
    }
    Ok(delivered)
}

// ─── inbox ────────────────────────────────────────────────────────────────────

/// `list_inbox` の絞り込み条件。
///
/// 「注入したら消す」ではなく `delivered_at`（送った）と `acked_at`（AI が確認した）を
/// 分けて残す方針（#120 §5.2）。**再送はしない**が未 ack は残るので、サイドカー失敗で
/// 注入が消えても人間が気づける（at-least-once は狙わない割り切り）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxFilter {
    /// まだ一度も送っていないもの。自動注入（SessionStart）はこれだけを使う。
    /// 未 ack 全件を毎セッション再掲すると「再送はしない」方針に反する。
    Undelivered,
    /// 未 ack のもの。エージェントが自分で取りに来る `oretachi_poll_inbox` 用。
    /// 自動注入を取りこぼしても、ここから必ず回収できる。
    Unacked,
    /// ack 済みも含む全件（監査・確認用）。
    All,
}

impl InboxFilter {
    fn where_clause(self) -> &'static str {
        match self {
            InboxFilter::Undelivered => " AND i.delivered_at IS NULL",
            InboxFilter::Unacked => " AND i.acked_at IS NULL",
            InboxFilter::All => "",
        }
    }
}

/// 指定タブの inbox を条件付きで返す。
pub async fn list_inbox(
    pool: &SqlitePool,
    terminal_id: &str,
    filter: InboxFilter,
) -> Result<Vec<InboxItem>, String> {
    let sql = format!(
        "SELECT i.id, i.subscriber_terminal_id, i.subscriber_worktree_id, i.event_id, i.state, i.created_at, i.delivered_at, i.acked_at, e.kind, e.body, e.source_worktree_id, e.actor \
         FROM inbox i JOIN events e ON e.id = i.event_id \
         WHERE i.subscriber_terminal_id = ?{} ORDER BY i.created_at ASC",
        filter.where_clause()
    );
    sqlx::query_as::<_, InboxItem>(&sql)
        .bind(terminal_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())
}

/// 未 ack 件数（未配送 / 配送済み未 ack の内訳付き）を返す。
pub async fn count_unacked(pool: &SqlitePool, terminal_id: &str) -> Result<(i64, i64), String> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN delivered_at IS NULL THEN 1 ELSE 0 END), 0) FROM inbox WHERE subscriber_terminal_id = ? AND acked_at IS NULL",
    )
    .bind(terminal_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row)
}

/// `?, ?, ?` 形式のプレースホルダを件数分作る（sqlx のランタイム版に配列バインドは無い）。
fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// 「送った」印を打つ。既に打刻済みの行は触らない（初回配送時刻を保つ）。
///
/// 1件ずつ UPDATE すると書き込みロックの取得を件数分繰り返し、競合時の待ち時間が
/// 件数倍になる。SessionStart フックのリクエストパスから呼ばれるので1文にまとめる。
pub async fn mark_delivered(pool: &SqlitePool, ids: &[String], now: i64) -> Result<u64, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "UPDATE inbox SET delivered_at = ? WHERE delivered_at IS NULL AND id IN ({})",
        placeholders(ids.len())
    );
    let mut q = sqlx::query(&sql).bind(now);
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected())
}

/// 「AI が確認した」印を打つ。ack 済みの再 ack は 0 件更新で成功扱い（冪等）。
/// 他タブ宛のメッセージは `subscriber_terminal_id` で弾かれるので件数に入らない。
pub async fn ack(
    pool: &SqlitePool,
    ids: &[String],
    terminal_id: &str,
    now: i64,
) -> Result<u64, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "UPDATE inbox SET acked_at = ?, state = ? WHERE subscriber_terminal_id = ? AND acked_at IS NULL AND id IN ({})",
        placeholders(ids.len())
    );
    let mut q = sqlx::query(&sql).bind(now).bind(STATE_ACKED).bind(terminal_id);
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected())
}

// ─── 掃除 ─────────────────────────────────────────────────────────────────────

/// 購読者ワークツリーがクローズされたときに、そのワークツリーのタブが持っていた
/// subscriptions / inbox を掃除する（#120 §5.6。しないとゴミが永久に残る）。
///
/// inbox は `subscriber_worktree_id` を自分で持っているので、購読行が先に消えていても
/// （unsubscribe 済み・失効削除済み）取り残さない。旧スキーマで積まれた
/// `subscriber_worktree_id IS NULL` の行だけは subscriptions 経由で引く。
pub async fn purge_subscriber_worktree(
    pool: &SqlitePool,
    worktree_id: &str,
) -> Result<(u64, u64), String> {
    let mut inbox_deleted = sqlx::query("DELETE FROM inbox WHERE subscriber_worktree_id = ?")
        .bind(worktree_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();

    // 後方互換: subscriber_worktree_id カラム追加前に積まれた行
    inbox_deleted += sqlx::query(
        "DELETE FROM inbox WHERE subscriber_worktree_id IS NULL AND subscriber_terminal_id IN (SELECT subscriber_terminal_id FROM subscriptions WHERE subscriber_worktree_id = ?)",
    )
    .bind(worktree_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    let subs_deleted = sqlx::query("DELETE FROM subscriptions WHERE subscriber_worktree_id = ?")
        .bind(worktree_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();

    Ok((subs_deleted, inbox_deleted))
}

/// 購読対象がクローズされたので、その対象を指す購読を削除する。
///
/// ワークツリー ID は再利用されないため、クローズ済み target を指す購読は二度と
/// マッチしない。放置すると `list_subscriptions` に `targetWorktreeName: null` の
/// 死んだ行として残り続ける（既定の購読は無期限なので `purge_expired` でも消えない）。
/// **配送（`fanout`）を終えた後に呼ぶこと。**
pub async fn delete_subscriptions_for_target(
    pool: &SqlitePool,
    target: &str,
) -> Result<u64, String> {
    Ok(sqlx::query("DELETE FROM subscriptions WHERE target = ?")
        .bind(target)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected())
}

/// 生存しているターミナルに紐づかない購読とその inbox を削除する。
///
/// `terminal_id` は PTY spawn ごとに発番され永続化されないため、**アプリを再起動すると
/// 既存の全購読はどのターミナルにも到達できなくなる**（`resolve_subscriber` が
/// 生存セッションしか受け付けないので list / unsubscribe / poll すべて不能）。
/// そのまま残すと `fanout` が死んだ宛先へ inbox を積み続け、既定が無期限の購読は
/// 単調増加する。到達不能な行は掃除して `list_subscriptions` を実態に合わせる。
///
/// 起動時は生存セッションが 0 件なので全購読が対象になる（これは仕様どおり:
/// 再起動を挟んだ購読は復活できない。タブ死亡時の復活は #125 の担当）。
pub async fn purge_orphaned_subscribers(
    pool: &SqlitePool,
    live_terminal_ids: &[String],
) -> Result<(u64, u64), String> {
    let keep = if live_terminal_ids.is_empty() {
        String::new()
    } else {
        format!(
            " AND subscriber_terminal_id NOT IN ({})",
            placeholders(live_terminal_ids.len())
        )
    };

    let inbox_sql = format!("DELETE FROM inbox WHERE 1=1{}", keep);
    let mut q = sqlx::query(&inbox_sql);
    for id in live_terminal_ids {
        q = q.bind(id);
    }
    let inbox_deleted = q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();

    let subs_sql = format!("DELETE FROM subscriptions WHERE 1=1{}", keep);
    let mut q = sqlx::query(&subs_sql);
    for id in live_terminal_ids {
        q = q.bind(id);
    }
    let subs_deleted = q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();

    Ok((subs_deleted, inbox_deleted))
}

/// 失効した購読と保持期限切れの inbox / 参照されなくなった events を削除する。
/// 起動時と各 fanout 後に回す。戻り値は (subscriptions, inbox, events) の削除件数。
pub async fn purge_expired(
    pool: &SqlitePool,
    now: i64,
    inbox_retention_ms: i64,
) -> Result<(u64, u64, u64), String> {
    // 失効した購読の inbox を先に落とす（購読行が消えた後では
    // `purge_subscriber_worktree` の後方互換パスから引けなくなる）。
    // 同じターミナルの**他の**購読宛のメッセージを巻き込まないよう event 単位で絞る。
    let mut inbox = sqlx::query(
        "DELETE FROM inbox WHERE EXISTS ( \
           SELECT 1 FROM subscriptions s JOIN events e ON e.id = inbox.event_id \
           WHERE s.subscriber_terminal_id = inbox.subscriber_terminal_id \
             AND s.target = e.source_worktree_id \
             AND s.expires_at IS NOT NULL AND s.expires_at <= ? )",
    )
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    let subs = sqlx::query("DELETE FROM subscriptions WHERE expires_at IS NOT NULL AND expires_at <= ?")
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();

    inbox += sqlx::query("DELETE FROM inbox WHERE created_at <= ?")
        .bind(now - inbox_retention_ms)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();

    // どの inbox からも参照されなくなった古いイベントを落とす（events だけ残しても使い道がない）
    let events = sqlx::query(
        "DELETE FROM events WHERE created_at <= ? AND id NOT IN (SELECT event_id FROM inbox)",
    )
    .bind(now - inbox_retention_ms)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    Ok((subs, inbox, events))
}

// ─── 表示用の整形 ─────────────────────────────────────────────────────────────

/// `worktree.closed` の body。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorktreeClosedBody {
    #[serde(rename = "worktreeId")]
    pub worktree_id: String,
    #[serde(rename = "worktreeName")]
    pub worktree_name: String,
    #[serde(rename = "branchName")]
    pub branch_name: String,
}

/// inbox 1件を人間 / AI が読める1行に整形する。
pub fn format_inbox_line(item: &InboxItem) -> String {
    let detail = match item.kind.as_str() {
        KIND_WORKTREE_CLOSED => serde_json::from_str::<WorktreeClosedBody>(&item.body)
            .map(|b| {
                let branch = if b.branch_name.trim().is_empty() {
                    String::new()
                } else {
                    format!("（ブランチ: {}）", b.branch_name)
                };
                format!(
                    "ワークツリー '{}' {} がクローズされました",
                    b.worktree_name, branch
                )
            })
            .unwrap_or_else(|_| format!("ワークツリー ID '{}' がクローズされました", item.source_worktree_id)),
        other => format!("イベント '{}': {}", other, item.body),
    };
    format!("- [{}] {}", item.id, detail)
}

/// SessionStart 注入用のテキストを組み立てる。
///
/// - `items`: 今回初めて配送する未配送分。本文を列挙する
/// - `carryover`: 既に配送済みだが未 ack のまま残っている件数。**本文は再掲しない**
///   （#120 §5.2 の「再送はしない」）が、件数だけは毎回知らせる。注入が失われた場合の
///   唯一の気付き手段がこれ（Phase 1 には UI が無い）。
///
/// どちらも空なら None（呼び出し側で注入をスキップできるようにする）。
pub fn format_inbox_digest(items: &[InboxItem], carryover: i64) -> Option<String> {
    if items.is_empty() && carryover <= 0 {
        return None;
    }
    if items.is_empty() {
        return Some(format!(
            "[oretachi] 未確認（未 ack）のワークツリーイベントが {} 件残っています。\
             oretachi_poll_inbox で内容を確認し、oretachi_ack_message で ack してください。",
            carryover
        ));
    }
    let lines: Vec<String> = items.iter().map(format_inbox_line).collect();
    let carryover_note = if carryover > 0 {
        format!(
            "\nこのほかに未 ack のまま残っているものが {} 件あります（oretachi_poll_inbox で確認できます）。",
            carryover
        )
    } else {
        String::new()
    };
    Some(format!(
        "[oretachi] 購読していたワークツリーイベントが {} 件届いています。\n{}\n\n\
         購読していた作業が完了したということなので、必要な動作確認や後続作業を進めてください。\
         内容を確認したら oretachi_ack_message に上記の [] 内の ID を渡して ack してください。\
         この本文は自動では再掲されません（ack しないまま忘れた場合は oretachi_poll_inbox で取り直せます）。{}",
        items.len(),
        lines.join("\n"),
        carryover_note
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(terminal: &str, worktree: Option<&str>, kinds: &str) -> SubscriptionRow {
        SubscriptionRow {
            id: "sub-1".to_string(),
            subscriber_terminal_id: terminal.to_string(),
            subscriber_worktree_id: worktree.map(str::to_string),
            subscriber_agent_session: None,
            target: "wt-target".to_string(),
            event_kinds: kinds.to_string(),
            delivery: DELIVERY_TURN_END.to_string(),
            spawn_if_closed: 0,
            created_at: 0,
            expires_at: None,
            state: STATE_ACTIVE.to_string(),
        }
    }

    fn event(source_worktree: &str, source_terminal: Option<&str>) -> EventRow {
        EventRow {
            id: "ev-1".to_string(),
            source_worktree_id: source_worktree.to_string(),
            source_terminal_id: source_terminal.map(str::to_string),
            kind: KIND_WORKTREE_CLOSED.to_string(),
            body: r#"{"worktreeId":"wt-target","worktreeName":"oretachi-abcd","branchName":"feature/x"}"#
                .to_string(),
            actor: Some("archive".to_string()),
            created_at: 100,
        }
    }

    #[test]
    fn test_event_kinds_match() {
        assert!(event_kinds_match(r#"["worktree.closed"]"#, KIND_WORKTREE_CLOSED));
        assert!(event_kinds_match(
            r#"["worktree.created","worktree.closed"]"#,
            KIND_WORKTREE_CLOSED
        ));
        assert!(!event_kinds_match(r#"["worktree.created"]"#, KIND_WORKTREE_CLOSED));
        // 空配列・不正 JSON・配列でない値はすべて「一致しない」（意図しない全配送を防ぐ）
        assert!(!event_kinds_match("[]", KIND_WORKTREE_CLOSED));
        assert!(!event_kinds_match("not json", KIND_WORKTREE_CLOSED));
        assert!(!event_kinds_match(r#""worktree.closed""#, KIND_WORKTREE_CLOSED));
    }

    #[test]
    fn test_is_self_echo_same_terminal() {
        let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
        let e = event("wt-target", Some("term-a"));
        assert!(is_self_echo(&s, &e));
    }

    #[test]
    fn test_is_self_echo_same_worktree() {
        // 発火元ワークツリーに属するタブには配らない
        let s = sub("term-a", Some("wt-target"), r#"["worktree.closed"]"#);
        let e = event("wt-target", None);
        assert!(is_self_echo(&s, &e));
    }

    #[test]
    fn test_is_self_echo_third_party() {
        let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
        let e = event("wt-target", Some("term-b"));
        assert!(!is_self_echo(&s, &e));
    }

    #[test]
    fn test_is_self_echo_unresolved_worktree() {
        // 購読者のワークツリーが逆引きできなかった場合は worktree 比較では抑止しない
        let s = sub("term-a", None, r#"["worktree.closed"]"#);
        let e = event("wt-target", None);
        assert!(!is_self_echo(&s, &e));
    }

    fn inbox_item(kind: &str, body: &str) -> InboxItem {
        InboxItem {
            id: "inbox-1".to_string(),
            subscriber_terminal_id: "term-a".to_string(),
            subscriber_worktree_id: Some("wt-subscriber".to_string()),
            event_id: "ev-1".to_string(),
            state: STATE_PENDING.to_string(),
            created_at: 100,
            delivered_at: None,
            acked_at: None,
            kind: kind.to_string(),
            body: body.to_string(),
            source_worktree_id: "wt-target".to_string(),
            actor: None,
        }
    }

    #[test]
    fn test_format_inbox_line_worktree_closed() {
        let item = inbox_item(
            KIND_WORKTREE_CLOSED,
            r#"{"worktreeId":"wt-target","worktreeName":"oretachi-abcd","branchName":"feature/x"}"#,
        );
        let line = format_inbox_line(&item);
        assert!(line.contains("inbox-1"));
        assert!(line.contains("oretachi-abcd"));
        assert!(line.contains("feature/x"));
    }

    #[test]
    fn test_format_inbox_line_body_broken_falls_back_to_id() {
        let item = inbox_item(KIND_WORKTREE_CLOSED, "{broken");
        let line = format_inbox_line(&item);
        assert!(line.contains("wt-target"));
    }

    #[test]
    fn test_format_inbox_line_omits_empty_branch() {
        let item = inbox_item(
            KIND_WORKTREE_CLOSED,
            r#"{"worktreeId":"wt-target","worktreeName":"oretachi-abcd","branchName":""}"#,
        );
        let line = format_inbox_line(&item);
        assert!(line.contains("oretachi-abcd"));
        assert!(!line.contains("ブランチ"));
    }

    #[test]
    fn test_format_inbox_digest_empty_is_none() {
        assert_eq!(format_inbox_digest(&[], 0), None);
    }

    /// 未配送が無くても未 ack が残っていれば件数だけは伝える
    /// （注入喪失に気づく唯一の手段。本文は再掲しない）。
    #[test]
    fn test_format_inbox_digest_carryover_only_mentions_count_without_body() {
        let digest = format_inbox_digest(&[], 3).unwrap();
        assert!(digest.contains("3 件"));
        assert!(digest.contains("oretachi_poll_inbox"));
        assert!(!digest.contains("がクローズされました"));
    }

    #[test]
    fn test_format_inbox_digest_appends_carryover_note() {
        let items = vec![inbox_item(
            KIND_WORKTREE_CLOSED,
            r#"{"worktreeId":"a","worktreeName":"wt-a","branchName":"b1"}"#,
        )];
        let digest = format_inbox_digest(&items, 2).unwrap();
        assert!(digest.contains("wt-a"));
        assert!(digest.contains("2 件"));
    }

    /// インメモリ DB を1本の接続で開く。`sqlite::memory:` は接続ごとに別 DB になるため
    /// `max_connections(1)` が必須（プールが2本目を張ると空の DB を見てしまう）。
    ///
    /// `tokio` に `macros` / `rt` feature が無く `#[tokio::test]` が使えないので、
    /// 同期テストの中から `tauri::async_runtime::block_on` で回す。
    fn with_pool<F, T>(f: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        tauri::async_runtime::block_on(f)
    }

    async fn memory_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        run_migrations(&pool).await.expect("migrate");
        pool
    }

    /// SQL 文と FromRow のマッピングを一通り通す往復テスト（cargo check では検出できない）。
    #[test]
    fn test_roundtrip_subscribe_fanout_deliver_ack() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let mut s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            let first_id = upsert_subscription(&pool, &s).await.unwrap();
            assert_eq!(first_id, s.id);
            // 同じ (terminal, target) の再購読は増えず内容だけ更新される。
            // id は既存のものが維持され、エージェントが持っている subscription_id が生き残る
            s.id = "ignored-new-id".to_string();
            s.delivery = DELIVERY_PASSIVE.to_string();
            let second_id = upsert_subscription(&pool, &s).await.unwrap();
            assert_eq!(second_id, first_id);
            let subs = list_subscriptions(&pool, "term-a", now).await.unwrap();
            assert_eq!(subs.len(), 1);
            assert_eq!(subs[0].id, first_id);
            assert_eq!(subs[0].delivery, DELIVERY_PASSIVE);
            s.delivery = DELIVERY_TURN_END.to_string();

            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout(&pool, &e, now).await.unwrap(), 1);
            // 同じイベントの再 fanout は UNIQUE 制約でマージされる
            assert_eq!(fanout(&pool, &e, now).await.unwrap(), 0);

            let items = list_inbox(&pool, "term-a", InboxFilter::Unacked).await.unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].kind, KIND_WORKTREE_CLOSED);
            assert_eq!(items[0].source_worktree_id, "wt-target");
            assert!(items[0].delivered_at.is_none());
            assert_eq!(count_unacked(&pool, "term-a").await.unwrap(), (1, 1));

            let ids = vec![items[0].id.clone()];
            assert_eq!(mark_delivered(&pool, &ids, now).await.unwrap(), 1);
            // 二度目は初回配送時刻を保つため更新しない
            assert_eq!(mark_delivered(&pool, &ids, now + 5).await.unwrap(), 0);
            assert_eq!(count_unacked(&pool, "term-a").await.unwrap(), (1, 0));

            assert_eq!(ack(&pool, &ids, "term-a", now).await.unwrap(), 1);
            assert_eq!(ack(&pool, &ids, "term-a", now).await.unwrap(), 0); // 冪等
            assert!(list_inbox(&pool, "term-a", InboxFilter::Unacked).await.unwrap().is_empty());
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().len(), 1);
            assert_eq!(count_unacked(&pool, "term-a").await.unwrap(), (0, 0));
        });
    }

    /// 自動注入（SessionStart）は「再送しない」（#120 §5.2）。
    /// 一度配送したものは Undelivered に出てこないが、Unacked（明示 pull）には残る。
    #[test]
    fn test_roundtrip_undelivered_is_not_resent_but_stays_unacked() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout(&pool, &e, now).await.unwrap();

            // 1回目の自動注入では拾える
            let first = list_inbox(&pool, "term-a", InboxFilter::Undelivered).await.unwrap();
            assert_eq!(first.len(), 1);
            mark_delivered(&pool, &[first[0].id.clone()], now).await.unwrap();

            // 2回目の自動注入では拾わない（再送しない）
            assert!(list_inbox(&pool, "term-a", InboxFilter::Undelivered)
                .await
                .unwrap()
                .is_empty());
            // ただし未 ack として残るので明示 pull では取り直せる
            assert_eq!(
                list_inbox(&pool, "term-a", InboxFilter::Unacked).await.unwrap().len(),
                1
            );
        });
    }

    /// 同一ワークツリーの2タブがそれぞれ購読しても、相手の inbox を覗けない。
    #[test]
    fn test_roundtrip_two_tabs_same_worktree_are_isolated() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let mut a = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &a).await.unwrap();
            // タブ2 は購読していない
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout(&pool, &e, now).await.unwrap();

            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::Unacked).await.unwrap().len(), 1);
            assert!(list_inbox(&pool, "term-b", InboxFilter::Unacked).await.unwrap().is_empty());
            // 他タブの ID を渡しても ack できない
            let items = list_inbox(&pool, "term-a", InboxFilter::Unacked).await.unwrap();
            let ids = vec![items[0].id.clone()];
            assert_eq!(ack(&pool, &ids, "term-b", now).await.unwrap(), 0);

            // タブ2 も購読すれば両方に届く
            a.id = "sub-b".to_string();
            a.subscriber_terminal_id = "term-b".to_string();
            upsert_subscription(&pool, &a).await.unwrap();
            let e2 = EventRow { id: "ev-2".to_string(), ..event("wt-target", None) };
            insert_event(&pool, &e2).await.unwrap();
            assert_eq!(fanout(&pool, &e2, now).await.unwrap(), 2);
        });
    }

    #[test]
    fn test_roundtrip_expired_subscription_is_not_delivered_and_purged() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let mut s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            s.expires_at = Some(now - 1);
            upsert_subscription(&pool, &s).await.unwrap();

            assert!(list_subscriptions(&pool, "term-a", now).await.unwrap().is_empty());
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout(&pool, &e, now).await.unwrap(), 0);

            let (subs, _, _) = purge_expired(&pool, now, INBOX_RETENTION_MS).await.unwrap();
            assert_eq!(subs, 1);
        });
    }

    #[test]
    fn test_roundtrip_purge_subscriber_worktree_removes_subs_and_inbox() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout(&pool, &e, now).await.unwrap();

            let (subs, inbox) = purge_subscriber_worktree(&pool, "wt-subscriber").await.unwrap();
            assert_eq!((subs, inbox), (1, 1));
            assert!(list_subscriptions(&pool, "term-a", now).await.unwrap().is_empty());
            assert!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().is_empty());
        });
    }

    /// 購読が先に消えていても（unsubscribe 済み）、購読者ワークツリーのクローズで
    /// inbox が取り残されない（inbox 側が subscriber_worktree_id を持っているため）。
    #[test]
    fn test_roundtrip_purge_subscriber_worktree_after_unsubscribe() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout(&pool, &e, now).await.unwrap();

            // 先に購読だけ解除する
            delete_subscription_by_target(&pool, "term-a", &s.target).await.unwrap();
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().len(), 1);

            let (subs, inbox) = purge_subscriber_worktree(&pool, "wt-subscriber").await.unwrap();
            assert_eq!((subs, inbox), (0, 1));
            assert!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().is_empty());
        });
    }

    /// 失効した購読の inbox は保持期限を待たずに落とす。
    /// ただし同じタブの**別の**購読宛のメッセージは巻き込まない。
    #[test]
    fn test_roundtrip_purge_expired_drops_only_its_own_inbox() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let mut expiring = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            expiring.target = "wt-expiring".to_string();
            expiring.expires_at = Some(now - 1);
            upsert_subscription(&pool, &expiring).await.unwrap();

            let mut alive = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            alive.id = "sub-alive".to_string();
            alive.target = "wt-alive".to_string();
            upsert_subscription(&pool, &alive).await.unwrap();

            // 失効前に届いていた分と、生きている購読の分を1件ずつ積む
            let e1 = EventRow { id: "ev-expiring".to_string(), ..event("wt-expiring", None) };
            let e2 = EventRow { id: "ev-alive".to_string(), ..event("wt-alive", None) };
            insert_event(&pool, &e1).await.unwrap();
            insert_event(&pool, &e2).await.unwrap();
            // 失効購読の分は fanout では積まれないので直接入れる（失効前に届いた状況の再現）
            sqlx::query("INSERT INTO inbox (id, subscriber_terminal_id, subscriber_worktree_id, event_id, state, created_at) VALUES (?, ?, ?, ?, ?, ?)")
                .bind("inbox-expiring")
                .bind("term-a")
                .bind("wt-subscriber")
                .bind("ev-expiring")
                .bind(STATE_PENDING)
                .bind(now)
                .execute(&pool)
                .await
                .unwrap();
            assert_eq!(fanout(&pool, &e2, now).await.unwrap(), 1);
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().len(), 2);

            let (subs, inbox, _) = purge_expired(&pool, now, INBOX_RETENTION_MS).await.unwrap();
            assert_eq!((subs, inbox), (1, 1));
            let left = list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap();
            assert_eq!(left.len(), 1);
            assert_eq!(left[0].event_id, "ev-alive");
        });
    }

    /// 生存ターミナルに紐づかない購読と inbox は掃除される（再起動後の到達不能行）。
    #[test]
    fn test_roundtrip_purge_orphaned_subscribers() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let a = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &a).await.unwrap();
            let mut b = sub("term-b", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            b.id = "sub-b".to_string();
            upsert_subscription(&pool, &b).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout(&pool, &e, now).await.unwrap(), 2);

            // term-a だけ生存している
            let (subs, inbox) =
                purge_orphaned_subscribers(&pool, &["term-a".to_string()]).await.unwrap();
            assert_eq!((subs, inbox), (1, 1));
            assert_eq!(list_subscriptions(&pool, "term-a", now).await.unwrap().len(), 1);
            assert!(list_subscriptions(&pool, "term-b", now).await.unwrap().is_empty());
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().len(), 1);
            assert!(list_inbox(&pool, "term-b", InboxFilter::All).await.unwrap().is_empty());

            // 生存 0 件（起動直後）なら全部消える
            let (subs, inbox) = purge_orphaned_subscribers(&pool, &[]).await.unwrap();
            assert_eq!((subs, inbox), (1, 1));
        });
    }

    /// クローズされた target を指す購読は配送後に削除される。
    #[test]
    fn test_roundtrip_delete_subscriptions_for_target() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout(&pool, &e, now).await.unwrap(), 1);

            assert_eq!(delete_subscriptions_for_target(&pool, "wt-target").await.unwrap(), 1);
            assert!(list_subscriptions(&pool, "term-a", now).await.unwrap().is_empty());
            // 既に積まれた inbox は残る（購読解除は配送済みメッセージを消さない）
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::Unacked).await.unwrap().len(), 1);
        });
    }

    #[test]
    fn test_roundtrip_inbox_retention_drops_old_rows_and_orphan_events() {
        with_pool(async {
            let pool = memory_pool().await;
            let old = 1_000_000i64;
            let now = old + INBOX_RETENTION_MS + 1;

            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout(&pool, &e, old).await.unwrap();

            let (_, inbox, events) = purge_expired(&pool, now, INBOX_RETENTION_MS).await.unwrap();
            assert_eq!((inbox, events), (1, 1));
            // 無期限の購読自体は残る
            assert_eq!(list_subscriptions(&pool, "term-a", now).await.unwrap().len(), 1);
        });
    }

    #[test]
    fn test_roundtrip_delete_subscription_scoped_to_own_terminal() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();

            // 他タブからは消せない
            assert_eq!(delete_subscription(&pool, &s.id, "term-b").await.unwrap(), 0);
            assert_eq!(
                delete_subscription_by_target(&pool, "term-b", &s.target).await.unwrap(),
                0
            );
            assert_eq!(list_subscriptions(&pool, "term-a", now).await.unwrap().len(), 1);

            assert_eq!(
                delete_subscription_by_target(&pool, "term-a", &s.target).await.unwrap(),
                1
            );
            assert!(list_subscriptions(&pool, "term-a", now).await.unwrap().is_empty());
        });
    }

    #[test]
    fn test_format_inbox_digest_mentions_ack_and_count() {
        let items = vec![
            inbox_item(
                KIND_WORKTREE_CLOSED,
                r#"{"worktreeId":"a","worktreeName":"wt-a","branchName":"b1"}"#,
            ),
            inbox_item(
                KIND_WORKTREE_CLOSED,
                r#"{"worktreeId":"b","worktreeName":"wt-b","branchName":"b2"}"#,
            ),
        ];
        let digest = format_inbox_digest(&items, 0).unwrap();
        assert!(digest.contains("2 件"));
        assert!(digest.contains("oretachi_ack_message"));
        assert!(digest.contains("wt-a"));
        assert!(digest.contains("wt-b"));
    }
}
