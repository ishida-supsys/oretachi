//! メインスレッド (tao/WebView2 UI スレッド) のブレッドクラム計装。
//!
//! UI スレッドで「最後に開始した処理」のラベルと開始時刻を、**ヒープ確保なし・ロックなし**の
//! atomic だけで保持する。`watchdog` がブロックを検知したとき [`snapshot`] を読めば、
//! 停止フレームが「同期 `#[tauri::command]` 内」か「ネイティブ event loop 側 (= Idle)」かを
//! 切り分けられる。これがフリーズ調査の核心的な手がかりになる。
//!
//! - [`enter`] は **UI スレッドからのみ** 呼ぶ。返り値の [`Guard`] が drop されると直前の値へ
//!   復元するため、入れ子で呼んでも整合する。
//! - [`snapshot`] は別スレッド (watchdog) から読む。Relaxed atomic 越しの読み取りなので
//!   ラベルと経過時間が一瞬ズレることはあるが、診断用途では問題ない (UB は発生しない)。
//! - プラットフォーム非依存 (atomic のみ)。

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// 経過 ms を測る単調時計の起点。`run()` 冒頭で [`init`] が一度だけ設定する。
static START: OnceLock<Instant> = OnceLock::new();
/// 現在 UI スレッドで実行中の処理ラベル ([`Activity`] の discriminant)。
static CUR_LABEL: AtomicU8 = AtomicU8::new(Activity::Idle as u8);
/// 現在の処理を開始した時刻 (START からの経過 ms)。
static CUR_SINCE_MS: AtomicU64 = AtomicU64::new(0);

/// UI スレッドで起こりうる処理の種別。`#[repr(u8)]` の discriminant が [`LABELS`] の添字に対応する
/// (順序を一致させること。テストで検証している)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Activity {
    Idle = 0,
    RunEvent,
    WatchdogProbe,
    WindowReady,
    PtyWrite,
    PtySetAiAgent,
    PtyIsAiAgent,
    GetSettings,
    SaveSettings,
    ApplyAcrylic,
    DetectIdes,
    DetectAiAgents,
    OpenInIde,
    RegenerateMcpApiKey,
    GetMcpStatus,
    PathExists,
    NotifyWorktreeArchived,
    NotifyWorktreeAdded,
    StartFsWatch,
    StopFsWatch,
    SetDebugMode,
    GetDebugMode,
    GetForceWizard,
}

/// [`Activity`] の discriminant 順に並んだ表示ラベル。enum と順序を一致させること。
const LABELS: [&str; 23] = [
    "idle",
    "run-event",
    "watchdog-probe",
    "window-ready",
    "cmd:pty_write",
    "cmd:pty_set_ai_agent",
    "cmd:pty_is_ai_agent",
    "cmd:get_settings",
    "cmd:save_settings",
    "cmd:apply_acrylic_effect",
    "cmd:detect_ides",
    "cmd:detect_ai_agents",
    "cmd:open_in_ide",
    "cmd:regenerate_mcp_api_key",
    "cmd:get_mcp_status",
    "cmd:path_exists",
    "cmd:notify_worktree_archived",
    "cmd:notify_worktree_added",
    "cmd:start_fs_watch",
    "cmd:stop_fs_watch",
    "cmd:set_debug_mode",
    "cmd:get_debug_mode",
    "cmd:get_force_wizard",
];

/// discriminant から表示ラベルを引く。範囲外は `"?"`。
fn name_for(v: u8) -> &'static str {
    LABELS.get(v as usize).copied().unwrap_or("?")
}

/// UI (tao/WebView2) スレッドの OS スレッド ID。`init()` が UI スレッドで記録する。
/// minidump 解析時に「どのスレッドが UI スレッドか」を名指しするために使う。
#[cfg(target_os = "windows")]
static UI_THREAD_ID: AtomicU64 = AtomicU64::new(0);

/// minidump を 1 回だけ書き出すためのガード。
#[cfg(target_os = "windows")]
static DUMP_WRITTEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 単調時計を初期化する。`run()` の冒頭 (= UI スレッド) で一度だけ呼ぶ。
pub fn init() {
    let _ = START.set(Instant::now());
    #[cfg(target_os = "windows")]
    unsafe {
        UI_THREAD_ID.store(
            windows_sys::Win32::System::Threading::GetCurrentThreadId() as u64,
            Ordering::Relaxed,
        );
    }
}

/// 記録済みの UI スレッド ID (Windows)。未記録なら 0。
#[cfg(target_os = "windows")]
pub fn ui_thread_id() -> u64 {
    UI_THREAD_ID.load(Ordering::Relaxed)
}

/// メインスレッドのハング初検知時に、プロセス全体のミニダンプを **1 回だけ** 書き出す (Windows 限定)。
///
/// `SuspendThread` + dbghelp `StackWalk64` の in-process シンボル化は、UI スレッドが
/// heap/loader/dbghelp ロックを保持していると診断スレッド側で二重デッドロックを起こすため採用しない。
/// 代わりにクラッシュダンプ用に設計された [`MiniDumpWriteDump`] でスナップショットをファイルに保存し、
/// シンボル化はオフラインで行う。専用スレッドで実行して tokio ランタイムや watchdog を塞がず、
/// 万一ダンプ自体がハングしても影響を 1 スレッドに限定する。`AtomicBool` で 1 回限りに制限する。
///
/// [`MiniDumpWriteDump`]: https://learn.microsoft.com/windows/win32/api/minidumpapiset/nf-minidumpapiset-minidumpwritedump
#[cfg(target_os = "windows")]
pub fn write_hang_minidump_once(dump_path: std::path::PathBuf) {
    if DUMP_WRITTEN.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::Diagnostics::Debug::{
            MiniDumpNormal, MiniDumpWithThreadInfo, MiniDumpWriteDump,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId};

        let file = match std::fs::File::create(&dump_path) {
            Ok(f) => f,
            Err(e) => {
                log::error!(
                    "[main-thread-watchdog] minidump ファイル作成失敗 {:?}: {}",
                    dump_path,
                    e
                );
                return;
            }
        };
        let hfile = file.as_raw_handle() as HANDLE;
        // スレッド情報付きの標準ダンプ。スタック歩行に必要な最小限のメモリを含む (シンボルは含まない)。
        let dump_type = MiniDumpNormal | MiniDumpWithThreadInfo;
        let ok = unsafe {
            MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                hfile,
                dump_type,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        // file の HANDLE は MiniDumpWriteDump 完了まで生かす。
        drop(file);
        if ok != 0 {
            log::error!(
                "[main-thread-watchdog] minidump 書き出し完了: {:?} (windbg/VS で UI スレッドのスタックを解析)",
                dump_path
            );
        } else {
            log::error!(
                "[main-thread-watchdog] MiniDumpWriteDump 失敗: {:?}",
                dump_path
            );
        }
    });
}

/// START からの経過 ms。未初期化なら 0。
fn now_ms() -> u64 {
    START.get().map(|s| s.elapsed().as_millis() as u64).unwrap_or(0)
}

/// 現在の UI スレッド処理を `a` に切り替える。返り値の [`Guard`] が drop されると直前の値へ復元する。
/// **UI スレッドからのみ** 呼ぶこと。
#[must_use = "Guard を drop すると直前のブレッドクラムへ復元される。即 drop すると計測されない"]
pub fn enter(a: Activity) -> Guard {
    let prev_label = CUR_LABEL.swap(a as u8, Ordering::Relaxed);
    let prev_since = CUR_SINCE_MS.swap(now_ms(), Ordering::Relaxed);
    Guard {
        prev_label,
        prev_since,
    }
}

/// [`enter`] のスコープガード。drop 時に直前のブレッドクラムへ復元する。
pub struct Guard {
    prev_label: u8,
    prev_since: u64,
}

impl Drop for Guard {
    fn drop(&mut self) {
        CUR_SINCE_MS.store(self.prev_since, Ordering::Relaxed);
        CUR_LABEL.store(self.prev_label, Ordering::Relaxed);
    }
}

/// watchdog がブロック検知時に読むスナップショット。
pub struct Snapshot {
    /// 現在 UI スレッドで実行中の処理ラベル。`"idle"` ならネイティブ event loop 側で停止している。
    pub label: &'static str,
    /// その処理が開始してからの経過 ms。
    pub age_ms: u64,
}

/// 現在のブレッドクラムを読み取る (別スレッドから呼んでよい)。
pub fn snapshot() -> Snapshot {
    let label = name_for(CUR_LABEL.load(Ordering::Relaxed));
    let since = CUR_SINCE_MS.load(Ordering::Relaxed);
    let age_ms = now_ms().saturating_sub(since);
    Snapshot { label, age_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_len_matches_max_discriminant() {
        // 最後の variant の discriminant + 1 が LABELS の長さと一致すること。
        assert_eq!(LABELS.len(), Activity::GetForceWizard as usize + 1);
    }

    #[test]
    fn enum_discriminant_maps_to_label() {
        // 代表的な variant が想定ラベルに対応すること (enum と LABELS の順序ズレ検出)。
        assert_eq!(name_for(Activity::Idle as u8), "idle");
        assert_eq!(name_for(Activity::RunEvent as u8), "run-event");
        assert_eq!(name_for(Activity::PtyWrite as u8), "cmd:pty_write");
        assert_eq!(name_for(Activity::SaveSettings as u8), "cmd:save_settings");
        assert_eq!(name_for(Activity::GetForceWizard as u8), "cmd:get_force_wizard");
    }

    #[test]
    fn name_for_out_of_range_is_question_mark() {
        assert_eq!(name_for(254), "?");
    }

    #[test]
    fn guard_restores_previous_breadcrumb() {
        init();
        // 入れ子: 外側 PtyWrite → 内側 SaveSettings → 内側 drop で PtyWrite へ復元
        let _outer = enter(Activity::PtyWrite);
        assert_eq!(snapshot().label, "cmd:pty_write");
        {
            let _inner = enter(Activity::SaveSettings);
            assert_eq!(snapshot().label, "cmd:save_settings");
        }
        assert_eq!(snapshot().label, "cmd:pty_write");
        drop(_outer);
        assert_eq!(snapshot().label, "idle");
    }
}
