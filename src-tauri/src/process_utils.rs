use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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

    // $SHELL と実際の設定ファイルが異なる場合（例: $SHELL=zsh だが設定は ~/.bashrc）に
    // ログインシェルが成功しても必要なパスが含まれないことがある。
    // そのため well-known paths は常にマージ対象に含める。
    let well_known = [
        format!("{}/.local/bin", home),
        "/usr/local/bin".to_string(),
        "/opt/homebrew/bin".to_string(),
        format!("{}/.cargo/bin", home),
    ];

    let additional = match shell_path_result {
        Ok(shell_path) => format!("{}:{}", shell_path, well_known.join(":")),
        Err(e) => {
            log::warn!("refresh_path_from_login_shell: {}; using well-known fallback paths", e);
            well_known.join(":")
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

// ============================================================
// Windows: NtAPI (PEB経由CWD読取) によるプロセス検索・kill
// CreateToolhelp32Snapshot + NtQueryInformationProcess + ReadProcessMemory
// ============================================================

/// 指定ディレクトリ以下をカレントディレクトリとして持つ外部プロセスを検索してkillする。
/// PTYManagerが管理しないプロセス（IDE、シェルがcd済みのもの等）も対象にする。
/// 返り値: killしたプロセス数
#[cfg(target_os = "windows")]
pub fn kill_external_processes_in_dir(dir: &str) -> usize {
    let pids = find_processes_by_cwd(dir);
    let count = pids.len();
    for pid in pids {
        kill_process_tree(pid);
    }
    count
}

#[cfg(not(target_os = "windows"))]
pub fn kill_external_processes_in_dir(_dir: &str) -> usize {
    0
}

/// 指定ディレクトリ以下をcwdとして持つプロセスのPIDリストを返す（Windows専用）。
/// 自プロセスは除外する。
#[cfg(target_os = "windows")]
fn find_processes_by_cwd(dir: &str) -> Vec<u32> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;

    let our_pid = std::process::id();
    // パスを正規化: 末尾スラッシュを除去してから小文字化
    let target = normalize_path_for_cmp(dir);

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        log::warn!("find_processes_by_cwd: CreateToolhelp32Snapshot failed");
        return vec![];
    }

    let mut results = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            let pid = entry.th32ProcessID;
            if pid != our_pid && pid != 0 {
                if let Some(cwd) = get_process_cwd(pid) {
                    let cwd_norm = normalize_path_for_cmp(&cwd);
                    if cwd_norm.starts_with(&target) {
                        // target がパスの途中に一致しないよう境界チェック
                        let rest = &cwd_norm[target.len()..];
                        if rest.is_empty() || rest.starts_with('\\') || rest.starts_with('/') {
                            log::info!(
                                "find_processes_by_cwd: pid={} cwd={} matches target={}",
                                pid, cwd, dir
                            );
                            results.push(pid);
                        }
                    }
                }
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }

    unsafe { windows_sys::Win32::Foundation::CloseHandle(snapshot) };
    results
}

/// パスを比較用に正規化: バックスラッシュをフォワードスラッシュに統一、末尾スラッシュ除去、小文字化
#[cfg(target_os = "windows")]
fn normalize_path_for_cmp(path: &str) -> String {
    path.replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

/// NtQueryInformationProcess + ReadProcessMemory で PEB から CWD を読み取る（Windows専用）。
/// アクセス不可・失敗時は None を返す。
#[cfg(target_os = "windows")]
fn get_process_cwd(pid: u32) -> Option<String> {
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };
    use windows_sys::Win32::Foundation::CloseHandle;

    // ntdll の NtQueryInformationProcess を動的に取得
    // HANDLE は windows-sys 0.59 では *mut c_void
    type NtQueryFn = unsafe extern "system" fn(
        *mut std::ffi::c_void,  // ProcessHandle
        u32,                    // ProcessInformationClass
        *mut u8,                // ProcessInformation
        u32,                    // ProcessInformationLength
        *mut u32,               // ReturnLength
    ) -> i32;

    let ntdll = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(
            windows_sys::core::w!("ntdll.dll")
        )
    };
    if ntdll.is_null() {
        return None;
    }

    let fn_name = b"NtQueryInformationProcess\0";
    let nt_query: NtQueryFn = unsafe {
        let addr = windows_sys::Win32::System::LibraryLoader::GetProcAddress(ntdll, fn_name.as_ptr());
        std::mem::transmute(addr?)
    };

    let handle = unsafe {
        OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid)
    };
    if handle.is_null() {
        return None;
    }

    let result = read_cwd_from_process(handle, nt_query);
    unsafe { CloseHandle(handle) };
    result
}

/// プロセスハンドルから PEB → ProcessParameters → CurrentDirectory を読み取る。
#[cfg(target_os = "windows")]
fn read_cwd_from_process(
    handle: *mut std::ffi::c_void,
    nt_query: unsafe extern "system" fn(*mut std::ffi::c_void, u32, *mut u8, u32, *mut u32) -> i32,
) -> Option<String> {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;

    // PROCESS_BASIC_INFORMATION: 6 × pointer-sized fields
    // PebBaseAddress は offset 1 (index 1)
    const PBI_SIZE: usize = 6 * 8; // 64bit 固定
    let mut pbi = [0u8; PBI_SIZE];
    let mut ret_len: u32 = 0;
    let status = unsafe {
        nt_query(handle, 0 /*ProcessBasicInformation*/, pbi.as_mut_ptr(), PBI_SIZE as u32, &mut ret_len)
    };
    if status != 0 {
        return None;
    }

    // PebBaseAddress: offset 8 (2nd pointer, 8 bytes each on x64)
    let peb_addr = i64::from_ne_bytes(pbi[8..16].try_into().ok()?) as usize;
    if peb_addr == 0 {
        return None;
    }

    // PEB を読む (最低 0x28 バイト必要)
    let mut peb_buf = [0u8; 0x28];
    let mut bytes_read: usize = 0;
    let ok = unsafe {
        ReadProcessMemory(
            handle,
            peb_addr as *const _,
            peb_buf.as_mut_ptr() as *mut _,
            peb_buf.len(),
            &mut bytes_read,
        )
    };
    if ok == 0 || bytes_read < 0x28 {
        return None;
    }

    // ProcessParameters アドレス: PEB+0x20 (x64)
    let proc_params_addr = usize::from_ne_bytes(peb_buf[0x20..0x28].try_into().ok()?);
    if proc_params_addr == 0 {
        return None;
    }

    // RTL_USER_PROCESS_PARAMETERS を読む (CurrentDirectory は offset 0x38 から)
    // UNICODE_STRING: Length(u16) + MaximumLength(u16) + [padding 4 bytes] + Buffer(*u16)
    // offset 0x38: Length (u16)
    // offset 0x3A: MaximumLength (u16)
    // offset 0x40: Buffer (u64 on x64)
    let read_size = 0x48usize;
    let mut pp_buf = vec![0u8; read_size];
    let mut bytes_read: usize = 0;
    let ok = unsafe {
        ReadProcessMemory(
            handle,
            proc_params_addr as *const _,
            pp_buf.as_mut_ptr() as *mut _,
            read_size,
            &mut bytes_read,
        )
    };
    if ok == 0 || bytes_read < read_size {
        return None;
    }

    let cwd_len = u16::from_ne_bytes(pp_buf[0x38..0x3A].try_into().ok()?) as usize;
    let cwd_buf_addr = usize::from_ne_bytes(pp_buf[0x40..0x48].try_into().ok()?);
    if cwd_len == 0 || cwd_buf_addr == 0 || cwd_len > 0x800 {
        return None;
    }

    // CWD 文字列（UTF-16LE）を読む
    let mut str_buf = vec![0u8; cwd_len];
    let mut bytes_read: usize = 0;
    let ok = unsafe {
        ReadProcessMemory(
            handle,
            cwd_buf_addr as *const _,
            str_buf.as_mut_ptr() as *mut _,
            cwd_len,
            &mut bytes_read,
        )
    };
    if ok == 0 || bytes_read < cwd_len {
        return None;
    }

    // UTF-16LE → String
    let u16_slice: Vec<u16> = str_buf
        .chunks_exact(2)
        .map(|b| u16::from_ne_bytes([b[0], b[1]]))
        .collect();
    // 末尾の NUL および末尾スラッシュを除去
    let s = String::from_utf16_lossy(&u16_slice);
    let s = s.trim_end_matches('\0').trim_end_matches(['\\', '/']).to_string();
    if s.is_empty() { None } else { Some(s) }
}

// ============================================================
// WorktreeRemoveManager: ワークツリー削除のキャンセルフラグ管理
// ============================================================

/// ワークツリー削除の永続リトライをキャンセルするためのフラグ管理。
/// キーは worktree_path（バックエンドコマンドが受け取るパスと一致させる）。
pub struct WorktreeRemoveManager {
    cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl WorktreeRemoveManager {
    pub fn new() -> Self {
        Self {
            cancel_flags: Mutex::new(HashMap::new()),
        }
    }

    /// キャンセルフラグを作成して返す（削除開始時に呼ぶ）
    pub fn create_cancel_flag(&self, key: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let mut map = self.cancel_flags.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(key.to_string(), flag.clone());
        flag
    }

    /// キャンセルフラグをセットする（キャンセル要求時に呼ぶ）。
    /// フラグが存在した場合 true を返す。
    pub fn cancel(&self, key: &str) -> bool {
        let map = self.cancel_flags.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(flag) = map.get(key) {
            flag.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// フラグを削除する（削除完了/キャンセル完了時に呼ぶ）
    pub fn remove(&self, key: &str) {
        let mut map = self.cancel_flags.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(key);
    }
}
