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

/// MCP の購読 / inbox 系ツールが引き継ぎの完了を待つ上限（#137）。
///
/// 引き継ぎ自体は `rebind_next_orphaned_group` の UPDATE 数本で、実測でもミリ秒台。
/// ここが効くのはワーカーが押し込み（`SUBMIT_DELAY` 込み）や spawn の処理中で
/// 順番待ちになっている場合だけなので、その待ち行列1回分を見込んだ長さにする。
const REBIND_WAIT_BUDGET: std::time::Duration = std::time::Duration::from_millis(1500);

/// 同じワークツリーへ続けて自動 spawn するまでの最小間隔。
/// spawn したタブでエージェントが起動しない（`claude` が PATH に無い等）と検出フラグが
/// 立たないため、これが無いと tick ごとに壊れたタブを上限まで積み増す。
const SPAWN_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);

/// これ以上ターミナルがあるときは、自動 spawn しても**警告を出す**（止めはしない）。
///
/// 端末数 15+ で webview ハングと強い相関があった（#101 の調査。正常時は 5〜7）ため、
/// かつてはこの値で自動 spawn を拒否していた。だが 15 という数字は**対策前の観測**で、
/// その後 WebGL を可視端末のみに限定する等の対策が入っている。加えて拒否は
/// 「spawn すると言ったのに何も起きない」を作り、purpose である配送そのものが止まる。
///
/// そこで**判断を人間に返す**: spawn は通し、危険域に入ったことだけ知らせる。
/// 実際の歯止めは `pty_manager::MAX_PTY_SESSIONS`(32) と、ワークツリーごとの
/// `SPAWN_COOLDOWN` / 単一フライトが担う。
const SPAWN_WARN_LIVE_SESSIONS: usize = 12;

/// 自動承認が有効なワークツリーへ押し込み / spawn してよいイベント種別（#120 §5.5）。
///
/// 自動承認が有効な宛先へは、別のワークツリーから注入された内容が**人間の確認なしに
/// 実行される**。
///
/// 許可の基準は「**押し込む文字列を誰が書いたか**」:
/// - 許可: oretachi 自身が組み立てた文字列しか PTY に出ない種別。可変部分はワークツリー名・
///   ブランチ名・件数だけで、いずれも `sanitize_for_pty` を通る
///   - `worktree.closed` / `worktree.created` … `format_inbox_line` が定型文に組み直す
///   - `worktree.message` … 本文は運ばず `format_pointer_text` の告知だけを出す（#137）。
///     本文は `oretachi_poll_inbox` の**ツール出力**として渡るので、押し込みが人間の入力を
///     偽装して `ai_judge` に「ユーザーが明示的に依頼した」と誤認させる経路が無い
/// - 不許可: エージェントが書いた本文がそのまま PTY へ出る種別。`format_inbox_line` の
///   `other` 分岐は未知種別の本文を丸ごと展開するので、**種別を足しただけでは自動的に
///   安全にならない**
///
/// **新しい種別を足すときは必ずこの基準で判断すること。** 迷ったら入れない（default-deny）。
/// 本文をインラインで運ぶかどうかは `event_db::is_free_text_kind` が持っており、
/// ここへ足すときは向こうも合わせて見直すこと。
const AUTO_APPROVAL_PUSHABLE_KINDS: &[&str] = &[
    event_db::KIND_WORKTREE_CLOSED,
    event_db::KIND_WORKTREE_CREATED,
    event_db::KIND_WORKTREE_MESSAGE,
];

// ─── ハンドル ─────────────────────────────────────────────────────────────────

/// `additionalContext` 経由で未読をまとめて渡すときの、呼び出し元の hook 種別（#124）。
///
/// PTY 押し込みと違い、どれも「文字列を返すだけ」で PTY には触らない。ただし
/// **`TurnEnd` だけはターンを開始させる**（`Stop` の `additionalContext` は会話を継続する）
/// ため、押し込みと同じく `passive` を巻き込まない制限を課す（`filter_for_turn_end`）。
///
/// 自動承認のガードは**経路を問わず全部に掛かる**（`filter_auto_approval` の doc 参照、#126）。
/// 「`SessionStart` / `PromptSubmit` は人間の操作が起点だから全件渡す」というのは
/// Phase 1 の設計で、#126 で撤回済み。
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

    /// この経路の注入が**エージェントのターンを開始させる**か。
    ///
    /// `Stop` の `additionalContext` は会話を継続させ、`UserPromptSubmit` は人間のプロンプトに
    /// 乗るのでどちらもターンが動く。`SessionStart` だけは文脈に載るだけで動かない ——
    /// だから `delivered_at` を打ってはいけない（`collect_digest` の打刻分岐参照）。
    fn starts_turn(&self) -> bool {
        !matches!(self, DigestReason::SessionStart)
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
    /// このタブへ、同じワークツリーの引き継ぎ待ちグループを1つ引き継ぐ。
    ///
    /// `reply` を渡すと完了を待てる。**引き継いだ結果を自分で読む経路は必ず待つこと**
    /// （`rebind_and_wait` 参照）。hook 経路は `CollectDigest` が内包している。
    Rebind {
        terminal_id: String,
        reply: Option<oneshot::Sender<()>>,
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
///
/// **引き継いだ購読 / 未読を自分で読む経路では使わないこと。** 引き継ぎが終わる前に
/// `subscriber_terminal_id` で SELECT すると、行はまだ死んだタブの ID を向いているので
/// 0 件に見える。そういう経路は `rebind_and_wait` を使う。
pub fn request_rebind(app: &AppHandle, terminal_id: String) {
    if let Some(h) = handle(app) {
        h.try_send(DeliveryMsg::Rebind {
            terminal_id,
            reply: None,
        });
    }
}

/// 引き継ぎの完了を待つ（#137）。
///
/// MCP の購読 / inbox 系ツールは `subscriber_terminal_id` で DB を引くので、タブを
/// 立て直した直後は**引き継ぎが終わるまで自分の購読も未読も見えない**。`collect_digest` が
/// `list_inbox` の前に `try_rebind_once` を await しているのと同じ理由で、読む前に待つ。
///
/// ワーカーが詰まっている / 応答が来ない場合は待ち続けずに諦めて先へ進む。引き継ぎは
/// 次の `Reconcile` や hook 経路でも走るので、取りこぼしても一時的に古い結果を返すだけ。
/// `oretachi_*` は MCP のツール呼び出しなので、ここで長く止めると呼び出し側が固まる。
pub async fn rebind_and_wait(app: &AppHandle, terminal_id: &str) {
    let (tx, rx) = oneshot::channel();
    {
        let Some(h) = handle(app) else { return };
        if h.tx
            .try_send(DeliveryMsg::Rebind {
                terminal_id: terminal_id.to_string(),
                reply: Some(tx),
            })
            .is_err()
        {
            log::debug!(
                "[delivery] 引き継ぎキューが満杯のため待たずに続行する terminal={}",
                terminal_id
            );
            return;
        }
    }
    if tokio::time::timeout(REBIND_WAIT_BUDGET, rx).await.is_err() {
        log::warn!(
            "[delivery] 引き継ぎの完了を待ちきれなかったため古い結果を返す可能性がある terminal={}",
            terminal_id
        );
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

/// 自動 spawn の単一フライト1件。
struct InflightSpawn {
    request_id: String,
    requested_at: Instant,
    /// `--resume` に渡した AI セッション。
    ///
    /// **これを持つのが要点。** spawn したタブは定義上このセッションの続きなので、
    /// 購読の引き継ぎ（同一セッション限定）はこの値で判定してよい。検出された
    /// `agent_session_id` を待つと、(1) エージェント検出は 10 秒周期で spawn 応答より遅く、
    /// (2) `claude --resume <uuid>` が `~/.claude/sessions/<pid>.json` に同じ UUID を
    /// 報告する保証がコード上に無い —— という2つの理由で引き継ぎを取り逃す。
    resume_session: Option<String>,
}

#[derive(Default)]
struct WorkerState {
    /// タブごとの最終押し込み時刻（レート制限）
    last_push: HashMap<String, Instant>,
    /// spawn 要求中のワークツリー（単一フライト）
    inflight_spawn: HashMap<String, InflightSpawn>,
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
    ///
    /// **値は「試したときに観測した AI セッション」。** 購読行の引き継ぎは同一セッションに
    /// 限られる（`event_db::RebindScope`）ので、セッションが変われば結論も変わる。
    /// 単なる `HashSet` だと、セッションがまだ検出できていない間（spawn 直後 / 状態ファイルが
    /// 読めない構成 / gemini・codex・cline のようにそもそも session UUID を持たないエージェント）に
    /// 抑止してしまい、**検出後も `Reconcile` まで引き継ぎが走らない**。逆に抑止しないと
    /// セッションが永久に `None` のタブが毎ターン DB を叩き続ける。鍵にセッションを含めると
    /// どちらも起きない。
    rebind_probed: HashMap<String, Option<String>>,
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
    /// タブごとの「AI エージェントが居ないと最初に判定した時刻」。
    /// `NO_AGENT_GRACE` を超えたら、そのタブ宛の未読を引き継ぎ待ちへ落とす。
    /// 押し込めた / 別の理由で見送った / タブが消えた時点で忘れる。
    no_agent_since: HashMap<String, Instant>,
    /// タブごとの直近の見送り理由。**同じ理由の連投を info で溢れさせないため**に持つ。
    /// 理由が変わった初回だけ info、以降は debug（見送り理由が debug のみだと、
    /// 「配送されない」の切り分けにログレベルの変更と再現待ちが必要になる）。
    last_skip_reason: HashMap<String, &'static str>,
    /// 直近に報告した「押し込みの窓から外れた未 ack」の (件数, 最古の created_at)。
    /// 件数だけを鍵にすると、同じ間隔で1件が窓から外れ1件が ack されたときに報告が消える。
    last_stale_report: Option<(i64, Option<i64>)>,
}

/// 引き継ぎ待ちの保持期限切れを掃除する間隔。
const RETENTION_PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

/// 「生存しているのに受け取れない」タブ宛の未読を別のタブへ移すまでの猶予。
///
/// エージェント検出は 10 秒周期のポーリング（`pty_manager::start_polling`）なので、
/// タブを立てて `claude` が起動しきるまでの間は正当に「居ない」に見える。人間が手で
/// 打ち直す猶予も含めて 2 分待つ。これを短くすると、起動途中のタブ宛の未読が毎回
/// 別タブへ吸われる。
const NO_AGENT_GRACE: std::time::Duration = std::time::Duration::from_secs(120);

/// `decide_push` の見送り理由のうち、**待っても自然には解消しない**もの。
///
/// タブは生きているので `mark_orphaned_subscribers` は何もしないが、この状態のままでは
/// 押し込みが永久に通らない。他のタブへ移す（`handoff_unreachable_terminal`）判断に使う。
///
/// - `SKIP_NO_AGENT`: `claude` が終了して素のシェルに戻ったタブ。CR を撃てない
/// - `SKIP_STATUS_UNKNOWN`: `claude` は居るが `~/.claude/sessions/<pid>.json` が読めない。
///   子セッション（`CLAUDE_CODE_CHILD_SESSION` を継承）や shim 経由で本体を辿れない構成で
///   起きる。busy か idle かが永久に分からないので、待っても押し込めない
///
/// 一時的な理由（`エージェントが走行中` / `出力が動いている` / 人間の入力待ち）は含めない。
const SKIP_NO_AGENT: &str = "AI エージェントが走っていない";
const SKIP_STATUS_UNKNOWN: &str = "エージェントの状態が不明";

/// 人間の入力待ち（Claude Code の `status: "waiting"`）。
///
/// 質問プロンプトや承認プロンプトの前で止まっている状態。**押し込んではいけない**
/// （プロンプトへの回答として解釈される）が、`エージェントが走行中` と混ぜると
/// 「なぜ配送されないのか」の切り分けを誤らせるので理由を分けている。
/// 人間が答えれば解消するので `handoff` の対象にはしない。ただし待機が長引くと
/// `PUSH_TTL_MS` を食い潰すので、その救済は `push_stale_pointers` が担う。
const SKIP_WAITING_INPUT: &str = "エージェントが人間の入力を待っている";

/// 待っても解消しない見送り理由か（＝別のタブへ移す価値があるか）。
fn skip_is_terminal(reason: &str) -> bool {
    reason == SKIP_NO_AGENT || reason == SKIP_STATUS_UNKNOWN
}

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
                    .retain(|_, f| f.requested_at.elapsed() < SPAWN_TIMEOUT);
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
                state.no_agent_since.retain(|t, _| live_terminal_ids.contains(t));
                state.last_skip_reason.retain(|t, _| live_terminal_ids.contains(t));
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
                        // **生まれた引き継ぎ待ちを、既に走っているエージェントへ引き取らせる。**
                        // `Rebind` の要求元は「エージェントの新規検出」と「MCP / hook 呼び出し」
                        // だけなので、既に立ち上がっていてアイドルで黙っているタブは、
                        // 次に何か喋るまで引き取らない。その間 `spawn_for_closed_tabs` も
                        // `has_agent` で見送るため出口が無く、未 ack バッジだけが残る（実機で踏んだ）。
                        if subs > 0 || inbox > 0 {
                            claim_orphans_with_live_agents(app, pool, state).await;
                        }
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
            // 自動引き継ぎは1タブにつき1グループまで。人間が明示的に選ぶ `RebindManual` は
            // この制限を受けない。
            let result = try_rebind_once(app, pool, state, &terminal_id).await;
            if result != (0, 0) {
                let _ = app.emit("event-inbox-changed", ());
            }
            // **`drive` より先に応答する。** 待っている側が読みたいのは引き継ぎの結果だけで、
            // 押し込みの完了ではない。逆順にすると押し込み（PTY への書き込みと
            // `SUBMIT_DELAY` の待ち）のぶんだけ MCP ツールの応答が遅れる。
            if let Some(reply) = reply {
                let _ = reply.send(());
            }
            drive(app, pool, state).await;
        }
        DeliveryMsg::RebindManual {
            worktree_id,
            dead_terminal_id,
            terminal_id,
            reply,
        } => {
            // **手動引き継ぎはセッションで絞らない。** 人間が引き継ぎ先を選んでいるので、
            // 自動引き継ぎが同一セッションに限られること（`RebindScope` の doc）の逃げ道が
            // ここしかない。縛ると引き継ぎ待ちが保持期限（7日）で黙って消える。
            let result = event_db::rebind_orphaned_group(
                pool,
                &worktree_id,
                &dead_terminal_id,
                &terminal_id,
                event_db::RebindScope::AnySession,
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
                .find(|(_, f)| f.request_id == request_id)
                .map(|(wt, _)| wt.clone());
            // `--resume` に渡したセッション。spawn したタブはこのセッションの続きなので、
            // 検出を待たずに購読の引き継ぎ判定へ使える（`InflightSpawn` の doc 参照）。
            let resume_session = worktree_id
                .as_ref()
                .and_then(|wt| state.inflight_spawn.remove(wt))
                .and_then(|f| f.resume_session);
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
                    // spawn 直後はエージェント検出（10 秒周期）より前なので、購読行の引き継ぎ
                    // 可否を決める session UUID がまだ無く、移るのは未読だけになりうる。
                    // **その場合は `claimed` に入れない**（`try_rebind_once` と同じ理由。
                    // 入れると恒久的に弾かれ、セッション判明後も購読を引き継げない。自動 spawn は
                    // 購読側の session を `--resume` するので、待てば一致しうる）。
                    let (moved, session_known) =
                        match worktree_and_session_of_terminal(app, &tid) {
                            Some((worktree_id, agent_session)) => {
                                // 検出済みの値を優先し、まだ無ければ `--resume` に渡した値を使う。
                                let session = agent_session.or(resume_session);
                                (
                                    rebind_group_for(pool, &worktree_id, &tid, session.as_deref())
                                        .await,
                                    session.is_some(),
                                )
                            }
                            None => ((0, 0), false),
                        };
                    if moved != (0, 0) && session_known {
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

/// 生存している AI タブに、そのワークツリーの引き継ぎ待ちを引き取らせる。
///
/// タブが死んだ直後（`mark_orphaned_subscribers` が引き継ぎ待ちを作った直後）に呼ぶ。
/// 引き取りは1タブ1グループまで（`try_rebind_once` の `claimed`）なので、生存タブの数だけ
/// グループが減る。引き取れるものが無ければ何も起きない。
///
/// 引き取れたら `drive` まで進める。移った先はアイドルのはずなので、次の 30 秒 tick を
/// 待たずに押し込みたい。
async fn claim_orphans_with_live_agents(
    app: &AppHandle,
    pool: &SqlitePool,
    state: &mut WorkerState,
) {
    // `session_id` 昇順で決定的に回す（どのタブがどのグループを引き取るかを再現可能にする）。
    let mut agents: Vec<crate::pty_manager::SessionInfo> = app
        .state::<crate::pty_manager::PtyManager>()
        .list_sessions()
        .into_iter()
        .filter(|s| s.exit_code.is_none() && s.is_ai_agent)
        .collect();
    agents.sort_by_key(|s| s.session_id);

    let mut claimed_any = false;
    for agent in agents {
        if try_rebind_once(app, pool, state, &agent.terminal_id).await != (0, 0) {
            claimed_any = true;
        }
    }
    if claimed_any {
        let _ = app.emit("event-inbox-changed", ());
        drive(app, pool, state).await;
    }
}

/// 1タブにつき1グループまでの引き継ぎを、無駄打ちを抑えつつ試す。
///
/// - 既に1グループ引き継いだタブ（`claimed`）は対象外。`resolve_subscriber` や hook 経路は
///   呼び出しのたびに要求してくるので、抑止しないと1タブが全グループを吸い上げる
/// - 前回試して**対象が無かった**タブ（`rebind_probed`）も対象外。`Stop` は毎ターン発火
///   するため、これが無いと全 CC タブが毎ターン `list_orphaned_groups` を叩き続ける。
///   新しい引き継ぎ待ちが生まれる `Reconcile`（生存タブ変化時）で忘れる。
///   **抑止の鍵にはそのタブの AI セッションを含める**（`rebind_probed` の doc 参照）
/// - **AI セッションが未検出のうちは `claimed` に入れない。** 未読だけが移った状態で
///   `claimed` を立てると、この関数の冒頭で恒久的に弾かれ、数秒後にセッションが判明しても
///   購読を引き継げない（エージェント検出の `request_rebind` は false→true 遷移でしか
///   撃たれないので二度目が来ない）。**同一セッションの `--resume` タブなのに引き継がれない**
///   という、この制限が成立させたい本命ケースが落ちる
async fn try_rebind_once(
    app: &AppHandle,
    pool: &SqlitePool,
    state: &mut WorkerState,
    terminal_id: &str,
) -> (u64, u64) {
    if state.claimed.contains(terminal_id) {
        return (0, 0);
    }
    // 抑止の判定にセッションが要るので先に引く。**この解決は1回だけ**にすること
    // （`SettingsManager::get()` は AppSettings 全体、`list_sessions()` は全セッションの
    // clone なので、毎ターン全 CC タブが通る経路で二重に払うと無駄が大きい）。
    let Some((worktree_id, agent_session)) = worktree_and_session_of_terminal(app, terminal_id)
    else {
        return (0, 0);
    };
    if state.rebind_probed.get(terminal_id) == Some(&agent_session) {
        return (0, 0);
    }
    let moved = rebind_group_for(pool, &worktree_id, terminal_id, agent_session.as_deref()).await;
    if moved == (0, 0) {
        state
            .rebind_probed
            .insert(terminal_id.to_string(), agent_session);
    } else if agent_session.is_some() {
        state.claimed.insert(terminal_id.to_string());
    }
    moved
}

/// 指定ワークツリーの引き継ぎ待ちを1グループ、このタブへ引き継ぐ。
///
/// ワークツリーとセッションの解決は**呼び出し側**で済ませて渡す（`try_rebind_once` の
/// コメント参照）。`agent_session` が `None` のときは購読行を移さず未読だけが移る。
async fn rebind_group_for(
    pool: &SqlitePool,
    worktree_id: &str,
    terminal_id: &str,
    agent_session: Option<&str>,
) -> (u64, u64) {
    match event_db::rebind_next_orphaned_group(pool, worktree_id, terminal_id, agent_session).await
    {
        Ok((subs, inbox)) => {
            if subs > 0 || inbox > 0 {
                log::info!(
                    "[delivery] 引き継ぎ完了 worktree={} terminal={} 購読={} 未読={}",
                    worktree_id,
                    terminal_id,
                    subs,
                    inbox
                );
                return (subs, inbox);
            }
            // **「引き継ぎ待ちバッジが残るのに何も起きない」を追跡可能にする。** 購読行の
            // 引き継ぎを同一 AI セッションに限った（`RebindScope` の doc）結果、別タスクの
            // セッションでは正当に引き継がれない。理由をログに出さないと「壊れている」と
            // 区別できない。この分岐は `rebind_probed` の抑止が効くので連投しない。
            if let Ok(groups) =
                event_db::list_orphaned_groups(pool, worktree_id, agent_session).await
            {
                let stuck: i64 = groups
                    .iter()
                    .filter(|g| g.terminal_id != terminal_id)
                    .map(|g| g.subscriptions)
                    .sum();
                if stuck > 0 {
                    log::info!(
                        "[delivery] 引き継ぎ待ちの購読 {} 件は AI セッション（{:?}）が別なので自動引き継ぎしない worktree={} terminal={}（UI から手動で引き継げます）",
                        stuck,
                        agent_session,
                        worktree_id,
                        terminal_id
                    );
                }
            }
            (0, 0)
        }
        Err(e) => {
            log::warn!("[delivery] rebind_next_orphaned_group failed: {}", e);
            (0, 0)
        }
    }
}

/// terminal_id → ワークツリー ID。生存セッションの cwd から前方一致＋最長一致で引く。
fn worktree_of_terminal(app: &AppHandle, terminal_id: &str) -> Option<String> {
    worktree_and_session_of_terminal(app, terminal_id).map(|(worktree_id, _)| worktree_id)
}

/// terminal_id → (ワークツリー ID, そのタブで走っている AI エージェントの session UUID)。
///
/// session UUID は購読行の引き継ぎを同一セッションに限るための鍵（`rebind_orphaned_group`
/// の doc 参照）。ポーリングが `~/.claude/sessions/<pid>.json` から拾うので、タブを立てた
/// 直後や Claude Code 以外のエージェントでは `None` になる。
fn worktree_and_session_of_terminal(
    app: &AppHandle,
    terminal_id: &str,
) -> Option<(String, Option<String>)> {
    let settings = app.state::<SettingsManager>().get();
    let sessions = app.state::<crate::pty_manager::PtyManager>().list_sessions();
    let session = sessions.iter().find(|s| s.terminal_id == terminal_id)?;
    let cwd = session.cwd.as_deref()?;
    crate::mcp_server::resolve_worktree_by_cwd(&settings, cwd)
        .map(|w| (w.id.clone(), session.agent_session_id.clone()))
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
    if reply.send(Some(digest)).is_err() {
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
    // **ターンを開始する経路と、しない経路で打刻先を分ける。**
    //
    // `SessionStart` の `additionalContext` は文脈に載るだけでターンを開始しない。ここで
    // `delivered_at` を打つと `list_pushable` から外れ、ターンを開始できる PTY 押し込みが
    // 候補を永久に失う（＝誰も動かないまま未 ack バッジが残る）。`notified_at` へ打てば
    // 「再掲はしない」だけを表現でき、押し込みの候補には残る。
    // `Stop` / `UserPromptSubmit` は実際にターンが動くので従来どおり `delivered_at`。
    let stamp = if reason.starts_turn() {
        event_db::mark_delivered(pool, &ids, event_db::now_ms()).await
    } else {
        event_db::mark_notified(pool, &ids, event_db::now_ms()).await
    };
    if let Err(e) = stamp {
        // 打刻に失敗しても本文は既に渡している。未配送のまま残るので次の機会に再度出る
        // （取りこぼすより一度重複するほうが安全）。
        log::warn!("[delivery] 配送の打刻に失敗: {}", e);
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
        return PushDecision::Skip(SKIP_NO_AGENT);
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
    // 人間の入力待ち（質問 / 承認プロンプト）には**`interrupt` でも割り込まない**。
    //
    // `interrupt` は「走行中のエージェントに割り込んでよい」というオプトインであって、
    // 「人間に向けられたプロンプトへ代わりに答えてよい」ではない。ここで押し込むと
    // ブラケットペーストと CR がプロンプトへの回答として解釈され、選択肢を勝手に確定させる。
    // 直前の `output_quiescent`（人間が入力中の行を壊さない）と同じ理由で最優先する。
    if session.agent_status.as_deref() == Some("waiting") {
        return PushDecision::Skip(SKIP_WAITING_INPUT);
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
        // `waiting`（人間の入力待ち）は interrupt の手前で確定済み。
        Some(other) => {
            // busy はターン境界配送（#124 の Stop フック）の担当。次の tick で再評価する。
            let _ = other;
            return PushDecision::Skip("エージェントが走行中");
        }
        None => return PushDecision::Skip(SKIP_STATUS_UNKNOWN),
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

/// `--resume` へ渡してよい session ID か（英数字とハイフンのみ）。
///
/// 組み立てたコマンド文字列は**シェルへそのまま流し込まれる**（`App.vue` が `\r` を足して
/// PTY へ書く）ので、引用符やセミコロンが混ざると別のコマンドとして解釈されうる。
/// 値の出所は Claude Code の session UUID なので普段は素通りするが、DB を経由する以上
/// 素性を検証してから渡す。
fn is_safe_session_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// 自動 spawn したタブへ渡す初期プロンプト（#137）。
///
/// **これが無いとエージェントは何もしない。** spawn 直後は SessionStart フックが
/// `additionalContext` で告知を渡すが、`additionalContext` は**ターンを開始させない**ので、
/// エージェントは告知を持ったままプロンプトで待機する。起動コマンドに位置引数のプロンプトを
/// 載せて、最初のターンをこちらから始める。
///
/// なお SessionStart 側の打刻は `delivered_at` ではなく `notified_at` なので、押し込みは
/// 候補を失わない（以前は `delivered_at` を打っており、押し込みまで死んでいた）。ただし
/// 押し込みが走るのは早くても次の tick（最大 30 秒 + エージェント検出の 10 秒）で、
/// `MIN_PUSH_INTERVAL` にも掛かる。spawn した意味を即座に出すためこのプロンプトは残す。
///
/// **oretachi 自身が書く固定文であること。** 発信側のメッセージ本文をここへ入れると、
/// 押し込みと同じく「人間が打ったプロンプト」に化ける（`format_pointer_text` の doc 参照）。
const SPAWN_INITIAL_PROMPT: &str =
    "[oretachi] 購読していたワークツリーイベントの未読があります。oretachi_poll_inbox で内容を確認し、\
     必要な対応を進めてから oretachi_ack_message で ack してください。";

/// 自動 spawn したタブで起動するエージェントコマンド。
///
/// 解決順はフロントの `useTaskExecution.ts` と同じ（ワークグループ → 全体設定 →
/// `claudeCode`）。Claude Code の permission-mode も同じく既定は `plan` にする ——
/// 別のワークツリーからの通知で立ち上がるタブなので、いきなり書き込みを許すのは危険。
///
/// `resume_session` があれば `--resume` でその会話の続きとして立ち上げる。購読を張った
/// エージェントの文脈を引き継げるので、「未読を確認しろ」だけを新規セッションに投げるより
/// 話が早い。**Claude Code 以外は初期プロンプトも `--resume` も付けない** —— CLI ごとに
/// 引数の意味が違い、誤って非対話モードで起動すると spawn したタブが即終了しうる。
fn agent_command_for_worktree(
    settings: &AppSettings,
    worktree: &WorktreeEntry,
    resume_session: Option<&str>,
) -> String {
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
            // session UUID は oretachi が発番したものではなく Claude Code 由来なので、
            // 万一おかしな文字が混ざってもシェルに渡さないよう形だけ検証する。
            let resume = resume_session
                .filter(|s| is_safe_session_id(s))
                .map(|s| format!(" --resume {}", s))
                .unwrap_or_default();
            format!(
                "claude{} --permission-mode {} \"{}\"",
                resume, mode, SPAWN_INITIAL_PROMPT
            )
        }
        AiAgentKind::GeminiCli => "gemini".to_string(),
        AiAgentKind::CodexCli => "codex".to_string(),
        AiAgentKind::ClineCli => "cline".to_string(),
    }
}

async fn drive(app: &AppHandle, pool: &SqlitePool, state: &mut WorkerState) {
    let now = event_db::now_ms();
    push_pending(app, pool, state, now).await;
    push_stale_pointers(app, pool, state, now).await;
    spawn_for_closed_tabs(app, pool, state, now).await;
    report_stale_unpushed(pool, state, now).await;
}

/// 押し込みの窓（`PUSH_TTL_MS`）から外れた未読を、**告知だけ**で救済する。
///
/// TTL は「何日も前の未配送分が再起動直後に一斉に割り込む」のを防ぐためのものだが、
/// 宛先が受け取れない状態（走行中が長引く / 人間の入力待ち / 検出不能）が 10 分続いただけで
/// 未読が黙って押し込み候補から消える。バッジは残るのでユーザーからは
/// 「アイドルなのに不発」に見える。窓を広げると TTL の目的が壊れるので、
/// **本文は出さず件数だけを 1 回押し込む**という別経路にした。
///
/// - 本文を運ばないので、鮮度が失われた内容をいまさら注入することにはならない
/// - 押し込む文字列は `format_pointer_push_text`（oretachi 自身の固定文）だけなので、
///   自動承認が有効な宛先でも「エージェントが書いた本文が人間の入力に化ける」経路が無い。
///   ただし種別ごとの保留判断は `auto_approval_allows` でそのまま行う
/// - `delivered_at` を打つので 1 タブにつき 1 回。本文は未 ack のまま DB に残り、
///   `oretachi_poll_inbox` でいつでも取れる（`format_inbox_push_text` の告知形と同じ扱い）
async fn push_stale_pointers(
    app: &AppHandle,
    pool: &SqlitePool,
    state: &mut WorkerState,
    now: i64,
) {
    let items = match event_db::list_stale_unpushed(pool, now, event_db::PUSH_TTL_MS).await {
        Ok(items) => items,
        Err(e) => {
            log::warn!("[delivery] list_stale_unpushed failed: {}", e);
            return;
        }
    };
    if items.is_empty() {
        return;
    }
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
            continue;
        };
        if let Some(prev) = state.last_push.get(&terminal_id) {
            if prev.elapsed() < MIN_PUSH_INTERVAL {
                continue;
            }
        }
        // 新鮮な行と同じ仕分けを通す（`passive` は自分で取りに来る、自動承認が有効な宛先へは
        // 許可種別のみ）。件数だけの告知でも「届いている」ことは伝わるので、保留対象の種別を
        // 件数に混ぜてはいけない。
        let allowed: Vec<_> = items
            .into_iter()
            .filter(|i| i.delivery != event_db::DELIVERY_PASSIVE)
            .filter(|i| {
                let worktree = i
                    .subscriber_worktree_id
                    .as_deref()
                    .and_then(|id| find_worktree(&settings, id));
                auto_approval_allows(worktree, &i.kind)
            })
            .collect();
        if allowed.is_empty() {
            continue;
        }
        // 押し込み可否の判定は新鮮な行と同一。`interrupt` で走行中に割り込むのは
        // 「鮮度のある通知」に対して認められた指定なので、ここでは使わない（`turn_end` 相当）。
        if let PushDecision::Skip(reason) = decide_push(session, event_db::DELIVERY_TURN_END, now) {
            // **新鮮な行と同じ救済判定を通す。** `push_pending` は `list_pushable`（窓の内側）
            // しか見ないので、鮮度切れの行だけを抱えた受け取れないタブはここでしか救えない。
            note_push_skip(app, pool, state, &terminal_id, reason, allowed.len()).await;
            continue;
        }

        let text = event_db::format_pointer_push_text(allowed.len() as i64);
        match write_push(app, session, &text, &terminal_id).await {
            PushWrite::Sent => {
                state.last_push.insert(terminal_id.clone(), Instant::now());
            }
            // 告知は入力欄に残っている。人間が Enter を押せば送れるので打刻はしない。
            PushWrite::PastedOnly => {
                state.last_push.insert(terminal_id.clone(), Instant::now());
                continue;
            }
            PushWrite::Failed => continue,
        }
        let ids: Vec<String> = allowed.iter().map(|i| i.id.clone()).collect();
        if let Err(e) = event_db::mark_delivered(pool, &ids, now).await {
            log::warn!("[delivery] mark_delivered failed: {}", e);
        }
        log::info!(
            "[delivery] 鮮度切れの未読 {} 件を件数のみで告知した terminal={} session={}",
            ids.len(),
            terminal_id,
            session.session_id
        );
        let _ = app.emit("event-inbox-changed", ());
    }
}

/// 押し込みの窓（`PUSH_TTL_MS`）から外れた未 ack 件数を報告する。
///
/// `list_pushable` / `list_spawn_candidates` の `created_at > now - TTL` は**黙って**候補を
/// 減らすので、「バッジは出ているのに何も起きない」の原因が特定できなかった。件数が変わった
/// ときだけ出す（毎 tick 出すと 30 秒ごとに同じ行が並ぶ）。
///
/// ここに出た分は `push_stale_pointers` が宛先の復帰を待って件数だけ告知するので、
/// 残り続けるのは**宛先が受け取れる状態に戻らない**ぶんだけ。
async fn report_stale_unpushed(pool: &SqlitePool, state: &mut WorkerState, now: i64) {
    let (count, oldest) = match event_db::count_stale_unpushed(pool, now, event_db::PUSH_TTL_MS)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            log::debug!("[delivery] count_stale_unpushed failed: {}", e);
            return;
        }
    };
    if state.last_stale_report == Some((count, oldest)) {
        return;
    }
    state.last_stale_report = Some((count, oldest));
    if count == 0 {
        return;
    }
    let age_min = oldest.map(|at| (now - at) / 60_000).unwrap_or(0);
    log::info!(
        "[delivery] 押し込みの窓（{} 分）から外れた未 ack が {} 件ある（最古 {} 分前）。宛先が受け取れる状態に戻れば件数のみ告知する",
        event_db::PUSH_TTL_MS / 60_000,
        count,
        age_min
    );
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
            note_push_skip(app, pool, state, &terminal_id, reason, allowed.len()).await;
            continue;
        }
        state.no_agent_since.remove(&terminal_id);
        state.last_skip_reason.remove(&terminal_id);
        // 長さ上限で載らなかった分は打刻しない（未配送のまま次回に回す）。
        let Some((text, used)) = event_db::format_inbox_push_text(&allowed) else {
            continue;
        };

        // 本文は `sanitize_for_pty` 済みなので、ペーストを脱出する終端シーケンスは混ざらない。
        // **write が成功してから打刻する**。キュー満杯などで捨てられた押し込みを
        // 配送済みにすると、二度と本文が出ないまま失われる。
        match write_push(app, session, &text, &terminal_id).await {
            PushWrite::Sent => {
                // ペーストが通った時点でレート制限を進める（Enter だけ失敗した場合も同じ）。
                state.last_push.insert(terminal_id.clone(), Instant::now());
            }
            PushWrite::PastedOnly => {
                state.last_push.insert(terminal_id.clone(), Instant::now());
                continue;
            }
            PushWrite::Failed => continue,
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

/// 押し込みの見送りを記録する（ログ + 受け取れないタブの救済）。
///
/// 新鮮な行（`push_pending`）と鮮度切れの行（`push_stale_pointers`）の両方から呼ぶ。
/// 片方だけに置くと、もう片方の経路しか候補を持たないタブが救済から漏れる。
///
/// ログは**理由が変わった初回だけ info**。同じ理由は 30 秒ごとに出るので debug に落とす。
/// 見送り理由が debug のみだと「配送されない」の切り分けにログレベル変更と再現待ちが必要で、
/// 実際にそれで原因特定が遅れた。
async fn note_push_skip(
    app: &AppHandle,
    pool: &SqlitePool,
    state: &mut WorkerState,
    terminal_id: &str,
    reason: &'static str,
    pending: usize,
) {
    if state.last_skip_reason.get(terminal_id) == Some(&reason) {
        log::debug!(
            "[delivery] 押し込みを見送る terminal={} 件数={} 理由={}",
            terminal_id,
            pending,
            reason
        );
    } else {
        state.last_skip_reason.insert(terminal_id.to_string(), reason);
        log::info!(
            "[delivery] 押し込みを見送る terminal={} 件数={} 理由={}",
            terminal_id,
            pending,
            reason
        );
    }
    if !skip_is_terminal(reason) {
        // 走行中 / 出力継続 / 人間の入力待ちは待てば解消するので、猶予は測り直す。
        state.no_agent_since.remove(terminal_id);
        return;
    }
    // **タブは生きているが受け取れない。** 待っても解消しないので、猶予を過ぎたら同じ
    // ワークツリーの生存 AI タブへ移す。これをやらないと `PUSH_TTL_MS` 経過で押し込み候補から
    // 静かに消え、未 ack バッジだけが残って誰も動かない状態で固定される。
    let since = state
        .no_agent_since
        .entry(terminal_id.to_string())
        .or_insert_with(Instant::now);
    if since.elapsed() >= NO_AGENT_GRACE {
        state.no_agent_since.remove(terminal_id);
        handoff_unreachable_terminal(app, pool, terminal_id, reason).await;
    }
}

/// 生存しているが受け取れないタブ宛の未読を、同じワークツリーの生存 AI タブへ移す。
///
/// `mark_orphaned_subscribers` は「タブが存在するか」しか見ないため、`claude` を終了して
/// 素のシェルに戻ったタブや、状態が永久に不明なタブ宛の未読は誰にも配送されないまま
/// `PUSH_TTL_MS` を過ぎ、押し込みの候補から静かに消える（未 ack バッジだけが残る）。
///
/// **`orphaned_at` は経由しない。** タブが生きているので、引き継ぎ待ちに落としても
/// `mark_orphaned_subscribers` の逆遷移が次の `Reconcile` で active に戻してしまう。
/// さらに引き継ぎ待ちにすると `spawn_if_closed` の候補にもなるため、**検出側が壊れている
/// 環境（実際に走っている `claude` を見つけられない）では働いているタブから未読を奪って
/// 重複タブを立て続ける**。移送先が特定できるときだけ直接書き換える。
///
/// 移送先が居ない場合は何もしない（＝変更前と同じ挙動）。バッジは残るが、
/// `report_stale_unpushed` が件数を、`push_stale_pointers` が復帰後の告知を担当する。
async fn handoff_unreachable_terminal(
    app: &AppHandle,
    pool: &SqlitePool,
    terminal_id: &str,
    reason: &str,
) {
    let Some(worktree_id) = worktree_of_terminal(app, terminal_id) else {
        return;
    };
    // 同じワークツリーの生存 AI タブ。`session_id` 昇順で決定的に選ぶ（同じ状況で
    // 毎回同じタブへ移す。tick ごとに移送先が揺れると追跡不能になる）。
    let candidate = {
        let settings = app.state::<SettingsManager>().get();
        let mut sessions: Vec<crate::pty_manager::SessionInfo> = app
            .state::<crate::pty_manager::PtyManager>()
            .list_sessions()
            .into_iter()
            .filter(|s| s.exit_code.is_none() && s.is_ai_agent && s.terminal_id != terminal_id)
            .filter(|s| {
                s.cwd
                    .as_deref()
                    .and_then(|c| crate::mcp_server::resolve_worktree_by_cwd(&settings, c))
                    .map(|w| w.id == worktree_id)
                    .unwrap_or(false)
            })
            .collect();
        sessions.sort_by_key(|s| s.session_id);
        sessions.into_iter().next().map(|s| s.terminal_id)
    };
    let Some(candidate) = candidate else {
        log::debug!(
            "[delivery] 受け取れないタブの移送先が同じワークツリーに居ない terminal={} 理由={}",
            terminal_id,
            reason
        );
        return;
    };

    match event_db::move_inbox_to_terminal(pool, &worktree_id, terminal_id, &candidate).await {
        Ok(0) => {}
        Ok(moved) => {
            log::info!(
                "[delivery] 受け取れないタブ（理由={}）の未読 {} 件を同じワークツリーの AI タブへ移した from={} to={}",
                reason,
                moved,
                terminal_id,
                candidate
            );
            let _ = app.emit("event-inbox-changed", ());
        }
        Err(e) => log::warn!("[delivery] move_inbox_to_terminal failed: {}", e),
    }
}

/// PTY 押し込みの書き込み結果。呼び出し元がレート制限と打刻の扱いを分けるために区別する。
enum PushWrite {
    /// ペーストと Enter の両方が通った
    Sent,
    /// ペーストは通ったが Enter に失敗した。本文は入力欄に残っているので**打刻しない**が、
    /// 次の tick で同じ本文を重ねて貼らないようレート制限は進める
    PastedOnly,
    /// ペースト自体が失敗した。何も起きていないので打刻もレート制限も進めない
    Failed,
}

/// ブラケットペーストで本文を流し、少し待ってから Enter を送る。
///
/// **ペーストと Enter は別の write に分ける。** `ESC[200~…ESC[201~\r` を1回で書くと
/// Claude Code はペースト終端と同じ読み取りチャンクに来た CR をペーストの一部として扱い、
/// 本文が入力欄に残ったままターンが始まらない（実機で確認）。CR を独立した write にし、
/// 間に `SUBMIT_DELAY` の猶予を入れると確実に送信される。
///
/// `text` は呼び出し元が `sanitize_for_pty` を通した1行であること。ESC が残っていると
/// ペーストを脱出して任意のコマンドが走る。
async fn write_push(
    app: &AppHandle,
    session: &crate::pty_manager::SessionInfo,
    text: &str,
    terminal_id: &str,
) -> PushWrite {
    let paste = format!("\x1b[200~{}\x1b[201~", text);
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
        return PushWrite::Failed;
    }
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
        return PushWrite::PastedOnly;
    }
    PushWrite::Sent
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
        // **止めずに知らせる。** 拒否すると配送そのものが止まり「spawn すると言ったのに
        // 何も起きない」になる。ハングの危険域に入ったかどうかの判断は人間に返す。
        if projected_live >= SPAWN_WARN_LIVE_SESSIONS {
            log::warn!(
                "[delivery] ターミナルが {} 個ある状態で自動 spawn する（webview ハングの危険域 {}+）worktree={} 未読={}",
                projected_live,
                SPAWN_WARN_LIVE_SESSIONS,
                worktree.name,
                pending
            );
            let _ = app.emit(
                "event-spawn-warning",
                serde_json::json!({
                    "worktreeId": worktree_id,
                    "worktreeName": worktree.name,
                    "liveSessions": projected_live,
                    "threshold": SPAWN_WARN_LIVE_SESSIONS,
                    "pending": pending,
                }),
            );
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        // 購読を張ったエージェントの会話を引き継ぐ。引けなくても spawn は続行する
        // （新規セッションになるだけで、初期プロンプトは同じように効く）。
        let resume_session = match event_db::latest_agent_session_for_worktree(pool, &worktree_id)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[delivery] resume 用の session 取得に失敗: {}", e);
                None
            }
        };
        let command = agent_command_for_worktree(&settings, worktree, resume_session.as_deref());
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
        state.inflight_spawn.insert(
            worktree_id,
            InflightSpawn {
                request_id,
                requested_at: Instant::now(),
                resume_session,
            },
        );
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

    fn skip_reason(d: PushDecision) -> &'static str {
        match d {
            PushDecision::Skip(reason) => reason,
            PushDecision::Push => panic!("押し込まない想定"),
        }
    }

    /// 「待っても解消しない」見送り理由が `skip_is_terminal` で拾えること。
    ///
    /// `push_pending` はこの判定で「タブは生きているが受け取れない」を見分け、未読を
    /// 同じワークツリーの別タブへ移す。理由文字列を書き換えると**静かに救済が止まる**。
    #[test]
    fn test_terminal_skip_reasons_are_classified() {
        // claude が終了して素のシェルに戻ったタブ
        let mut shell = session(Some("claude"), Some("idle"), true);
        shell.is_ai_agent = false;
        shell.agent_name = None;
        let reason = skip_reason(decide_push(&shell, event_db::DELIVERY_TURN_END, 1_000_000));
        assert_eq!(reason, SKIP_NO_AGENT);
        assert!(skip_is_terminal(reason));

        // claude は居るが session ファイルが読めない（状態が永久に不明）
        let unknown = session(Some("claude"), None, true);
        let reason = skip_reason(decide_push(&unknown, event_db::DELIVERY_TURN_END, 1_000_000));
        assert_eq!(reason, SKIP_STATUS_UNKNOWN);
        assert!(skip_is_terminal(reason));
    }

    /// 一時的な理由は移送の対象にしない（待てば解消する）。
    #[test]
    fn test_transient_skip_reasons_are_not_terminal() {
        let busy = session(Some("claude"), Some("busy"), true);
        assert!(!skip_is_terminal(skip_reason(decide_push(
            &busy,
            event_db::DELIVERY_TURN_END,
            1_000_000
        ))));

        let noisy = session(Some("claude"), Some("idle"), false);
        assert!(!skip_is_terminal(skip_reason(decide_push(
            &noisy,
            event_db::DELIVERY_TURN_END,
            1_000_000
        ))));
    }

    /// Claude Code の `status: "waiting"`（人間の入力待ち）は押し込まないが、
    /// 走行中とは理由を分ける。プロンプトへの回答に化けるので押してはいけないが、
    /// 「走行中」と同じ文言だと切り分けを誤らせる。
    #[test]
    fn test_waiting_for_input_has_its_own_reason() {
        let s = session(Some("claude"), Some("waiting"), true);
        let reason = skip_reason(decide_push(&s, event_db::DELIVERY_TURN_END, 1_000_000));
        assert_eq!(reason, SKIP_WAITING_INPUT);
        // 人間が答えれば解消するので別タブへ移さない
        assert!(!skip_is_terminal(reason));
    }

    /// `interrupt` でも人間の入力待ちには割り込まない（プロンプトを壊す）。
    #[test]
    fn test_interrupt_does_not_answer_a_human_prompt() {
        let s = session(Some("claude"), Some("waiting"), true);
        assert!(!is_push(decide_push(&s, event_db::DELIVERY_INTERRUPT, 1_000_000)));
    }

    /// `SessionStart` だけはターンを開始しない = `delivered_at` を打ってはいけない。
    #[test]
    fn test_only_session_start_does_not_start_turn() {
        assert!(!DigestReason::SessionStart.starts_turn());
        assert!(DigestReason::TurnEnd { prompt_id: None }.starts_turn());
        assert!(DigestReason::PromptSubmit.starts_turn());
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

    // ─── 自動 spawn のコマンド組み立て（#137） ────────────────────────────────

    /// 初期プロンプトが必ず載ること。**これが無いとエージェントは何もしない**
    /// （SessionStart の additionalContext はターンを開始させない）。
    #[test]
    fn test_spawn_command_always_carries_initial_prompt() {
        let settings = AppSettings::default();
        let cmd = agent_command_for_worktree(&settings, &worktree(None), None);
        assert!(cmd.starts_with("claude "), "{}", cmd);
        assert!(cmd.contains("--permission-mode plan"), "{}", cmd);
        assert!(cmd.contains("oretachi_poll_inbox"), "{}", cmd);
        assert!(!cmd.contains("--resume"), "session が無ければ付けない: {}", cmd);
    }

    /// session UUID があれば `--resume` でその会話の続きとして立ち上げる。
    #[test]
    fn test_spawn_command_resumes_subscriber_session() {
        let settings = AppSettings::default();
        let cmd = agent_command_for_worktree(
            &settings,
            &worktree(None),
            Some("3d1ac64b-36ee-4f56-92fa-bc170cb0043f"),
        );
        assert!(
            cmd.contains("--resume 3d1ac64b-36ee-4f56-92fa-bc170cb0043f"),
            "{}",
            cmd
        );
        assert!(cmd.contains("oretachi_poll_inbox"), "{}", cmd);
    }

    /// 組み立てた文字列はシェルへそのまま流れるので、素性の怪しい session ID は落とす。
    /// 落としても spawn は続ける（`--resume` が付かないだけ）。
    #[test]
    fn test_spawn_command_rejects_unsafe_session_id() {
        let settings = AppSettings::default();
        for evil in ["a\" ; rm -rf /", "a b", "a$(id)", "", &"x".repeat(65)] {
            let cmd = agent_command_for_worktree(&settings, &worktree(None), Some(evil));
            assert!(!cmd.contains("--resume"), "evil={:?} cmd={}", evil, cmd);
            assert!(cmd.contains("oretachi_poll_inbox"), "{}", cmd);
        }
        assert!(is_safe_session_id("3d1ac64b-36ee-4f56-92fa-bc170cb0043f"));
    }

    /// プロンプトを囲む二重引用符を、プロンプト自身が壊さないこと。
    #[test]
    fn test_spawn_initial_prompt_has_no_quote_chars() {
        assert!(!SPAWN_INITIAL_PROMPT.contains('"'));
        assert!(!SPAWN_INITIAL_PROMPT.contains('\''));
        assert!(!SPAWN_INITIAL_PROMPT.contains('\r'));
        assert!(!SPAWN_INITIAL_PROMPT.contains('\n'));
    }

    #[test]
    fn test_auto_approval_allows_canned_kinds_only() {
        let on = worktree(Some(true));
        // oretachi 自身が定型 JSON で組み立てるイベントは許可
        assert!(auto_approval_allows(Some(&on), event_db::KIND_WORKTREE_CLOSED));
        assert!(auto_approval_allows(Some(&on), event_db::KIND_WORKTREE_CREATED));
        // `worktree.message` は本文を運ばず告知だけを押し込むようにしたので許可（#137）。
        // 押し込まれる文字列は `format_pointer_text` の固定文で、発信側は書けない
        assert!(auto_approval_allows(Some(&on), event_db::KIND_WORKTREE_MESSAGE));
        // 知らない種別は default-deny。`format_inbox_line` の `other` 分岐が本文を
        // 丸ごと展開するので、許可リストに足さない限り自動承認宛には出さない
        assert!(!auto_approval_allows(Some(&on), "worktree.unknown"));
    }

    /// 許可リストと「本文をインラインで運ぶ種別」は別の軸で、**両方の判断が要る**。
    /// `worktree.message` は「押し込んでよい」が「本文は運ばない」側にいる。
    #[test]
    fn test_pushable_kinds_and_inline_body_are_separate_axes() {
        for kind in [event_db::KIND_WORKTREE_CLOSED, event_db::KIND_WORKTREE_CREATED] {
            assert!(AUTO_APPROVAL_PUSHABLE_KINDS.contains(&kind));
            assert!(!event_db::is_free_text_kind(kind), "定型種別は本文を運ぶ");
        }
        assert!(AUTO_APPROVAL_PUSHABLE_KINDS.contains(&event_db::KIND_WORKTREE_MESSAGE));
        assert!(
            event_db::is_free_text_kind(event_db::KIND_WORKTREE_MESSAGE),
            "message は押し込めるが本文は運ばない"
        );
        // 未知種別は両方で拒否側
        assert!(!AUTO_APPROVAL_PUSHABLE_KINDS.contains(&"worktree.unknown"));
        assert!(event_db::is_free_text_kind("worktree.unknown"));
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
    /// 同じタブに許可種別と拒否種別が混ざっていても**拒否種別だけが保留**され、残りは通る。
    #[test]
    fn test_auto_approval_partition_blocks_only_free_form() {
        let on = worktree(Some(true));
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_MESSAGE),
            item("c", event_db::DELIVERY_TURN_END, "worktree.unknown"),
        ];
        let (allowed, blocked): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|i| auto_approval_allows(Some(&on), &i.kind));
        assert_eq!(ids(&allowed), vec!["a", "b"]);
        assert_eq!(ids(&blocked), vec!["c"]);
    }

    /// 保留された行は配送戦略の決定に影響してはいけない。`interrupt` な拒否種別が
    /// 混ざっているだけで `turn_end` の許可種別が走行中のエージェントへ
    /// 割り込む、という取り違えを固定する（`push_pending` は仕分け後に
    /// `strongest_delivery` を取る）。
    #[test]
    fn test_blocked_items_do_not_escalate_delivery() {
        let on = worktree(Some(true));
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_INTERRUPT, "worktree.unknown"),
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

    /// `spawn_for_closed_tabs` の判定と同じ形。押し込めない種別の未読しか無い宛先では、
    /// 自動承認が有効なら spawn しない（押し込めない未読を根拠にタブを立てない）。
    ///
    /// `worktree.message` は #137 で許可側へ移った。spawn したタブへ出るのも
    /// `format_pointer_text` の告知だけなので、押し込みと同じ基準で通してよい。
    #[test]
    fn test_spawn_pending_count_ignores_blocked_kinds() {
        let on = worktree(Some(true));
        let off = worktree(Some(false));
        let by_kind = vec![
            ("worktree.unknown".to_string(), 3i64),
            (event_db::KIND_WORKTREE_CLOSED.to_string(), 2i64),
            (event_db::KIND_WORKTREE_MESSAGE.to_string(), 4i64),
        ];
        let count = |wt: &WorktreeEntry| -> i64 {
            by_kind
                .iter()
                .filter(|(kind, _)| auto_approval_allows(Some(wt), kind))
                .map(|(_, c)| *c)
                .sum()
        };
        assert_eq!(count(&on), 6, "自動承認 ON では未知種別を数えない");
        assert_eq!(count(&off), 9, "自動承認 OFF なら従来どおり全部数える");

        // 拒否種別だけの宛先は spawn 対象にならない（pending == 0）
        let only_blocked = vec![("worktree.unknown".to_string(), 3i64)];
        let pending: i64 = only_blocked
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
            notified_at: None,
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
    /// 許可リストに無い種別がその場で default-deny に落ちるのが目的。
    #[test]
    fn test_filter_for_turn_end_respects_auto_approval() {
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_TURN_END, "worktree.unknown"),
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
        let mut a = item("a", event_db::DELIVERY_TURN_END, "worktree.unknown");
        a.subscriber_worktree_id = Some("wt-manual".to_string());
        let mut b = item("b", event_db::DELIVERY_TURN_END, "worktree.unknown");
        b.subscriber_worktree_id = Some("wt-auto".to_string());

        assert_eq!(ids(&filter_for_turn_end(vec![a, b], &settings)), vec!["a"]);
    }

    /// **自動承認のガードは全注入経路に掛かること。** `Stop` だけに掛けていると、
    /// `SessionStart` / `UserPromptSubmit` の additionalContext から拒否種別が素通りし、
    /// 自動承認が有効な宛先で人間の確認なしにツール実行へつながる（#126）。
    /// `passive` の除外だけが `Stop` 固有。
    #[test]
    fn test_auto_approval_filter_applies_to_all_injection_paths() {
        let items = vec![
            item("a", event_db::DELIVERY_TURN_END, event_db::KIND_WORKTREE_CLOSED),
            item("b", event_db::DELIVERY_TURN_END, "worktree.unknown"),
            item("c", event_db::DELIVERY_PASSIVE, event_db::KIND_WORKTREE_CLOSED),
        ];
        let auto_on = settings_with(Some(true));
        // SessionStart / UserPromptSubmit 経路: 拒否種別は落とし、passive は回収する
        assert_eq!(
            ids(&filter_auto_approval(items.clone(), &auto_on)),
            vec!["a", "c"]
        );
        // Stop 経路: 拒否種別に加えて passive も落とす
        assert_eq!(ids(&filter_for_turn_end(items.clone(), &auto_on)), vec!["a"]);
        // 自動承認 OFF なら拒否種別も通る（passive の扱いだけ経路で違う）
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
        let mut a = item("a", event_db::DELIVERY_TURN_END, "worktree.unknown");
        a.subscriber_worktree_id = Some("wt-gone".to_string());
        let b = item("b", event_db::DELIVERY_TURN_END, "worktree.unknown"); // wt-1 = 自動承認 ON
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
