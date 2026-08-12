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

/// イベント種別: ワークツリーが追加された（#126）。
/// body は oretachi 自身が組み立てる定型 JSON（`WorktreeCreatedBody`）。
pub const KIND_WORKTREE_CREATED: &str = "worktree.created";

/// イベント種別: エージェントが `notify_worktree` から送る**自由文**メッセージ（#126）。
///
/// `notify_worktree` の既存パラメータ `kind`（`hook` / `approval` / `completed` / `general`）は
/// トーストの**通知種別**であって購読イベント種別ではない。名前空間を分けるため、
/// 購読イベントは別パラメータ `event_kind` で受ける（#120 §1）。
pub const KIND_WORKTREE_MESSAGE: &str = "worktree.message";

/// 購読で受け付けるイベント種別。
pub const SUPPORTED_EVENT_KINDS: &[&str] = &[
    KIND_WORKTREE_CLOSED,
    KIND_WORKTREE_CREATED,
    KIND_WORKTREE_MESSAGE,
];

/// 購読対象のワイルドカード（#126）。**まだ存在しないワークツリーの `created` を購読したい**
/// という要求はワークツリー ID 固定の target では表現できないため、`worktree.created` と
/// セットで必要になる。
///
/// - `*`: 全ワークツリー
/// - `workgroup:<id>`: そのワークグループに属するワークツリー
/// - `repo:<name>`: そのリポジトリのワークツリー（名前は `normalize_target` で正規化）
///
/// 上記以外はワークツリー ID の厳密一致として扱う。
pub const TARGET_ALL: &str = "*";
pub const TARGET_WORKGROUP_PREFIX: &str = "workgroup:";
pub const TARGET_REPO_PREFIX: &str = "repo:";

/// 配送戦略。
/// - `turn_end`: 既定。`Stop` フック / `SessionStart` 回収でターン境界に届ける
/// - `interrupt`: 待機中なら即 PTY へ押し込む（#125）
/// - `passive`: 積むだけ。エージェントが `oretachi_poll_inbox` で自分から取る
pub const DELIVERY_TURN_END: &str = "turn_end";
pub const DELIVERY_INTERRUPT: &str = "interrupt";
pub const DELIVERY_PASSIVE: &str = "passive";
pub const SUPPORTED_DELIVERIES: &[&str] =
    &[DELIVERY_TURN_END, DELIVERY_INTERRUPT, DELIVERY_PASSIVE];

pub const STATE_ACTIVE: &str = "active";
/// 購読者タブが生存していない状態。**配送（inbox への蓄積）は続けるが押し込みはしない**。
/// 同じワークツリーで新しい AI タブが立った時点で `rebind_next_orphaned_group` が引き継ぐ。
pub const STATE_ORPHANED: &str = "orphaned";
pub const STATE_PENDING: &str = "pending";
pub const STATE_ACKED: &str = "acked";

/// inbox の保持期限。積みっぱなしで肥大するのを防ぐ（#120 §5.6）。
/// ack 済み・未 ack を問わず `created_at` からこの期間で削除する。
pub const INBOX_RETENTION_DAYS: i64 = 30;
pub const INBOX_RETENTION_MS: i64 = INBOX_RETENTION_DAYS * 24 * 60 * 60 * 1000;

/// orphaned に落ちてから引き継がれずに残った購読 / inbox の保持期限。
///
/// active な購読は無期限（#120 本文どおり）だが、到達不能なまま残る行は有界にする必要がある。
/// アプリを再起動して二度と開かないワークツリーの購読が単調増加するのを防ぐ。
pub const ORPHANED_RETENTION_DAYS: i64 = 7;
pub const ORPHANED_RETENTION_MS: i64 = ORPHANED_RETENTION_DAYS * 24 * 60 * 60 * 1000;

/// イベント連鎖の深さ上限（#120 §5.4 / #125 §3）。これを超えたイベントは配送せず破棄する。
///
/// `worktree.closed` / `worktree.created` はユーザー操作からしか発火しないので常に 0。
/// 自由文の `worktree.message`（#126）でエージェント同士が往復し始めたときに効く。
pub const MAX_EVENT_DEPTH: i64 = 3;

/// 連鎖とみなす時間窓（#126）。`worktree.message` を送るときの `depth` は
/// 「この窓の内側で受け取ったイベントの最大 depth + 1」として自動計算する。
///
/// **窓を切るのが要点。** 窓なしに「そのタブが今までに受け取った最大 depth」を使うと、
/// 一度でも `MAX_EVENT_DEPTH` の深さのイベントを受けたタブが以後**永久に発言不能**になる。
/// 窓があれば A↔B の高速な往復は閾値で止まり、人間ペースで再開した会話は depth 0 から始まる。
pub const CHAIN_WINDOW_MS: i64 = 10 * 60 * 1000;

/// PTY 押し込みを許すイベントの鮮度。これより古いイベントは押し込まない。
///
/// 再起動直後に何日も前の未配送分が一斉に割り込むのを防ぐ。inbox には残るので
/// `SessionStart` 回収 / `oretachi_poll_inbox` では取れる（取りこぼしにはならない）。
pub const PUSH_TTL_MS: i64 = 10 * 60 * 1000;

/// PTY へ流す1メッセージの最大文字数。`sanitize_for_pty` で切り詰める。
const PTY_TEXT_MAX_CHARS: usize = 600;

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
    /// 0 / 1。タブが死んでいる宛先へ新しいタブを立てて配送してよいか（明示オプトイン）
    pub spawn_if_closed: i64,
    pub created_at: i64,
    /// None は無期限
    pub expires_at: Option<i64>,
    /// active | orphaned
    pub state: String,
    /// orphaned に落ちた時刻。active なら None。保持期限と引き継ぎ順序に使う
    #[sqlx(default)]
    pub orphaned_at: Option<i64>,
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
    /// 連鎖の深さ。`MAX_EVENT_DEPTH` を超えたら配送しない
    #[sqlx(default)]
    pub depth: i64,
    /// 発生源の種別（`archive` / `mcp` / `delivery:<terminal_id>` 等）。監査とループ解析用
    #[sqlx(default)]
    pub origin: Option<String>,
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
    /// 購読者タブが死んで宙に浮いた時刻。**inbox 側にも持つのが要点**:
    /// `worktree.closed` は `fanout` 直後に `delete_subscriptions_for_target` で購読行が
    /// 消えるため（`lib.rs` の `fire_worktree_closed`）、再起動を挟んだ回収を購読テーブル
    /// 起点で行うと何も残っていない。再バインドの駆動元はこちら。
    #[sqlx(default)]
    pub orphaned_at: Option<i64>,
    /// 積んだ時点の購読の配送戦略。購読行は先に消えるのでここに焼き付ける
    #[sqlx(default)]
    pub delivery: String,
    /// 積んだ時点の `spawn_if_closed`（0 / 1）。同上
    #[sqlx(default)]
    pub spawn_if_closed: i64,
    // events からの結合分
    pub kind: String,
    pub body: String,
    pub source_worktree_id: String,
    pub actor: Option<String>,
}

/// 引き継ぎ待ちの1グループ（死亡した1タブが残した購読と未読）。
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct OrphanedGroup {
    /// 死亡したタブの terminal_id
    pub terminal_id: String,
    pub worktree_id: String,
    pub orphaned_at: i64,
    /// グループ内の最も古い購読の作成時刻（引き継ぎ順序の tie-break 用）
    pub created_at: i64,
    pub subscriptions: i64,
    /// 未 ack の inbox 件数
    pub pending: i64,
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
            state                    TEXT NOT NULL DEFAULT 'active',
            orphaned_at              INTEGER
        )"#,
    )
    .execute(pool)
    .await?;
    // 既存 DB 向け（#125）。既にあればエラーを握りつぶす。
    let _ = sqlx::query("ALTER TABLE subscriptions ADD COLUMN orphaned_at INTEGER")
        .execute(pool)
        .await;
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
            created_at         INTEGER NOT NULL,
            depth              INTEGER NOT NULL DEFAULT 0,
            origin             TEXT
        )"#,
    )
    .execute(pool)
    .await?;
    // 既存 DB 向け（#125 の暴走防止）。既にあればエラーを握りつぶす。
    let _ = sqlx::query("ALTER TABLE events ADD COLUMN depth INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE events ADD COLUMN origin TEXT")
        .execute(pool)
        .await;
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
            orphaned_at            INTEGER,
            delivery               TEXT NOT NULL DEFAULT 'turn_end',
            spawn_if_closed        INTEGER NOT NULL DEFAULT 0,
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
    // 同（#125）。再バインドの駆動元は購読ではなく inbox 側のこの列。
    let _ = sqlx::query("ALTER TABLE inbox ADD COLUMN orphaned_at INTEGER")
        .execute(pool)
        .await;
    // 配送戦略と spawn 可否は購読側にもあるが、`worktree.closed` は fanout 直後に
    // `delete_subscriptions_for_target` で購読行が消えるため、積んだ時点の値を inbox に
    // 焼き付けておかないと後から配送方法を決められない（#125）。
    let _ = sqlx::query("ALTER TABLE inbox ADD COLUMN delivery TEXT NOT NULL DEFAULT 'turn_end'")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE inbox ADD COLUMN spawn_if_closed INTEGER NOT NULL DEFAULT 0")
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
        "INSERT INTO subscriptions (id, subscriber_terminal_id, subscriber_worktree_id, subscriber_agent_session, target, event_kinds, delivery, spawn_if_closed, created_at, expires_at, state, orphaned_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(subscriber_terminal_id, target) DO UPDATE SET \
           subscriber_worktree_id = excluded.subscriber_worktree_id, \
           subscriber_agent_session = excluded.subscriber_agent_session, \
           event_kinds = excluded.event_kinds, \
           delivery = excluded.delivery, \
           spawn_if_closed = excluded.spawn_if_closed, \
           expires_at = excluded.expires_at, \
           state = excluded.state, \
           orphaned_at = excluded.orphaned_at",
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
    .bind(sub.orphaned_at)
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
        "INSERT OR REPLACE INTO events (id, source_worktree_id, source_terminal_id, kind, body, actor, created_at, depth, origin) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&event.id)
    .bind(&event.source_worktree_id)
    .bind(&event.source_terminal_id)
    .bind(&event.kind)
    .bind(&event.body)
    .bind(&event.actor)
    .bind(event.created_at)
    .bind(event.depth)
    .bind(&event.origin)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// あるワークツリーについて、その種別のイベントが既に記録されているか。
///
/// `worktree.created` は「ワークツリーごとに一度きり」の意味を持つが、フロントの
/// `notify_worktree_added` はセットアップ失敗時にも呼ばれる（`useTaskExecution.ts` の
/// try / catch 両分岐）ため、経路次第で二度叩かれうる。発火前にここで弾く。
pub async fn has_event(pool: &SqlitePool, source_worktree_id: &str, kind: &str) -> Result<bool, String> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events WHERE source_worktree_id = ? AND kind = ?")
            .bind(source_worktree_id)
            .bind(kind)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(row.0 > 0)
}

/// このタブが連鎖ウィンドウ内に受け取ったイベントの最大 `depth`（#126）。
///
/// エージェントが送る `worktree.message` の `depth` はこれ + 1 になる。**エージェントに
/// depth を申告させない**のが要点で、そうしないと「返信するときに depth を足す」という
/// 約束を破るだけで `MAX_EVENT_DEPTH` のガードを無効化できてしまう。
///
/// 対象は inbox に積まれた（＝実際にこのタブへ配られた）ぶんだけ。まだ ack していなくても
/// 「受け取った」とみなす —— エージェントは押し込み / SessionStart 注入で本文を見た時点で
/// 反応できるため、ack を条件にすると連鎖の起点を数え落とす。
///
/// **数えるのは `worktree.message` だけ。** 往復ループを作れるのは自由文メッセージだけで、
/// `worktree.created` / `worktree.closed` はワークツリーの追加・削除というユーザー操作から
/// しか発火せず必ず depth 0 で始まる。これらを数に入れると、`*` を購読して作成通知を
/// 受けているだけのタブが常に depth 1 以上から送ることになり、**本来の会話が使える
/// 往復回数だけが減る**（ループ耐性は変わらない）。
///
/// 戻り値 `None` は「窓の内側に受信が無い」＝連鎖の起点（depth 0 で送ってよい）。
pub async fn max_inbound_depth(
    pool: &SqlitePool,
    terminal_id: &str,
    now: i64,
    window_ms: i64,
) -> Result<Option<i64>, String> {
    let row: (Option<i64>,) = sqlx::query_as(
        "SELECT MAX(e.depth) FROM inbox i JOIN events e ON e.id = i.event_id \
         WHERE i.subscriber_terminal_id = ? AND i.created_at > ? AND e.kind = ?",
    )
    .bind(terminal_id)
    .bind(now - window_ms)
    .bind(KIND_WORKTREE_MESSAGE)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.0)
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

/// 購読対象文字列を正規化する（#126）。
///
/// **登録時と照合時で必ず同じ関数を通すこと。** `repo:` はリポジトリ名という人間が打つ
/// 文字列なので大文字小文字と前後空白を吸収する。ワークツリー ID / ワークグループ ID は
/// oretachi が発番した UUID なので厳密一致のまま（小文字化しても実害はないが、
/// 「ID は一字一句一致」という不変条件を崩さない）。
pub fn normalize_target(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix(TARGET_REPO_PREFIX) {
        return format!("{}{}", TARGET_REPO_PREFIX, rest.trim().to_lowercase());
    }
    if let Some(rest) = trimmed.strip_prefix(TARGET_WORKGROUP_PREFIX) {
        return format!("{}{}", TARGET_WORKGROUP_PREFIX, rest.trim());
    }
    trimmed.to_string()
}

/// イベント1件がマッチしうる `target` 文字列の**全集合**を返す（#126）。
///
/// ワイルドカードを SQL の `LIKE` やアプリ側の全件走査で実装せず、発火側で「この
/// イベントに該当する target はこの数個だけ」と列挙して `WHERE target IN (…)` に渡す。
/// こうするとインデックス（`idx_subs_target`）がそのまま効き、照合規則は純粋関数として
/// 単体テストできる（`event_kinds_match` / `is_self_echo` と同じ流儀）。
pub fn matching_targets(
    source_worktree_id: &str,
    workgroup_id: Option<&str>,
    repository_name: Option<&str>,
) -> Vec<String> {
    let mut targets = vec![source_worktree_id.to_string(), TARGET_ALL.to_string()];
    if let Some(gid) = workgroup_id.map(str::trim).filter(|s| !s.is_empty()) {
        targets.push(normalize_target(&format!("{}{}", TARGET_WORKGROUP_PREFIX, gid)));
    }
    if let Some(repo) = repository_name.map(str::trim).filter(|s| !s.is_empty()) {
        targets.push(normalize_target(&format!("{}{}", TARGET_REPO_PREFIX, repo)));
    }
    targets
}

/// 自己エコー抑止（#120 §5.4）。発火元へ配り返すと往復ループの起点になる。
///
/// - 同じタブが発火元なら配送しない
/// - 発火元ワークツリーに属するタブへも配送しない。`worktree.closed` は close 経路が
///   terminal_id を運んでいないので `source_terminal_id` が None であり、実質こちらが効く
///   （そもそも閉じたワークツリーのタブは既に存在しないので配送先として無意味）
///
/// `*` 購読（#126）では**自ワークツリー発のイベントも必ず target にマッチする**ため、
/// ここが唯一の防波堤になる。同一ワークツリーの別タブへも配らないのは、
/// `worktree.message` が同じワークツリー内で往復する経路を最初から塞ぐため。
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
///
/// **`orphaned` な購読にも積む**（#125）。タブが死んでいた / アプリが止まっていた間に届いた
/// イベントを捨てると、「別ワークツリーの対応がクローズしたのを後で検知したい」という
/// #120 の動機②そのものが成立しない。押し込み（PTY 注入 / spawn）だけを保留する。
/// `targets` はこのイベントにマッチしうる購読対象の全集合（`matching_targets` の戻り値）。
/// ワイルドカード（`*` / `workgroup:` / `repo:`）はここで解決済みの形で渡される（#126）。
pub async fn fanout(
    pool: &SqlitePool,
    event: &EventRow,
    targets: &[String],
    now: i64,
) -> Result<usize, String> {
    // 暴走防止（#120 §5.4）: 連鎖が深すぎるイベントは配送しない。
    if event.depth > MAX_EVENT_DEPTH {
        log::warn!(
            "[event-db] depth {} > {} のイベントを破棄: id={} kind={} origin={:?}",
            event.depth,
            MAX_EVENT_DEPTH,
            event.id,
            event.kind,
            event.origin
        );
        return Ok(0);
    }
    if targets.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "SELECT * FROM subscriptions WHERE target IN ({}) AND state IN ('active', 'orphaned') AND (expires_at IS NULL OR expires_at > ?)",
        placeholders(targets.len())
    );
    let mut query = sqlx::query_as::<_, SubscriptionRow>(&sql);
    for t in targets {
        query = query.bind(t);
    }
    let candidates = query
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
        // orphaned な購読へ積む行は最初から orphaned として印を付ける。そうしないと
        // 「タブが死んでいる間に届いた分」が再バインドの対象から漏れる。
        //
        // **時刻は購読の `orphaned_at` ではなく `now`。** 購読側の時刻を継承すると、
        // 6日前に orphaned になった購読へ今届いたメッセージが「6日前から待っている」扱いに
        // なり、保持期限（7日）まで数時間しか残らないまま消える。引き継ぎ順序
        // （`orphaned_at DESC`）でも最後尾に回り、本命のメッセージほど後回しになる。
        let orphaned_at = if sub.state == STATE_ORPHANED {
            Some(now)
        } else {
            None
        };
        let result = sqlx::query(
            "INSERT OR IGNORE INTO inbox (id, subscriber_terminal_id, subscriber_worktree_id, event_id, state, created_at, orphaned_at, delivery, spawn_if_closed) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&sub.subscriber_terminal_id)
        .bind(&sub.subscriber_worktree_id)
        .bind(&event.id)
        .bind(STATE_PENDING)
        .bind(now)
        .bind(orphaned_at)
        .bind(&sub.delivery)
        .bind(sub.spawn_if_closed)
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
        "SELECT i.id, i.subscriber_terminal_id, i.subscriber_worktree_id, i.event_id, i.state, i.created_at, i.delivered_at, i.acked_at, i.orphaned_at, i.delivery, i.spawn_if_closed, e.kind, e.body, e.source_worktree_id, e.actor \
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

// ─── orphaned 遷移と再バインド（#125） ────────────────────────────────────────

/// 生存タブに紐づかない購読と未 ack の inbox を `orphaned` にする（**削除しない**）。
///
/// `terminal_id` は PTY spawn ごとの発番で永続化されないため、アプリを再起動すると
/// 前回の購読はどのタブからも到達できなくなる。Phase 1 はこれを全削除していたが、
/// それでは #120 の動機②（数時間〜数日待って別ワークツリーのクローズを検知する）が
/// 再起動を挟んだ瞬間に壊れる。行は保持し、同じワークツリーで新しい AI タブが立った
/// 時点で `rebind_next_orphaned_group` が引き継ぐ。
///
/// `subscriber_worktree_id` が NULL の行だけは引き継ぎ先を解決できないので削除する
/// （現行の `subscribe` は `Some` を強制するので実質レガシー行のみ）。
///
/// 起動時は生存タブが 0 件なので全行が orphaned になる。これは意図どおり。
///
/// 戻り値は (orphaned にした購読数, orphaned にした inbox 数, 削除した到達不能行数)。
pub async fn mark_orphaned_subscribers(
    pool: &SqlitePool,
    live_terminal_ids: &[String],
    now: i64,
) -> Result<(u64, u64, u64), String> {
    let not_live = if live_terminal_ids.is_empty() {
        String::new()
    } else {
        format!(
            " AND subscriber_terminal_id NOT IN ({})",
            placeholders(live_terminal_ids.len())
        )
    };
    // 引き継ぎ先を解決できない行は保持しても永久に到達できないので削除する。
    let sql = format!(
        "DELETE FROM inbox WHERE subscriber_worktree_id IS NULL{}",
        not_live
    );
    let mut q = sqlx::query(&sql);
    for id in live_terminal_ids {
        q = q.bind(id);
    }
    let mut deleted = q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();

    let sql = format!(
        "DELETE FROM subscriptions WHERE subscriber_worktree_id IS NULL{}",
        not_live
    );
    let mut q = sqlx::query(&sql);
    for id in live_terminal_ids {
        q = q.bind(id);
    }
    deleted += q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();

    let sql = format!(
        "UPDATE subscriptions SET state = ?, orphaned_at = ? WHERE state = ?{}",
        not_live
    );
    let mut q = sqlx::query(&sql)
        .bind(STATE_ORPHANED)
        .bind(now)
        .bind(STATE_ACTIVE);
    for id in live_terminal_ids {
        q = q.bind(id);
    }
    let subs = q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();

    // ack 済みの行は引き継ぐ意味がない（監査証跡としてそのまま残す）。
    let sql = format!(
        "UPDATE inbox SET orphaned_at = ? WHERE orphaned_at IS NULL AND acked_at IS NULL{}",
        not_live
    );
    let mut q = sqlx::query(&sql).bind(now);
    for id in live_terminal_ids {
        q = q.bind(id);
    }
    let inbox = q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();

    // 逆遷移: 生存しているタブの行が orphaned のまま残っていたら active に戻す。
    //
    // ポーリングは生存タブ一覧をスナップショットしてから非同期に投げるため、その隙に
    // spawn されたタブが subscribe すると**古い一覧**で orphaned に落とされうる。戻す道が
    // 無いと、そのタブは生存しているのに `rebind_next_orphaned_group` が自分自身のグループを
    // 除外する（`terminal_id != new_terminal_id`）ので永久に引き継げず、押し込みも
    // `list_pushable` の `orphaned_at IS NULL` から外れて一切来なくなる。
    if !live_terminal_ids.is_empty() {
        let in_live = format!(
            " AND subscriber_terminal_id IN ({})",
            placeholders(live_terminal_ids.len())
        );
        let sql = format!(
            "UPDATE subscriptions SET state = ?, orphaned_at = NULL WHERE state = ?{}",
            in_live
        );
        let mut q = sqlx::query(&sql).bind(STATE_ACTIVE).bind(STATE_ORPHANED);
        for id in live_terminal_ids {
            q = q.bind(id);
        }
        let restored = q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();

        let sql = format!(
            "UPDATE inbox SET orphaned_at = NULL WHERE orphaned_at IS NOT NULL AND acked_at IS NULL{}",
            in_live
        );
        let mut q = sqlx::query(&sql);
        for id in live_terminal_ids {
            q = q.bind(id);
        }
        let restored_inbox = q.execute(pool).await.map_err(|e| e.to_string())?.rows_affected();
        if restored > 0 || restored_inbox > 0 {
            log::info!(
                "[event-db] 生存タブの引き継ぎ待ちを解除: 購読 {} 件 / 未読 {} 件",
                restored,
                restored_inbox
            );
        }
    }

    Ok((subs, inbox, deleted))
}

/// 指定ワークツリーの引き継ぎ待ちグループを、引き継ぎ順（先頭が次に引き継がれる）で返す。
///
/// **購読と inbox の和集合**をグループの母集合にするのが要点。`worktree.closed` は
/// `fanout` 直後に `delete_subscriptions_for_target` で購読行が消えるため、本命シナリオ
/// （待っていた対象がクローズしてから再起動）では inbox 行しか残っていない。
///
/// 並び順は `orphaned_at DESC, created_at DESC, terminal_id ASC` の**安定な全順序**。
/// 起動時の一括遷移では `orphaned_at` が全グループで同値になるため tie-break が必須。
pub async fn list_orphaned_groups(
    pool: &SqlitePool,
    worktree_id: &str,
) -> Result<Vec<OrphanedGroup>, String> {
    sqlx::query_as::<_, OrphanedGroup>(
        "SELECT terminal_id, ? AS worktree_id, MAX(orphaned_at) AS orphaned_at, MIN(created_at) AS created_at, \
                SUM(is_sub) AS subscriptions, SUM(1 - is_sub) AS pending \
         FROM ( \
           SELECT subscriber_terminal_id AS terminal_id, orphaned_at, created_at, 1 AS is_sub \
             FROM subscriptions \
            WHERE state = 'orphaned' AND orphaned_at IS NOT NULL AND subscriber_worktree_id = ? \
           UNION ALL \
           SELECT subscriber_terminal_id AS terminal_id, orphaned_at, created_at, 0 AS is_sub \
             FROM inbox \
            WHERE orphaned_at IS NOT NULL AND acked_at IS NULL AND subscriber_worktree_id = ? \
         ) \
         GROUP BY terminal_id \
         ORDER BY orphaned_at DESC, created_at DESC, terminal_id ASC",
    )
    .bind(worktree_id)
    .bind(worktree_id)
    .bind(worktree_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

/// 死亡タブ1つ分の購読と未読を、生存している新しいタブへ引き継ぐ。
///
/// 粒度が「死亡タブ単位」なのは、同一ワークツリーの複数タブがそれぞれ別の対象を購読して
/// いた場合に、新タブ1つが全部を吸い上げてタブ間の宛先分離が消えるのを避けるため
/// （#120 §2 の「A タブ宛を B タブが抜き取らない」原則）。引き継がれずに残ったグループは
/// UI に「引き継ぎ待ち」として出し、人間が任意のタブへ手動で引き継げるようにする。
///
/// 2つの UNIQUE 制約（`subscriptions(terminal_id, target)` / `inbox(terminal_id, event_id)`）
/// と衝突しうるので `UPDATE OR IGNORE` + 残骸の DELETE にする。**孤児側を捨てるのが
/// 両方向で正しい**:
///
/// | 生存側(新タブ) | 孤児側(旧タブ) | 孤児を捨てると |
/// |---|---|---|
/// | 配送済み | 未配送 | 新タブは既に本文を見ている。正しい |
/// | 未配送 | 配送済み | 新タブは次の SessionStart で見る。正しい |
/// | ack 済み | 任意 | 処理済み。正しい |
///
/// 一連の UPDATE / DELETE は1トランザクションで行う。分割すると、隙間に走った `fanout`
/// （orphaned な購読にも積む）が死んだ terminal_id 宛の行を差し込んで取り残す。
pub async fn rebind_orphaned_group(
    pool: &SqlitePool,
    worktree_id: &str,
    dead_terminal_id: &str,
    new_terminal_id: &str,
) -> Result<(u64, u64), String> {
    if dead_terminal_id == new_terminal_id {
        return Ok((0, 0));
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let subs = sqlx::query(
        "UPDATE OR IGNORE subscriptions SET subscriber_terminal_id = ?, state = ?, orphaned_at = NULL \
         WHERE state = ? AND subscriber_worktree_id = ? AND subscriber_terminal_id = ?",
    )
    .bind(new_terminal_id)
    .bind(STATE_ACTIVE)
    .bind(STATE_ORPHANED)
    .bind(worktree_id)
    .bind(dead_terminal_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    // UNIQUE(subscriber_terminal_id, target) に負けた残骸 = 新タブが既に同じ対象を購読済み
    sqlx::query(
        "DELETE FROM subscriptions WHERE state = ? AND subscriber_worktree_id = ? AND subscriber_terminal_id = ?",
    )
    .bind(STATE_ORPHANED)
    .bind(worktree_id)
    .bind(dead_terminal_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let inbox = sqlx::query(
        "UPDATE OR IGNORE inbox SET subscriber_terminal_id = ?, orphaned_at = NULL \
         WHERE orphaned_at IS NOT NULL AND acked_at IS NULL AND subscriber_worktree_id = ? AND subscriber_terminal_id = ?",
    )
    .bind(new_terminal_id)
    .bind(worktree_id)
    .bind(dead_terminal_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    // UNIQUE(subscriber_terminal_id, event_id) に負けた残骸（上表の理由で捨ててよい）
    sqlx::query(
        "DELETE FROM inbox WHERE orphaned_at IS NOT NULL AND acked_at IS NULL AND subscriber_worktree_id = ? AND subscriber_terminal_id = ?",
    )
    .bind(worktree_id)
    .bind(dead_terminal_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok((subs, inbox))
}

/// 引き継ぎ待ちの先頭グループ1つを新しいタブへ引き継ぐ。自動トリガ（SessionStart /
/// AI エージェント検出）はこちらを使う。引き継ぐものが無ければ `(0, 0)`（冪等なので
/// 複数の経路から同時に呼ばれても無害）。
pub async fn rebind_next_orphaned_group(
    pool: &SqlitePool,
    worktree_id: &str,
    new_terminal_id: &str,
) -> Result<(u64, u64), String> {
    let groups = list_orphaned_groups(pool, worktree_id).await?;
    let Some(group) = groups
        .into_iter()
        .find(|g| g.terminal_id != new_terminal_id)
    else {
        return Ok((0, 0));
    };
    rebind_orphaned_group(pool, worktree_id, &group.terminal_id, new_terminal_id).await
}

/// 引き継がれないまま保持期限を過ぎた orphaned 行を削除する。
/// active な購読は無期限（#120 本文どおり）なので触らない。
pub async fn purge_orphaned_expired(
    pool: &SqlitePool,
    now: i64,
    orphaned_retention_ms: i64,
) -> Result<(u64, u64), String> {
    let deadline = now - orphaned_retention_ms;
    let inbox = sqlx::query("DELETE FROM inbox WHERE orphaned_at IS NOT NULL AND orphaned_at <= ?")
        .bind(deadline)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();
    let subs = sqlx::query(
        "DELETE FROM subscriptions WHERE state = ? AND orphaned_at IS NOT NULL AND orphaned_at <= ?",
    )
    .bind(STATE_ORPHANED)
    .bind(deadline)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    Ok((subs, inbox))
}

// ─── 押し込み対象の抽出（#125） ───────────────────────────────────────────────

/// PTY 押し込みの候補（未配送・未 ack・鮮度内・引き継ぎ済み）をタブごとにまとめて返す。
///
/// `orphaned_at IS NOT NULL` の行は宛先タブが存在しないので対象外（`spawn_if_closed` の
/// 判断は購読側を見る）。`created_at` が `push_ttl_ms` より古いものも外す —— 再起動直後に
/// 何日も前の未配送分が一斉に割り込むのを防ぐ。inbox には残るので `SessionStart` 回収や
/// `oretachi_poll_inbox` では取れる。
pub async fn list_pushable(
    pool: &SqlitePool,
    now: i64,
    push_ttl_ms: i64,
) -> Result<Vec<InboxItem>, String> {
    sqlx::query_as::<_, InboxItem>(
        "SELECT i.id, i.subscriber_terminal_id, i.subscriber_worktree_id, i.event_id, i.state, i.created_at, i.delivered_at, i.acked_at, i.orphaned_at, i.delivery, i.spawn_if_closed, e.kind, e.body, e.source_worktree_id, e.actor \
         FROM inbox i JOIN events e ON e.id = i.event_id \
         WHERE i.delivered_at IS NULL AND i.acked_at IS NULL AND i.orphaned_at IS NULL AND i.created_at > ? \
         ORDER BY i.created_at ASC",
    )
    .bind(now - push_ttl_ms)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

/// 宛先タブが居ないまま未読が溜まっており、かつ `spawn_if_closed` が立っている
/// ワークツリーと件数を返す（新しいタブを立てて配送する候補）。
///
/// 購読テーブルではなく inbox を見るのは、`worktree.closed` では購読行が fanout 直後に
/// 消えているため。`spawn_if_closed` は積んだ時点の値が inbox に焼き付けてある。
///
/// 戻り値は `(購読者ワークツリー, イベント種別, 件数)`。**種別を返すのが要点（#126）。**
/// 呼び出し元は自動承認が有効な宛先に対して種別ごとに許可判定する必要があり、
/// 種別を落として件数だけ返すと自由文の `worktree.message` が定型イベントのふりをして
/// spawn を引き起こせてしまう。
pub async fn list_spawn_candidates(
    pool: &SqlitePool,
    now: i64,
    push_ttl_ms: i64,
) -> Result<Vec<(String, String, i64)>, String> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT i.subscriber_worktree_id, e.kind, COUNT(*) FROM inbox i JOIN events e ON e.id = i.event_id \
         WHERE i.orphaned_at IS NOT NULL AND i.acked_at IS NULL AND i.delivered_at IS NULL \
           AND i.spawn_if_closed = 1 AND i.subscriber_worktree_id IS NOT NULL AND i.created_at > ? \
         GROUP BY i.subscriber_worktree_id, e.kind",
    )
    .bind(now - push_ttl_ms)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows)
}

// ─── UI 向けの読み出し（#120 §7） ─────────────────────────────────────────────

/// 全タブ分の有効な購読を返す。UI の購読一覧用（MCP 経由の `list_subscriptions` は
/// 呼び出し元タブのぶんだけ返すので、人間向けには横断の一覧が要る）。
pub async fn list_all_subscriptions(
    pool: &SqlitePool,
    now: i64,
) -> Result<Vec<SubscriptionRow>, String> {
    sqlx::query_as::<_, SubscriptionRow>(
        "SELECT * FROM subscriptions WHERE expires_at IS NULL OR expires_at > ? ORDER BY created_at DESC",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

/// タブごとの (未 ack 件数, うち未配送件数)。タブの未読バッジに使う。
pub async fn count_unacked_by_terminal(pool: &SqlitePool) -> Result<Vec<(String, i64, i64)>, String> {
    sqlx::query_as(
        "SELECT subscriber_terminal_id, COUNT(*), COALESCE(SUM(CASE WHEN delivered_at IS NULL THEN 1 ELSE 0 END), 0) \
         FROM inbox WHERE acked_at IS NULL GROUP BY subscriber_terminal_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

/// 全ワークツリーの引き継ぎ待ちグループ。UI の「引き継ぎ待ち」表示と手動引き継ぎ用。
pub async fn list_all_orphaned_groups(pool: &SqlitePool) -> Result<Vec<OrphanedGroup>, String> {
    sqlx::query_as::<_, OrphanedGroup>(
        "SELECT terminal_id, worktree_id, MAX(orphaned_at) AS orphaned_at, MIN(created_at) AS created_at, \
                SUM(is_sub) AS subscriptions, SUM(1 - is_sub) AS pending \
         FROM ( \
           SELECT subscriber_terminal_id AS terminal_id, subscriber_worktree_id AS worktree_id, orphaned_at, created_at, 1 AS is_sub \
             FROM subscriptions \
            WHERE state = 'orphaned' AND orphaned_at IS NOT NULL AND subscriber_worktree_id IS NOT NULL \
           UNION ALL \
           SELECT subscriber_terminal_id AS terminal_id, subscriber_worktree_id AS worktree_id, orphaned_at, created_at, 0 AS is_sub \
             FROM inbox \
            WHERE orphaned_at IS NOT NULL AND acked_at IS NULL AND subscriber_worktree_id IS NOT NULL \
         ) \
         GROUP BY terminal_id, worktree_id \
         ORDER BY orphaned_at DESC, created_at DESC, terminal_id ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

/// 指定タブの未 ack を全件 ack する。UI の「既読にする」用。
///
/// エージェントが ack しないまま忘れた分を人間が畳めるようにする。MCP 側の `ack` が
/// ID 指定なのは「読んだものだけ確認済みにする」ためだが、人間は一覧で件数を見ているので
/// ID を持っていない。
pub async fn ack_all(pool: &SqlitePool, terminal_id: &str, now: i64) -> Result<u64, String> {
    Ok(sqlx::query(
        "UPDATE inbox SET acked_at = ?, state = ? WHERE subscriber_terminal_id = ? AND acked_at IS NULL",
    )
    .bind(now)
    .bind(STATE_ACKED)
    .bind(terminal_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected())
}

/// UI からの購読解除。MCP 経由（`delete_subscription`）と違い呼び出し元タブに
/// 縛られない —— 人間は引き継ぎ待ちの（＝どのタブからも触れない）購読も消せる必要がある。
pub async fn delete_subscription_by_id(pool: &SqlitePool, id: &str) -> Result<u64, String> {
    Ok(sqlx::query("DELETE FROM subscriptions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected())
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
    //
    // 既知の限界（#126）: 突合が `s.target = e.source_worktree_id` なので、
    // ワイルドカード購読（`*` / `workgroup:` / `repo:`）が失効した場合はここで拾えず、
    // その未読は `INBOX_RETENTION_DAYS`(30日) まで残る。inbox は行がどの購読由来かを
    // 持たないため、ワイルドカードを緩く含めると**同じタブの別購読宛のメッセージまで
    // 消しうる**。取りこぼしより誤削除のほうが害が大きいので、有界な滞留を選んでいる。
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

/// `worktree.created` の body（#126）。`closed` と違い、購読者がどのリポジトリ /
/// ワークグループの追加かを判断できるよう解決済みの値も載せる。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorktreeCreatedBody {
    #[serde(rename = "worktreeId")]
    pub worktree_id: String,
    #[serde(rename = "worktreeName")]
    pub worktree_name: String,
    #[serde(rename = "branchName")]
    pub branch_name: String,
    #[serde(rename = "repositoryName", default)]
    pub repository_name: Option<String>,
    #[serde(rename = "workgroupId", default)]
    pub workgroup_id: Option<String>,
}

/// `worktree.message` の body（#126）。**`text` はエージェントが書いた自由文**なので、
/// PTY へ流す経路では必ず `sanitize_for_pty` を通すこと。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorktreeMessageBody {
    pub text: String,
    /// 送信元ワークツリー名（表示用。ID から引けなくなった後でも読めるように焼き付ける）
    #[serde(rename = "sourceWorktreeName", default)]
    pub source_worktree_name: Option<String>,
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
        KIND_WORKTREE_CREATED => serde_json::from_str::<WorktreeCreatedBody>(&item.body)
            .map(|b| {
                let branch = if b.branch_name.trim().is_empty() {
                    String::new()
                } else {
                    format!("（ブランチ: {}）", b.branch_name)
                };
                let repo = b
                    .repository_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|r| format!("[{}] ", r))
                    .unwrap_or_default();
                format!(
                    "{}ワークツリー '{}' {} が作成されました",
                    repo, b.worktree_name, branch
                )
            })
            .unwrap_or_else(|_| {
                format!("ワークツリー ID '{}' が作成されました", item.source_worktree_id)
            }),
        KIND_WORKTREE_MESSAGE => serde_json::from_str::<WorktreeMessageBody>(&item.body)
            .map(|b| {
                let from = b
                    .source_worktree_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| item.source_worktree_id.clone());
                format!("'{}' からのメッセージ: {}", from, b.text)
            })
            .unwrap_or_else(|_| {
                format!("'{}' からのメッセージ: {}", item.source_worktree_id, item.body)
            }),
        other => format!("イベント '{}': {}", other, item.body),
    };
    format!("- [{}] {}", item.id, detail)
}

/// PTY へ流す文字列から制御文字を落とし、長さを切り詰める。
///
/// ワークツリー名 / ブランチ名は利用者が自由に付けられる文字列で、`format_inbox_line` を
/// 経由してそのまま PTY へ流れる。押し込みは `ESC[200~ … ESC[201~CR` のブラケットペーストで
/// 囲むため、本文に `ESC[201~` や生の CR が混ざるとペーストを脱出して**任意のコマンドが
/// そのまま実行される**。ESC と C0 制御文字を全部落として構造的に潰す。
pub fn sanitize_for_pty(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        // ESC(0x1b) を含む C0 制御文字と DEL を除去する
        .filter(|c| !c.is_control())
        .collect();
    // 連続空白を1つに畳んでから長さで切る（改行を空白にした分が伸びるため）
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= PTY_TEXT_MAX_CHARS {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(PTY_TEXT_MAX_CHARS).collect();
    format!("{}…", truncated)
}

/// 待機中のエージェントへ PTY で押し込む1行テキストを組み立てる。
///
/// SessionStart 用の `format_inbox_digest` と分けているのは、押し込みは**1行**でなければ
/// ならないため（改行はそのままターン送信になる）。本文は `sanitize_for_pty` を通す。
///
/// 戻り値は `(本文, 本文に載せた件数)`。**載らなかった分は呼び出し元が打刻してはいけない。**
/// 全件の ID を `mark_delivered` に渡すと、長さ上限で切り落とされた分が「配送済みだが
/// 本文は誰も見ていない」状態になり、再送しない方針（#120 §5.2）のせいで二度と出てこない。
/// 載らなかった分は未配送のまま残り、次の押し込み / SessionStart 回収で拾われる。
pub fn format_inbox_push_text(items: &[InboxItem]) -> Option<(String, usize)> {
    if items.is_empty() {
        return None;
    }
    let head = "[oretachi] 購読していたワークツリーイベントが届きました。";
    // **種別に依存しない文言にすること（#126）。** `worktree.closed` だけだった頃の
    // 「購読していた作業が完了した」という決め打ちは、`created` / `message` が混ざると
    // 嘘の前提をエージェントに与える（作成通知を見て「完了した」と解釈させてしまう）。
    let tail = " 内容に応じて必要な動作確認や後続作業を進めてください。\
                確認したら oretachi_ack_message に [] 内の ID を渡して ack してください。";
    // 固定文言のぶんを引いた残りに、入る行だけを詰める（最低1件は必ず載せる）。
    // 残件の告知（`more`）も後ろに付くので、そのぶんの余白も先に引いておく。引かないと
    // 全体が上限を超え、`sanitize_for_pty` が**末尾から**切るせいで
    // 「oretachi_ack_message で ack してください」という一番効かせたい指示が欠ける。
    const MORE_ALLOWANCE: usize = 48;
    let budget = PTY_TEXT_MAX_CHARS
        .saturating_sub(head.chars().count() + tail.chars().count() + MORE_ALLOWANCE);
    let mut body = String::new();
    let mut used = 0usize;
    for item in items {
        let line = sanitize_for_pty(&format_inbox_line(item));
        let sep = if used == 0 { "" } else { " / " };
        if used > 0 && body.chars().count() + sep.len() + line.chars().count() > budget {
            break;
        }
        body.push_str(sep);
        body.push_str(&line);
        used += 1;
    }
    let more = if used < items.len() {
        format!("（ほかに {} 件あります。oretachi_poll_inbox で取得できます）", items.len() - used)
    } else {
        String::new()
    };
    Some((
        sanitize_for_pty(&format!("{}{}{}{}", head, body, more, tail)),
        used,
    ))
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
         内容に応じて必要な動作確認や後続作業を進めてください。\
         確認したら oretachi_ack_message に上記の [] 内の ID を渡して ack してください。\
         この本文は自動では再掲されません（ack しないまま忘れた場合は oretachi_poll_inbox で取り直せます）。{}",
        items.len(),
        lines.join("\n"),
        carryover_note
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 所属情報を持たないイベントの `fanout`。ワークツリー ID 厳密一致と `*` にだけ
    /// マッチする（#126 でワイルドカードが入る前の既存テストと同じ挙動）。
    async fn fanout_all(pool: &SqlitePool, event: &EventRow, now: i64) -> Result<usize, String> {
        let targets = matching_targets(&event.source_worktree_id, None, None);
        fanout(pool, event, &targets, now).await
    }

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
            orphaned_at: None,
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
            depth: 0,
            origin: Some("archive".to_string()),
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
            orphaned_at: None,
            delivery: DELIVERY_TURN_END.to_string(),
            spawn_if_closed: 0,
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
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 1);
            // 同じイベントの再 fanout は UNIQUE 制約でマージされる
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 0);

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
            fanout_all(&pool, &e, now).await.unwrap();

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
            fanout_all(&pool, &e, now).await.unwrap();

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
            assert_eq!(fanout_all(&pool, &e2, now).await.unwrap(), 2);
        });
    }

    /// #124 の完了条件「同一ワークツリーに複数タブがある状態で誤配送しない」。
    ///
    /// `Stop` フックは発火したタブの `terminal_id` を運んでくるので、配送はその ID で
    /// 未配送を引いて打刻する。B タブの `Stop` が A タブ宛の未読を抜き取らないこと、
    /// および同じタブの2回目が空になること（`delivered_at` による二重注入防止）を固定する。
    #[test]
    fn test_roundtrip_turn_end_drain_is_scoped_to_one_tab() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            for (id, term) in [("sub-a", "term-a"), ("sub-b", "term-b")] {
                let mut s = sub(term, Some("wt-subscriber"), r#"["worktree.closed"]"#);
                s.id = id.to_string();
                upsert_subscription(&pool, &s).await.unwrap();
            }
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 2, "両タブに積まれる");

            // B タブで Stop が発火した相当の drain
            let b = list_inbox(&pool, "term-b", InboxFilter::Undelivered).await.unwrap();
            assert_eq!(b.len(), 1);
            let ids: Vec<String> = b.iter().map(|i| i.id.clone()).collect();
            assert_eq!(mark_delivered(&pool, &ids, now).await.unwrap(), 1);

            // A タブ宛は未配送のまま残っている（抜き取られていない）
            let a = list_inbox(&pool, "term-a", InboxFilter::Undelivered).await.unwrap();
            assert_eq!(a.len(), 1, "A タブ宛は B タブの Stop で消えない");
            assert!(a[0].delivered_at.is_none());

            // B タブの次の Stop では本文が出ない（二重注入防止）
            assert!(list_inbox(&pool, "term-b", InboxFilter::Undelivered)
                .await
                .unwrap()
                .is_empty());
            // ただし未 ack としては残るので poll_inbox で取り直せる
            assert_eq!(
                list_inbox(&pool, "term-b", InboxFilter::Unacked).await.unwrap().len(),
                1
            );
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
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 0);

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
            fanout_all(&pool, &e, now).await.unwrap();

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
            fanout_all(&pool, &e, now).await.unwrap();

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
            assert_eq!(fanout_all(&pool, &e2, now).await.unwrap(), 1);
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().len(), 2);

            let (subs, inbox, _) = purge_expired(&pool, now, INBOX_RETENTION_MS).await.unwrap();
            assert_eq!((subs, inbox), (1, 1));
            let left = list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap();
            assert_eq!(left.len(), 1);
            assert_eq!(left[0].event_id, "ev-alive");
        });
    }

    /// 生存ターミナルに紐づかない購読と inbox は**削除されず** orphaned になる。
    /// 生存しているタブのぶんは触られない。
    #[test]
    fn test_roundtrip_mark_orphaned_spares_live_terminals() {
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
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 2);

            // term-a だけ生存している
            let (subs, inbox, deleted) =
                mark_orphaned_subscribers(&pool, &["term-a".to_string()], now).await.unwrap();
            assert_eq!((subs, inbox, deleted), (1, 1, 0));

            let live = list_subscriptions(&pool, "term-a", now).await.unwrap();
            assert_eq!(live[0].state, STATE_ACTIVE);
            assert_eq!(live[0].orphaned_at, None);
            let dead = list_subscriptions(&pool, "term-b", now).await.unwrap();
            assert_eq!(dead[0].state, STATE_ORPHANED, "削除ではなく orphaned へ遷移する");
            assert_eq!(dead[0].orphaned_at, Some(now));
            // inbox も両方残る（死んだ側は引き継ぎ待ちの印が付く）
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap()[0].orphaned_at, None);
            assert_eq!(
                list_inbox(&pool, "term-b", InboxFilter::All).await.unwrap()[0].orphaned_at,
                Some(now)
            );
        });
    }

    /// 起動直後（生存 0 件）は全行が orphaned になるが、**1行も消えない**。
    /// Phase 1 はここで全削除しており、それが「再起動を挟むと購読が消える」の直接原因だった。
    #[test]
    fn test_roundtrip_mark_orphaned_at_startup_keeps_everything() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, now).await.unwrap();

            let (subs, inbox, deleted) = mark_orphaned_subscribers(&pool, &[], now).await.unwrap();
            assert_eq!((subs, inbox, deleted), (1, 1, 0));
            assert_eq!(list_subscriptions(&pool, "term-a", now).await.unwrap().len(), 1);
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().len(), 1);

            // 冪等: 二度目は既に orphaned なので 0 件
            let (subs, inbox, _) = mark_orphaned_subscribers(&pool, &[], now).await.unwrap();
            assert_eq!((subs, inbox), (0, 0));
        });
    }

    /// ワークツリーを逆引きできない行は引き継ぎ先が決まらないので削除する。
    #[test]
    fn test_roundtrip_mark_orphaned_deletes_rows_without_worktree() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", None, r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();

            let (subs, inbox, deleted) = mark_orphaned_subscribers(&pool, &[], now).await.unwrap();
            assert_eq!((subs, inbox, deleted), (0, 0, 1));
            assert!(list_subscriptions(&pool, "term-a", now).await.unwrap().is_empty());
        });
    }

    /// 引き継がれないまま保持期限を過ぎた orphaned 行は削除される（単調増加の防止）。
    #[test]
    fn test_roundtrip_purge_orphaned_expired() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, now).await.unwrap();
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();

            // 期限内は消えない
            let (subs, inbox) = purge_orphaned_expired(&pool, now + 1000, ORPHANED_RETENTION_MS)
                .await
                .unwrap();
            assert_eq!((subs, inbox), (0, 0));

            let (subs, inbox) =
                purge_orphaned_expired(&pool, now + ORPHANED_RETENTION_MS + 1, ORPHANED_RETENTION_MS)
                    .await
                    .unwrap();
            assert_eq!((subs, inbox), (1, 1));
        });
    }

    /// **本 issue の核心**: `worktree.closed` は fanout 直後に購読行が消えるため、
    /// 再起動を挟んだ回収は inbox 行だけを頼りに行う必要がある。
    #[test]
    fn test_rebind_recovers_inbox_without_subscription() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-old", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 1);
            // 対象がクローズされたので購読行は消える（fire_worktree_closed と同じ順序）
            delete_subscriptions_for_target(&pool, "wt-target").await.unwrap();
            assert!(list_subscriptions(&pool, "term-old", now).await.unwrap().is_empty());

            // アプリ再起動
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();

            // 同じワークツリーで新しいタブが立つ
            let (subs, inbox) =
                rebind_next_orphaned_group(&pool, "wt-subscriber", "term-new").await.unwrap();
            assert_eq!((subs, inbox), (0, 1), "購読は既に無いが未読は引き継がれる");
            let recovered = list_inbox(&pool, "term-new", InboxFilter::Undelivered).await.unwrap();
            assert_eq!(recovered.len(), 1);
            assert_eq!(recovered[0].orphaned_at, None, "引き継ぎ後は押し込み対象になる");
            assert!(list_inbox(&pool, "term-old", InboxFilter::All).await.unwrap().is_empty());
        });
    }

    /// 引き継ぎは死亡タブ単位で1グループずつ。新タブ1つが全部を吸い上げない。
    #[test]
    fn test_rebind_claims_one_group_at_a_time() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            for (i, term) in ["term-1", "term-2", "term-3"].iter().enumerate() {
                let mut s = sub(term, Some("wt-subscriber"), r#"["worktree.closed"]"#);
                s.id = format!("sub-{}", i);
                s.target = format!("wt-target-{}", i);
                upsert_subscription(&pool, &s).await.unwrap();
            }
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();
            assert_eq!(list_orphaned_groups(&pool, "wt-subscriber").await.unwrap().len(), 3);

            let (subs, _) = rebind_next_orphaned_group(&pool, "wt-subscriber", "new-a").await.unwrap();
            assert_eq!(subs, 1);
            assert_eq!(list_orphaned_groups(&pool, "wt-subscriber").await.unwrap().len(), 2);

            let (subs, _) = rebind_next_orphaned_group(&pool, "wt-subscriber", "new-b").await.unwrap();
            assert_eq!(subs, 1);
            assert_eq!(list_orphaned_groups(&pool, "wt-subscriber").await.unwrap().len(), 1);
            // 引き継ぎ先の購読は active に戻っている
            assert_eq!(
                list_subscriptions(&pool, "new-a", now).await.unwrap()[0].state,
                STATE_ACTIVE
            );
        });
    }

    /// 引き継ぎは購読者ワークツリーの中に閉じる（他ワークツリーの分を奪わない）。
    #[test]
    fn test_rebind_scoped_to_worktree() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let mut a = sub("term-a", Some("wt-a"), r#"["worktree.closed"]"#);
            a.id = "sub-a".to_string();
            upsert_subscription(&pool, &a).await.unwrap();
            let mut b = sub("term-b", Some("wt-b"), r#"["worktree.closed"]"#);
            b.id = "sub-b".to_string();
            upsert_subscription(&pool, &b).await.unwrap();
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();

            rebind_next_orphaned_group(&pool, "wt-a", "new-a").await.unwrap();
            assert_eq!(list_subscriptions(&pool, "new-a", now).await.unwrap().len(), 1);
            assert_eq!(list_orphaned_groups(&pool, "wt-b").await.unwrap().len(), 1);
        });
    }

    /// 生存タブ（active）の購読は引き継ぎで奪われない。
    #[test]
    fn test_rebind_does_not_steal_from_active_subscription() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-live", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();

            let (subs, inbox) =
                rebind_next_orphaned_group(&pool, "wt-subscriber", "new-a").await.unwrap();
            assert_eq!((subs, inbox), (0, 0));
            assert_eq!(list_subscriptions(&pool, "term-live", now).await.unwrap().len(), 1);
        });
    }

    /// 引き継ぐものが無ければ (0,0)。冪等なので複数経路から同時に呼ばれても無害。
    #[test]
    fn test_rebind_returns_zero_when_nothing_orphaned() {
        with_pool(async {
            let pool = memory_pool().await;
            assert_eq!(
                rebind_next_orphaned_group(&pool, "wt-subscriber", "new-a").await.unwrap(),
                (0, 0)
            );
        });
    }

    /// 新タブが既に同じ対象を購読していた場合、UNIQUE(terminal_id, target) に負けた
    /// 孤児側を捨て、生存側の設定を保つ。
    #[test]
    fn test_rebind_subscription_unique_conflict_keeps_live_row() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let old = sub("term-old", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &old).await.unwrap();
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();

            // 新タブが同じ target を購読（delivery が違う）
            let mut new = sub("term-new", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            new.id = "sub-new".to_string();
            new.delivery = DELIVERY_INTERRUPT.to_string();
            upsert_subscription(&pool, &new).await.unwrap();

            rebind_next_orphaned_group(&pool, "wt-subscriber", "term-new").await.unwrap();
            let rows = list_subscriptions(&pool, "term-new", now).await.unwrap();
            assert_eq!(rows.len(), 1, "重複せず1行だけ残る");
            assert_eq!(rows[0].delivery, DELIVERY_INTERRUPT, "生存側の設定が勝つ");
            assert!(list_subscriptions(&pool, "term-old", now).await.unwrap().is_empty());
        });
    }

    /// 新タブが既に同じイベントを持っていた場合、UNIQUE(terminal_id, event_id) に負けた
    /// 孤児側を捨てる。生存側の delivered_at が保たれる。
    #[test]
    fn test_rebind_inbox_unique_conflict_drops_orphan() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let old = sub("term-old", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &old).await.unwrap();
            let mut new = sub("term-new", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            new.id = "sub-new".to_string();
            upsert_subscription(&pool, &new).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 2);

            // term-new は既に本文を受け取っている
            let new_ids: Vec<String> = list_inbox(&pool, "term-new", InboxFilter::All)
                .await
                .unwrap()
                .iter()
                .map(|i| i.id.clone())
                .collect();
            mark_delivered(&pool, &new_ids, now).await.unwrap();
            // term-old だけ死亡
            mark_orphaned_subscribers(&pool, &["term-new".to_string()], now).await.unwrap();

            rebind_orphaned_group(&pool, "wt-subscriber", "term-old", "term-new").await.unwrap();
            let rows = list_inbox(&pool, "term-new", InboxFilter::All).await.unwrap();
            assert_eq!(rows.len(), 1, "同じイベントが二重にならない");
            assert_eq!(rows[0].delivered_at, Some(now), "生存側の打刻が保たれる");
            assert!(list_inbox(&pool, "term-old", InboxFilter::All).await.unwrap().is_empty());
        });
    }

    /// ack 済みの inbox は監査証跡なので引き継ぎでも動かさない / 消さない。
    #[test]
    fn test_rebind_ignores_acked_rows() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-old", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, now).await.unwrap();
            let ids: Vec<String> = list_inbox(&pool, "term-old", InboxFilter::All)
                .await
                .unwrap()
                .iter()
                .map(|i| i.id.clone())
                .collect();
            ack(&pool, &ids, "term-old", now).await.unwrap();
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();

            let (_, inbox) =
                rebind_next_orphaned_group(&pool, "wt-subscriber", "term-new").await.unwrap();
            assert_eq!(inbox, 0);
            assert_eq!(list_inbox(&pool, "term-old", InboxFilter::All).await.unwrap().len(), 1);
        });
    }

    /// orphaned な購読にも配送は続ける（タブが死んでいた間のイベントを失わない）。
    #[test]
    fn test_fanout_delivers_to_orphaned_subscription() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();

            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 1);
            // 積まれた行も引き継ぎ待ちの印が付いている（＝押し込みはされない）
            let items = list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap();
            assert_eq!(items[0].orphaned_at, Some(now));
            assert!(list_pushable(&pool, now, PUSH_TTL_MS).await.unwrap().is_empty());

            // 引き継げば押し込み対象になる
            rebind_next_orphaned_group(&pool, "wt-subscriber", "term-new").await.unwrap();
            assert_eq!(list_pushable(&pool, now, PUSH_TTL_MS).await.unwrap().len(), 1);
        });
    }

    /// 古いイベントは押し込まない（再起動直後の一斉割り込みを防ぐ）。inbox には残る。
    #[test]
    fn test_list_pushable_excludes_stale_events() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, now).await.unwrap();

            assert_eq!(list_pushable(&pool, now, PUSH_TTL_MS).await.unwrap().len(), 1);
            let later = now + PUSH_TTL_MS + 1;
            assert!(list_pushable(&pool, later, PUSH_TTL_MS).await.unwrap().is_empty());
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::Unacked).await.unwrap().len(), 1);
        });
    }

    /// 連鎖が深すぎるイベントは配送しない（#120 §5.4）。
    #[test]
    fn test_fanout_drops_over_max_depth() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let mut e = event("wt-target", None);
            e.depth = MAX_EVENT_DEPTH + 1;
            insert_event(&pool, &e).await.unwrap();
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 0);
        });
    }

    /// `spawn_if_closed` と `delivery` は積んだ時点の値が inbox に焼き付く
    /// （購読行は fanout 直後に消えるので、後から参照できない）。
    #[test]
    fn test_spawn_candidates_use_inbox_snapshot() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let mut s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            s.spawn_if_closed = 1;
            s.delivery = DELIVERY_INTERRUPT.to_string();
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, now).await.unwrap();
            delete_subscriptions_for_target(&pool, "wt-target").await.unwrap();

            let items = list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap();
            assert_eq!(items[0].spawn_if_closed, 1);
            assert_eq!(items[0].delivery, DELIVERY_INTERRUPT);

            // タブが死ねば spawn 候補になる
            assert!(list_spawn_candidates(&pool, now, PUSH_TTL_MS).await.unwrap().is_empty());
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();
            let candidates = list_spawn_candidates(&pool, now, PUSH_TTL_MS).await.unwrap();
            assert_eq!(
                candidates,
                vec![("wt-subscriber".to_string(), KIND_WORKTREE_CLOSED.to_string(), 1)]
            );
        });
    }

    // ─── #126: target ワイルドカード ─────────────────────────────────────────

    #[test]
    fn test_normalize_target_lowercases_repo_only() {
        // repo 名は人間が打つ文字列なので大小と空白を吸収する
        assert_eq!(normalize_target("  repo:  OreTachi  "), "repo:oretachi");
        // ID は oretachi 発番なので一字一句そのまま
        assert_eq!(normalize_target(" workgroup: WG-1 "), "workgroup:WG-1");
        assert_eq!(normalize_target("  wt-ABC  "), "wt-ABC");
        assert_eq!(normalize_target("*"), "*");
    }

    #[test]
    fn test_matching_targets_covers_wildcards() {
        let t = matching_targets("wt-1", Some("wg-1"), Some("OreTachi"));
        assert!(t.contains(&"wt-1".to_string()));
        assert!(t.contains(&"*".to_string()));
        assert!(t.contains(&"workgroup:wg-1".to_string()));
        // 照合側も `normalize_target` を通るので、購読登録時の正規化と必ず一致する
        assert!(t.contains(&"repo:oretachi".to_string()));

        // 所属情報が無いイベントでも ID 一致と `*` は必ず候補に入る
        let t = matching_targets("wt-1", None, None);
        assert_eq!(t, vec!["wt-1".to_string(), "*".to_string()]);
        // 空文字は所属無しと同じ扱い（空の `workgroup:` を作らない）
        let t = matching_targets("wt-1", Some("  "), Some(""));
        assert_eq!(t, vec!["wt-1".to_string(), "*".to_string()]);
    }

    /// `*` / `workgroup:` / `repo:` の購読へ `worktree.created` が届く。
    /// **まだ存在しないワークツリーの作成を購読できる**ことがワイルドカードの存在理由。
    #[test]
    fn test_roundtrip_fanout_matches_wildcard_targets() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            for (i, target) in ["*", "workgroup:wg-1", "repo:oretachi"].iter().enumerate() {
                let mut s = sub(
                    &format!("term-{}", i),
                    Some(&format!("wt-sub-{}", i)),
                    r#"["worktree.created"]"#,
                );
                s.id = format!("sub-{}", i);
                s.target = target.to_string();
                upsert_subscription(&pool, &s).await.unwrap();
            }
            // 対象外の購読（別ワークグループ / 別リポジトリ）は拾わないこと
            let mut miss = sub("term-x", Some("wt-sub-x"), r#"["worktree.created"]"#);
            miss.id = "sub-x".to_string();
            miss.target = "workgroup:wg-other".to_string();
            upsert_subscription(&pool, &miss).await.unwrap();

            let mut e = event("wt-new", None);
            e.kind = KIND_WORKTREE_CREATED.to_string();
            e.body = r#"{"worktreeId":"wt-new","worktreeName":"n","branchName":"b"}"#.to_string();
            insert_event(&pool, &e).await.unwrap();
            let targets = matching_targets("wt-new", Some("wg-1"), Some("oretachi"));
            assert_eq!(fanout(&pool, &e, &targets, now).await.unwrap(), 3);
            assert_eq!(list_inbox(&pool, "term-x", InboxFilter::All).await.unwrap().len(), 0);
        });
    }

    /// `*` 購読は自ワークツリー発のイベントにも必ずマッチするので、`is_self_echo` が
    /// 唯一の防波堤になる（#126 の安全性④）。
    #[test]
    fn test_roundtrip_wildcard_suppresses_self_echo() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            // 同じワークツリーの別タブ（term-b）も含めて `*` を購読する
            for (i, terminal) in ["term-a", "term-b"].iter().enumerate() {
                let mut s = sub(terminal, Some("wt-src"), r#"["worktree.message"]"#);
                s.id = format!("sub-{}", i);
                s.target = TARGET_ALL.to_string();
                upsert_subscription(&pool, &s).await.unwrap();
            }
            let mut other = sub("term-c", Some("wt-other"), r#"["worktree.message"]"#);
            other.id = "sub-c".to_string();
            other.target = TARGET_ALL.to_string();
            upsert_subscription(&pool, &other).await.unwrap();

            let mut e = event("wt-src", Some("term-a"));
            e.kind = KIND_WORKTREE_MESSAGE.to_string();
            e.body = r#"{"text":"hello"}"#.to_string();
            insert_event(&pool, &e).await.unwrap();
            // 発火元タブ(term-a)も、同じワークツリーの別タブ(term-b)も受け取らない
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 1);
            assert_eq!(list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap().len(), 0);
            assert_eq!(list_inbox(&pool, "term-b", InboxFilter::All).await.unwrap().len(), 0);
            assert_eq!(list_inbox(&pool, "term-c", InboxFilter::All).await.unwrap().len(), 1);
        });
    }

    // ─── #126: depth の伝播と往復の終端 ──────────────────────────────────────

    /// A↔B の往復が `MAX_EVENT_DEPTH` で止まる。**本 issue で自由文が入ることで初めて
    /// 実在するようになった経路**なので、実際に止まることをここで固定する。
    #[test]
    fn test_roundtrip_message_ping_pong_stops_at_max_depth() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            // A と B が互いを購読する
            for (i, (terminal, worktree, target)) in [
                ("term-a", "wt-a", "wt-b"),
                ("term-b", "wt-b", "wt-a"),
            ]
            .iter()
            .enumerate()
            {
                let mut s = sub(terminal, Some(worktree), r#"["worktree.message"]"#);
                s.id = format!("sub-{}", i);
                s.target = target.to_string();
                upsert_subscription(&pool, &s).await.unwrap();
            }

            // 「受け取ったら返信する」を交互に繰り返す。返信は相手に届いた場合にだけ
            // 次の送信を誘発するので、配送されなくなった時点で往復は自然に止まる。
            let mut depths = Vec::new();
            let mut round = 0usize;
            loop {
                let (src_wt, src_term) = if round % 2 == 0 {
                    ("wt-a", "term-a")
                } else {
                    ("wt-b", "term-b")
                };
                // 送信側の depth は「直近に受け取った最大 depth + 1」（申告制にしない）
                let depth = max_inbound_depth(&pool, src_term, now, CHAIN_WINDOW_MS)
                    .await
                    .unwrap()
                    .map_or(0, |d| d + 1);
                let mut e = event(src_wt, Some(src_term));
                e.id = format!("ev-{}", round);
                e.kind = KIND_WORKTREE_MESSAGE.to_string();
                e.body = r#"{"text":"ping"}"#.to_string();
                e.depth = depth;
                insert_event(&pool, &e).await.unwrap();
                depths.push(depth);
                let delivered = fanout_all(&pool, &e, now).await.unwrap();
                if delivered == 0 {
                    break;
                }
                round += 1;
                assert!(round < 20, "往復が止まらない: {:?}", depths);
            }

            // 片道4本（depth 0→3）まで届き、5本目の depth 4 が MAX_EVENT_DEPTH(3) 超過で
            // 破棄されて連鎖が切れる。相手には何も届かないので次の返信も誘発されない。
            assert_eq!(depths, vec![0, 1, 2, 3, 4]);

            // 送れなくなった側が再送しても、受信済みの最大 depth は変わらないので通らない
            let depth = max_inbound_depth(&pool, "term-a", now, CHAIN_WINDOW_MS)
                .await
                .unwrap()
                .map_or(0, |d| d + 1);
            assert_eq!(depth, MAX_EVENT_DEPTH + 1);
            let mut retry = event("wt-a", Some("term-a"));
            retry.id = "ev-retry".to_string();
            retry.kind = KIND_WORKTREE_MESSAGE.to_string();
            retry.depth = depth;
            insert_event(&pool, &retry).await.unwrap();
            assert_eq!(fanout_all(&pool, &retry, now).await.unwrap(), 0);
        });
    }

    /// `created` / `closed` は必ず depth 0 のユーザー操作起点なので、連鎖の深さに数えない。
    /// 数えると `*` を購読して作成通知を受けているだけのタブが常に depth 1 から送ることになり、
    /// 本来の会話に使える往復回数だけが削られる。
    #[test]
    fn test_roundtrip_chain_depth_counts_messages_only() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let mut s = sub("term-a", Some("wt-a"), r#"["worktree.created","worktree.message"]"#);
            s.target = TARGET_ALL.to_string();
            upsert_subscription(&pool, &s).await.unwrap();

            // 作成通知を受けても連鎖の起点のまま
            let mut created = event("wt-b", None);
            created.id = "ev-created".to_string();
            created.kind = KIND_WORKTREE_CREATED.to_string();
            insert_event(&pool, &created).await.unwrap();
            assert_eq!(fanout_all(&pool, &created, now).await.unwrap(), 1);
            assert_eq!(
                max_inbound_depth(&pool, "term-a", now, CHAIN_WINDOW_MS).await.unwrap(),
                None
            );

            // 自由文メッセージを受けたときだけ連鎖が進む
            let mut msg = event("wt-b", Some("term-b"));
            msg.id = "ev-msg".to_string();
            msg.kind = KIND_WORKTREE_MESSAGE.to_string();
            msg.depth = 1;
            insert_event(&pool, &msg).await.unwrap();
            fanout_all(&pool, &msg, now).await.unwrap();
            assert_eq!(
                max_inbound_depth(&pool, "term-a", now, CHAIN_WINDOW_MS).await.unwrap(),
                Some(1)
            );
        });
    }

    /// 連鎖ウィンドウを跨げば depth は 0 に戻る。窓を切らないと、一度深い連鎖に
    /// 巻き込まれたタブが**以後永久に発言不能**になる。
    #[test]
    fn test_roundtrip_chain_window_resets_depth() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let mut s = sub("term-a", Some("wt-a"), r#"["worktree.message"]"#);
            s.target = "wt-b".to_string();
            upsert_subscription(&pool, &s).await.unwrap();
            let mut e = event("wt-b", Some("term-b"));
            e.kind = KIND_WORKTREE_MESSAGE.to_string();
            e.depth = MAX_EVENT_DEPTH;
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, now).await.unwrap();

            // 窓の内側では連鎖の続きとして扱う
            assert_eq!(
                max_inbound_depth(&pool, "term-a", now, CHAIN_WINDOW_MS).await.unwrap(),
                Some(MAX_EVENT_DEPTH)
            );
            // 窓を過ぎれば起点に戻る
            let later = now + CHAIN_WINDOW_MS + 1;
            assert_eq!(
                max_inbound_depth(&pool, "term-a", later, CHAIN_WINDOW_MS).await.unwrap(),
                None
            );
        });
    }

    // ─── #126: 自動承認 default-deny の材料 ──────────────────────────────────

    /// spawn 候補は種別ごとに分かれて返る。潰して件数だけにすると、押し込めない
    /// 自由文メッセージが「タブを立てさせる」ところまで通ってしまう。
    #[test]
    fn test_roundtrip_spawn_candidates_are_split_by_kind() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;

            let mut s = sub("term-a", Some("wt-sub"), r#"["worktree.closed","worktree.message"]"#);
            s.spawn_if_closed = 1;
            s.target = TARGET_ALL.to_string();
            upsert_subscription(&pool, &s).await.unwrap();

            for (i, kind) in [KIND_WORKTREE_CLOSED, KIND_WORKTREE_MESSAGE].iter().enumerate() {
                let mut e = event(&format!("wt-src-{}", i), None);
                e.id = format!("ev-{}", i);
                e.kind = kind.to_string();
                insert_event(&pool, &e).await.unwrap();
                fanout_all(&pool, &e, now).await.unwrap();
            }
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();

            let mut candidates = list_spawn_candidates(&pool, now, PUSH_TTL_MS).await.unwrap();
            candidates.sort();
            assert_eq!(
                candidates,
                vec![
                    ("wt-sub".to_string(), KIND_WORKTREE_CLOSED.to_string(), 1),
                    ("wt-sub".to_string(), KIND_WORKTREE_MESSAGE.to_string(), 1),
                ]
            );
        });
    }

    /// `worktree.created` はワークツリーごとに一度きり。二重に積むと購読者が同じ作成を
    /// 2回読むことになる（`notify_worktree_added` は失敗経路からも呼ばれる）。
    #[test]
    fn test_roundtrip_has_event_guards_duplicate_created() {
        with_pool(async {
            let pool = memory_pool().await;
            assert!(!has_event(&pool, "wt-new", KIND_WORKTREE_CREATED).await.unwrap());
            let mut e = event("wt-new", None);
            e.kind = KIND_WORKTREE_CREATED.to_string();
            insert_event(&pool, &e).await.unwrap();
            assert!(has_event(&pool, "wt-new", KIND_WORKTREE_CREATED).await.unwrap());
            // 種別違い / ワークツリー違いは別扱い
            assert!(!has_event(&pool, "wt-new", KIND_WORKTREE_CLOSED).await.unwrap());
            assert!(!has_event(&pool, "wt-other", KIND_WORKTREE_CREATED).await.unwrap());
        });
    }

    // ─── #126: 表示整形 ───────────────────────────────────────────────────────

    #[test]
    fn test_format_inbox_line_renders_new_kinds() {
        let item = inbox_item(
            KIND_WORKTREE_CREATED,
            r#"{"worktreeId":"wt-1","worktreeName":"oretachi-x","branchName":"feature/y","repositoryName":"oretachi"}"#,
        );
        let line = format_inbox_line(&item);
        assert!(line.contains("oretachi-x"), "{}", line);
        assert!(line.contains("作成されました"), "{}", line);
        assert!(line.contains("[oretachi]"), "{}", line);

        let item = inbox_item(
            KIND_WORKTREE_MESSAGE,
            r#"{"text":"レビュー待ちです","sourceWorktreeName":"design"}"#,
        );
        let line = format_inbox_line(&item);
        assert!(line.contains("design"), "{}", line);
        assert!(line.contains("レビュー待ちです"), "{}", line);
    }

    /// 自由文メッセージがブラケットペーストを脱出できない（#126 の安全性⑤）。
    /// ブランチ名経由（#125）と違い、本文全体が攻撃者制御になるのがこちら。
    #[test]
    fn test_format_inbox_push_text_sanitizes_free_form_message() {
        let body = serde_json::json!({
            "text": "ok\u{1b}[201~\rrm -rf /\r",
            "sourceWorktreeName": "evil\u{1b}[201~\r",
        })
        .to_string();
        let item = inbox_item(KIND_WORKTREE_MESSAGE, &body);
        let (text, used) = format_inbox_push_text(std::slice::from_ref(&item)).unwrap();
        assert_eq!(used, 1);
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\r'));
        assert!(!text.contains('\n'));
        assert!(text.contains("rm -rf /"), "本文自体は落とさず1行に潰すだけ: {}", text);
    }

    /// 押し込み本文の締めが種別に依存していないこと。`closed` 決め打ちの文言のままだと
    /// `created` / `message` を「作業が完了した」と誤読させる。
    #[test]
    fn test_format_inbox_push_text_is_kind_agnostic() {
        let item = inbox_item(
            KIND_WORKTREE_CREATED,
            r#"{"worktreeId":"wt-1","worktreeName":"n","branchName":"b"}"#,
        );
        let (text, _) = format_inbox_push_text(std::slice::from_ref(&item)).unwrap();
        assert!(!text.contains("作業が完了した"), "{}", text);
        assert!(text.contains("oretachi_ack_message"), "{}", text);
    }

    /// ブランチ名にペースト終端シーケンスを埋め込んでも脱出できない。
    #[test]
    fn test_sanitize_for_pty_strips_escape_sequences() {
        let evil = "feature/x\x1b[201~\rrm -rf /\r";
        let cleaned = sanitize_for_pty(evil);
        assert!(!cleaned.contains('\x1b'));
        assert!(!cleaned.contains('\r'));
        assert!(!cleaned.contains('\n'));
        assert!(cleaned.contains("feature/x"));
    }

    #[test]
    fn test_sanitize_for_pty_caps_length() {
        let long = "あ".repeat(5000);
        let cleaned = sanitize_for_pty(&long);
        assert!(cleaned.chars().count() <= PTY_TEXT_MAX_CHARS + 1);
    }

    /// 押し込み用テキストは1行（改行が混ざるとその場でターン送信になる）。
    #[test]
    fn test_format_inbox_push_text_is_single_line() {
        let items = vec![inbox_item(
            KIND_WORKTREE_CLOSED,
            r#"{"worktreeId":"a","worktreeName":"wt-a","branchName":"feature/x"}"#,
        )];
        let (text, used) = format_inbox_push_text(&items).unwrap();
        assert!(!text.contains('\n') && !text.contains('\r'));
        assert!(text.contains("wt-a"));
        assert_eq!(used, 1);
        assert!(format_inbox_push_text(&[]).is_none());
    }

    /// 長さ上限で本文に載らなかった分の件数は返さない。
    /// 全件を配送済みにすると、切り落とされた分は「再送しない」方針のせいで二度と出ない。
    #[test]
    fn test_format_inbox_push_text_reports_only_what_fits() {
        let items: Vec<InboxItem> = (0..40)
            .map(|i| {
                let mut item = inbox_item(
                    KIND_WORKTREE_CLOSED,
                    r#"{"worktreeId":"a","worktreeName":"very-long-worktree-name-for-truncation","branchName":"feature/quite-long-branch-name"}"#,
                );
                item.id = format!("inbox-{:02}", i);
                item
            })
            .collect();
        let (text, used) = format_inbox_push_text(&items).unwrap();
        assert!(used > 0 && used < items.len(), "一部だけ載る想定 (used={})", used);
        assert!(text.chars().count() <= PTY_TEXT_MAX_CHARS + 1);
        // 残件告知を足しても上限を超えず、末尾の ack 指示が切られていないこと
        assert!(text.ends_with("ack してください。"), "末尾が切れている: {}", text);
        // 載った分だけが本文にあり、載らなかった分は残件として告知される
        assert!(text.contains(&format!("inbox-{:02}", used - 1)));
        assert!(!text.contains(&format!("inbox-{:02}", used)));
        assert!(text.contains(&format!("ほかに {} 件", items.len() - used)));
    }

    /// 引き継ぎ待ちの購読へ後から届いたメッセージは、購読が orphaned になった時刻ではなく
    /// **届いた時刻**で保持期限を数える（数時間で消えないように）。
    #[test]
    fn test_fanout_stamps_inbox_with_arrival_time_not_subscription_time() {
        with_pool(async {
            let pool = memory_pool().await;
            let long_ago = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            mark_orphaned_subscribers(&pool, &[], long_ago).await.unwrap();

            // 6日後にイベントが届く
            let arrival = long_ago + 6 * 24 * 60 * 60 * 1000;
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, arrival).await.unwrap();

            let items = list_inbox(&pool, "term-a", InboxFilter::All).await.unwrap();
            assert_eq!(items[0].orphaned_at, Some(arrival));
            // 到着から7日以内なので保持期限では消えない
            let (_, purged) =
                purge_orphaned_expired(&pool, arrival + 1000, ORPHANED_RETENTION_MS).await.unwrap();
            assert_eq!(purged, 0);
        });
    }

    /// 生存しているタブが（古い生存一覧のせいで）orphaned に落ちたら active へ戻す。
    /// 戻す道が無いと、そのタブは生存しているのに引き継ぎ対象からも押し込み対象からも外れる。
    #[test]
    fn test_mark_orphaned_restores_live_terminals() {
        with_pool(async {
            let pool = memory_pool().await;
            let now = 1_000_000i64;
            let s = sub("term-a", Some("wt-subscriber"), r#"["worktree.closed"]"#);
            upsert_subscription(&pool, &s).await.unwrap();
            let e = event("wt-target", None);
            insert_event(&pool, &e).await.unwrap();
            fanout_all(&pool, &e, now).await.unwrap();

            // 古い生存一覧で誤って orphaned に落ちる
            mark_orphaned_subscribers(&pool, &[], now).await.unwrap();
            assert_eq!(list_subscriptions(&pool, "term-a", now).await.unwrap()[0].state, STATE_ORPHANED);
            assert!(list_pushable(&pool, now, PUSH_TTL_MS).await.unwrap().is_empty());

            // 次の tick で生存が確認されれば戻る
            mark_orphaned_subscribers(&pool, &["term-a".to_string()], now).await.unwrap();
            let rows = list_subscriptions(&pool, "term-a", now).await.unwrap();
            assert_eq!(rows[0].state, STATE_ACTIVE);
            assert_eq!(rows[0].orphaned_at, None);
            assert_eq!(list_pushable(&pool, now, PUSH_TTL_MS).await.unwrap().len(), 1);
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
            assert_eq!(fanout_all(&pool, &e, now).await.unwrap(), 1);

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
            fanout_all(&pool, &e, old).await.unwrap();

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
