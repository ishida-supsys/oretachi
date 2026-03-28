use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

const AI_AGENT_NAMES: &[&str] = &["claude", "gemini", "codex", "cline"];
const MAX_PTY_SESSIONS: usize = 32;

struct PtySession {
    writer: Box<dyn Write + Send>,
    master: Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>>,
    child_killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    child_pid: Option<u32>,
    alive: Arc<Mutex<bool>>,
    watcher_handle: Option<std::thread::JoinHandle<()>>,
    is_ai_agent: bool,
    cwd: Option<String>,
}

pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<u32, PtySession>>>,
    next_id: Mutex<u32>,
    polling_alive: Arc<Mutex<bool>>,
}

#[derive(serde::Serialize, Clone)]
pub struct PtyOutputPayload {
    #[serde(rename = "sessionId")]
    pub session_id: u32,
    pub data: Vec<u8>,
}

#[derive(serde::Serialize, Clone)]
pub struct PtyExitPayload {
    #[serde(rename = "sessionId")]
    pub session_id: u32,
}

#[derive(serde::Serialize, Clone)]
pub struct AiAgentChangedPayload {
    /// sessionId → isAiAgent のマップ
    pub sessions: HashMap<u32, bool>,
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

/// 指定PIDのサブツリーにAIエージェントプロセスが含まれるか判定（最大depth段）
fn has_ai_agent_in_subtree(
    root_pid: u32,
    children_map: &HashMap<u32, Vec<(u32, String)>>,
    depth: u32,
) -> bool {
    if depth == 0 {
        return false;
    }
    if let Some(children) = children_map.get(&root_pid) {
        for (child_pid, child_name) in children {
            let name_lower = child_name.to_lowercase();
            let name_stem = name_lower.trim_end_matches(".exe");
            if AI_AGENT_NAMES.iter().any(|&a| name_stem == a) {
                return true;
            }
            if has_ai_agent_in_subtree(*child_pid, children_map, depth - 1) {
                return true;
            }
        }
    }
    false
}

impl PtyManager {
    pub fn new() -> Self {
        PtyManager {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_id: Mutex::new(1),
            polling_alive: Arc::new(Mutex::new(true)),
        }
    }

    /// AIエージェントフラグを明示的にセットする（executeAgentWorktree 用）
    pub fn set_ai_agent(&self, session_id: u32, is_agent: bool) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.is_ai_agent = is_agent;
            Ok(())
        } else {
            Err(format!("Session {} not found", session_id))
        }
    }

    /// AIエージェントフラグを参照する
    pub fn is_ai_agent(&self, session_id: u32) -> Result<bool, String> {
        let sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
        if let Some(session) = sessions.get(&session_id) {
            Ok(session.is_ai_agent)
        } else {
            Err(format!("Session {} not found", session_id))
        }
    }

    /// バックグラウンドでポーリングスレッドを起動し、AIエージェント状態の変化をイベントで通知する
    pub fn start_polling(&self, app_handle: AppHandle) {
        let sessions_arc = self.sessions.clone();
        let polling_alive = self.polling_alive.clone();

        std::thread::spawn(move || {
            let mut last_status: HashMap<u32, bool> = HashMap::new();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));

                if !*polling_alive.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }

                // セッション情報を取得
                let session_pids: Vec<(u32, Option<u32>)> = {
                    let sessions = match sessions_arc.lock() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
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

                let mut current_status: HashMap<u32, bool> = HashMap::new();
                let mut new_statuses = Vec::new();

                for (session_id, child_pid) in session_pids {
                    let status = if let Some(pid) = child_pid {
                        has_ai_agent_in_subtree(pid, &children_map, 4)
                    } else {
                        false
                    };
                    current_status.insert(session_id, status);
                    new_statuses.push((session_id, status));
                }

                // 内部状態を更新
                if let Ok(mut sessions) = sessions_arc.lock() {
                    for (id, status) in &new_statuses {
                        if let Some(session) = sessions.get_mut(id) {
                            session.is_ai_agent = *status;
                        }
                    }
                }

                // 前回との差分を検出
                let changed: HashMap<u32, bool> = current_status
                    .iter()
                    .filter(|(&id, &status)| last_status.get(&id) != Some(&status))
                    .map(|(&id, &status)| (id, status))
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
            let sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
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
        let watcher_handle = std::thread::spawn(move || {
            let child_opt = child_watcher.lock().unwrap().take();
            if let Some(mut child) = child_opt {
                // try_wait() ポーリング: alive=false (kill() 呼び出し済み) なら即座に終了
                let exited = loop {
                    if !*alive_watcher.lock().unwrap() {
                        break false;
                    }
                    match child.try_wait() {
                        Ok(Some(_)) => break true,
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                        Err(_) => break false,
                    }
                };
                // 自然終了した場合のみ master を drop して reader に EOF を送る
                if exited {
                    let mut alive_guard = alive_watcher.lock().unwrap();
                    if *alive_guard {
                        *alive_guard = false;
                        drop(alive_guard);
                        let _ = master_watcher.lock().unwrap().take();
                    }
                }
            }
        });

        // 読み取りスレッド起動
        let app_handle_reader = app_handle.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        let _ = app_handle_reader
                            .emit("pty-output", PtyOutputPayload { session_id, data });
                    }
                    Err(_) => break,
                }
            }
            let _ = app_handle_reader.emit("pty-exit", PtyExitPayload { session_id });
        });

        let session = PtySession {
            writer,
            master: master_arc,
            child_killer,
            child_pid,
            alive,
            watcher_handle: Some(watcher_handle),
            is_ai_agent: false,
            cwd,
        };

        self.sessions.lock().map_err(|e| format!("lock error: {}", e))?.insert(session_id, session);

        log::debug!("[Terminal] pty_manager::spawn done session_id={} rows={} cols={}", session_id, rows, cols);
        Ok(session_id)
    }

    pub fn write(&self, session_id: u32, data: Vec<u8>) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        session
            .writer
            .write_all(&data)
            .map_err(|e| format!("Write error: {}", e))?;
        session
            .writer
            .flush()
            .map_err(|e| format!("Flush error: {}", e))?;

        Ok(())
    }

    pub fn resize(&self, session_id: u32, rows: u16, cols: u16) -> Result<(), String> {
        log::debug!("[Terminal] pty_manager::resize session_id={} rows={} cols={}", session_id, rows, cols);
        let sessions = self.sessions.lock().map_err(|e| format!("lock error: {}", e))?;
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        if let Some(master) = session.master.lock().map_err(|e| format!("lock error: {}", e))?.as_ref() {
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

    pub fn kill(&self, session_id: u32) -> Result<(), String> {
        log::debug!("[Terminal] pty_manager::kill session_id={}", session_id);
        // sessions ロックのスコープを最小化: remove + alive=false の設定のみ行い、
        // 重い処理（taskkill, join）はロック外で実行する。
        // taskkill /F /T は Windows 上で数秒かかることがあり、ロック保持中に実行すると
        // pty_write, pty_resize 等すべてのセッション操作がブロックされる。
        let removed = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(session) = sessions.remove(&session_id) {
                // poison でも alive=false を確実にセット（ウォッチャースレッドの停止に必要）
                match session.alive.lock() {
                    Ok(mut alive) => *alive = false,
                    Err(e) => *e.into_inner() = false,
                }
                // master を取り出して reader に EOF を送る準備
                let master = session.master.lock().ok().and_then(|mut g| g.take());
                Some((session.child_pid, session.child_killer, master, session.writer, session.watcher_handle))
            } else {
                None
            }
        }; // ← sessions ロック解放

        if let Some((child_pid, mut child_killer, master, writer, watcher_handle)) = removed {
            // ロック外でプロセスkill（taskkillが遅くても他の操作をブロックしない）
            if let Some(pid) = child_pid {
                crate::process_utils::kill_process_tree(pid);
            }
            // child_killer でバックアップ kill（child が監視スレッドに渡済みでも動作）
            let _ = child_killer.kill();
            // master / writer を drop して reader に EOF を送る
            drop(master);
            drop(writer);
            // watcher スレッドの終了を待つ（alive=false を検知して必ず終了する）
            if let Some(handle) = watcher_handle {
                let _ = handle.join();
            }
        }
        Ok(())
    }

    pub fn kill_all(&self) {
        let ids: Vec<u32> = self.sessions.lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys().cloned().collect();
        for id in ids {
            let _ = self.kill(id);
        }
    }

    /// 指定ディレクトリ以下をcwdとして持つ全PTYセッションをkillする。
    /// ワークツリー削除前にそのディレクトリを掴んでいる子プロセスを解放するために使用。
    /// 返り値: killしたセッション数（> 0 なら呼び出し側でプロセス終了待機が必要）
    pub fn kill_sessions_in_dir(&self, dir: &str) -> usize {
        let target = std::path::Path::new(dir);
        let ids: Vec<u32> = {
            let sessions = match self.sessions.lock() {
                Ok(s) => s,
                Err(e) => e.into_inner(),
            };
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
            log::info!("[Terminal] kill_sessions_in_dir: killing session {} (worktree={})", id, dir);
            let _ = self.kill(*id);
        }
        ids.len()
    }
}

impl Drop for PtyManager {
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
            self.kill_all();
        }
    }
}
