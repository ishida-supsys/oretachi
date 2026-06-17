//! Windows Job Object によるプロセス終了時の子プロセス巻き込み終了。
//!
//! 起動時に `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` を設定した Job を作り、自プロセスを
//! 割り当てる。子プロセス（WebView2 の `msedgewebview2.exe` 等）は既定で親の Job を継承する
//! ため、親プロセスが終了（正常/異常/クラッシュ/強制終了問わず）して最後の Job ハンドルが
//! 閉じられると、OS が Job 内の全プロセスを終了させる。
//!
//! これは `RunEvent::Exit` 経由のクリーンアップ（通常終了）が走らないケース
//! （クラッシュ・タスクマネージャからの強制終了）でも効く、OS レベルのセーフティネット。

#[cfg(target_os = "windows")]
mod imp {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    /// 割当済みフラグ兼ハンドル記録。
    /// このハンドルは **決して CloseHandle しない**（最後のハンドルを閉じると
    /// KILL_ON_JOB_CLOSE により自プロセスごと即終了してしまうため）。プロセス終了時に
    /// OS が暗黙クローズするのに委ねる。
    static JOB: OnceLock<usize> = OnceLock::new();

    pub fn assign_current_process_to_job() {
        if JOB.get().is_some() {
            return;
        }
        unsafe {
            let job: HANDLE = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                log::warn!("[job] CreateJobObjectW failed; child processes may be orphaned on exit");
                return;
            }

            // UI limits は設定しない（設定すると nested job への割当が失敗する）。
            // KILL_ON_JOB_CLOSE のみ設定する。SILENT_BREAKAWAY も設定しない（孤児化を招くため）。
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                log::warn!("[job] SetInformationJobObject failed; child processes may be orphaned on exit");
                return;
            }

            // 既に別 Job（UI limits 付き等）配下だと失敗し得る。致命にせず従来挙動へ degrade。
            let ok = AssignProcessToJobObject(job, GetCurrentProcess());
            if ok == 0 {
                log::warn!("[job] AssignProcessToJobObject failed; child processes may be orphaned on exit");
                return;
            }

            let _ = JOB.set(job as usize);
            log::info!("[job] assigned current process to kill-on-close job object");
        }
    }
}

#[cfg(target_os = "windows")]
pub use imp::assign_current_process_to_job;

/// 非 Windows では no-op。
#[cfg(not(target_os = "windows"))]
pub fn assign_current_process_to_job() {}
