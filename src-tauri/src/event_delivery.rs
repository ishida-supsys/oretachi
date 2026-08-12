//! ワークツリー間イベントの**配送**（issue #120 §4 / #125）。
//!
//! `event_db` が「発行 → 照合 → 蓄積」までを持つのに対し、ここは「蓄積 → 実際に相手を
//! 動かす」を担う。受信側の状態で手段が変わるのが本質的な難しさで、判断表は以下:
//!
//! | 受信側 | 判定元 | 動作 |
//! |---|---|---|
//! | 生存 Claude Code + `idle` + 静穏 | `~/.claude/sessions/<pid>.json` の `status` | PTY 押し込み |
//! | 生存 Claude Code + `busy` / 不明 / 出力継続中 | 同上 | 押し込まない（`delivery=interrupt` の明示指定時のみ押し込む） |
//! | 生存 非 CC エージェント | `agent_name` | PTY 押し込み。**`status` の取得元が存在せず busy/idle を判定できない**（#120 §5.7） |
//! | 生存タブ・エージェント無し | `is_ai_agent` | 押し込まない（素のシェルに CR を撃たない）。`spawn_if_closed` なら spawn |
//! | タブ不在（orphaned） | — | `spawn_if_closed` なら spawn、false なら保留（次に AI タブが立てば引き継がれる） |
//!
//! ## 直列化
//!
//! 全ての再バインドと押し込みは**単一のワーカータスク**が順に処理する。これが #125 §3 の
//! 「宛先ごとの直列キュー」の実体で、宛先ごとより強い保証になる。ポーリングスレッド /
//! `/session-context`（axum）/ MCP ツールハンドラが同時に再バインドを要求しても、キューを
//! 通るので SELECT→UPDATE のインターリーブが構造的に起きない。
//!
//! ## DB 初期化失敗時
//!
//! `DeliveryHandle` は `EventPool` と同じく `init_event_db` が成功したときだけ `manage` される。
//! 生産者は全て `try_state::<DeliveryHandle>()` を通すので、DB が無ければ静かに no-op になる。

use crate::event_db;
use crate::settings::{AppSettings, SettingsManager, WorktreeEntry};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot};

/// 定期的な再評価の間隔。`busy` で見送った宛先や、spawn 待ちの解放をここで拾う。
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// キュー長。溢れたら捨てる（生産者はどれも待てない: `fire_worktree_closed` は
/// fire-and-forget、`/session-context` はサイドカーの 2 秒デッドラインの内側）。
const QUEUE_CAPACITY: usize = 64;

/// 同じタブへ続けて押し込むまでの最小間隔（#125 §3 のレート制限）。
const MIN_PUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// `agent_status` のサンプルをどれだけ新しければ信用するか。ポーリング周期 10 秒の 2 倍。
/// 1 回取りこぼしても配送は止まらないが、ポーリングが死んでいれば押し込みも止まる。
const STATUS_MAX_AGE_MS: i64 = 20_000;

/// ブラケットペーストを流してから Enter を送るまでの猶予。
/// Claude Code はペースト終端と同じ読み取りチャンクに来た CR を送信として扱わない。
const SUBMIT_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// spawn 要求を出してからフロントの応答を待つ上限。過ぎたら単一フライトを解放する。
const SPAWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// 同じワークツリーへ続けて自動 spawn するまでの最小間隔。
/// spawn したタブでエージェントが起動しない（`claude` が PATH に無い等）と検出フラグが
/// 立たないため、これが無いと tick ごとに壊れたタブを上限まで積み増す。
const SPAWN_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);

/// これ以上ターミナルがあるときは自動 spawn しない。
/// 端末数 15+ で webview ハングと強い相関があるため、自動増殖には手前で天井を設ける。
const SPAWN_MAX_LIVE_SESSIONS: usize = 12;

/// 自動承認が有効なワークツリーへ押し込み / spawn してよいイベント種別（#120 §5.5）。
///
/// 自動承認が有効な宛先へは、別のワークツリーから注入された内容が**人間の確認なしに
/// 実行される**。
///
/// 許可の基準は「**本文を誰が書いたか**」:
/// - 許可: oretachi 自身が定型 JSON として組み立てるイベント
///   （`worktree.closed` / `worktree.created`）。可変部分はワークツリー名・ブランチ名だけで、
///   いずれも `sanitize_for_pty` を通る
/// - 不許可: エージェントが本文を書けるイベント（`worktree.message`, #126）。押し込まず
///   inbox に残し、人間が UI で確認して手動で渡すか、自動承認を切ることを要求する
///
/// **新しい種別を足すときは必ずこの基準で判断すること。** 迷ったら入れない（default-deny）。
const AUTO_APPROVAL_PUSHABLE_KINDS: &[&str] = &[
    event_db::KIND_WORKTREE_CLOSED,
    event_db::KIND_WORKTREE_CREATED,
];

// ─── ハンドル ─────────────────────────────────────────────────────────────────

/// `additionalContext` 経由で未読をまとめて渡すときの、呼び出し元の hook 種別（#124）。
///
/// PTY 押し込みと違い、どれも「文字列を返すだけ」で PTY には触らない。ただし
/// **`TurnEnd` だけはターンを開始させる**（`Stop` の `additionalContext` は会話を継続する）
/// ため、押し込みと同じ制限（`passive` を巻き込まない・自動承認が有効な宛先へは定型
/// イベントのみ）を課す。`SessionStart` / `PromptSubmit` は人間の操作が起点なので、
/// Phase 1 から変わらず全件を渡す。
#[derive(Debug, Clone, PartialEq)]
pub enum DigestReason {
    /// SessionStart フック。アプリ停止中に溜まった分の回収（#123）
    SessionStart,
    /// Stop フック。ターン境界配送の本命（#124）。`prompt_id` は「1ターン1回」の鍵
    TurnEnd { prompt_id: Option<String> },
    /// UserPromptSubmit フック。Stop の取りこぼし回収（#124）
    PromptSubmit,
}

impl DigestReason {
    /// ログに出す識別子（`event-delivered` の `method` は #137 で廃止した）。
    fn label(&self) -> &'static str {
        match self {
            DigestReason::SessionStart => "session",
            DigestReason::TurnEnd { .. } => "stop",
            DigestReason::PromptSubmit => "prompt",
        }
    }

    fn is_turn_end(&self) -> bool {
        matches!(self, DigestReason::TurnEnd { .. })
    }

    /// 本文が0件でも「未 ack が N 件残っています」だけを伝えてよい経路か。
    ///
    /// `SessionStart` だけ `true`。発火は startup / resume / clear / compact に限られ
    /// （`claude_plugin.rs` の SessionStart フックは matcher なしで登録している）、
    /// 毎ターン・毎プロンプトの `Stop` / `UserPromptSubmit` とは頻度が桁違いに低い。
    /// この頻度なら Phase 1 の「注入が失われても件数で気づける」（#120 §5.2）が
    /// 実用になる。`Stop` / `UserPromptSubmit` で同じことをやると、`Stop` は残件を
    /// 報告するためだけに会話が継続し、`UserPromptSubmit` は ack するまで
    /// **ユーザーの全プロンプトの先頭に催促が付きまとう**。
    fn reports_carryover_only(&self) -> bool {
        matches!(self, DigestReason::SessionStart)
    }
}

pub enum DeliveryMsg {
    /// 生存タブの一覧。ここに無い terminal_id の購読 / inbox は orphaned にする
    Reconcile { live_terminal_ids: Vec<String> },
    /// hook 経路（SessionStart / Stop / UserPromptSubmit）へ渡す未読テキストを組む。
    /// **押し込みと同じワーカーで直列に処理するのが要点**（下の `collect_digest_and_wait` 参照）
    CollectDigest {
        terminal_id: String,
        reason: DigestReason,
        reply: oneshot::Sender<Option<String>>,
    },
    /// このタブへ、同じワークツリーの引き継ぎ待ちグループを1つ引き継ぐ（応答は待たない）。
    /// 完了を待ちたい経路（hook からの回収）は `CollectDigest` が内包している。
    Rebind { terminal_id: String },
    /// UI からの手動引き継ぎ（引き継ぎ先グループを人間が選ぶ）
    RebindManual {
        worktree_id: String,
        dead_terminal_id: String,
        terminal_id: String,
        reply: Option<oneshot::Sender<(u64, u64)>>,
    },
    /// 新しいイベントが inbox に積まれた
    EventQueued,
    /// フロントからの spawn 完了応答（失敗時は `session_id: None`）
    SpawnResult {
        request_id: String,
        session_id: Option<u32>,
    },
}

pub struct DeliveryHandle {
    tx: mpsc::Sender<DeliveryMsg>,
}

impl DeliveryHandle {
    /// 非ブロッキング送信。キューが詰まっていたら捨てる（次の tick で再評価される）。
    pub fn try_send(&self, msg: DeliveryMsg) {
        if self.tx.try_send(msg).is_err() {
            log::debug!("[delivery] キューが満杯のため要求を捨てた（次の tick で再評価する）");
        }
    }
}

fn handle(app: &AppHandle) -> Option<tauri::State<'_, DeliveryHandle>> {
    app.try_state::<DeliveryHandle>()
}

/// ポーリングスレッドから毎 tick 呼ぶ。死んだタブの購読を orphaned に落とす。
pub fn reconcile_live_terminals(app: &AppHandle, live_terminal_ids: Vec<String>) {
    if let Some(h) = handle(app) {
        h.try_send(DeliveryMsg::Reconcile { live_terminal_ids });
    }
}

/// AI エージェントが立ち上がったタブから引き継ぎを要求する（応答は待たない）。
pub fn request_rebind(app: &AppHandle, terminal_id: String) {
    if let Some(h) = handle(app) {
        h.try_send(DeliveryMsg::Rebind { terminal_id });
    }
}

/// 新しいイベントが積まれたことを知らせる。
pub fn notify_event_queued(app: &AppHandle) {
    if let Some(h) = handle(app) {
        h.try_send(DeliveryMsg::EventQueued);
    }
}

/// hook 経路へ渡す未読テキストを組み、出した分に `delivered_at` を打つ（#124）。
///
/// **押し込み (`push_pending`) と同じワーカーで処理させるのが要点。** 別経路で
/// 「未配送を SELECT → 注入 → 打刻」をやると、その隙間に走った押し込みが同じ行を
/// PTY へ流し、同じ本文が二重に届く（`mark_delivered` の `WHERE delivered_at IS NULL`
/// は打刻を冪等にするだけで、読み取りと注入のインターリーブは防げない）。
///
/// ワーカーが詰まっていたら `None`（＝今回は注入しない）。呼び出し元は上位のタイム
/// アウトの内側で使うので、取りこぼしではなく次の機会に回るだけで済む。
pub async fn collect_digest_and_wait(
    app: &AppHandle,
    terminal_id: &str,
    reason: DigestReason,
) -> Option<String> {
    let (tx, rx) = oneshot::channel();
    {
        let h = handle(app)?;
        if h.tx
            .try_send(DeliveryMsg::CollectDigest {
                terminal_id: terminal_id.to_string(),
                reason,
                reply: tx,
            })
            .is_err()
        {
            return None;
        }
    }
    rx.await.ok().flatten()
}

// ─── ワーカー ─────────────────────────────────────────────────────────────────

#[derive(Default)]
struct WorkerState {
    /// タブごとの最終押し込み時刻（レート制限）
    last_push: HashMap<String, Instant>,
    /// spawn 要求中のワークツリー（単一フライト）。request_id と発行時刻
    inflight_spawn: HashMap<String, (String, Instant)>,
    /// ワークツリーごとの最終 spawn 時刻。単一フライトは応答で即解除されるので、
    /// 「spawn したがエージェントが検出されない」（`claude` が PATH に無い等）ときに
    /// tick ごとに新しいタブを積み増すのを止めるためのクールダウン。
    last_spawn: HashMap<String, Instant>,
    /// 自動引き継ぎを済ませたタブ。`resolve_subscriber` は購読系ツールの呼び出しごとに
    /// 引き継ぎを要求するため、抑止が無いと `oretachi_poll_inbox` を数回叩くだけで
    /// 1タブが同じワークツリーの死亡タブ全グループを吸い上げ、「新しいタブ1つにつき
    /// 1グループ」という設計不変条件が崩れる。タブが消えたら忘れる。
    claimed: std::collections::HashSet<String>,
    /// 引き継ぎを試したが**対象が1グループも無かった**タブ（#124 のレビュー指摘）。
    ///
    /// `claimed` は「1グループ引き継いだ」タブしか入らないので、引き継ぐものが無いタブは
    /// 永久に `claimed` に入らない。`Stop` / `UserPromptSubmit` は毎ターン・毎プロンプト
    /// 発火するため、抑止しないと**全 CC タブが毎ターン `list_orphaned_groups` を叩き続ける**
    /// （events.db は非 WAL なので書き込みと競合しうる）。
    ///
    /// 引き継ぎ待ちグループが新しく生まれるのは `mark_orphaned_subscribers`（＝生存タブ一覧が
    /// 変化した `Reconcile`）だけなので、そこで捨てれば取りこぼさない。
    rebind_probed: std::collections::HashSet<String>,
    /// タブごとの「最後に Stop 経路で注入したターン」の `prompt_id`（#124）。
    ///
    /// `Stop` の `additionalContext` は会話を継続させるので、継続したターンが終われば
    /// また `Stop` が発火する。`stop_hook_active` で弾くのが第一の防波堤だが、CC 側の
    /// 仕様変更で無防備にならないよう `prompt_id` 単位の上限も持つ。実測（#121）で
    /// **1ユーザー発話に対する 9 回の発火すべてで `prompt_id` は同一**だったので、
    /// これがターン境界の正確な鍵になる（`session_id` は複数ターンで共通なので粗い）。
    /// タブが消えたら忘れる。
    last_turn: HashMap<String, String>,
    /// 前回 `Reconcile` を処理したときの生存タブ一覧（ソート済み）。
    ///
    /// ポーリングは 10 秒ごとに送ってくるが、`events.db` は WAL ではない（sqlx は
    /// 明示指定が無いと `journal_mode` を触らない）ので**書き込みが読み取りをブロックする**。
    /// この DB は SessionStart フックのリクエストパス（`collect_inbox_digest`、busy_timeout
    /// 800ms / 全体 1200ms）からも読まれるため、変化が無いときは書き込みを一切走らせない。
    last_live: Option<Vec<String>>,
    /// 保持期限切れの掃除を最後に回した時刻。期限は7日なので毎 tick 回す必要はない。
    last_retention_purge: Option<Instant>,
}

/// 引き継ぎ待ちの保持期限切れを掃除する間隔。
const RETENTION_PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

pub fn start(app: AppHandle, pool: SqlitePool) -> DeliveryHandle {
    let (tx, mut rx) = mpsc::channel::<DeliveryMsg>(QUEUE_CAPACITY);
    tauri::async_runtime::spawn(async move {
        let mut state = WorkerState::default();
        // 前回の定期再評価。**経過時間で判定する**のが要点: ポーリングスレッドが 10 秒ごとに
        // `Reconcile` を送ってくるため、`timeout(TICK, recv())` の Err 分岐だけに頼ると
        // タイムアウトが毎回リセットされて定期再評価が永久に走らない（実機で踏んだ）。
        let mut last_drive = Instant::now();
        loop {
            // `tokio` の `macros` / `rt` feature に依存しないため `select!` は使わない。
            match tokio::time::timeout(TICK, rx.recv()).await {
                Ok(None) => break, // 送信側が全て drop = アプリ終了
                Ok(Some(msg)) => {
                    handle_msg(&app, &pool, &mut state, msg).await;
                }
                Err(_) => {}
            }
            if last_drive.elapsed() >= TICK {
                last_drive = Instant::now();
                // busy / 出力継続中で見送った宛先の再評価と、spawn 待ちの解放
                state
                    .inflight_spawn
                    .retain(|_, (_, at)| at.elapsed() < SPAWN_TIMEOUT);
                drive(&app, &pool, &mut state).await;
            }
        }
        log::debug!("[delivery] worker stopped");
    });
    DeliveryHandle { tx }
}

async fn handle_msg(app: &AppHandle, pool: &SqlitePool, state: &mut WorkerState, msg: DeliveryMsg) {
    match msg {
        DeliveryMsg::Reconcile { live_terminal_ids } => {
            let now = event_db::now_ms();
            let mut live_sorted = live_terminal_ids.clone();
            live_sorted.sort();
            // 生存タブに変化が無ければ DB へは触らない。10 秒ごとに無条件で書き込むと、
            // WAL でないこの DB では SessionStart フックの読み取りと競合しうる。
            if state.last_live.as_ref() != Some(&live_sorted) {
                state.last_live = Some(live_sorted);
                // 消えたタブの「引き継ぎ済み」マークは忘れる（terminal_id は再利用されないので
                // 放置しても誤動作はしないが、長時間稼働で単調増加する）。
                state.claimed.retain(|t| live_terminal_ids.contains(t));
                state.last_turn.retain(|t, _| live_terminal_ids.contains(t));
                // 引き継ぎ待ちグループが増えるのはこの直後の `mark_orphaned_subscribers`
                // だけなので、「対象が無かった」という記憶はここで捨てれば十分。
                state.rebind_probed.clear();
                match event_db::mark_orphaned_subscribers(pool, &live_terminal_ids, now).await {
                    Ok((subs, inbox, deleted)) if subs > 0 || inbox > 0 || deleted > 0 => {
                        log::info!(
                            "[delivery] タブ死亡を検出: 購読 {} 件 / 未読 {} 件を引き継ぎ待ちにした（到達不能な {} 件は削除）",
                            subs, inbox, deleted
                        );
                        let _ = app.emit("event-inbox-changed", ());
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("[delivery] mark_orphaned_subscribers failed: {}", e),
                }
            }
            // 保持期限は7日なので、掃除は1時間おきで十分。
            let purge_due = state
                .last_retention_purge
                .map(|at| at.elapsed() >= RETENTION_PURGE_INTERVAL)
                .unwrap_or(true);
            if purge_due {
                state.last_retention_purge = Some(Instant::now());
                match event_db::purge_orphaned_expired(pool, now, event_db::ORPHANED_RETENTION_MS)
                    .await
                {
                    Ok((subs, inbox)) if subs > 0 || inbox > 0 => log::info!(
                        "[delivery] 保持期限（{}日）を過ぎた引き継ぎ待ちを削除: 購読 {} 件 / 未読 {} 件",
                        event_db::ORPHANED_RETENTION_DAYS,
                        subs,
                        inbox
                    ),
                    Ok(_) => {}
                    Err(e) => log::warn!("[delivery] purge_orphaned_expired failed: {}", e),
                }
            }
        }
        DeliveryMsg::Rebind { terminal_id } => {
            // 自動引き継ぎは1タブにつき1グループまで。人間が明示的に選ぶ `RebindManual` は
            // この制限を受けない。
            let result = try_rebind_once(app, pool, state, &terminal_id).await;
            if result != (0, 0) {
                let _ = app.emit("event-inbox-changed", ());
            }
            drive(app, pool, state).await;
        }
        DeliveryMsg::RebindManual {
            worktree_id,
            dead_terminal_id,
            terminal_id,
            reply,
        } => {
            let result = event_db::rebind_orphaned_group(
                pool,
                &worktree_id,
                &dead_terminal_id,
                &terminal_id,
            )
            .await
            .unwrap_or_else(|e| {
                log::warn!("[delivery] 手動引き継ぎに失敗: {}", e);
                (0, 0)
            });
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
            let _ = app.emit("event-inbox-changed", ());
            drive(app, pool, state).await;
        }
        DeliveryMsg::CollectDigest {
            terminal_id,
            reason,
            reply,
        } => {
            collect_digest(app, pool, state, &terminal_id, &reason, reply).await;
        }
        DeliveryMsg::EventQueued => {
            let _ = app.emit("event-inbox-changed", ());
            drive(app, pool, state).await;
        }
        DeliveryMsg::SpawnResult {
            request_id,
            session_id,
        } => {
            let worktree_id = state
                .inflight_spawn
                .iter()
                .find(|(_, (rid, _))| *rid == request_id)
                .map(|(wt, _)| wt.clone());
            if let Some(wt) = &worktree_id {
                state.inflight_spawn.remove(wt);
            }
            let Some(session_id) = session_id else {
                // サブウィンドウへ分離済みのワークツリーは session_id を回収できないため、
                // 成功しても None で返ってくる（引き継ぎはエージェント検出のポーリングに任せる）。
                // 失敗と区別できないので warn ではなく info に留める。
                log::info!(
                    "[delivery] spawn 応答に session_id が無い（失敗、またはサブウィンドウ経由の成功）request={} worktree={:?}",
                    request_id,
                    worktree_id
                );
                return;
            };
            // 新しいタブの terminal_id を解決して引き継ぐ。`claude` の起動は
            // pendingCommand で流し込まれるだけなので、AI エージェント検出（10 秒周期）
            // より先にここへ来る。押し込みは検出後の tick に回る。
            let terminal_id = app
                .state::<crate::pty_manager::PtyManager>()
                .list_sessions()
                .into_iter()
                .find(|s| s.session_id == session_id)
                .map(|s| s.terminal_id);
            match terminal_id {
                Some(tid) => {
                    let moved = rebind_for_terminal(app, pool, &tid).await;
                    if moved != (0, 0) {
                        // `Rebind` 経路と同じく引き継ぎ済みの印を付ける。付けないと、この直後に
                        // エージェント検出や購読系ツール呼び出しで再び `Rebind` が来たときに
                        // **2つ目のグループまで吸い上げ**、「1タブ1グループ」が崩れる。
                        state.claimed.insert(tid.clone());
                    }
                    log::info!(
                        "[delivery] spawn したタブへ引き継いだ session_id={} 購読={} 未読={}",
                        session_id,
                        moved.0,
                        moved.1
                    );
                    let _ = app.emit("event-inbox-changed", ());
                }
                None => log::warn!(
                    "[delivery] spawn 応答の session_id={} に対応する PTY が見つからない",
                    session_id
                ),
            }
        }
    }
}

/// 1タブにつき1グループまでの引き継ぎを、無駄打ちを抑えつつ試す。
///
/// - 既に1グループ引き継いだタブ（`claimed`）は対象外。`resolve_subscriber` や hook 経路は
///   呼び出しのたびに要求してくるので、抑止しないと1タブが全グループを吸い上げる
/// - 前回試して**対象が無かった**タブ（`rebind_probed`）も対象外。`Stop` は毎ターン発火
///   するため、これが無いと全 CC タブが毎ターン `list_orphaned_groups` を叩き続ける。
///   新しい引き継ぎ待ちが生まれる `Reconcile`（生存タブ変化時）で忘れる
async fn try_rebind_once(
    app: &AppHandle,
    pool: &SqlitePool,
    state: &mut WorkerState,
    terminal_id: &str,
) -> (u64, u64) {
    if state.claimed.contains(terminal_id) || state.rebind_probed.contains(terminal_id) {
        return (0, 0);
    }
    let moved = rebind_for_terminal(app, pool, terminal_id).await;
    if moved != (0, 0) {
        state.claimed.insert(terminal_id.to_string());
    } else {
        state.rebind_probed.insert(terminal_id.to_string());
    }
    moved
}

/// タブの cwd からワークツリーを引いて、そのワークツリーの引き継ぎ待ちを1グループ引き継ぐ。
async fn rebind_for_terminal(app: &AppHandle, pool: &SqlitePool, terminal_id: &str) -> (u64, u64) {
    let Some(worktree_id) = worktree_of_terminal(app, terminal_id) else {
        return (0, 0);
    };
    match event_db::rebind_next_orphaned_group(pool, &worktree_id, terminal_id).await {
        Ok((subs, inbox)) => {
            if subs > 0 || inbox > 0 {
                log::info!(
                    "[delivery] 引き継ぎ完了 worktree={} terminal={} 購読={} 未読={}",
                    worktree_id,
                    terminal_id,
                    subs,
                    inbox
                );
            }
            (subs, inbox)
        }
        Err(e) => {
            log::warn!("[delivery] rebind_next_orphaned_group failed: {}", e);
            (0, 0)
        }
    }
}

/// terminal_id → ワークツリー ID。生存セッションの cwd から前方一致＋最長一致で引く。
fn worktree_of_terminal(app: &AppHandle, terminal_id: &str) -> Option<String> {
    let settings = app.state::<SettingsManager>().get();
    let sessions = app.state::<crate::pty_manager::PtyManager>().list_sessions();
    let session = sessions.iter().find(|s| s.terminal_id == terminal_id)?;
    let cwd = session.cwd.as_deref()?;
    crate::mcp_server::resolve_worktree_by_cwd(&settings, cwd).map(|w| w.id.clone())
}

// ─── hook 経路への注入（#124） ────────────────────────────────────────────────

/// 同じターンへ二度注入しないか判定する（#124 §5.1）。
///
/// `prompt_id` が取れない場合は通す。`Stop` 由来の継続ターンは `stop_hook_active` で
/// 先に落ちているので、ここは CC の仕様変更に備えた二枚目の防波堤という位置づけ。
fn should_deliver_turn(last: Option<&str>, prompt_id: Option<&str>) -> bool {
    match prompt_id {
        Some(p) => last != Some(p),
        None => true,
    }
}

/// 自動承認が有効な宛先へ注入してよい未読だけに絞る（#120 §5.5）。
///
/// **`Stop` / `SessionStart` / `UserPromptSubmit` のすべての注入経路に掛けること。**
/// `autoApproval` は Claude Code の承認プロンプトを自動応答する機能なので、どの経路で
/// 入ったかにかかわらず、注入された自由文は人間の確認なしにツール実行へつながる。
/// 経路によって掛けたり掛けなかったりすると、`AUTO_APPROVAL_PUSHABLE_KINDS` が宣言する
/// 「自動承認が有効な宛先へは定型イベントのみ」という不変条件が破れる（#126）。
///
/// 判定は**行ごとにその行自身の `subscriber_worktree_id` で行う**。1タブが `cd` して
/// 別ワークツリーで再購読すると同じ `terminal_id` に異なるワークツリー宛の行が混ざるため、
/// 代表1件から解決すると**実際の宛先ではないワークツリーの設定で全件を判定してしまう**
/// （#131 が手動引き継ぎで塞いだのと同じ穴）。
fn filter_auto_approval(
    items: Vec<event_db::InboxItem>,
    settings: &AppSettings,
) -> Vec<event_db::InboxItem> {
    items
        .into_iter()
        .filter(|i| {
            let worktree = i
                .subscriber_worktree_id
                .as_deref()
                .and_then(|id| find_worktree(settings, id));
            auto_approval_allows(worktree, &i.kind)
        })
        .collect()
}

/// `Stop` 経路で注入してよい未読だけに絞る。
///
/// `Stop` の `additionalContext` は**会話を継続させる＝ターンを開始させる**ので、
/// PTY 押し込みと同じ危険度を持つ。自動承認の制限（上記）に加えて、
/// `delivery=passive` を除外する —— これは「押し込まないでほしい」という購読側の明示指定で、
/// 同じタブに他の戦略が混ざっていても巻き込まない（Phase 3 で踏んだバグと同じ轍を踏まない）。
///
/// `SessionStart` / `UserPromptSubmit` は人間の操作が起点でターンを勝手に始めないため、
/// `passive` も含めて回収する（購読ツールの説明どおり「どの戦略でもセッション開始時の
/// 回収は使える」）。
fn filter_for_turn_end(
    items: Vec<event_db::InboxItem>,
    settings: &AppSettings,
) -> Vec<event_db::InboxItem> {
    let items: Vec<_> = items
        .into_iter()
        .filter(|i| i.delivery != event_db::DELIVERY_PASSIVE)
        .collect();
    filter_auto_approval(items, settings)
}

/// 指定タブ宛の未読を hook の `additionalContext` 用テキストにまとめ、本文を出した分に
/// `delivered_at` を打つ。
///
/// 本文を列挙するのは **未配送のみ**。未 ack 全件を毎回再掲すると「再送はしない」方針
/// （#120 §5.2）に反する。ただしそれだけだと、注入が失われたとき（サイドカーの読み取り
/// タイムアウト等）にエージェントも人間も気づけない。そこで**未 ack の残存件数だけは
/// 毎回伝える**。件数を見れば `oretachi_poll_inbox` で取り直せる。
///
/// **本文を呼び出し元へ渡せたときだけ `delivered_at` を打つ。** 呼び出し元（axum ハンドラ）は
/// `INBOX_DIGEST_BUDGET_MS` のタイムアウトを持っており、ワーカーが詰まっている間に
/// 諦めると受信側が消える。先に打刻すると「配送済みだが誰も見ていない」本文が生まれ、
/// 再送しない方針（#120 §5.2）のせいで二度と出てこない。`push_pending` の
/// 「`pty.write` が成功してから打刻する」と同じ不変条件。
async fn collect_digest(
    app: &AppHandle,
    pool: &SqlitePool,
    state: &mut WorkerState,
    terminal_id: &str,
    reason: &DigestReason,
    reply: oneshot::Sender<Option<String>>,
) {
    if let DigestReason::TurnEnd { prompt_id } = reason {
        if !should_deliver_turn(
            state.last_turn.get(terminal_id).map(String::as_str),
            prompt_id.as_deref(),
        ) {
            log::debug!(
                "[delivery] このターンへは注入済みなので見送る terminal={} prompt_id={:?}",
                terminal_id,
                prompt_id
            );
            let _ = reply.send(None);
            return;
        }
    }
    // このタブが名乗り出たので、同じワークツリーの引き継ぎ待ち（アプリ再起動やタブ死亡で
    // 宙に浮いた購読と未読）を引き継ぐ。**回収より先に行う**必要がある。引き継ぎ前に
    // list_inbox すると、前回の terminal_id 宛のままの行が見えない。
    let moved = try_rebind_once(app, pool, state, terminal_id).await;
    if moved != (0, 0) {
        log::info!(
            "[delivery] 引き継ぎ待ちを回収した terminal={} 購読={} 未読={} 経路={}",
            terminal_id,
            moved.0,
            moved.1,
            reason.label()
        );
        let _ = app.emit("event-inbox-changed", ());
    }

    let items = match event_db::list_inbox(pool, terminal_id, event_db::InboxFilter::Undelivered)
        .await
    {
        Ok(items) => items,
        Err(e) => {
            log::warn!("[delivery] inbox 取得に失敗: {}", e);
            let _ = reply.send(None);
            return;
        }
    };
    // 自動承認のガードは**全経路**に掛ける。`Stop` はさらに `passive` も外す。
    let items = {
        let settings = app.state::<SettingsManager>().get();
        let before = items.len();
        let items = if reason.is_turn_end() {
            filter_for_turn_end(items, &settings)
        } else {
            filter_auto_approval(items, &settings)
        };
        if items.len() < before {
            log::debug!(
                "[delivery] {} 経路で {} 件を注入対象から外した（自動承認 / passive）terminal={}",
                reason.label(),
                before - items.len(),
                terminal_id
            );
        }
        items
    };

    // **件数だけの通知を毎回出さない。** `format_inbox_digest` は本文が0件でも未 ack の
    // 残件があれば「N 件残っています」を返す。これは「注入が失われても人間とエージェントが
    // 気づけるように」という Phase 1 の設計（#120 §5.2）だが、**発火頻度が高い経路では害になる**:
    //
    // - `Stop`: 残件を報告するためだけに会話が継続してしまう（ターンを開始させる経路なので致命的）
    // - `UserPromptSubmit`: ack されない限り**ユーザーの全プロンプトの先頭に ack 催促が付く**
    //
    // `SessionStart` は1セッション1回なので Phase 1 のまま件数を伝える（本来の目的どおり）。
    if items.is_empty() && !reason.reports_carryover_only() {
        let _ = reply.send(None);
        return;
    }

    // 配送済みだが未 ack のまま残っている件数（今回本文を出す分は差し引く）
    let carryover = match event_db::count_unacked(pool, terminal_id).await {
        Ok((unacked, _)) => (unacked - items.len() as i64).max(0),
        Err(e) => {
            log::warn!("[delivery] 未 ack 件数の取得に失敗: {}", e);
            0
        }
    };
    let Some((digest, used)) = event_db::format_inbox_digest(&items, carryover) else {
        let _ = reply.send(None);
        return;
    };

    // **先に渡してから打刻する。** 呼び出し元がタイムアウトで諦めていれば送信は失敗し、
    // その場合は打刻しない（未配送のまま残るので次の機会に再度出る）。逆順にすると
    // 「配送済みだが誰も見ていない」本文が生まれ、再送しない方針のせいで永久に失われる。
    if reply.send(Some(digest.clone())).is_err() {
        log::warn!(
            "[delivery] 注入本文の受け取り手が既に居ない（タイムアウト）ため打刻しない terminal={} 経路={}",
            terminal_id,
            reason.label()
        );
        return;
    }

    // 上限で載らなかった分は打刻しない（未配送のまま次の機会に回す）。全件打刻すると
    // 本文に出ていない未読が「配送済み」になり、再送しない方針のせいで二度と出ない。
    let ids: Vec<String> = items.iter().take(used).map(|i| i.id.clone()).collect();
    if let Err(e) = event_db::mark_delivered(pool, &ids, event_db::now_ms()).await {
        // 打刻に失敗しても本文は既に渡している。未配送のまま残るので次の機会に再度出る
        // （取りこぼすより一度重複するほうが安全）。
        log::warn!("[delivery] mark_delivered に失敗: {}", e);
    }
    if let DigestReason::TurnEnd { prompt_id } = reason {
        if let Some(p) = prompt_id {
            state.last_turn.insert(terminal_id.to_string(), p.clone());
        }
    }
    if !ids.is_empty() {
        log::info!(
            "[delivery] {} 件を {} 経由で注入した terminal={}",
            ids.len(),
            reason.label(),
            terminal_id
        );
        let _ = app.emit("event-inbox-changed", ());
    }
    // 配送ごとのトースト (`event-delivered`) は #137 で廃止した。知りたいのは個々の
    // 配送イベントではなく購読関係の現況で、それはカードのバッジが常時見せている。
    // 状態の変化そのものは直前の `event-inbox-changed` がフロントへ伝えている。
}

// ─── 配送の駆動 ───────────────────────────────────────────────────────────────

/// 押し込みを見送る理由（ログと将来の UI 表示用）。
enum PushDecision {
    Push,
    Skip(&'static str),
}

/// 1タブへの押し込み可否を決める。判定材料は `SessionInfo` に載っている値だけで、
/// ここでは I/O をしない（テストしやすさのためにも純粋に近い形に保つ）。
fn decide_push(
    session: &crate::pty_manager::SessionInfo,
    delivery: &str,
    now: i64,
) -> PushDecision {
    if session.exit_code.is_some() {
        return PushDecision::Skip("タブが終了済み");
    }
    if !session.is_ai_agent {
        // 素のシェルへ CR を撃つと任意のコマンドが走る。エージェントが居るときだけ押し込む。
        return PushDecision::Skip("AI エージェントが走っていない");
    }
    if delivery == event_db::DELIVERY_PASSIVE {
        return PushDecision::Skip("delivery=passive（エージェントが自分で取りに来る）");
    }
    // 出力が動いている間は誰にも押し込まない。人間が入力中の行にブラケットペーストを
    // 重ねるとその行を壊すため、エージェント種別や `interrupt` 指定より優先する
    // （`idle` の判定は最大10秒古いので、これが最後の砦になる）。
    if !session.output_quiescent {
        return PushDecision::Skip("出力が動いている（入力中の可能性）");
    }
    // `interrupt` は「走行中でも割り込んでよい」という購読側の明示的なオプトイン。
    if delivery == event_db::DELIVERY_INTERRUPT {
        return PushDecision::Push;
    }
    // 非 Claude Code エージェント（gemini / codex / cline）は `~/.claude/sessions/<pid>.json`
    // に相当するものが無く busy / idle を判定できない。Stop フック経路も存在しないので、
    // 押し込まないと永久に届かない。仕様として押し込む（#120 §5.7）。
    if session.agent_name.as_deref() != Some("claude") {
        return PushDecision::Push;
    }
    match session.agent_status.as_deref() {
        Some("idle") => {}
        Some(other) => {
            // busy はターン境界配送（#124 の Stop フック）の担当。次の tick で再評価する。
            let _ = other;
            return PushDecision::Skip("エージェントが走行中");
        }
        None => return PushDecision::Skip("エージェントの状態が不明"),
    }
    // 鮮度はファイル側の statusUpdatedAt ではなく**こちらがサンプルした時刻**で見る。
    // 長いターンは正当に古い busy を残すので、ファイル側の古さを「不明」と解釈すると
    // 押してはいけない場面で押すことになる。
    match session.agent_status_sampled_at {
        Some(at) if now - at <= STATUS_MAX_AGE_MS => {}
        _ => return PushDecision::Skip("状態のサンプルが古い"),
    }
    PushDecision::Push
}

/// 自動承認が有効な宛先へ押し込んでよい種別か（#120 §5.5）。
fn auto_approval_allows(worktree: Option<&WorktreeEntry>, kind: &str) -> bool {
    let auto = worktree.and_then(|w| w.auto_approval).unwrap_or(false);
    if !auto {
        return true;
    }
    AUTO_APPROVAL_PUSHABLE_KINDS.contains(&kind)
}

fn find_worktree<'a>(settings: &'a AppSettings, id: &str) -> Option<&'a WorktreeEntry> {
    settings.worktrees.iter().find(|w| w.id == id)
}

/// 自動 spawn したタブで起動するエージェントコマンド。
///
/// 解決順はフロントの `useTaskExecution.ts` と同じ（ワークグループ → 全体設定 →
/// `claudeCode`）。Claude Code の permission-mode も同じく既定は `plan` にする ——
/// 別のワークツリーからの通知で立ち上がるタブなので、いきなり書き込みを許すのは危険。
fn agent_command_for_worktree(settings: &AppSettings, worktree: &WorktreeEntry) -> String {
    use crate::ai_provider::AiAgentKind;
    let group = worktree
        .workgroup_id
        .as_deref()
        .and_then(|gid| settings.workgroups.iter().find(|g| g.id == gid));
    let kind = group
        .and_then(|g| g.task_add_agent.clone())
        .or_else(|| settings.ai_agent.as_ref().and_then(|a| a.task_add_agent.clone()))
        .or_else(|| settings.ai_agent.as_ref().and_then(|a| a.approval_agent.clone()))
        .unwrap_or(AiAgentKind::ClaudeCode);
    match kind {
        AiAgentKind::ClaudeCode => {
            let mode = match group.and_then(|g| g.claude_code_mode.as_deref()) {
                Some("manual") => "default",
                Some("acceptEdit") => "acceptEdits",
                Some("auto") => "auto",
                _ => "plan",
            };
            format!("claude --permission-mode {}", mode)
        }
        AiAgentKind::GeminiCli => "gemini".to_string(),
        AiAgentKind::CodexCli => "codex".to_string(),
        AiAgentKind::ClineCli => "cline".to_string(),
    }
}

async fn drive(app: &AppHandle, pool: &SqlitePool, state: &mut WorkerState) {
    let now = event_db::now_ms();
    push_pending(app, pool, state, now).await;
    spawn_for_closed_tabs(app, pool, state, now).await;
}

/// 生存タブへの PTY 押し込み。
async fn push_pending(app: &AppHandle, pool: &SqlitePool, state: &mut WorkerState, now: i64) {
    let items = match event_db::list_pushable(pool, now, event_db::PUSH_TTL_MS).await {
        Ok(items) => items,
        Err(e) => {
            log::warn!("[delivery] list_pushable failed: {}", e);
            return;
        }
    };
    if items.is_empty() {
        return;
    }

    // State を await 越しに持たないよう、必要な情報だけ先に取り出す。
    let sessions = app.state::<crate::pty_manager::PtyManager>().list_sessions();
    let settings = app.state::<SettingsManager>().get();

    let mut by_terminal: HashMap<String, Vec<event_db::InboxItem>> = HashMap::new();
    for item in items {
        by_terminal
            .entry(item.subscriber_terminal_id.clone())
            .or_default()
            .push(item);
    }

    for (terminal_id, items) in by_terminal {
        let Some(session) = sessions.iter().find(|s| s.terminal_id == terminal_id) else {
            // タブが消えている。次の Reconcile で orphaned に落ちる。
            continue;
        };
        if let Some(prev) = state.last_push.get(&terminal_id) {
            if prev.elapsed() < MIN_PUSH_INTERVAL {
                continue;
            }
        }
        // `passive` は「押し込まないでほしい」という購読側の明示指定なので、同じタブに
        // 他の戦略が混ざっていても巻き込んで押し込んではいけない。先に除外する。
        let items: Vec<_> = items
            .into_iter()
            .filter(|i| i.delivery != event_db::DELIVERY_PASSIVE)
            .collect();
        if items.is_empty() {
            continue;
        }
        // 自動承認が有効な宛先へは定型イベントしか押し込まない。判定は
        // `filter_for_turn_end` と同じく**行ごとにその行自身の `subscriber_worktree_id`**
        // で行う。代表1件から解決すると、1タブが `cd` して別ワークツリーで再購読した
        // ときに「実際の宛先ではないワークツリーの設定」で全件を通してしまう。
        //
        // **配送戦略を決める前に仕分ける。** 後回しにすると、保留されるはずの
        // `interrupt` な自由文メッセージが `strongest_delivery` を押し上げ、
        // `turn_end` しか指定していない定型イベントまで走行中のエージェントへ
        // 割り込ませてしまう（保留した行が押し込み判定に影響を残す）。
        let (allowed, blocked): (Vec<_>, Vec<_>) = items.into_iter().partition(|i| {
            let worktree = i
                .subscriber_worktree_id
                .as_deref()
                .and_then(|id| find_worktree(&settings, id));
            auto_approval_allows(worktree, &i.kind)
        });
        if !blocked.is_empty() {
            // 保留された分は未配送のまま残り、tick ごとに再評価される（＝毎回ここを通る）ので
            // debug に留める。人間への提示は UI の未読表示が担う。
            log::debug!(
                "[delivery] 自動承認が有効な宛先のため {} 件の押し込みを保留した（人間の確認が必要）terminal={}",
                blocked.len(),
                terminal_id
            );
        }
        if allowed.is_empty() {
            continue;
        }
        // 残りに interrupt が混ざっていれば走行中でも割り込む。
        let delivery = strongest_delivery(&allowed);
        let decision = decide_push(session, &delivery, now);
        if let PushDecision::Skip(reason) = decision {
            log::debug!(
                "[delivery] 押し込みを見送る terminal={} 件数={} 理由={}",
                terminal_id,
                allowed.len(),
                reason
            );
            continue;
        }
        // 長さ上限で載らなかった分は打刻しない（未配送のまま次回に回す）。
        let Some((text, used)) = event_db::format_inbox_push_text(&allowed) else {
            continue;
        };

        // ブラケットペーストで囲む。本文は `sanitize_for_pty` 済みなので終端シーケンスを
        // 埋め込んでペーストを脱出することはできない。
        //
        // **ペーストと Enter は別の write に分ける。** `ESC[200~…ESC[201~\r` を1回で書くと
        // Claude Code はペースト終端と同じ読み取りチャンクに来た CR をペーストの一部として
        // 扱い、本文が入力欄に残ったままターンが始まらない（実機で確認）。CR を独立した
        // write にし、間に猶予を入れると確実に送信される。
        let paste = format!("\x1b[200~{}\x1b[201~", text);
        // **write が成功してから打刻する**。キュー満杯などで捨てられた押し込みを
        // 配送済みにすると、二度と本文が出ないまま失われる。
        if let Err(e) = app
            .state::<crate::pty_manager::PtyManager>()
            .write(session.session_id, paste.into_bytes())
        {
            log::warn!(
                "[delivery] PTY への押し込みに失敗 terminal={} session={}: {}",
                terminal_id,
                session.session_id,
                e
            );
            continue;
        }
        // ペーストが通った時点でレート制限を進める。この後の Enter が失敗しても本文は
        // 既に入力欄にあるので、次の tick で同じ本文を重ねて貼らないようにする。
        state.last_push.insert(terminal_id.clone(), Instant::now());
        tokio::time::sleep(SUBMIT_DELAY).await;
        if let Err(e) = app
            .state::<crate::pty_manager::PtyManager>()
            .write(session.session_id, b"\r".to_vec())
        {
            // 本文は入力欄に残っているので人間が Enter を押せば送れる。打刻はしない
            // （未配送のままにして次の tick / SessionStart 回収で拾えるようにする）。
            log::warn!(
                "[delivery] 押し込み後の Enter に失敗 terminal={} session={}: {}",
                terminal_id,
                session.session_id,
                e
            );
            continue;
        }
        let ids: Vec<String> = allowed.iter().take(used).map(|i| i.id.clone()).collect();
        if let Err(e) = event_db::mark_delivered(pool, &ids, now).await {
            log::warn!("[delivery] mark_delivered failed: {}", e);
        }
        log::info!(
            "[delivery] 待機中のエージェントへ {} 件を押し込んだ terminal={} session={} agent={:?} delivery={}",
            ids.len(),
            terminal_id,
            session.session_id,
            session.agent_name,
            delivery
        );
        // 配送トースト (`event-delivered`) は #137 で廃止した。代わりに未配送件数が
        // 減ったことを伝える（この経路には `event-inbox-changed` が無く、購読パネルと
        // カードのバッジが押し込み後も古い件数のまま残っていた）。
        let _ = app.emit("event-inbox-changed", ());
    }
}

fn strongest_delivery(items: &[event_db::InboxItem]) -> String {
    if items.iter().any(|i| i.delivery == event_db::DELIVERY_INTERRUPT) {
        return event_db::DELIVERY_INTERRUPT.to_string();
    }
    if items.iter().any(|i| i.delivery == event_db::DELIVERY_TURN_END) {
        return event_db::DELIVERY_TURN_END.to_string();
    }
    event_db::DELIVERY_PASSIVE.to_string()
}

/// AI タブが居ないワークツリーへ、`spawn_if_closed` が立っていれば新しいタブを立てる。
///
/// spawn は**明示オプトインのみ**（#120 §5.4）。加えて:
/// - ワークツリーごとの単一フライト（応答があるか 60 秒経つまで次を出さない）
/// - 生存ターミナル数の上限（webview ハングの手前で止める）
/// - 自動承認が有効な宛先へは定型イベントのみ
///
/// フロントは spawn の成否にかかわらず `event_spawn_result` を返す。返らない場合も
/// 60 秒で単一フライトが解放されるだけで、tick ごとに撃ち続けることはない。
async fn spawn_for_closed_tabs(
    app: &AppHandle,
    pool: &SqlitePool,
    state: &mut WorkerState,
    now: i64,
) {
    let rows = match event_db::list_spawn_candidates(pool, now, event_db::PUSH_TTL_MS).await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[delivery] list_spawn_candidates failed: {}", e);
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    // 種別ごとの内訳のままワークツリー単位にまとめ直す。**件数だけに潰さない**のは、
    // 自動承認が有効な宛先で「押し込めない種別の未読」を根拠に spawn してしまわないため。
    let mut candidates: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    for (worktree_id, kind, count) in rows {
        candidates.entry(worktree_id).or_default().push((kind, count));
    }
    let sessions = app.state::<crate::pty_manager::PtyManager>().list_sessions();
    // 同じ tick で複数ワークツリーが spawn 候補になったときに上限を素通りしないよう、
    // 発行した要求ぶんを足し込みながら判定する（`live` を1回だけ数えて全件に使うと、
    // 候補が5件あれば一気に5タブ増えて webview ハングの領域に入る）。
    let mut projected_live = sessions.iter().filter(|s| s.exit_code.is_none()).count()
        + state.inflight_spawn.len();
    let settings = app.state::<SettingsManager>().get();

    for (worktree_id, by_kind) in candidates {
        if state.inflight_spawn.contains_key(&worktree_id) {
            continue;
        }
        if let Some(prev) = state.last_spawn.get(&worktree_id) {
            if prev.elapsed() < SPAWN_COOLDOWN {
                continue;
            }
        }
        let Some(worktree) = find_worktree(&settings, &worktree_id) else {
            // 購読者ワークツリー自体が消えている。`purge_subscriber_worktree` の対象。
            continue;
        };
        // そのワークツリーに既に AI タブがあるなら spawn しない（引き継ぎで届く）。
        let has_agent = sessions.iter().any(|s| {
            s.exit_code.is_none()
                && s.is_ai_agent
                && s.cwd
                    .as_deref()
                    .and_then(|c| crate::mcp_server::resolve_worktree_by_cwd(&settings, c))
                    .map(|w| w.id == worktree_id)
                    .unwrap_or(false)
        });
        if has_agent {
            continue;
        }
        // 自動承認が有効な宛先では、押し込みが許される種別の未読だけを spawn の根拠にする。
        // 種別を無視して合計件数で判断すると、自由文の `worktree.message` が
        // 「新しいタブを立てさせる」ところまでは通ってしまう（#126）。
        let pending: i64 = by_kind
            .iter()
            .filter(|(kind, _)| auto_approval_allows(Some(worktree), kind))
            .map(|(_, count)| *count)
            .sum();
        if pending == 0 {
            log::info!(
                "[delivery] 自動承認が有効な宛先のため spawn を見送った worktree={} 保留種別={:?}",
                worktree.name,
                by_kind.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>()
            );
            continue;
        }
        if projected_live >= SPAWN_MAX_LIVE_SESSIONS {
            log::warn!(
                "[delivery] ターミナルが {} 個あるため自動 spawn を拒否した（上限 {}）worktree={} 未読={}",
                projected_live,
                SPAWN_MAX_LIVE_SESSIONS,
                worktree.name,
                pending
            );
            let _ = app.emit(
                "event-spawn-rejected",
                serde_json::json!({
                    "worktreeId": worktree_id,
                    "worktreeName": worktree.name,
                    "liveSessions": projected_live,
                    "limit": SPAWN_MAX_LIVE_SESSIONS,
                    "pending": pending,
                }),
            );
            continue;
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let command = agent_command_for_worktree(&settings, worktree);
        if app
            .emit(
                "event-spawn-terminal",
                serde_json::json!({
                    "requestId": request_id,
                    "worktreeId": worktree_id,
                    "worktreeName": worktree.name,
                    "command": command,
                    "pending": pending,
                }),
            )
            .is_err()
        {
            continue;
        }
        log::info!(
            "[delivery] 未読 {} 件のため新しいタブを要求した worktree={} request={}",
            pending,
            worktree.name,
            request_id
        );
        projected_live += 1;
        state.last_spawn.insert(worktree_id.clone(), Instant::now());
        state
            .inflight_spawn
            .insert(worktree_id, (request_id, Instant::now()));
    }
}

// ─── UI 向け Tauri コマンド（#120 §7） ────────────────────────────────────────

use crate::mcp_server::describe_target;

/// 購読一覧の1行。DB の生の行に、人間が読むための解決済みの名前と生存状況を足したもの。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionView {
    pub id: String,
    pub subscriber_terminal_id: String,
    pub subscriber_worktree_id: Option<String>,
    pub subscriber_worktree_name: Option<String>,
    /// 購読者タブが生存していれば PTY セッション ID（UI のタブ突合用）
    pub subscriber_session_id: Option<u32>,
    /// `claude` / `gemini` / `codex` / `cline`。非 CC は Stop フック経路が存在しない
    pub agent_name: Option<String>,
    /// DB に入っている生の `target`。ワイルドカードの場合は `*` / `workgroup:<id>` /
    /// `repo:<name>` がそのまま入る（#126）
    pub target_worktree_id: String,
    /// 厳密一致 target のワークツリー名。クローズ済み / ワイルドカードでは None
    pub target_worktree_name: Option<String>,
    /// `worktree` | `all` | `workgroup` | `repo`（#126）。UI はこれを見て
    /// 「クローズ済み」バッジを出すかどうかを決める。**種別を渡さずに
    /// `target_worktree_name` が None であることだけで判断すると、ワイルドカード購読が
    /// すべて「クローズ済み」と誤表示される。**
    pub target_kind: String,
    /// 人間向けの表示名。ワークグループは表示名、リポジトリは名前、`*` は None
    /// （UI 側でローカライズした文言を出す）
    pub target_label: Option<String>,
    pub event_kinds: Vec<String>,
    pub delivery: String,
    pub spawn_if_closed: bool,
    pub state: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub orphaned_at: Option<i64>,
    pub unacked: i64,
    pub undelivered: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanedGroupView {
    pub terminal_id: String,
    pub worktree_id: String,
    pub worktree_name: Option<String>,
    pub orphaned_at: i64,
    pub subscriptions: i64,
    pub pending: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalIdEntry {
    pub session_id: u32,
    pub terminal_id: String,
    pub agent_name: Option<String>,
    pub worktree_id: Option<String>,
    pub worktree_name: Option<String>,
    pub is_ai_agent: bool,
    pub unacked: i64,
}

fn pool_of(app: &AppHandle) -> Result<SqlitePool, String> {
    app.try_state::<event_db::EventPool>()
        .map(|p| p.0.clone())
        .ok_or_else(|| "イベント DB が初期化されていません".to_string())
}

#[tauri::command]
pub async fn event_list_subscriptions(
    app_handle: AppHandle,
) -> Result<Vec<SubscriptionView>, String> {
    let pool = pool_of(&app_handle)?;
    let now = event_db::now_ms();
    let rows = event_db::list_all_subscriptions(&pool, now).await?;
    let counts = event_db::count_unacked_by_terminal(&pool).await?;
    // State を await 越しに保持しない（他の DB 経路と同じ流儀）
    let sessions = app_handle
        .state::<crate::pty_manager::PtyManager>()
        .list_sessions();
    let settings = app_handle.state::<SettingsManager>().get();
    let name_of = |id: &str| settings.worktrees.iter().find(|w| w.id == id).map(|w| w.name.clone());

    Ok(rows
        .into_iter()
        .map(|r| {
            let session = sessions
                .iter()
                .find(|s| s.terminal_id == r.subscriber_terminal_id && s.exit_code.is_none());
            let (unacked, undelivered) = counts
                .iter()
                .find(|(t, _, _)| *t == r.subscriber_terminal_id)
                .map(|(_, a, b)| (*a, *b))
                .unwrap_or((0, 0));
            let (target_kind, target_label) = describe_target(&settings, &r.target);
            SubscriptionView {
                subscriber_worktree_name: r.subscriber_worktree_id.as_deref().and_then(name_of),
                subscriber_session_id: session.map(|s| s.session_id),
                agent_name: session.and_then(|s| s.agent_name.clone()),
                target_worktree_name: name_of(&r.target),
                target_kind,
                target_label,
                target_worktree_id: r.target,
                event_kinds: serde_json::from_str(&r.event_kinds).unwrap_or_default(),
                delivery: r.delivery,
                spawn_if_closed: r.spawn_if_closed != 0,
                state: r.state,
                created_at: r.created_at,
                expires_at: r.expires_at,
                orphaned_at: r.orphaned_at,
                unacked,
                undelivered,
                id: r.id,
                subscriber_terminal_id: r.subscriber_terminal_id,
                subscriber_worktree_id: r.subscriber_worktree_id,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn event_list_orphaned_groups(
    app_handle: AppHandle,
) -> Result<Vec<OrphanedGroupView>, String> {
    let pool = pool_of(&app_handle)?;
    let groups = event_db::list_all_orphaned_groups(&pool).await?;
    let settings = app_handle.state::<SettingsManager>().get();
    Ok(groups
        .into_iter()
        .map(|g| OrphanedGroupView {
            worktree_name: settings
                .worktrees
                .iter()
                .find(|w| w.id == g.worktree_id)
                .map(|w| w.name.clone()),
            terminal_id: g.terminal_id,
            worktree_id: g.worktree_id,
            orphaned_at: g.orphaned_at,
            subscriptions: g.subscriptions,
            pending: g.pending,
        })
        .collect())
}

/// 引き継ぎ待ちグループを、人間が選んだ生存タブへ手動で引き継ぐ。
///
/// 自動の引き継ぎは「新しい AI タブ1つにつき1グループ」なので、前回より少ないタブしか
/// 開かなかった場合にグループが残る。その取り残しを人間が解消するための経路。
#[tauri::command]
pub async fn event_rebind_group(
    app_handle: AppHandle,
    worktree_id: String,
    dead_terminal_id: String,
    session_id: u32,
) -> Result<(u64, u64), String> {
    let session = app_handle
        .state::<crate::pty_manager::PtyManager>()
        .list_sessions()
        .into_iter()
        .find(|s| s.session_id == session_id && s.exit_code.is_none())
        .ok_or_else(|| format!("session_id {} に対応する生存ターミナルがありません", session_id))?;
    // 引き継ぎ先が本当にそのワークツリーのタブか、バックエンド側でも確かめる。
    // `subscriber_worktree_id` は引き継ぎでも変わらないので、別ワークツリーのタブへ渡すと
    // 以後その行の押し込みが**実際の宛先ではない**ワークツリーの autoApproval 設定で
    // 判定されることになる（フロントの候補リストが古いだけで起きうる）。
    {
        let settings = app_handle.state::<SettingsManager>().get();
        let actual = session
            .cwd
            .as_deref()
            .and_then(|c| crate::mcp_server::resolve_worktree_by_cwd(&settings, c))
            .map(|w| w.id.clone());
        if actual.as_deref() != Some(worktree_id.as_str()) {
            return Err(format!(
                "session_id {} はワークツリー {} のターミナルではありません（実際: {:?}）",
                session_id, worktree_id, actual
            ));
        }
    }
    let terminal_id = session.terminal_id;
    let (tx, rx) = oneshot::channel();
    {
        let h = handle(&app_handle).ok_or_else(|| "配送ワーカーが起動していません".to_string())?;
        h.tx.try_send(DeliveryMsg::RebindManual {
            worktree_id,
            dead_terminal_id,
            terminal_id,
            reply: Some(tx),
        })
        .map_err(|_| "配送ワーカーが混雑しています。少し待って再試行してください".to_string())?;
    }
    rx.await.map_err(|_| "配送ワーカーが応答しませんでした".to_string())
}

#[tauri::command]
pub async fn event_unsubscribe(app_handle: AppHandle, subscription_id: String) -> Result<u64, String> {
    let pool = pool_of(&app_handle)?;
    let deleted = event_db::delete_subscription_by_id(&pool, &subscription_id).await?;
    let _ = app_handle.emit("event-inbox-changed", ());
    Ok(deleted)
}

/// UI からの既読化。エージェントが ack しないまま放置した分を人間が畳めるようにする。
/// 対象は指定タブの未 ack 全件（人間は一覧で件数しか見ていないので ID を持っていない）。
#[tauri::command]
pub async fn event_ack_all(app_handle: AppHandle, terminal_id: String) -> Result<u64, String> {
    let pool = pool_of(&app_handle)?;
    let acked = event_db::ack_all(&pool, &terminal_id, event_db::now_ms()).await?;
    let _ = app_handle.emit("event-inbox-changed", ());
    Ok(acked)
}

/// タブごとの未読件数（UI のバッジ用）。`session_id` はフロントが握っている ID で、
/// `terminal_id` はフロントからは見えないためここで突き合わせて返す。
#[tauri::command]
pub async fn event_terminal_unread(app_handle: AppHandle) -> Result<Vec<TerminalIdEntry>, String> {
    let pool = pool_of(&app_handle)?;
    let counts = event_db::count_unacked_by_terminal(&pool).await?;
    let sessions = app_handle
        .state::<crate::pty_manager::PtyManager>()
        .list_sessions();
    let settings = app_handle.state::<SettingsManager>().get();
    Ok(sessions
        .into_iter()
        .filter(|s| s.exit_code.is_none())
        .map(|s| {
            let worktree = s
                .cwd
                .as_deref()
                .and_then(|c| crate::mcp_server::resolve_worktree_by_cwd(&settings, c));
            TerminalIdEntry {
                unacked: counts
                    .iter()
                    .find(|(t, _, _)| *t == s.terminal_id)
                    .map(|(_, a, _)| *a)
                    .unwrap_or(0),
                worktree_id: worktree.map(|w| w.id.clone()),
                worktree_name: worktree.map(|w| w.name.clone()),
                session_id: s.session_id,
                terminal_id: s.terminal_id,
                agent_name: s.agent_name,
                is_ai_agent: s.is_ai_agent,
            }
        })
        .collect())
}

/// フロントからの spawn 完了応答（#125）。
///
/// **成否にかかわらず必ず呼ぶこと。** 呼ばれないと単一フライトが 60 秒間解放されないが、
/// 逆に言えば「応答が来ないから撃ち直す」という無限ループにはならない。
#[tauri::command]
pub fn event_spawn_result(app_handle: AppHandle, request_id: String, session_id: Option<u32>) {
    if let Some(h) = handle(&app_handle) {
        h.try_send(DeliveryMsg::SpawnResult {
            request_id,
            session_id,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty_manager::SessionInfo;

    fn session(agent: Option<&str>, status: Option<&str>, quiescent: bool) -> SessionInfo {
        SessionInfo {
            session_id: 1,
            terminal_id: "term-a".to_string(),
            cwd: Some("X:/wt".to_string()),
            is_ai_agent: agent.is_some(),
            agent_name: agent.map(str::to_string),
            agent_session_id: None,
            agent_status: status.map(str::to_string),
            agent_status_sampled_at: Some(1_000_000),
            output_quiescent: quiescent,
            exit_code: None,
            last_command_exit_code: None,
        }
    }

    fn is_push(d: PushDecision) -> bool {
        matches!(d, PushDecision::Push)
    }

    #[test]
    fn test_push_when_claude_idle_and_quiescent() {
        let s = session(Some("claude"), Some("idle"), true);
        assert!(is_push(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000)));
    }

    #[test]
    fn test_skip_when_claude_busy() {
        let s = session(Some("claude"), Some("busy"), true);
        assert!(!is_push(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000)));
    }

    /// `interrupt` は「走行中でも割り込む」という購読側の明示オプトイン。
    #[test]
    fn test_interrupt_pushes_even_when_busy() {
        let s = session(Some("claude"), Some("busy"), true);
        assert!(is_push(decide_push(&s, event_db::DELIVERY_INTERRUPT, 1_000_000)));
    }

    /// 出力が動いている間は誰にも押し込まない。人間が入力中の行を壊さないための最後の砦で、
    /// エージェント種別や `interrupt` 指定より優先する。
    #[test]
    fn test_output_quiescence_overrides_everything() {
        for (agent, status, delivery) in [
            (Some("claude"), Some("idle"), event_db::DELIVERY_TURN_END),
            (Some("claude"), Some("busy"), event_db::DELIVERY_INTERRUPT),
            (Some("codex"), None, event_db::DELIVERY_TURN_END),
        ] {
            let s = session(agent, status, false);
            assert!(
                !is_push(decide_push(&s, delivery, 1_000_000)),
                "agent={:?} delivery={} は出力継続中なら押し込まない",
                agent,
                delivery
            );
        }
    }

    #[test]
    fn test_skip_when_status_sample_is_stale() {
        let s = session(Some("claude"), Some("idle"), true);
        // サンプルから 20 秒以上経過
        assert!(!is_push(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000 + 20_001)));
    }

    #[test]
    fn test_skip_when_output_is_moving() {
        let s = session(Some("claude"), Some("idle"), false);
        assert!(!is_push(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000)));
    }

    /// 非 CC エージェントは status を取得できないので、押し込まないと永久に届かない。
    #[test]
    fn test_push_for_non_claude_agent_without_status() {
        for name in ["gemini", "codex", "cline"] {
            let s = session(Some(name), None, true);
            assert!(
                is_push(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000)),
                "{} は押し込み対象",
                name
            );
        }
    }

    /// 素のシェルへ CR を撃つと任意のコマンドが走ってしまう。
    #[test]
    fn test_skip_when_no_agent_running() {
        let s = session(None, None, true);
        assert!(!is_push(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000)));
    }

    #[test]
    fn test_skip_passive() {
        let s = session(Some("claude"), Some("idle"), true);
        assert!(!is_push(decide_push(&s, event_db::DELIVERY_PASSIVE, 1_000_000)));
    }

    #[test]
    fn test_skip_exited_session() {
        let mut s = session(Some("claude"), Some("idle"), true);
        s.exit_code = Some(0);
        assert!(!is_push(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000)));
    }

    fn worktree(auto: Option<bool>) -> WorktreeEntry {
        WorktreeEntry {
            id: "wt-1".to_string(),
            name: "wt".to_string(),
            repository_id: "r".to_string(),
            repository_name: "r".to_string(),
            path: "X:/wt".to_string(),
            branch_name: "b".to_string(),
            hotkey_char: None,
            auto_approval: auto,
            auto_approval_prompt: None,
            description: None,
            description_open: None,
            workgroup_id: None,
            is_home: false,
            is_repository: false,
        }
    }

    #[test]
    fn test_auto_approval_allows_canned_kinds_only() {
        let on = worktree(Some(true));
        // oretachi 自身が定型 JSON で組み立てるイベントは許可
        assert!(auto_approval_allows(Some(&on), event_db::KIND_WORKTREE_CLOSED));
        assert!(auto_approval_allows(Some(&on), event_db::KIND_WORKTREE_CREATED));
        // エージェントが本文を書ける自由文（#126）は自動承認宛には押し込まない
        assert!(!auto_approval_allows(Some(&on), event_db::KIND_WORKTREE_MESSAGE));
        // 知らない種別も default-deny
        assert!(!auto_approval_allows(Some(&on), "worktree.unknown"));
    }

    #[test]
    fn test_auto_approval_off_allows_everything() {
        let off = worktree(Some(false));
        assert!(auto_approval_allows(Some(&off), "worktree.message"));
        let unset = worktree(None);
        assert!(auto_approval_allows(Some(&unset), "worktree.message"));
        assert!(auto_approval_allows(None, "worktree.message"));
    }

    /// `push_pending` / `filter_for_turn_end` の仕分けと同じ形。自動承認が有効な宛先では、
    /// 同じタブに定型と自由文が混ざっていても**自由文だけが保留**され、定型は通る（#126）。
    #[test]
    fn test_auto_approval_partition_blocks_only_free_form() {
        let on = worktree(Some(true));
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CREATED),
            item("c", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_MESSAGE),
        ];
        let (allowed, blocked): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|i| auto_approval_allows(Some(&on), &i.kind));
        assert_eq!(ids(&allowed), vec!["a", "b"]);
        assert_eq!(ids(&blocked), vec!["c"]);
    }

    /// 保留された行は配送戦略の決定に影響してはいけない。`interrupt` な自由文が
    /// 混ざっているだけで `turn_end` の定型イベントが走行中のエージェントへ
    /// 割り込む、という取り違えを固定する（`push_pending` は仕分け後に
    /// `strongest_delivery` を取る）。
    #[test]
    fn test_blocked_items_do_not_escalate_delivery() {
        let on = worktree(Some(true));
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_INTERRUPT, event_db::KIND_WORKTREE_MESSAGE),
        ];
        // 仕分け前の全体では interrupt に見える
        assert_eq!(strongest_delivery(&items), event_db::DELIVERY_INTERRUPT);
        let allowed: Vec<_> = items
            .into_iter()
            .filter(|i| auto_approval_allows(Some(&on), &i.kind))
            .collect();
        // 仕分け後は turn_end。走行中のエージェントへは押し込まれない
        assert_eq!(strongest_delivery(&allowed), event_db::DELIVERY_TURN_END);
        let busy = session(Some("claude"), Some("busy"), true);
        assert!(!is_push(decide_push(&busy, &strongest_delivery(&allowed), 1_000_000)));
    }

    /// `spawn_for_closed_tabs` の判定と同じ形。自由文の未読しか無い宛先では、自動承認が
    /// 有効なら spawn しない（押し込めない未読を根拠にタブを立てない）。
    #[test]
    fn test_spawn_pending_count_ignores_blocked_kinds() {
        let on = worktree(Some(true));
        let off = worktree(Some(false));
        let by_kind = vec![
            (event_db::KIND_WORKTREE_MESSAGE.to_string(), 3i64),
            (event_db::KIND_WORKTREE_CLOSED.to_string(), 2i64),
        ];
        let count = |wt: &WorktreeEntry| -> i64 {
            by_kind
                .iter()
                .filter(|(kind, _)| auto_approval_allows(Some(wt), kind))
                .map(|(_, c)| *c)
                .sum()
        };
        assert_eq!(count(&on), 2, "自動承認 ON では自由文を数えない");
        assert_eq!(count(&off), 5, "自動承認 OFF なら従来どおり全部数える");

        // 自由文だけの宛先は spawn 対象にならない（pending == 0）
        let only_message = vec![(event_db::KIND_WORKTREE_MESSAGE.to_string(), 3i64)];
        let pending: i64 = only_message
            .iter()
            .filter(|(kind, _)| auto_approval_allows(Some(&on), kind))
            .map(|(_, c)| *c)
            .sum();
        assert_eq!(pending, 0);
    }

    // ─── ターン境界配送（#124） ──────────────────────────────────────────────

    fn item(id: &str, delivery: &str, kind: &str) -> event_db::InboxItem {
        event_db::InboxItem {
            id: id.to_string(),
            subscriber_terminal_id: "t-1".to_string(),
            subscriber_worktree_id: Some("wt-1".to_string()),
            event_id: format!("ev-{}", id),
            state: event_db::STATE_PENDING.to_string(),
            created_at: 1_000_000,
            delivered_at: None,
            acked_at: None,
            orphaned_at: None,
            delivery: delivery.to_string(),
            spawn_if_closed: 0,
            kind: kind.to_string(),
            body: "{}".to_string(),
            source_worktree_id: "wt-src".to_string(),
            actor: None,
        }
    }

    fn ids(items: &[event_db::InboxItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    /// `Stop` の additionalContext は会話を継続させるので、同じ prompt_id へ二度返すと
    /// 永久に回る。実測（#121）では 1 発話に対する 9 回の発火すべてで prompt_id が同一。
    #[test]
    fn test_should_deliver_turn_once_per_prompt_id() {
        assert!(should_deliver_turn(None, Some("p-1")));
        assert!(!should_deliver_turn(Some("p-1"), Some("p-1")));
        // 次のユーザー発話は別の prompt_id なので通す
        assert!(should_deliver_turn(Some("p-1"), Some("p-2")));
    }

    /// prompt_id が取れないときは通す。継続ターンは stop_hook_active が先に落としており、
    /// ここで止めると「一度も配送できない」に倒れてしまう。
    #[test]
    fn test_should_deliver_turn_without_prompt_id() {
        assert!(should_deliver_turn(None, None));
        assert!(should_deliver_turn(Some("p-1"), None));
    }

    /// `passive` は「押し込まないでほしい」の明示指定。Stop の注入はターンを開始させる
    /// ので押し込みと同じ扱いにする（同じタブの他戦略に巻き込まない）。
    /// `worktree()` が返す wt-1 を1件だけ持つ settings。
    fn settings_with(auto: Option<bool>) -> AppSettings {
        let mut s = AppSettings::default();
        s.worktrees = vec![worktree(auto)];
        s
    }

    #[test]
    fn test_filter_for_turn_end_excludes_passive() {
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_PASSIVE, event_db::KIND_WORKTREE_CLOSED),
            item("c", event_db::DELIVERY_INTERRUPT, event_db::KIND_WORKTREE_CLOSED),
        ];
        // interrupt は「走行中でも割り込んでよい」の明示指定なので、ターン境界でも当然通す
        assert_eq!(
            ids(&filter_for_turn_end(items, &settings_with(None))),
            vec!["a", "c"]
        );
    }

    /// 自動承認が有効な宛先は、注入された内容を人間の確認なしに実行する（#120 §5.5）。
    /// #126 で自由文 (`worktree.message`) が入った瞬間に default-deny になるのが目的。
    #[test]
    fn test_filter_for_turn_end_respects_auto_approval() {
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_TURN_END, "worktree.message"),
        ];
        assert_eq!(
            ids(&filter_for_turn_end(items.clone(), &settings_with(Some(true)))),
            vec!["a"]
        );
        assert_eq!(
            ids(&filter_for_turn_end(items, &settings_with(Some(false)))),
            vec!["a", "b"]
        );
    }

    /// 1タブが `cd` して別ワークツリーで再購読すると、同じ terminal_id の inbox に
    /// 異なる `subscriber_worktree_id` の行が混ざる。代表1件から自動承認を判定すると
    /// **実際の宛先ではないワークツリーの設定で全件を通してしまう**ので、行ごとに引く。
    #[test]
    fn test_filter_for_turn_end_judges_each_row_by_its_own_worktree() {
        let mut auto_on = worktree(Some(true));
        auto_on.id = "wt-auto".to_string();
        let mut auto_off = worktree(Some(false));
        auto_off.id = "wt-manual".to_string();
        let mut settings = AppSettings::default();
        settings.worktrees = vec![auto_on, auto_off];

        // 1件目は自動承認 OFF のワークツリー宛（＝代表にすると全件が通ってしまう）
        let mut a = item("a", event_db::DELIVERY_TURN_END, "worktree.message");
        a.subscriber_worktree_id = Some("wt-manual".to_string());
        let mut b = item("b", event_db::DELIVERY_TURN_END, "worktree.message");
        b.subscriber_worktree_id = Some("wt-auto".to_string());

        assert_eq!(ids(&filter_for_turn_end(vec![a, b], &settings)), vec!["a"]);
    }

    /// **自動承認のガードは全注入経路に掛かること。** `Stop` だけに掛けていると、
    /// `SessionStart` / `UserPromptSubmit` の additionalContext から自由文が素通りし、
    /// 自動承認が有効な宛先で人間の確認なしにツール実行へつながる（#126）。
    /// `passive` の除外だけが `Stop` 固有。
    #[test]
    fn test_auto_approval_filter_applies_to_all_injection_paths() {
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_MESSAGE),
            item("c", event_db::DELIVERY_PASSIVE, event_db::KIND_WORKTREE_CLOSED),
        ];
        let auto_on = settings_with(Some(true));
        // SessionStart / UserPromptSubmit 経路: 自由文は落とし、passive は回収する
        assert_eq!(
            ids(&filter_auto_approval(items.clone(), &auto_on)),
            vec!["a", "c"]
        );
        // Stop 経路: 自由文に加えて passive も落とす
        assert_eq!(ids(&filter_for_turn_end(items.clone(), &auto_on)), vec!["a"]);
        // 自動承認 OFF なら自由文も通る（passive の扱いだけ経路で違う）
        let auto_off = settings_with(Some(false));
        assert_eq!(
            ids(&filter_auto_approval(items.clone(), &auto_off)),
            vec!["a", "b", "c"]
        );
        assert_eq!(ids(&filter_for_turn_end(items, &auto_off)), vec!["a", "b"]);
    }

    /// 宛先ワークツリーが解決できない行（設定から消えた等）は、自動承認の判定材料が
    /// 無いので `auto_approval_allows(None, _)` の既定（許可）に落ちる。1件目が
    /// これだったせいで他の行まで巻き込まれないことを固定する。
    #[test]
    fn test_filter_for_turn_end_unresolvable_row_does_not_leak_to_others() {
        let mut a = item("a", event_db::DELIVERY_TURN_END, "worktree.message");
        a.subscriber_worktree_id = Some("wt-gone".to_string());
        let b = item("b", event_db::DELIVERY_TURN_END, "worktree.message"); // wt-1 = 自動承認 ON
        assert_eq!(
            ids(&filter_for_turn_end(vec![a, b], &settings_with(Some(true)))),
            vec!["a"]
        );
    }

    #[test]
    fn test_digest_reason_labels() {
        assert_eq!(DigestReason::SessionStart.label(), "session");
        assert_eq!(DigestReason::PromptSubmit.label(), "prompt");
        assert_eq!(DigestReason::TurnEnd { prompt_id: None }.label(), "stop");
        assert!(DigestReason::TurnEnd { prompt_id: None }.is_turn_end());
        assert!(!DigestReason::SessionStart.is_turn_end());
        assert!(!DigestReason::PromptSubmit.is_turn_end());
    }

    /// 本文0件のときに「未 ack が N 件残っています」だけを注入してよいのは
    /// `SessionStart` だけ。`Stop` は残件報告のためだけに会話を継続させてしまい、
    /// `UserPromptSubmit` は ack するまでユーザーの全プロンプトに催促が付きまとう。
    #[test]
    fn test_only_session_start_reports_carryover_only() {
        assert!(DigestReason::SessionStart.reports_carryover_only());
        assert!(!DigestReason::PromptSubmit.reports_carryover_only());
        assert!(!DigestReason::TurnEnd { prompt_id: Some("p".into()) }.reports_carryover_only());
    }
}
