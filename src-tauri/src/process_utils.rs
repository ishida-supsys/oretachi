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

/// Windowsレジストリからシステム・ユーザーのPATHを読み取り、環境変数展開して結合して返す。
/// アップデート後の再起動でPATHが不完全になる問題の対策。
/// REG_EXPAND_SZ の %SystemRoot% 等を ExpandEnvironmentStringsW で展開する。
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
    let system_path: String = system_key.get_value("Path").unwrap_or_default();

    let user_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Environment", KEY_READ)
        .map_err(|e| format!("Failed to open HKCU key: {}", e))?;
    let user_path: String = user_key.get_value("Path").unwrap_or_default();

    let combined = format!("{};{}", system_path, user_path);
    Ok(expand_env_vars(&combined))
}

/// `%VAR%` 形式の環境変数プレースホルダを ExpandEnvironmentStringsW で展開する
#[cfg(target_os = "windows")]
fn expand_env_vars(s: &str) -> String {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;

    let wide: Vec<u16> = OsString::from(s).encode_wide().chain(std::iter::once(0)).collect();
    let needed = unsafe { ExpandEnvironmentStringsW(wide.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        return s.to_string();
    }
    let mut buf: Vec<u16> = vec![0u16; needed as usize];
    let written = unsafe { ExpandEnvironmentStringsW(wide.as_ptr(), buf.as_mut_ptr(), needed) };
    if written == 0 || written > needed {
        return s.to_string();
    }
    // written には終端 NUL を含む文字数が返る
    OsString::from_wide(&buf[..written as usize - 1]).to_string_lossy().into_owned()
}

/// macOS のログインシェルから PATH を取得して現在の PATH とマージして返す。
/// GUI アプリは launchd の最小 PATH しか継承しないため、ユーザーの完全な PATH を取得する。
#[cfg(target_os = "macos")]
pub fn refresh_path_from_login_shell() -> Result<String, String> {
    use std::sync::mpsc;
    use std::thread;

    let current_path = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    // ログインシェルから PATH を取得（2秒タイムアウト）
    let shell_path_result: Result<String, String> = {
        let (tx, rx) = mpsc::channel();
        let shell_clone = shell.clone();
        thread::spawn(move || {
            let result = Command::new(&shell_clone)
                .args(["-l", "-c", "echo $PATH"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    // motd/バナー出力を避けるため最後の非空行を取得
                    stdout.lines().filter(|l| !l.trim().is_empty()).last().map(|s| s.trim().to_string())
                })
                .filter(|s| !s.is_empty() && s.contains('/'));
            let _ = tx.send(result);
        });
        match rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Some(path)) => Ok(path),
            Ok(None) => Err("login shell returned empty PATH".to_string()),
            Err(_) => Err("login shell timed out".to_string()),
        }
    };

    let additional = match shell_path_result {
        Ok(shell_path) => shell_path,
        Err(e) => {
            log::warn!("refresh_path_from_login_shell: {}; using well-known fallback paths", e);
            // フォールバック: 既知の一般的なパス
            [
                format!("{}/.local/bin", home),
                "/usr/local/bin".to_string(),
                "/opt/homebrew/bin".to_string(),
                format!("{}/.cargo/bin", home),
            ]
            .join(":")
        }
    };

    // 現在の PATH と追加パスをマージして重複排除
    Ok(merge_paths(&current_path, &additional))
}

#[cfg(target_os = "macos")]
fn merge_paths(base: &str, additions: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for p in base.split(':').chain(additions.split(':')) {
        let p = p.trim();
        if !p.is_empty() && seen.insert(p.to_string()) {
            result.push(p.to_string());
        }
    }
    result.join(":")
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
