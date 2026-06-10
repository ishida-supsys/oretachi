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
const MAX_PTY_SESSIONS: usize = 32;
const OUTPUT_HISTORY_BYTES: usize = 65_536;
/// シェル本体が自然終了したセッションを map に残しておく寿命。
/// この期間内なら MCP クライアントから exit code や最終ログを参照できる。
/// MAX_PTY_SESSIONS の枠を一時的に圧迫し得るが、TTL 経過後は lazy sweep で除去される。
const EXITED_SESSION_TTL: Duration = Duration::from_secs(30);

struct PtySession {
    /// PTY 入力の送信キュー。書き込み本体はセッション毎の writer スレッドが行う。
    /// ConPTY の入力パイプへの write は子プロセスが stdin を読まないと無期限に
    /// ブロックしうるため、Tauri コマンド（メインスレッド）からは enqueue のみ行い、
    /// 実 I/O をメインスレッドから隔離する。全 Sender drop で writer スレッドは終了する。
    input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    master: Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>>,
    child_killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    child_pid: Option<u32>,
    alive: Arc<Mutex<bool>>,
    watcher_handle: Option<std::thread::JoinHandle<()>>,
    is_ai_agent: bool,
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
    pub cwd: Option<String>,
    pub is_ai_agent: bool,
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

/// 全プロセス一覧を (pid, parent_pid, name) のリストで返す。
/// タイムアウト付き: wmic/ps が応答しない場合は空リストを返す。
///
/// stdout の読み取りとプロセス終了待ちを別スレッドで並行実行する。
/// パイプバッファ（Windows: ~4KB）が溢れると子プロセスが書き込みブロックし、
/// 逐次実行ではデッドロックするため。
fn scan_all_processes() -> Vec<(u32, u32, String)> {
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    /// 子プロセスを spawn し、stdout 読み取りとタイムアウト付き wait を並行実行する。
    /// パイプバッファデッドロックを回避するため、stdout は別スレッドで先に読み切る。
    fn run_with_timeout(mut child: std::process::Child) -> Option<String> {
        let stdout = child.stdout.take()?;
        // stdout を別スレッドで読み切る（パイプバッファ満杯によるデッドロック回避）
        // read_to_end + from_utf8_lossy を使い、非UTF8プロセス名（日本語Windows等）でも失敗しない
        let reader_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut std::io::BufReader::new(stdout), &mut buf).ok()?;
            Some(String::from_utf8_lossy(&buf).into_owned())
        });

        // タイムアウト付きで終了を待機
        let deadline = std::time::Instant::now() + TIMEOUT;
        let mut kill_failed = false;
        let exited = loop {
            match child.try_wait() {
                Ok(Some(_)) => break true,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        log::warn!("[Terminal] scan_all_processes: process timed out after {:?}", TIMEOUT);
                        if child.kill().is_ok() {
                            let _ = child.wait();
                        } else {
                            // kill 失敗時は wait() を呼ばない（永久ブロック回避）
                            kill_failed = true;
                        }
                        break false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => break false,
            }
        };

        // kill 失敗時はプロセスが生きており reader スレッドも stdout を待機中のため、
        // join() せずデタッチしてブロックを回避する
        if kill_failed {
            drop(reader_handle);
            return None;
        }

        // タイムアウト/エラー時もreaderスレッドの終了を待つ（デタッチ防止）
        let result = reader_handle.join().ok().flatten();
        if !exited {
            return None;
        }
        result
    }

    #[cfg(target_os = "windows")]
    {
        let child = match crate::process_utils::make_command("wmic")
            .args(["process", "get", "Name,ProcessId,ParentProcessId", "/FORMAT:CSV"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let text = match run_with_timeout(child) {
            Some(t) => t,
            None => return vec![],
        };

        let mut result = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            // Skip header and empty lines
            if line.is_empty() || line.starts_with("Node") {
                continue;
            }
            let parts: Vec<&str> = line.split(',').collect();
            // wmic CSV columns (alphabetical): Node, Name, ParentProcessId, ProcessId
            if parts.len() >= 4 {
                let name = parts[1].trim().to_string();
                if let (Ok(ppid), Ok(pid)) = (
                    parts[2].trim().parse::<u32>(),
                    parts[3].trim().parse::<u32>(),
                ) {
                    if !name.is_empty() {
                        result.push((pid, ppid, name));
                    }
                }
            }
        }
        result
    }

    #[cfg(not(target_os = "windows"))]
    {
        let child = match std::process::Command::new("ps")
            .args(["axo", "pid,ppid,comm"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let text = match run_with_timeout(child) {
            Some(t) => t,
            None => return vec![],
        };

        let mut result = Vec::new();
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let (Ok(pid), Ok(ppid)) = (
                    parts[0].parse::<u32>(),
                    parts[1].parse::<u32>(),
                ) {
                    result.push((pid, ppid, parts[2].to_string()));
                }
            }
        }
        result
    }
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
            let name_stem = name_lower.trim_end_matches(".exe");
            if AI_AGENT_NAMES.iter().any(|&a| name_stem == a) {
                return Some((name_stem.to_string(), *child_pid));
            }
            if let Some(found) = find_ai_agent_in_subtree(*child_pid, children_map, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// Claude Code の PID から ~/.claude/sessions/<pid>.json を読んでセッション UUID を返す
fn get_claude_session_id_by_pid(pid: u32) -> Option<String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let session_file = std::path::Path::new(&home)
        .join(".claude")
        .join("sessions")
        .join(format!("{}.json", pid));
    let content = std::fs::read_to_string(&session_file).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("sessionId")?.as_str().map(|s| s.to_string())
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
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));

                if !*polling_alive.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }

                // セッション情報を取得
                let session_pids: Vec<(u32, Option<u32>)> = {
                    let mut sessions = match sessions_arc.lock() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    sweep_exited(&mut sessions);
                    sessions.iter().map(|(&id, s)| (id, s.child_pid)).collect()
                };

                if session_pids.is_empty() {
                    last_status.clear();
                    continue;
                }

                // プロセス一覧を一括取得して子プロセスマップを構築
                let all_procs = scan_all_processes();
                let mut children_map: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
                for (pid, ppid, name) in &all_procs {
                    children_map.entry(*ppid).or_default().push((*pid, name.clone()));
                }

                // pty_session_id → AiAgentInfo のマップを構築
                let mut current_status: HashMap<u32, (bool, Option<String>)> = HashMap::new();
                let mut new_infos: Vec<(u32, bool, AiAgentInfo)> = Vec::new();

                for (session_id, child_pid) in session_pids {
                    let (is_agent, info) = if let Some(pid) = child_pid {
                        match find_ai_agent_in_subtree(pid, &children_map, 4) {
                            Some((agent_name, agent_pid)) => {
                                let claude_session_id = if agent_name == "claude" {
                                    get_claude_session_id_by_pid(agent_pid)
                                } else {
                                    None
                                };
                                (true, AiAgentInfo {
                                    is_agent: true,
                                    agent_name: Some(agent_name),
                                    session_id: claude_session_id,
                                })
                            }
                            None => (false, AiAgentInfo { is_agent: false, agent_name: None, session_id: None }),
                        }
                    } else {
                        (false, AiAgentInfo { is_agent: false, agent_name: None, session_id: None })
                    };
                    let session_id_val = info.session_id.clone();
                    current_status.insert(session_id, (is_agent, session_id_val));
                    new_infos.push((session_id, is_agent, info));
                }

                // 内部状態を更新
                if let Ok(mut sessions) = sessions_arc.lock() {
                    for (id, is_agent, _) in &new_infos {
                        if let Some(session) = sessions.get_mut(id) {
                            session.is_ai_agent = *is_agent;
                        }
                    }
                }

                // 前回との差分を検出（is_agent または session_id が変わった場合）
                let changed: HashMap<u32, AiAgentInfo> = new_infos
                    .into_iter()
                    .filter(|(id, _, info)| {
                        let prev = last_status.get(id);
                        match prev {
                            None => true,
                            Some((prev_is, prev_sid)) => {
                                *prev_is != info.is_agent || *prev_sid != info.session_id
                            }
                        }
                    })
                    .map(|(id, _, info)| (id, info))
                    .collect();

                if !changed.is_empty() {
                    let _ = app_handle.emit("pty-ai-agent-changed", AiAgentChangedPayload { sessions: changed });
                }

                last_status = current_status;
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

        let mut cmd = CommandBuilder::new(&shell_cmd);
        cmd.env("TERM", "xterm-256color");

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

        let session_id = {
            let mut id = self.next_id.lock().map_err(|e| format!("lock error: {}", e))?;
            let current = *id;
            *id += 1;
            current
        };

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
        let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
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
            input_tx,
            master: master_arc,
            child_killer,
            child_pid,
            alive,
            watcher_handle: Some(watcher_handle),
            is_ai_agent: false,
            cwd,
            output_history,
            output_pending,
            total_bytes_written,
            exit_status,
            last_command_exit_code,
            exited_at,
        };

        self.sessions.lock().map_err(|e| format!("lock error: {}", e))?.insert(session_id, session);

        log::debug!("[Terminal] pty_manager::spawn done session_id={} rows={} cols={}", session_id, rows, cols);
        Ok(session_id)
    }

    /// PTY への入力をセッションの writer スレッドへ enqueue する（非ブロッキング）。
    /// 実 I/O は writer スレッドが行うため、本関数は ConPTY 入力パイプの状態に
    /// かかわらず即座に返る。I/O エラーは writer スレッド側でログされ、以降の
    /// send が "input channel closed" で失敗するようになる。
    pub fn write(&self, session_id: u32, data: Vec<u8>) -> Result<(), String> {
        let tx = {
            let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
            sweep_exited(&mut sessions);
            let session = sessions
                .get(&session_id)
                .ok_or_else(|| format!("Session {} not found", session_id))?;
            session.input_tx.clone()
        };

        tx.send(data)
            .map_err(|_| format!("Write error: input channel closed (session {})", session_id))
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
                // master を取り出して reader に EOF を送る準備。
                // session の残りフィールド (input_tx 含む) はこのスコープ末尾で drop され、
                // input_tx の drop により writer スレッドの recv が解けてスレッドが終了する。
                let master = session.master.lock().ok().and_then(|mut g| g.take());
                Some((session.child_pid, session.child_killer, master, session.watcher_handle))
            } else {
                None
            }
        }; // ← sessions ロック解放

        if let Some((child_pid, mut child_killer, master, watcher_handle)) = removed {
            // ロック外でプロセスkill（taskkillが遅くても他の操作をブロックしない）
            if let Some(pid) = child_pid {
                crate::process_utils::kill_process_tree(pid);
            }
            // child_killer でバックアップ kill（child が監視スレッドに渡済みでも動作）
            let _ = child_killer.kill();
            // master を drop して ConPTY を破棄し、reader に EOF を送る
            // （入力パイプも閉じられ、writer スレッドがブロック中でも write エラーで解ける）
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
                cwd: s.cwd.clone(),
                is_ai_agent: s.is_ai_agent,
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
