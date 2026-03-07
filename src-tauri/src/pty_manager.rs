use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

struct PtySession {
    writer: Box<dyn Write + Send>,
    master: Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>>,
    child_killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    child_pid: Option<u32>,
    alive: Arc<Mutex<bool>>,
}

pub struct PtyManager {
    sessions: Mutex<HashMap<u32, PtySession>>,
    next_id: Mutex<u32>,
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

fn kill_process_tree_by_pid(_pid: Option<u32>) {
    #[cfg(target_os = "windows")]
    if let Some(pid) = _pid {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    #[cfg(not(target_os = "windows"))]
    if let Some(pid) = _pid {
        // プロセスグループごと SIGTERM (負の PID でグループ指定)
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
        }
        // フォールバック: プロセス単体も kill
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
}

impl PtyManager {
    pub fn new() -> Self {
        PtyManager {
            sessions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }

    pub fn spawn(
        &self,
        app_handle: AppHandle,
        rows: u16,
        cols: u16,
        shell: Option<String>,
        cwd: Option<String>,
    ) -> Result<u32, String> {
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
                std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
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
            cmd.arg("-NoExit");
            cmd.arg("-Command");
            cmd.arg(
                r#"$__p=$function:prompt;function prompt{$ec=$LASTEXITCODE;[Console]::Write([char]27+']777;exit_code;'+$ec+[char]7);if($__p){&$__p}else{"PS $($executionContext.SessionState.Path.CurrentLocation)$('>'*($nestedPromptLevel+1)) "}}"#,
            );
        }

        if let Some(dir) = cwd {
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
            let mut id = self.next_id.lock().unwrap();
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
        std::thread::spawn(move || {
            let child_opt = child_watcher.lock().unwrap().take();
            if let Some(mut child) = child_opt {
                let _ = child.wait();
                // プロセス終了後、alive が true (kill() 未呼び出し) なら master を drop
                let mut alive_guard = alive_watcher.lock().unwrap();
                if *alive_guard {
                    *alive_guard = false;
                    drop(alive_guard);
                    // master を drop して reader に EOF を送る
                    let _ = master_watcher.lock().unwrap().take();
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
        };

        self.sessions.lock().unwrap().insert(session_id, session);

        Ok(session_id)
    }

    pub fn write(&self, session_id: u32, data: Vec<u8>) -> Result<(), String> {
        let mut sessions = self.sessions.lock().unwrap();
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
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        if let Some(master) = session.master.lock().unwrap().as_ref() {
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
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(mut session) = sessions.remove(&session_id) {
            *session.alive.lock().unwrap() = false;
            // PID ベースで子プロセスツリーを kill
            kill_process_tree_by_pid(session.child_pid);
            // child_killer でバックアップ kill（child が監視スレッドに渡済みでも動作）
            let _ = session.child_killer.kill();
            // master を drop して reader に EOF を送る
            let _ = session.master.lock().unwrap().take();
            drop(session.writer);
        }
        Ok(())
    }

    pub fn kill_all(&self) {
        let ids: Vec<u32> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            let _ = self.kill(id);
        }
    }
}

impl Drop for PtyManager {
    fn drop(&mut self) {
        self.kill_all();
    }
}
