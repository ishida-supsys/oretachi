use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const AI_AGENT_NAMES: &[&str] = &["claude", "gemini", "codex", "cline"];
/// AI エージェントを探すサブツリーの深さ。
///
/// シェル直下に `claude.exe` が来る構成なら 1 で足りるが、npm shim 経由だと
/// `pwsh → cmd.exe → node.exe → …` と伸びる。プロセス一覧は Win32 API の直読みなので
/// 深く辿ってもコストは無視できる（旧実装は 10 秒ごとに `wmic` を spawn していた）。
const AGENT_SEARCH_DEPTH: u32 = 8;
const MAX_PTY_SESSIONS: usize = 32;
const OUTPUT_HISTORY_BYTES: usize = 65_536;
/// シェル本体が自然終了したセッションを map に残しておく寿命。
/// この期間内なら MCP クライアントから exit code や最終ログを参照できる。
/// MAX_PTY_SESSIONS の枠を一時的に圧迫し得るが、TTL 経過後は lazy sweep で除去される。
const EXITED_SESSION_TTL: Duration = Duration::from_secs(30);

struct PtySession {
    /// oretachi が spawn 時に発番する UUID。タブと 1:1 で、`ORETACHI_TERMINAL_ID` として
    /// PTY の子プロセス（シェル → エージェント → hook）へ env 継承される。
    /// hook 通知や MCP からの同定に使う。`session_id` (u32) と違いアプリ再起動を跨いで衝突しない。
    terminal_id: String,
    /// PTY 入力の送信キュー。書き込み本体はセッション毎の writer スレッドが行う。
    /// ConPTY の入力パイプへの write は子プロセスが stdin を読まないと無期限に
    /// ブロックしうるため、Tauri コマンド（メインスレッド）からは enqueue のみ行い、
    /// 実 I/O をメインスレッドから隔離する。全 Sender drop で writer スレッドは終了する。
    /// 有界 (INPUT_QUEUE_MAX_CHUNKS) で、満杯時の write() は即時エラーになる。
    input_tx: std::sync::mpsc::SyncSender<Vec<u8>>,
    master: Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>>,
    child_killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    child_pid: Option<u32>,
    alive: Arc<Mutex<bool>>,
    watcher_handle: Option<std::thread::JoinHandle<()>>,
    is_ai_agent: bool,
    /// ポーリング (`start_polling`) が検出した AI エージェント名（"claude" 等）。未検出なら `None`。
    agent_name: Option<String>,
    /// `agent_name == "claude"` のときポーリングが取得する Claude Code のセッション UUID。
    /// hook の stdin JSON の `session_id` と同一値（#120 で実測済み）。
    agent_session_id: Option<String>,
    /// `~/.claude/sessions/<pid>.json` の `status`（`"busy"` | `"idle"`）。Claude Code 以外の
    /// エージェントには対応するファイルが無いので常に `None`（#125 の PTY 押し込み判定用）。
    agent_status: Option<String>,
    /// 上の値をポーリングがサンプルした時刻（epoch ms）。鮮度判定はこちらで行う。
    agent_status_sampled_at: Option<i64>,
    /// 直前のポーリング tick から PTY 出力が1バイトも増えていないか。
    /// `idle` でも最大10秒古いので、押し込み直前の第二の安全弁として使う
    /// （人間が入力中の行にブラケットペーストを重ねて破壊するのを避ける）。
    output_quiescent: bool,
    cwd: Option<String>,
    output_history: Arc<Mutex<VecDeque<u8>>>,
    /// flush ループへ渡す未配送バッファ。reader が append し、16ms 周期の flush が
    /// drain して 1 回にまとめて emit する（チャンク毎 emit による WebView2 飽和を防ぐ）。
    output_pending: Arc<Mutex<VecDeque<u8>>>,
    /// reader 経由で書き込まれた累積バイト数。MCP の差分読み (cursor) の起点として参照する。
    total_bytes_written: Arc<AtomicU64>,
    /// プロセス全体（PTY が走らせているシェル本体）の exit code。watcher が拾う。
    exit_status: Arc<Mutex<Option<i64>>>,
    /// 直近コマンドの exit code（シェル統合の OSC 777 を reader thread が拾って保存）。
    last_command_exit_code: Arc<Mutex<Option<i64>>>,
    /// シェル本体が自然終了した時刻。`Some` ならゾンビ状態（map 上は残っているが死亡）。
    /// `EXITED_SESSION_TTL` 経過後の lazy sweep で除去される。
    exited_at: Arc<Mutex<Option<Instant>>>,
}

/// 寿命の切れた exited セッションを `sessions` map から除去する。
/// `sessions.lock()` を取った直後の各 path から呼び出すことで、MCP クライアントが
/// 最大 `EXITED_SESSION_TTL` の間は exit code / 最終ログを参照できる状態を保つ。
fn sweep_exited(map: &mut HashMap<u32, PtySession>) {
    map.retain(|_, s| {
        let exited = s.exited_at.lock().ok().and_then(|g| *g);
        match exited {
            None => true,
            Some(t) => t.elapsed() < EXITED_SESSION_TTL,
        }
    });
}

/// 1 回の flush で emit する保留出力の上限。これを超えた分は次周期へ持ち越す（バックプレッシャ）。
const MAX_FLUSH_BYTES: usize = 256 * 1024;
/// 未配送バッファ（`output_pending`）が保持する最大バイト数。
/// drain 速度（256KB/16ms ≒ 16MB/s）を持続的に上回る出力ではバッファが無制限に増大して
/// メモリを食い潰すため、上限超過時は最古を捨てる。直近の出力は `output_history`（64KB）が
/// 別途保持するため MCP の差分読みには影響しない（極端な過負荷時に画面表示の取りこぼしに留まる）。
const MAX_PENDING_BYTES: usize = 8 * 1024 * 1024;

/// PTY 入力キュー（writer スレッドへの sync_channel）の最大チャンク数。
/// 子プロセスが stdin を読まない場合の無制限なメモリ蓄積を防ぐ。
/// 満杯時は write() が即時エラーを返す（旧実装の write ブロックに相当する
/// バックプレッシャをエラーとして可視化する）。
const INPUT_QUEUE_MAX_CHUNKS: usize = 1024;

/// セッションの保留バッファを最大 `MAX_FLUSH_BYTES` drain し、base64 エンコードして
/// `pty-output` を 1 回 emit する。保留が空なら何もしない。
/// flush ループと reader 終了時の最終 flush の双方から呼ばれる。
///
/// drain と emit を **同一の lock critical section で行う**。flush ループと reader 最終 flush は
/// 同じ session の `output_pending` に対して並行に本関数を呼びうるため、drain だけをロックで
/// 直列化して emit をロック外に出すと「A が先に drain・B が先に emit」となり出力チャンクの
/// 順序が逆転する／drain 済みだが未 emit のチャンクを残したまま reader が `pty-exit` を
/// 先行 emit してしまう。lock を emit まで保持すれば FIFO の drain 順 = emit 順が保証され、
/// reader が `remaining == 0` を観測した時点で全 drain 済みチャンクは emit 済みになる。
fn flush_session_output(app: &AppHandle, session_id: u32, pending: &Arc<Mutex<VecDeque<u8>>>) {
    let mut pend = match pending.lock() {
        Ok(p) => p,
        Err(e) => e.into_inner(),
    };
    if pend.is_empty() {
        return;
    }
    let take = pend.len().min(MAX_FLUSH_BYTES);
    let chunk: Vec<u8> = pend.drain(..take).collect();
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(&chunk);
    // lock 保持中に emit して drain↔emit を不可分にする（順序保証のため）。
    // emit はイベントをキューに載せるだけで pty_manager に同期再入しないため、deadlock しない。
    let _ = app.emit("pty-output", PtyOutputPayload { session_id, data });
}

#[derive(Clone)]
pub struct SessionInfo {
    pub session_id: u32,
    /// spawn 時に発番した UUID（`ORETACHI_TERMINAL_ID`）。hook 側が運んでくる値と突合できる。
    pub terminal_id: String,
    pub cwd: Option<String>,
    pub is_ai_agent: bool,
    pub agent_name: Option<String>,
    pub agent_session_id: Option<String>,
    /// `"busy"` | `"idle"`。Claude Code 以外は `None`（#125）
    pub agent_status: Option<String>,
    /// `agent_status` をサンプルした時刻（epoch ms）
    pub agent_status_sampled_at: Option<i64>,
    /// 直前の tick から PTY 出力が増えていない
    pub output_quiescent: bool,
    pub exit_code: Option<i64>,
    pub last_command_exit_code: Option<i64>,
}

pub struct ReadHistoryResult {
    pub data: Vec<u8>,
    /// data 末尾に対応する累積バイト位置。次回呼び出しで `from_cursor` に渡せば差分が取れる。
    pub cursor: u64,
    /// 要求 cursor がリングバッファ範囲外だったときに失われた先頭バイト数。
    pub lost_bytes: u64,
}

/// PtyManager の実体。Drop 時に kill_all を行うため、Clone される外殻 (`PtyManager`)
/// とは分離し、最後の参照が消えたときだけ一度 Drop が走るようにする。
pub struct PtyManagerCore {
    sessions: Arc<Mutex<HashMap<u32, PtySession>>>,
    next_id: Mutex<u32>,
    polling_alive: Arc<Mutex<bool>>,
}

/// Tauri の State として管理する PTY マネージャ。
/// `Arc` の newtype なので clone が安価で、async コマンドから
/// `tauri::async_runtime::spawn_blocking` へ move して使える ('static 化)。
#[derive(Clone)]
pub struct PtyManager(Arc<PtyManagerCore>);

impl PtyManager {
    pub fn new() -> Self {
        PtyManager(Arc::new(PtyManagerCore {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_id: Mutex::new(1),
            polling_alive: Arc::new(Mutex::new(true)),
        }))
    }
}

impl std::ops::Deref for PtyManager {
    type Target = PtyManagerCore;
    fn deref(&self) -> &PtyManagerCore {
        &self.0
    }
}

#[derive(serde::Serialize, Clone)]
pub struct PtyOutputPayload {
    #[serde(rename = "sessionId")]
    pub session_id: u32,
    /// base64 エンコードした PTY 出力。number[] (Vec<u8>) のままだと巨大な eval 文字列に
    /// なり WebView2 IPC を飽和させるため、サイズを 1/3〜1/4 に圧縮して送る。
    pub data: String,
}

#[derive(serde::Serialize, Clone)]
pub struct PtyExitPayload {
    #[serde(rename = "sessionId")]
    pub session_id: u32,
}

#[derive(serde::Serialize, Clone)]
pub struct AiAgentInfo {
    #[serde(rename = "isAgent")]
    pub is_agent: bool,
    #[serde(rename = "agentName")]
    pub agent_name: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub struct AiAgentChangedPayload {
    /// pty_session_id → AiAgentInfo のマップ
    pub sessions: HashMap<u32, AiAgentInfo>,
}

/// 指定PIDのサブツリーからAIエージェントプロセスを探す（最大depth段）
/// 見つかった場合は (agent_name, agent_pid) を返す
fn find_ai_agent_in_subtree(
    root_pid: u32,
    children_map: &HashMap<u32, Vec<(u32, String)>>,
    depth: u32,
) -> Option<(String, u32)> {
    if depth == 0 {
        return None;
    }
    if let Some(children) = children_map.get(&root_pid) {
        for (child_pid, child_name) in children {
            let name_lower = child_name.to_lowercase();
            // 拡張子は全部落として比較する。`claude.exe` だけでなく npm / scoop が置く
            // `claude.cmd` / `claude.ps1` のような shim もエージェント本体として扱う
            // （shim が居るなら実体もその配下に居るが、実体は `node.exe` 等で名前から
            // 判別できないため、shim の時点で確定させる）。
            let name_stem = name_lower.split('.').next().unwrap_or(&name_lower);
            if AI_AGENT_NAMES.contains(&name_stem) {
                return Some((name_stem.to_string(), *child_pid));
            }
            if let Some(found) = find_ai_agent_in_subtree(*child_pid, children_map, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// `~/.claude/sessions/<pid>.json` から読み取る値。
pub struct ClaudeSessionFile {
    /// hook の stdin JSON の `session_id` と同一値（#120 で実測済み）
    pub session_id: Option<String>,
    /// `"busy"` | `"idle"` | `"waiting"`（人間の入力待ち）。走行中 / 待機中の判定に使う
    /// （#121 で実測。`waiting` は Claude Code 2.1.234 で観測）
    pub status: Option<String>,
    /// `"interactive"` など。子孫まで辿って拾ったファイルが**タブ本体のものか**を見分けるのに使う
    /// （`find_claude_session_in_subtree` 参照）。古いバージョンでは無いことがある
    pub kind: Option<String>,
}

/// Claude Code の PID から ~/.claude/sessions/<pid>.json を読む。
///
/// `status` は「走行中か待機中か」の唯一の一次情報。ファイル側の `statusUpdatedAt` は
/// 鮮度判定に**使わない** —— 長いターンは正当に古い `busy` を残すので「古い＝不明」と
/// 扱うと押し込んではいけない場面で押し込むことになる。鮮度はこちらがサンプルした
/// 時刻（`agent_status_sampled_at`）で見る。
fn read_claude_session_file(pid: u32) -> Option<ClaudeSessionFile> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let session_file = std::path::Path::new(&home)
        .join(".claude")
        .join("sessions")
        .join(format!("{}.json", pid));
    let content = std::fs::read_to_string(&session_file).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(ClaudeSessionFile {
        session_id: v.get("sessionId").and_then(|s| s.as_str()).map(str::to_string),
        status: v.get("status").and_then(|s| s.as_str()).map(str::to_string),
        kind: v.get("kind").and_then(|s| s.as_str()).map(str::to_string),
    })
}

/// `claude` として検出した PID とその子孫から、最初に見つかった session ファイルを返す。
///
/// `~/.claude/sessions/<pid>.json` を書くのは Claude Code の**本体プロセス**だが、
/// 検出で当たるのはその手前の shim（`claude.cmd` → `cmd.exe`）や
/// ランチャ（`claude.exe` → 実体）であることがある。検出した PID だけを見ると
/// `status` が取れず、`decide_push` が「エージェントの状態が不明」で押し込みを見送る。
/// 子孫まで辿れば idle 判定を諦めずに済む。
///
/// **幅優先で探す。** タブから `claude -p` を起動しているとその子も自分の session ファイルを
/// 書くため、深さ優先だとサブエージェントの `status` を本体のものと誤認しうる。浅い側ほど
/// 本体に近いので、同じ深さなら先に見つかったものを採る。
/// なお本体自身のファイルが読めるなら（`claude.exe` 直下の通常構成）そこで確定するので、
/// この探索に入るのは shim 経由でファイルが見つからなかった場合だけ。
fn find_claude_session_in_subtree(
    pid: u32,
    children_map: &HashMap<u32, Vec<(u32, String)>>,
    depth: u32,
) -> Option<ClaudeSessionFile> {
    // 検出した PID 自身のファイルが読めればそれが正解。ここで確定するのが通常構成
    // （`claude.exe` がシェルの直下に居る）。
    if let Some(file) = read_claude_session_file(pid) {
        return Some(file);
    }
    let mut frontier = children_of(pid, children_map);
    for _ in 0..depth {
        if frontier.is_empty() {
            return None;
        }
        let mut next = Vec::new();
        for p in &frontier {
            match read_claude_session_file(*p) {
                // **対話セッションのファイルだけ採る。** `claude -p` などの非対話セッションは
                // 自分の `status` を書くので、そのまま採ると本体が走行中でも `idle` に見えて
                // 走行中のエージェントへ押し込みうる。`kind` が無い旧バージョンは通す。
                Some(file)
                    if file.kind.as_deref().map(|k| k == "interactive").unwrap_or(true) =>
                {
                    log::debug!(
                        "[Terminal] エージェント pid={} の session ファイルが無いので子孫 pid={} のものを使う",
                        pid,
                        p
                    );
                    return Some(file);
                }
                _ => {}
            }
            next.extend(children_of(*p, children_map));
        }
        frontier = next;
    }
    None
}

fn children_of(pid: u32, children_map: &HashMap<u32, Vec<(u32, String)>>) -> Vec<u32> {
    children_map
        .get(&pid)
        .map(|c| c.iter().map(|(child_pid, _)| *child_pid).collect())
        .unwrap_or_default()
}

/// UNIX epoch からのミリ秒（`event_db::now_ms` と同じ流儀。依存を増やさないため再実装）。
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl PtyManagerCore {
    /// AIエージェントフラグを明示的にセットする（executeAgentWorktree 用）
    pub fn set_ai_agent(&self, session_id: u32, is_agent: bool) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
        sweep_exited(&mut sessions);
        if let Some(session) = sessions.get_mut(&session_id) {
            session.is_ai_agent = is_agent;
            Ok(())
        } else {
            Err(format!("Session {} not found", session_id))
        }
    }

    /// AIエージェントフラグを参照する
    pub fn is_ai_agent(&self, session_id: u32) -> Result<bool, String> {
        let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
        sweep_exited(&mut sessions);
        if let Some(session) = sessions.get(&session_id) {
            Ok(session.is_ai_agent)
        } else {
            Err(format!("Session {} not found", session_id))
        }
    }

    /// 各セッションの保留出力を 16ms 周期でまとめて emit する flush ループを起動する。
    /// reader スレッドのチャンク毎 emit を置き換え、emit 頻度を出力量と無関係に
    /// 約 62 回/秒/セッションへ上限化して WebView2 IPC の飽和（ハング）を防ぐ。
    pub fn start_output_flush(&self, app_handle: AppHandle) {
        let sessions_arc = self.sessions.clone();
        let polling_alive = self.polling_alive.clone();

        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(16));

                if !*polling_alive.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }

                // sessions ロックは Arc clone のみ（瞬時）→ flush 中は他セッション操作をブロックしない
                let pendings: Vec<(u32, Arc<Mutex<VecDeque<u8>>>)> = {
                    let sessions = match sessions_arc.lock() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    sessions
                        .iter()
                        .map(|(&id, s)| (id, s.output_pending.clone()))
                        .collect()
                };

                for (session_id, pending) in pendings {
                    flush_session_output(&app_handle, session_id, &pending);
                }
            }
        });
    }

    /// バックグラウンドでポーリングスレッドを起動し、AIエージェント状態の変化をイベントで通知する
    pub fn start_polling(&self, app_handle: AppHandle) {
        let sessions_arc = self.sessions.clone();
        let polling_alive = self.polling_alive.clone();

        std::thread::spawn(move || {
            // (is_agent, session_id) のペアで差分検出
            let mut last_status: HashMap<u32, (bool, Option<String>)> = HashMap::new();
            // 前 tick の PTY 累積出力量。押し込み前の静穏判定に使う
            let mut last_bytes: HashMap<u32, u64> = HashMap::new();
            // 直前の tick でプロセス列挙に失敗したか（ログを溢れさせないための抑止）
            let mut scan_failed_before = false;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));

                if !*polling_alive.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }

                // セッション情報を取得（terminal_id と出力量も一緒に拾う）
                let session_pids: Vec<(u32, Option<u32>, String, u64)> = {
                    let mut sessions = match sessions_arc.lock() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    sweep_exited(&mut sessions);
                    sessions
                        .iter()
                        .map(|(&id, s)| {
                            (
                                id,
                                s.child_pid,
                                s.terminal_id.clone(),
                                s.total_bytes_written.load(std::sync::atomic::Ordering::Relaxed),
                            )
                        })
                        .collect()
                };

                // 生存タブの terminal_id を購読側へ通知して、死んだタブの購読を orphaned に
                // 落とす（#125）。exited セッションも EXITED_SESSION_TTL の間はここに含まれる。
                // 一瞬の再起動で orphaned へフラップさせないよう、あえて除外しない。
                crate::event_delivery::reconcile_live_terminals(
                    &app_handle,
                    session_pids.iter().map(|(_, _, tid, _)| tid.clone()).collect(),
                );

                if session_pids.is_empty() {
                    last_status.clear();
                    last_bytes.clear();
                    continue;
                }

                // プロセス一覧を一括取得して子プロセスマップを構築
                let all_procs = crate::process_utils::scan_all_processes();
                // **列挙に失敗した tick は状態を更新しない。** 空を「子プロセスが居ない」と
                // 解釈すると全タブが `is_ai_agent = false` に落ち、`decide_push` が
                // 「AI エージェントが走っていない」で押し込みを見送り続ける。それが 10 分
                // （`event_db::PUSH_TTL_MS`）続くと未読は押し込み候補から永久に外れる。
                // 最終値を保持して次の tick に任せるほうが安全。
                if all_procs.is_empty() {
                    if scan_failed_before {
                        log::debug!("[Terminal] プロセス一覧の列挙に失敗（継続中）");
                    } else {
                        log::warn!(
                            "[Terminal] プロセス一覧の列挙に失敗したため AI エージェント状態を更新しない（前回値を保持）"
                        );
                    }
                    scan_failed_before = true;
                    continue;
                }
                if scan_failed_before {
                    log::info!("[Terminal] プロセス一覧の列挙が回復した");
                    scan_failed_before = false;
                }
                let mut children_map: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
                for (pid, ppid, name) in &all_procs {
                    children_map.entry(*ppid).or_default().push((*pid, name.clone()));
                }

                // pty_session_id → AiAgentInfo のマップを構築
                let mut current_status: HashMap<u32, (bool, Option<String>)> = HashMap::new();
                let mut current_bytes: HashMap<u32, u64> = HashMap::new();
                // (session_id, is_agent, info, claude_status, quiescent, terminal_id)
                let mut new_infos: Vec<(u32, bool, AiAgentInfo, Option<String>, bool, String)> =
                    Vec::new();
                let sampled_at = now_ms();

                for (session_id, child_pid, terminal_id, bytes) in session_pids {
                    let (is_agent, info, claude_status) = if let Some(pid) = child_pid {
                        match find_ai_agent_in_subtree(pid, &children_map, AGENT_SEARCH_DEPTH) {
                            Some((agent_name, agent_pid)) => {
                                // Claude Code 以外（gemini / codex / cline）は
                                // ~/.claude/sessions/<pid>.json を持たないため status を取れない。
                                // 「非 CC は busy/idle 判定不能」という #125 §5 の仕様の実体がここ。
                                let file = if agent_name == "claude" {
                                    find_claude_session_in_subtree(
                                        agent_pid,
                                        &children_map,
                                        AGENT_SEARCH_DEPTH,
                                    )
                                } else {
                                    None
                                };
                                let (sid, status) = match file {
                                    Some(f) => (f.session_id, f.status),
                                    None => (None, None),
                                };
                                (
                                    true,
                                    AiAgentInfo {
                                        is_agent: true,
                                        agent_name: Some(agent_name),
                                        session_id: sid,
                                    },
                                    status,
                                )
                            }
                            None => (
                                false,
                                AiAgentInfo { is_agent: false, agent_name: None, session_id: None },
                                None,
                            ),
                        }
                    } else {
                        (
                            false,
                            AiAgentInfo { is_agent: false, agent_name: None, session_id: None },
                            None,
                        )
                    };
                    let quiescent = last_bytes.get(&session_id) == Some(&bytes);
                    current_bytes.insert(session_id, bytes);
                    let session_id_val = info.session_id.clone();
                    current_status.insert(session_id, (is_agent, session_id_val));
                    new_infos.push((session_id, is_agent, info, claude_status, quiescent, terminal_id));
                }

                // 内部状態を更新。
                // exited セッション（EXITED_SESSION_TTL の間 map に残る死体）は子プロセスが
                // 既に居ないため検出結果が必ず「エージェント無し」になる。TTL 中は exit code /
                // 最終ログを参照できるという既存方針に合わせ、エージェント情報も最終値を凍結する。
                let mut exited_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
                if let Ok(mut sessions) = sessions_arc.lock() {
                    for (id, is_agent, info, status, quiescent, _) in &new_infos {
                        if let Some(session) = sessions.get_mut(id) {
                            if session.exited_at.lock().ok().and_then(|g| *g).is_some() {
                                exited_ids.insert(*id);
                                continue;
                            }
                            session.is_ai_agent = *is_agent;
                            session.agent_name = info.agent_name.clone();
                            session.agent_session_id = info.session_id.clone();
                            session.agent_status = status.clone();
                            session.agent_status_sampled_at = Some(sampled_at);
                            session.output_quiescent = *quiescent;
                        }
                    }
                }

                // AI エージェントが立ち上がったタブは、そのワークツリーの引き継ぎ待ち購読を
                // 受け取れる。SessionStart フックが無いエージェント（gemini / codex / cline）や
                // ユーザーが手で `claude` と打った場合もここで拾える（最大10秒の遅延）。
                for (id, is_agent, _, _, _, terminal_id) in &new_infos {
                    if !*is_agent || exited_ids.contains(id) {
                        continue;
                    }
                    let was_agent = last_status.get(id).map(|(a, _)| *a).unwrap_or(false);
                    if !was_agent {
                        crate::event_delivery::request_rebind(&app_handle, terminal_id.clone());
                    }
                }

                // 前回との差分を検出（is_agent または session_id が変わった場合）。
                // 凍結した exited セッションは emit からも外し、フロントの表示と
                // バックエンドが保持する最終値が食い違わないようにする
                // （タブ自体は pty-exit で既に消えているので通知する相手も居ない）。
                let changed: HashMap<u32, AiAgentInfo> = new_infos
                    .into_iter()
                    .filter(|(id, _, info, _, _, _)| {
                        if exited_ids.contains(id) {
                            return false;
                        }
                        let prev = last_status.get(id);
                        match prev {
                            None => true,
                            Some((prev_is, prev_sid)) => {
                                *prev_is != info.is_agent || *prev_sid != info.session_id
                            }
                        }
                    })
                    .map(|(id, _, info, _, _, _)| (id, info))
                    .collect();

                if !changed.is_empty() {
                    let _ = app_handle.emit("pty-ai-agent-changed", AiAgentChangedPayload { sessions: changed });
                }

                last_status = current_status;
                last_bytes = current_bytes;
            }
        });
    }

    pub fn spawn(
        &self,
        app_handle: AppHandle,
        rows: u16,
        cols: u16,
        shell: Option<String>,
        cwd: Option<String>,
    ) -> Result<u32, String> {
        log::debug!("[Terminal] pty_manager::spawn rows={} cols={} shell={:?} cwd={:?}", rows, cols, shell, cwd);
        {
            let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
            sweep_exited(&mut sessions);
            if sessions.len() >= MAX_PTY_SESSIONS {
                return Err(format!(
                    "PTYセッション数の上限（{}）に達しています。不要なターミナルを閉じてください",
                    MAX_PTY_SESSIONS
                ));
            }
        }
        let pty_system = native_pty_system();

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| format!("PTY open error: {}", e))?;

        let shell_cmd = shell.unwrap_or_else(|| {
            #[cfg(target_os = "windows")]
            {
                "powershell.exe".to_string()
            }
            #[cfg(not(target_os = "windows"))]
            {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
            }
        });

        // セッション ID / terminal_id は CommandBuilder より **前** に確定させる。
        // terminal_id を子プロセスの env に載せる必要があるため、spawn 後の採番では間に合わない。
        // spawn に失敗するとこの session_id は空費されるが、単調増加カウンタなので実害はない。
        let session_id = {
            let mut id = self.next_id.lock().map_err(|e| format!("lock error: {}", e))?;
            let current = *id;
            *id += 1;
            current
        };
        let terminal_id = uuid::Uuid::new_v4().to_string();

        let mut cmd = CommandBuilder::new(&shell_cmd);
        cmd.env("TERM", "xterm-256color");
        // タブ（シェル）が親なので、ユーザーが手で `claude` と打っても、MCP 経由で
        // コマンドを流し込んでも、その先の hook まで env として継承される。
        // CC 2.1.207 以降 hook へ oretachi 由来の情報を渡せる経路はこれだけ。
        cmd.env("ORETACHI_TERMINAL_ID", &terminal_id);

        // シェル統合: OSC 777 で終了コードをフロントエンドに通知
        let shell_name = std::path::Path::new(&shell_cmd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();

        if shell_name.contains("bash") {
            // bash: PROMPT_COMMAND 経由で OSC シーケンスを出力
            let existing = std::env::var("PROMPT_COMMAND").unwrap_or_default();
            let hook = r#"printf '\033]777;exit_code;%s\007' "$?""#;
            let prompt_cmd = if existing.is_empty() {
                hook.to_string()
            } else {
                format!("{};{}", hook, existing)
            };
            cmd.env("PROMPT_COMMAND", prompt_cmd);
        } else if shell_name.contains("zsh") {
            // zsh: ZDOTDIR を一時ディレクトリに向けて precmd フックを注入
            let integration_dir = std::env::temp_dir().join("omaera-zsh");
            if std::fs::create_dir_all(&integration_dir).is_ok() {
                let orig_zdotdir = std::env::var("ZDOTDIR").unwrap_or_else(|_| {
                    std::env::var("HOME").unwrap_or_default()
                });
                let zshenv_content =
                    "[ -n \"$OMAERA_ORIG_ZDOTDIR\" ] && [ -f \"$OMAERA_ORIG_ZDOTDIR/.zshenv\" ] && source \"$OMAERA_ORIG_ZDOTDIR/.zshenv\"\n";
                let zshrc_content = concat!(
                    "[ -n \"$OMAERA_ORIG_ZDOTDIR\" ] && [ -f \"$OMAERA_ORIG_ZDOTDIR/.zshrc\" ] && source \"$OMAERA_ORIG_ZDOTDIR/.zshrc\"\n",
                    "__omaera_precmd() { printf '\\033]777;exit_code;%s\\007' \"$?\" }\n",
                    "precmd_functions+=(__omaera_precmd)\n",
                    "ZDOTDIR=\"$OMAERA_ORIG_ZDOTDIR\"\n",
                );
                let _ = std::fs::write(integration_dir.join(".zshenv"), zshenv_content);
                let _ = std::fs::write(integration_dir.join(".zshrc"), zshrc_content);
                cmd.env("OMAERA_ORIG_ZDOTDIR", orig_zdotdir);
                cmd.env("ZDOTDIR", &integration_dir);
            }
        } else if shell_name.contains("powershell") || shell_name.contains("pwsh") {
            // PowerShell: -NoExit -Command で prompt 関数をラップして注入
            // 注意: portable-pty の CommandBuilder は Windows 標準の \" エスケープを行うが
            // PowerShell は \" を認識しないため、スクリプト内でダブルクォートを使わない
            // $? を [int]!$? で 0/1 に変換 ($LASTEXITCODE は cmdlet では更新されないため使わない)
            cmd.arg("-NoExit");
            cmd.arg("-Command");
            cmd.arg(
                r#"$__p=$function:prompt;function prompt{$code=[int]!$?;[Console]::Write([char]27+']777;exit_code;'+$code+[char]7);if($__p){&$__p}else{('PS '+$executionContext.SessionState.Path.CurrentLocation+('>'*($nestedPromptLevel+1))+' ')}}"#,
            );
        }

        if let Some(ref dir) = cwd {
            cmd.cwd(dir);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Spawn error: {}", e))?;

        // slave は spawn 後 drop
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Writer error: {}", e))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Reader error: {}", e))?;

        let alive = Arc::new(Mutex::new(true));

        // PTY 出力リングバッファ（MCP の oretachi_read_terminal で参照される）
        let output_history: Arc<Mutex<VecDeque<u8>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_HISTORY_BYTES)));

        // flush ループへ渡す未配送バッファ（reader が append し flush が drain して emit）
        let output_pending: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

        // 差分読みのカーソル基点 / プロセス exit / OSC 777 直近コマンド exit
        let total_bytes_written: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let exit_status: Arc<Mutex<Option<i64>>> = Arc::new(Mutex::new(None));
        let last_command_exit_code: Arc<Mutex<Option<i64>>> = Arc::new(Mutex::new(None));
        let exited_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

        // child_pid と child_killer を spawn 直後に取得
        let child_pid = child.process_id();
        let child_killer = child.clone_killer();

        let child_arc: Arc<Mutex<Option<Box<dyn portable_pty::Child + Send>>>> =
            Arc::new(Mutex::new(Some(child)));

        // master を Arc<Mutex<Option<...>>> で管理 (監視スレッドと kill() で共有)
        let master_arc: Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>> =
            Arc::new(Mutex::new(Some(pair.master)));

        // 子プロセス監視スレッド: プロセス終了を検知して master を drop → reader に EOF
        let alive_watcher = alive.clone();
        let master_watcher = master_arc.clone();
        let child_watcher = child_arc.clone();
        let exit_status_setter = exit_status.clone();
        let exited_at_setter = exited_at.clone();
        let watcher_handle = std::thread::spawn(move || {
            let child_opt = match child_watcher.lock() {
                Ok(mut g) => g.take(),
                Err(e) => e.into_inner().take(),
            };
            if let Some(mut child) = child_opt {
                // try_wait() ポーリング: alive=false (kill() 呼び出し済み) なら即座に終了
                let mut captured_exit: Option<i64> = None;
                let exited = loop {
                    let alive = match alive_watcher.lock() {
                        Ok(g) => *g,
                        Err(e) => *e.into_inner(),
                    };
                    if !alive {
                        break false;
                    }
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            // u32 → i64: Windows の異常終了コード (0xC000013A 等) を符号反転させない
                            captured_exit = Some(status.exit_code() as i64);
                            break true;
                        }
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                        Err(_) => break false,
                    }
                };
                // 自然終了した場合のみ master を drop して reader に EOF を送る
                if exited {
                    if let Some(code) = captured_exit {
                        match exit_status_setter.lock() {
                            Ok(mut s) => *s = Some(code),
                            Err(e) => *e.into_inner() = Some(code),
                        }
                    }
                    let should_drop = match alive_watcher.lock() {
                        Ok(mut g) => {
                            if *g { *g = false; true } else { false }
                        }
                        Err(e) => {
                            let mut g = e.into_inner();
                            if *g { *g = false; true } else { false }
                        }
                    };
                    if should_drop {
                        match master_watcher.lock() {
                            Ok(mut g) => { g.take(); }
                            Err(e) => { e.into_inner().take(); }
                        }
                    }
                    // 自然終了したセッションは map に残し、exited_at をセットする。
                    // MCP クライアントが exit code / 最終ログを参照できるよう EXITED_SESSION_TTL の
                    // 間は保持し、各 sessions.lock() path の sweep_exited で TTL 切れを除去する。
                    // UI タブは pty-exit イベントで消える（フロント側の整合性は維持）。
                    match exited_at_setter.lock() {
                        Ok(mut g) => *g = Some(Instant::now()),
                        Err(e) => *e.into_inner() = Some(Instant::now()),
                    }
                }
            }
        });

        // 読み取りスレッド起動
        let app_handle_reader = app_handle.clone();
        let history_for_reader = output_history.clone();
        let pending_for_reader = output_pending.clone();
        let total_for_reader = total_bytes_written.clone();
        let last_cmd_exit_for_reader = last_command_exit_code.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            // OSC 777 のシーケンスは reader read の境界をまたぐ可能性があるため、
            // 末尾数バイトを次回読み取り時の頭にくっつけてパースする
            let mut osc_lookback: Vec<u8> = Vec::new();
            const OSC_LOOKBACK_MAX: usize = 256;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        // リングバッファへ push と total 更新を **同じ critical section で実行**。
                        // 別 lock にすると read_output_history が中間状態（hist は新しいが total は古い）を観測し、
                        // buf_start = total - hist.len() が巻き戻って差分読みが重複する。
                        if let Ok(mut hist) = history_for_reader.lock() {
                            if hist.len() + n > OUTPUT_HISTORY_BYTES {
                                let drop_n = hist.len() + n - OUTPUT_HISTORY_BYTES;
                                hist.drain(..drop_n);
                            }
                            hist.extend(data.iter().copied());
                            total_for_reader.fetch_add(n as u64, Ordering::Relaxed);
                        }

                        // OSC 777 直近コマンド exit code を抽出
                        osc_lookback.extend_from_slice(&buf[..n]);
                        while let Some(code) = consume_osc_777_exit_code(&mut osc_lookback) {
                            match last_cmd_exit_for_reader.lock() {
                                Ok(mut s) => *s = Some(code),
                                Err(e) => *e.into_inner() = Some(code),
                            }
                        }
                        if osc_lookback.len() > OSC_LOOKBACK_MAX {
                            let drop_n = osc_lookback.len() - OSC_LOOKBACK_MAX;
                            osc_lookback.drain(..drop_n);
                        }

                        // チャンク毎 emit はやめ、保留バッファへ append するだけにする。
                        // 実際の emit は 16ms 周期の flush ループがまとめて行う。
                        if let Ok(mut pend) = pending_for_reader.lock() {
                            pend.extend(data.iter().copied());
                            // 出力が drain 速度を持続的に上回るとき、保留が無制限に増大しないよう最古を捨てる
                            if pend.len() > MAX_PENDING_BYTES {
                                let drop_n = pend.len() - MAX_PENDING_BYTES;
                                pend.drain(..drop_n);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            // EOF / エラーで reader を抜ける前に、保留分を全て flush し切ってから exit を通知する
            // （flush ループより先に pty-exit が届いて末尾出力が失われる／順序が乱れるのを防ぐ）。
            loop {
                let remaining = pending_for_reader
                    .lock()
                    .map(|p| p.len())
                    .unwrap_or(0);
                if remaining == 0 {
                    break;
                }
                flush_session_output(&app_handle_reader, session_id, &pending_for_reader);
            }
            let _ = app_handle_reader.emit("pty-exit", PtyExitPayload { session_id });
        });

        // writer スレッド: 入力キューを順番に ConPTY 入力パイプへ書き込む。
        // 子プロセスが stdin を読まずパイプが満杯のときは write_all がブロックするが、
        // ブロックするのはこのスレッドだけで、enqueue 側 (Tauri コマンド) は影響を受けない。
        // kill 時は session drop で全 Sender が消え recv が Err になりスレッドが終了する。
        // ブロック中でも kill が master を drop → ConPTY 破棄で write がエラーになり解ける。
        let (input_tx, input_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(INPUT_QUEUE_MAX_CHUNKS);
        std::thread::spawn(move || {
            let mut writer = writer;
            while let Ok(data) = input_rx.recv() {
                if let Err(e) = writer.write_all(&data).and_then(|_| writer.flush()) {
                    log::warn!(
                        "[Terminal] writer thread exiting on write error session_id={}: {}",
                        session_id,
                        e
                    );
                    return;
                }
            }
        });

        let session = PtySession {
            terminal_id: terminal_id.clone(),
            input_tx,
            master: master_arc,
            child_killer,
            child_pid,
            alive,
            watcher_handle: Some(watcher_handle),
            is_ai_agent: false,
            agent_name: None,
            agent_session_id: None,
            agent_status: None,
            agent_status_sampled_at: None,
            output_quiescent: false,
            cwd,
            output_history,
            output_pending,
            total_bytes_written,
            exit_status,
            last_command_exit_code,
            exited_at,
        };

        self.sessions.lock().map_err(|e| format!("lock error: {}", e))?.insert(session_id, session);

        log::debug!(
            "[Terminal] pty_manager::spawn done session_id={} terminal_id={} rows={} cols={}",
            session_id, terminal_id, rows, cols
        );
        Ok(session_id)
    }

    /// PTY への入力をセッションの writer スレッドへ enqueue する（非ブロッキング）。
    /// 実 I/O は writer スレッドが行うため、本関数は ConPTY 入力パイプの状態に
    /// かかわらず即座に返る。I/O エラーは writer スレッド側でログされ、以降の
    /// send が "input channel closed" で失敗するようになる。キューが満杯
    /// （子プロセスが stdin を読まず writer がブロック中）の場合も即時エラーを返す。
    pub fn write(&self, session_id: u32, data: Vec<u8>) -> Result<(), String> {
        let tx = {
            let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
            sweep_exited(&mut sessions);
            let session = sessions
                .get(&session_id)
                .ok_or_else(|| format!("Session {} not found", session_id))?;
            session.input_tx.clone()
        };

        tx.try_send(data).map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => format!(
                "Write error: input queue full (session {}; child process not reading stdin?)",
                session_id
            ),
            std::sync::mpsc::TrySendError::Disconnected(_) => {
                format!("Write error: input channel closed (session {})", session_id)
            }
        })
    }

    /// 指定セッションの出力履歴を取得する。
    /// - `from_cursor` を指定すると、それ以降の新規バイトのみを返す（差分読み）。
    ///   要求 cursor がリングバッファ範囲外の場合は `lost_bytes` で先頭の欠落量を通知する。
    /// - `from_cursor` 未指定の場合は、バッファ末尾から `max_bytes` バイトを返す（従来挙動）。
    /// - 開始位置がバッファ先頭でない場合のみ、UTF-8 / ANSI 境界補正を実施する:
    ///   1) UTF-8 継続バイト (0b10xxxxxx) を読み飛ばし文字境界に揃える
    ///   2) 直近 512 バイト以内に LF があればその直後まで進める（ANSI 中断残骸の回避）
    ///   ただし from_cursor 連続呼び出し（lost_bytes == 0）では cursor が文字境界済みなので
    ///   補正をスキップして差分の先頭バイトを欠落させない。
    pub fn read_output_history(
        &self,
        session_id: u32,
        max_bytes: Option<usize>,
        from_cursor: Option<u64>,
    ) -> Result<ReadHistoryResult, String> {
        let (history_arc, total_arc) = {
            let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
            sweep_exited(&mut sessions);
            let session = sessions
                .get(&session_id)
                .ok_or_else(|| format!("Session {} not found", session_id))?;
            (session.output_history.clone(), session.total_bytes_written.clone())
        };
        // hist lock を取った後に total を load し、reader thread の hist push + total update が
        // 同じ lock の中で行われていることに合わせる（race による cursor 後退対策）。
        let hist = history_arc
            .lock()
            .map_err(|e| format!("history lock error: {}", e))?;
        let total = total_arc.load(Ordering::Relaxed);
        let buf_len = hist.len() as u64;
        let buf_start = total.saturating_sub(buf_len);

        let req_start = match from_cursor {
            None => total.saturating_sub(max_bytes.unwrap_or(usize::MAX) as u64),
            Some(c) => c.min(total),
        };
        let actual_start = req_start.max(buf_start);
        let lost_bytes = actual_start - req_start;

        let mut buf_idx = (actual_start - buf_start) as usize;

        let needs_alignment = from_cursor.is_none() || lost_bytes > 0;
        if needs_alignment && buf_idx > 0 && buf_idx < hist.len() {
            while buf_idx < hist.len() && (hist[buf_idx] & 0xC0) == 0x80 {
                buf_idx += 1;
            }
            const NEWLINE_SCAN_WINDOW: usize = 512;
            let scan_end = (buf_idx + NEWLINE_SCAN_WINDOW).min(hist.len());
            for i in buf_idx..scan_end {
                if hist[i] == b'\n' {
                    buf_idx = i + 1;
                    break;
                }
            }
        }

        let take_n = max_bytes
            .unwrap_or(usize::MAX)
            .min(hist.len() - buf_idx);
        let data: Vec<u8> = hist.iter().skip(buf_idx).take(take_n).copied().collect();
        let cursor = buf_start + buf_idx as u64 + take_n as u64;

        Ok(ReadHistoryResult { data, cursor, lost_bytes })
    }

    pub fn resize(&self, session_id: u32, rows: u16, cols: u16) -> Result<(), String> {
        log::debug!("[Terminal] pty_manager::resize session_id={} rows={} cols={}", session_id, rows, cols);
        let master_arc = {
            let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
            sweep_exited(&mut sessions);
            let session = sessions
                .get(&session_id)
                .ok_or_else(|| format!("Session {} not found", session_id))?;
            session.master.clone()
        };

        if let Some(master) = master_arc.lock().map_err(|e| format!("lock error: {}", e))?.as_ref() {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("Resize error: {}", e))?;
        }

        Ok(())
    }

    pub fn kill(&self, session_id: u32, source: &str) -> Result<(), String> {
        log::info!("[Terminal] pty_manager::kill session_id={} source={}", session_id, source);
        // sessions ロックのスコープを最小化: remove + alive=false の設定のみ行い、
        // 重い処理（taskkill, join）はロック外で実行する。
        // taskkill /F /T は Windows 上で数秒かかることがあり、ロック保持中に実行すると
        // pty_write, pty_resize 等すべてのセッション操作がブロックされる。
        let removed = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sweep_exited(&mut sessions);
            if let Some(session) = sessions.remove(&session_id) {
                // poison でも alive=false を確実にセット（ウォッチャースレッドの停止に必要）
                match session.alive.lock() {
                    Ok(mut alive) => *alive = false,
                    Err(e) => *e.into_inner() = false,
                }
                // master の take は sessions ロック解放後に行う。resize() が master ロックを
                // 保持したまま ConPTY RPC でハングしている場合、ここで master.lock() を待つと
                // sessions ロックを保持し続け、メインスレッド上の pty_write (sessions.lock())
                // まで巻き込んでフリーズするため、Arc ごと持ち出してロック外で take する。
                // session の残りフィールド (input_tx 含む) はこのスコープ末尾で drop され、
                // input_tx の drop により writer スレッドの recv が解けてスレッドが終了する。
                Some((session.child_pid, session.child_killer, session.master.clone(), session.watcher_handle))
            } else {
                None
            }
        }; // ← sessions ロック解放

        if let Some((child_pid, mut child_killer, master_arc, watcher_handle)) = removed {
            // ロック外でプロセスkill（taskkillが遅くても他の操作をブロックしない）
            if let Some(pid) = child_pid {
                crate::process_utils::kill_process_tree(pid);
            }
            // child_killer でバックアップ kill（child が監視スレッドに渡済みでも動作）
            let _ = child_killer.kill();
            // master を取り出して drop し ConPTY を破棄、reader に EOF を送る
            // （入力パイプも閉じられ、writer スレッドがブロック中でも write エラーで解ける）
            let master = master_arc.lock().unwrap_or_else(|e| e.into_inner()).take();
            drop(master);
            // watcher スレッドの終了を待つ（alive=false を検知して必ず終了する）
            if let Some(handle) = watcher_handle {
                let _ = handle.join();
            }
        }
        Ok(())
    }

    pub fn kill_all(&self, source: &str) {
        let ids: Vec<u32> = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sweep_exited(&mut sessions);
            sessions.keys().cloned().collect()
        };
        log::info!("[Terminal] pty_manager::kill_all source={} count={}", source, ids.len());
        for id in ids {
            let _ = self.kill(id, source);
        }
    }

    /// アプリ終了経路専用の高速 kill_all。
    ///
    /// 逐次 `kill`（各セッション taskkill 最大10秒 + 無期限 `watcher_handle.join()`）は
    /// N セッションで最悪 N×10秒 UI スレッドをブロックし、ユーザーの強制終了 → 孤児
    /// WebView2 / ポート保持を招く。本関数は次の最適化で終了時間を有界化する:
    /// 1. sessions ロックを最小スコープで全 drain し、ロック外で並列に kill する。
    /// 2. taskkill は `kill_process_tree_exit`（1.5秒上限）を使う。
    /// 3. `watcher_handle.join()` は無期限に待たず、合計 deadline まで有界に待つだけ。
    ///    （プロセス終了時にスレッドも消えるため未 join でも問題ない。取りこぼした
    ///     子プロセスは起動時に設定する Job Object が OS レベルで回収する。）
    pub fn kill_all_fast(&self, source: &str) {
        // (child_pid, child_killer, master(Arc), watcher_handle) を持ち出す。
        // session 本体（input_tx 含む）はこのスコープ末尾で drop され、writer スレッドが終了する。
        type Drained = (
            Option<u32>,
            Box<dyn portable_pty::ChildKiller + Send + Sync>,
            Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>>,
            Option<std::thread::JoinHandle<()>>,
        );
        let drained: Vec<Drained> = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sweep_exited(&mut sessions);
            let ids: Vec<u32> = sessions.keys().cloned().collect();
            let mut out: Vec<Drained> = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(session) = sessions.remove(&id) {
                    // ウォッチャースレッド停止のため alive=false を確実にセット（poison でも）。
                    match session.alive.lock() {
                        Ok(mut alive) => *alive = false,
                        Err(e) => *e.into_inner() = false,
                    }
                    out.push((
                        session.child_pid,
                        session.child_killer,
                        session.master.clone(),
                        session.watcher_handle,
                    ));
                    // session（残りフィールド: input_tx 等）はここで drop される。
                }
            }
            out
        }; // ← sessions ロック解放

        let count = drained.len();
        log::info!("[Terminal] pty_manager::kill_all_fast source={} count={}", source, count);
        if count == 0 {
            return;
        }

        // 各セッションを並列スレッドで kill。1 スレッドが master.lock() でハングしても
        // 他スレッドと UI スレッドを巻き込まないよう、スレッド内で master take を行う。
        let remaining = Arc::new(std::sync::atomic::AtomicUsize::new(count));
        for (child_pid, mut child_killer, master_arc, watcher_handle) in drained {
            let remaining = remaining.clone();
            std::thread::spawn(move || {
                // child_killer でバックアップ kill（即時）
                let _ = child_killer.kill();
                // master を take して drop し ConPTY を破棄、reader に EOF を送る
                let master = master_arc.lock().unwrap_or_else(|e| e.into_inner()).take();
                drop(master);
                // プロセスツリーを 1.5 秒上限で kill
                if let Some(pid) = child_pid {
                    crate::process_utils::kill_process_tree_exit(pid);
                }
                // watcher_handle は join せず drop（detach）。alive=false で自然終了する。
                drop(watcher_handle);
                remaining.fetch_sub(1, Ordering::SeqCst);
            });
        }

        // 合計 deadline（2秒）まで有界に待つ。超過してもブロックを打ち切って終了を進める。
        let deadline = Instant::now() + Duration::from_millis(2000);
        while remaining.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let left = remaining.load(Ordering::SeqCst);
        if left > 0 {
            log::warn!(
                "[Terminal] kill_all_fast: {} session(s) not confirmed within deadline (detached)",
                left
            );
        }
    }

    /// 指定ディレクトリ以下をcwdとして持つ全PTYセッションをkillする。
    /// ワークツリー削除前にそのディレクトリを掴んでいる子プロセスを解放するために使用。
    /// 返り値: killしたセッション数（> 0 なら呼び出し側でプロセス終了待機が必要）
    pub fn kill_sessions_in_dir(&self, dir: &str) -> usize {
        let target = std::path::Path::new(dir);
        let ids: Vec<u32> = {
            let mut sessions = match self.sessions.lock() {
                Ok(s) => s,
                Err(e) => e.into_inner(),
            };
            sweep_exited(&mut sessions);
            sessions
                .iter()
                .filter(|(_, s)| {
                    s.cwd.as_deref().map_or(false, |cwd| {
                        std::path::Path::new(cwd).starts_with(target)
                    })
                })
                .map(|(id, _)| *id)
                .collect()
        };
        for id in &ids {
            let source = format!("kill_sessions_in_dir(worktree={})", dir);
            let _ = self.kill(*id, &source);
        }
        ids.len()
    }

    /// 全 PTY セッションを `SessionInfo` のリストで返す。
    /// `exit_code` は watcher が拾ったプロセス全体（シェル本体）の exit code。
    /// `last_command_exit_code` はシェル統合 OSC 777 が出した直近コマンドの exit code。
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        let mut sessions = match self.sessions.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };
        sweep_exited(&mut sessions);
        sessions
            .iter()
            .map(|(id, s)| SessionInfo {
                session_id: *id,
                terminal_id: s.terminal_id.clone(),
                cwd: s.cwd.clone(),
                is_ai_agent: s.is_ai_agent,
                agent_name: s.agent_name.clone(),
                agent_session_id: s.agent_session_id.clone(),
                agent_status: s.agent_status.clone(),
                agent_status_sampled_at: s.agent_status_sampled_at,
                output_quiescent: s.output_quiescent,
                exit_code: s.exit_status.lock().ok().and_then(|g| *g),
                last_command_exit_code: s.last_command_exit_code.lock().ok().and_then(|g| *g),
            })
            .collect()
    }
}

impl Drop for PtyManagerCore {
    fn drop(&mut self) {
        match self.polling_alive.lock() {
            Ok(mut alive) => *alive = false,
            Err(e) => *e.into_inner() = false,
        }
        // sessions が空なら既に kill_all 済みなのでスキップ
        // poison されていても into_inner() で中身を取り出してチェックする
        let has_sessions = match self.sessions.lock() {
            Ok(s) => !s.is_empty(),
            Err(e) => !e.into_inner().is_empty(),
        };
        if has_sessions {
            self.kill_all("PtyManager::drop");
        }
    }
}

/// シェル統合の OSC 777 シーケンス `\x1b]777;exit_code;<digits>(\x07|\x1b\\)` を
/// 1 個消費し、パースした exit code を返す。
/// - 終端 (BEL or ESC\) まで届いていない場合は `None` を返し、buf を保持する
///   （次回の reader read で続きが届くのを待つ）。
/// - 一致したシーケンスとそれより前のゴミは buf から drain される。
/// 末尾不完全 ESC は OSC_LOOKBACK_MAX で切り詰められる呼び出し側に任せる。
pub fn consume_osc_777_exit_code(buf: &mut Vec<u8>) -> Option<i64> {
    let prefix = b"\x1b]777;exit_code;";
    let start = buf.windows(prefix.len()).position(|w| w == prefix)?;
    let payload_start = start + prefix.len();
    let mut term_end: Option<(usize, usize)> = None;
    let mut i = payload_start;
    while i < buf.len() {
        if buf[i] == 0x07 {
            term_end = Some((i, i + 1));
            break;
        }
        if buf[i] == 0x1b && i + 1 < buf.len() && buf[i + 1] == b'\\' {
            term_end = Some((i, i + 2));
            break;
        }
        i += 1;
    }
    let (digits_end, total_end) = term_end?;
    // 終端まで来た以上、parse 成否にかかわらずシーケンスは消費する。
    // 不正 payload を残すと次回呼び出しで同じ位置に再ヒットし、後続の正常 OSC 777 が
    // OSC_LOOKBACK_MAX で潰されるまで検出されなくなる（last_command_exit_code の固着）。
    let parsed = std::str::from_utf8(&buf[payload_start..digits_end])
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok());
    buf.drain(..total_end);
    parsed
}

/// ANSI/VT100 エスケープシーケンスを除去する単純なストリッパ。
/// CSI (`ESC [ ... letter`)、OSC (`ESC ] ... BEL` or `ESC \`)、その他 ESC+1byte を除去。
/// 改行・タブは保持。完全な VT100 emulation ではないが AI が読む用途には十分。
pub fn strip_ansi(input: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b == 0x1b && i + 1 < input.len() {
            let next = input[i + 1];
            if next == b'[' {
                i += 2;
                while i < input.len() && !(0x40..=0x7e).contains(&input[i]) {
                    i += 1;
                }
                if i < input.len() {
                    i += 1;
                }
                continue;
            } else if next == b']' {
                i += 2;
                while i < input.len() {
                    if input[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            } else {
                i += 2;
                continue;
            }
        }
        if b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t' {
            out.push(b);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(pid, ppid, name)` の列から `find_ai_agent_in_subtree` 用の子マップを作る。
    fn children_of(procs: &[(u32, u32, &str)]) -> HashMap<u32, Vec<(u32, String)>> {
        let mut map: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
        for (pid, ppid, name) in procs {
            map.entry(*ppid).or_default().push((*pid, name.to_string()));
        }
        map
    }

    /// npm shim 経由だと `pwsh → cmd → node → …` と伸びる。旧実装の深さ 4 では届かず、
    /// エージェントが走っているのに `is_ai_agent = false` になって押し込みが永久に来なかった。
    #[test]
    fn find_agent_deep_in_subtree() {
        let procs = [
            (2, 1, "cmd.exe"),
            (3, 2, "node.exe"),
            (4, 3, "cmd.exe"),
            (5, 4, "node.exe"),
            (6, 5, "claude.exe"),
        ];
        let map = children_of(&procs);
        assert_eq!(
            find_ai_agent_in_subtree(1, &map, AGENT_SEARCH_DEPTH),
            Some(("claude".to_string(), 6))
        );
        // 旧来の深さでは届かないことも押さえる（回帰時に気づけるように）
        assert_eq!(find_ai_agent_in_subtree(1, &map, 4), None);
    }

    /// npm / scoop が置く shim（`claude.cmd` / `claude.ps1`）もエージェント本体として扱う。
    /// 実体は `node.exe` で名前から判別できないため、shim の時点で確定させるしかない。
    #[test]
    fn find_agent_matches_shim_names() {
        for name in ["claude.cmd", "claude.ps1", "CLAUDE.EXE", "claude"] {
            let map = children_of(&[(2, 1, name)]);
            assert_eq!(
                find_ai_agent_in_subtree(1, &map, AGENT_SEARCH_DEPTH),
                Some(("claude".to_string(), 2)),
                "{} を拾えていない",
                name
            );
        }
        // 部分一致で誤検出しない
        let map = children_of(&[(2, 1, "claudex.exe"), (3, 1, "node.exe")]);
        assert_eq!(find_ai_agent_in_subtree(1, &map, AGENT_SEARCH_DEPTH), None);
    }

    /// プロセス列挙は自プロセスを必ず含む。空 Vec は「列挙失敗」の意味なので、
    /// 呼び出し側（ポーリング）が状態更新をスキップする判定に使える。
    #[test]
    fn scan_all_processes_finds_self() {
        let procs = crate::process_utils::scan_all_processes();
        assert!(!procs.is_empty(), "プロセス列挙が空（wmic 依存を外した意味が無い）");
        let me = std::process::id();
        assert!(
            procs.iter().any(|(pid, _, _)| *pid == me),
            "自プロセス pid={} が列挙に含まれない",
            me
        );
    }

    /// 実機のプロセス表で、走っているエージェントを本当に検出できるかの煙試験。
    ///
    /// `scan_all_processes` の列挙と `find_ai_agent_in_subtree` の探索を**実データで**通す。
    /// エージェントが1つも走っていない環境では意味を持たないので `#[ignore]`。
    /// 手元確認は `cargo test --lib -- --ignored detects_running_agent_on_this_machine`。
    #[test]
    #[ignore = "エージェントが走っている環境でのみ意味がある"]
    fn detects_running_agent_on_this_machine() {
        let procs = crate::process_utils::scan_all_processes();
        let mut map: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
        for (pid, ppid, name) in &procs {
            map.entry(*ppid).or_default().push((*pid, name.clone()));
        }
        let agents: Vec<&(u32, u32, String)> = procs
            .iter()
            .filter(|(_, _, name)| {
                let lower = name.to_lowercase();
                AI_AGENT_NAMES.contains(&lower.split('.').next().unwrap_or(&lower))
            })
            .collect();
        assert!(!agents.is_empty(), "エージェントが走っていないので判定できない");
        for (pid, ppid, name) in agents {
            let found = find_ai_agent_in_subtree(*ppid, &map, AGENT_SEARCH_DEPTH);
            assert!(
                found.is_some(),
                "親 pid={} の配下に居る {} (pid={}) を検出できない",
                ppid,
                name,
                pid
            );
        }
    }

    #[test]
    fn strip_ansi_plain_text() {
        assert_eq!(strip_ansi(b"hello\nworld"), "hello\nworld");
    }

    #[test]
    fn strip_ansi_csi_color() {
        // ESC[31m red ESC[0m
        let input = b"\x1b[31mred\x1b[0m text";
        assert_eq!(strip_ansi(input), "red text");
    }

    #[test]
    fn strip_ansi_osc_title_bel() {
        // ESC]0;title\x07
        let input = b"\x1b]0;window title\x07hello";
        assert_eq!(strip_ansi(input), "hello");
    }

    #[test]
    fn strip_ansi_osc_title_st() {
        // ESC]0;title ESC\\
        let input = b"\x1b]0;t\x1b\\done";
        assert_eq!(strip_ansi(input), "done");
    }

    #[test]
    fn strip_ansi_incomplete_esc_at_end() {
        // 末尾が不完全 ESC のみ → ESC は単独でも丸ごと捨てる
        let input = b"abc\x1b";
        assert_eq!(strip_ansi(input), "abc");
    }

    #[test]
    fn strip_ansi_keeps_tab_and_crlf() {
        let input = b"a\tb\r\nc";
        assert_eq!(strip_ansi(input), "a\tb\r\nc");
    }

    #[test]
    fn strip_ansi_drops_non_print_control() {
        // 0x01 (SOH) など制御文字は除去
        let input = b"a\x01b";
        assert_eq!(strip_ansi(input), "ab");
    }

    #[test]
    fn strip_ansi_empty() {
        assert_eq!(strip_ansi(b""), "");
    }

    #[test]
    fn strip_ansi_utf8_passthrough() {
        let input = "日本語\x1b[1mbold\x1b[0m".as_bytes();
        assert_eq!(strip_ansi(input), "日本語bold");
    }

    #[test]
    fn osc_777_bel_terminator() {
        let mut buf = b"prefix\x1b]777;exit_code;0\x07tail".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(0));
        // start より前の "prefix" もシーケンス全体と一緒に drain される
        assert_eq!(buf, b"tail");
    }

    #[test]
    fn osc_777_st_terminator() {
        let mut buf = b"\x1b]777;exit_code;42\x1b\\rest".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(42));
        assert_eq!(buf, b"rest");
    }

    #[test]
    fn osc_777_split_across_reads() {
        // 1 回目: 終端なし → None、buf はそのまま
        let mut buf = b"\x1b]777;exit_code;1".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), None);
        assert_eq!(buf, b"\x1b]777;exit_code;1");
        // 2 回目: 続きを足して再パース
        buf.extend_from_slice(b"23\x07after");
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(123));
        assert_eq!(buf, b"after");
    }

    #[test]
    fn osc_777_multiple_in_one_buffer() {
        let mut buf = b"\x1b]777;exit_code;0\x07ok\x1b]777;exit_code;7\x07tail".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(0));
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(7));
        assert_eq!(consume_osc_777_exit_code(&mut buf), None);
        assert_eq!(buf, b"tail");
    }

    #[test]
    fn osc_777_invalid_digits_drains() {
        // 不正な digits でも終端まで来ているなら buf から消費する。
        // 残すと後続の正常 OSC 777 が OSC_LOOKBACK_MAX で潰されるまで検出されない（リグレッション防止）
        let mut buf = b"\x1b]777;exit_code;abc\x07after".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), None);
        assert_eq!(buf, b"after");
    }

    #[test]
    fn osc_777_empty_payload_drains() {
        let mut buf = b"\x1b]777;exit_code;\x07after".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), None);
        assert_eq!(buf, b"after");
    }

    #[test]
    fn osc_777_invalid_then_valid_recovered() {
        // 不正シーケンスの後ろに正常シーケンスがある場合、不正分を消費した上で
        // 次の呼び出しで正常分が拾える
        let mut buf = b"\x1b]777;exit_code;abc\x07\x1b]777;exit_code;42\x07tail".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), None);
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(42));
        assert_eq!(buf, b"tail");
    }

    #[test]
    fn osc_777_negative_exit_code() {
        // i64 化したので Unix 系の負の signal kill 表現も保持できる
        let mut buf = b"\x1b]777;exit_code;-1\x07".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(-1));
    }

    #[test]
    fn osc_777_large_windows_exit_code() {
        // Windows の Ctrl-C kill (0xC000013A = 3221225786) は u32 のため i32 では負値化するが、
        // OSC 777 は シェルが文字列で吐くため i64 でそのまま受け取れる
        let mut buf = b"\x1b]777;exit_code;3221225786\x07".to_vec();
        assert_eq!(consume_osc_777_exit_code(&mut buf), Some(3221225786));
    }
}
