use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
pub const CREATE_NO_WINDOW: u32 = 0x08000000;

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
