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
/// 実行される**。oretachi が定型文で組み立てるイベント（`worktree.closed`）だけを許可し、
/// 自由文（`worktree.message`, #126）は押し込まず inbox に残して人間の確認を要求する。
/// 現時点では対応種別が `worktree.closed` のみなので挙動は変わらないが、自由文が入った
/// 瞬間に default-deny になるのが目的。
const AUTO_APPROVAL_PUSHABLE_KINDS: &[&str] = &[event_db::KIND_WORKTREE_CLOSED];

// ─── ハンドル ─────────────────────────────────────────────────────────────────

pub enum DeliveryMsg {
    /// 生存タブの一覧。ここに無い terminal_id の購読 / inbox は orphaned にする
    Reconcile { live_terminal_ids: Vec<String> },
    /// このタブへ、同じワークツリーの引き継ぎ待ちグループを1つ引き継ぐ
    Rebind {
        terminal_id: String,
        reply: Option<oneshot::Sender<(u64, u64)>>,
    },
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
        h.try_send(DeliveryMsg::Rebind {
            terminal_id,
            reply: None,
        });
    }
}

/// 新しいイベントが積まれたことを知らせる。
pub fn notify_event_queued(app: &AppHandle) {
    if let Some(h) = handle(app) {
        h.try_send(DeliveryMsg::EventQueued);
    }
}

/// 引き継ぎを要求し、完了まで待つ（`SessionStart` から使う）。
///
/// ワーカーが詰まっていたら `None` を返す。呼び出し元は上位のタイムアウトの内側で
/// 使うので、「今回は引き継がない」に劣化するだけで済む。
pub async fn rebind_and_wait(app: &AppHandle, terminal_id: &str) -> Option<(u64, u64)> {
    let (tx, rx) = oneshot::channel();
    {
        let h = handle(app)?;
        if h.tx
            .try_send(DeliveryMsg::Rebind {
                terminal_id: terminal_id.to_string(),
                reply: Some(tx),
            })
            .is_err()
        {
            return None;
        }
    }
    rx.await.ok()
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
        DeliveryMsg::Rebind { terminal_id, reply } => {
            // 自動引き継ぎは1タブにつき1グループまで。`resolve_subscriber` は購読系ツールの
            // 呼び出しごとに要求してくるので、抑止しないと1タブが全グループを吸い上げる。
            // 人間が明示的に選ぶ `RebindManual` はこの制限を受けない。
            let result = if state.claimed.contains(&terminal_id) {
                (0, 0)
            } else {
                let moved = rebind_for_terminal(app, pool, &terminal_id).await;
                if moved != (0, 0) {
                    state.claimed.insert(terminal_id.clone());
                }
                moved
            };
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
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
        // 残りに interrupt が混ざっていれば走行中でも割り込む。
        let delivery = strongest_delivery(&items);
        let decision = decide_push(session, &delivery, now);
        if let PushDecision::Skip(reason) = decision {
            log::debug!(
                "[delivery] 押し込みを見送る terminal={} 件数={} 理由={}",
                terminal_id,
                items.len(),
                reason
            );
            continue;
        }

        // 自動承認が有効な宛先へは定型イベントしか押し込まない。
        let worktree = items
            .first()
            .and_then(|i| i.subscriber_worktree_id.as_deref())
            .and_then(|id| find_worktree(&settings, id));
        let (allowed, blocked): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|i| auto_approval_allows(worktree, &i.kind));
        if !blocked.is_empty() {
            // 保留された分は未配送のまま残り、tick ごとに再評価される（＝毎回ここを通る）ので
            // debug に留める。人間への提示は UI の未読表示が担う。
            log::debug!(
                "[delivery] 自動承認が有効な宛先のため {} 件の押し込みを保留した（人間の確認が必要）terminal={}",
                blocked.len(),
                terminal_id
            );
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
        let _ = app.emit(
            "event-delivered",
            serde_json::json!({
                "terminalId": terminal_id,
                "sessionId": session.session_id,
                "worktreeId": worktree.map(|w| w.id.clone()),
                "worktreeName": worktree.map(|w| w.name.clone()),
                "agentName": session.agent_name,
                "count": ids.len(),
                "text": text,
                "method": "pty",
            }),
        );
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
    let candidates = match event_db::list_spawn_candidates(pool, now, event_db::PUSH_TTL_MS).await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[delivery] list_spawn_candidates failed: {}", e);
            return;
        }
    };
    if candidates.is_empty() {
        return;
    }
    let sessions = app.state::<crate::pty_manager::PtyManager>().list_sessions();
    // 同じ tick で複数ワークツリーが spawn 候補になったときに上限を素通りしないよう、
    // 発行した要求ぶんを足し込みながら判定する（`live` を1回だけ数えて全件に使うと、
    // 候補が5件あれば一気に5タブ増えて webview ハングの領域に入る）。
    let mut projected_live = sessions.iter().filter(|s| s.exit_code.is_none()).count()
        + state.inflight_spawn.len();
    let settings = app.state::<SettingsManager>().get();

    for (worktree_id, pending) in candidates {
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
        if !auto_approval_allows(Some(worktree), event_db::KIND_WORKTREE_CLOSED) {
            log::info!(
                "[delivery] 自動承認が有効なため spawn を見送った worktree={}",
                worktree.name
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
    pub target_worktree_id: String,
    pub target_worktree_name: Option<String>,
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
            SubscriptionView {
                subscriber_worktree_name: r.subscriber_worktree_id.as_deref().and_then(name_of),
                subscriber_session_id: session.map(|s| s.session_id),
                agent_name: session.and_then(|s| s.agent_name.clone()),
                target_worktree_name: name_of(&r.target),
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
        assert!(auto_approval_allows(Some(&on), event_db::KIND_WORKTREE_CLOSED));
        // 自由文（#126 で入る予定）は自動承認宛には押し込まない
        assert!(!auto_approval_allows(Some(&on), "worktree.message"));
    }

    #[test]
    fn test_auto_approval_off_allows_everything() {
        let off = worktree(Some(false));
        assert!(auto_approval_allows(Some(&off), "worktree.message"));
        let unset = worktree(None);
        assert!(auto_approval_allows(Some(&unset), "worktree.message"));
        assert!(auto_approval_allows(None, "worktree.message"));
    }
}
