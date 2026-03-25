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

/// Windowsレジストリからシステム・ユーザーのPATHを読み取り結合して返す。
/// アップデート後の再起動でPATHが不完全になる問題の対策。
#[cfg(target_os = "windows")]
pub fn refresh_path_from_registry() -> Result<String, String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    let system_key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            KEY_READ,
        )
        .map_err(|e| format!("Failed to open HKLM key: {}", e))?;
    let system_path: String = system_key
        .get_value("Path")
        .unwrap_or_default();

    let user_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Environment", KEY_READ)
        .map_err(|e| format!("Failed to open HKCU key: {}", e))?;
    let user_path: String = user_key
        .get_value("Path")
        .unwrap_or_default();

    Ok(format!("{};{}", system_path, user_path))
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
        // SIGTERM を無視するプロセス対策: 短い猶予後に SIGKILL で強制終了
        std::thread::sleep(std::time::Duration::from_millis(100));
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
}
