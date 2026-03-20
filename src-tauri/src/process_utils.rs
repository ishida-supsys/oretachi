use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x08000000;

/// コンソールウィンドウを表示しない設定で std::process::Command を作成する
pub fn make_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// コンソールウィンドウを表示しない設定で tokio::process::Command を作成する
pub fn make_async_command(program: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// キー（ワークツリーID等）ごとに進行中プロセスのPIDを追跡し、キャンセルを可能にする
pub struct CancellableManager {
    in_progress: Mutex<HashMap<String, u32>>,
}

impl CancellableManager {
    pub fn new() -> Self {
        Self {
            in_progress: Mutex::new(HashMap::new()),
        }
    }

    /// キーが進行中かチェック
    pub fn is_in_progress(&self, key: &str) -> Result<bool, String> {
        let map = self.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        Ok(map.contains_key(key))
    }

    /// PIDを登録
    pub fn register(&self, key: String, pid: u32) -> Result<(), String> {
        let mut map = self.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        map.insert(key, pid);
        Ok(())
    }

    /// PIDを削除して返す
    pub fn remove(&self, key: &str) -> Result<Option<u32>, String> {
        let mut map = self.in_progress.lock().map_err(|e| format!("lock error: {}", e))?;
        Ok(map.remove(key))
    }

    /// PIDを削除してプロセスツリーをkill
    pub fn cancel(&self, key: &str) -> Result<(), String> {
        if let Some(pid) = self.remove(key)? {
            kill_process_tree(pid);
        }
        Ok(())
    }
}

/// プロセスツリーを強制終了する
pub fn kill_process_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let _ = make_command("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }

    #[cfg(not(target_os = "windows"))]
    {
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
        }
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
}
