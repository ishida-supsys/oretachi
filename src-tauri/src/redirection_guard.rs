//! Windows RedirectionGuard (PROCESS_MITIGATION_REDIRECTION_TRUST_POLICY) 対策。
//!
//! インストーラ/アップデータの「完了後に起動」で立ち上がった oretachi.exe は、
//! 起動チェーンから RedirectionGuard 緩和策 (Enforce=1) を継承することがある。
//! 緩和策は子プロセスに遺伝するため、ターミナル内の pnpm 等で NTFS ジャンクションの
//! トラバースが「信頼されていないマウントポイント」エラーになる (Issue #70)。
//!
//! 起動時に自プロセスのポリシーを照会し、Enforce=1 なら explorer.exe (Enforce=0) 経由で
//! セルフリローンチしてクリーンな親から起動し直す。
//!
//! 検証済みの挙動 (2026-06-11 実機確認):
//! - 親が SetProcessMitigationPolicy で自己設定した Enforce=1 は子プロセスに継承される
//! - Enforce=1 を自プロセスから解除しようとすると ERROR_ACCESS_DENIED で拒否される
//!   (片方向 API のためリローンチ以外に回復手段がない)
//! - Enforce=1 のプロセスから explorer.exe 経由で起動した子は Enforce=0 になる

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// 起動ガードの判定結果。tauri-plugin-log の初期化前に実行されるため、
/// ログ出力は setup() に持ち越して行う。
pub enum GuardOutcome {
    /// Enforce=0 の通常起動
    NotGuarded,
    /// リローンチ直後の起動 (Enforce=0 + 直近マーカー有り)
    RelaunchedPreviously,
    /// リローンチ後もなお Enforce=1 (ループ防止のためそのまま起動継続)
    StillGuarded,
    /// Enforce=1 だがリローンチを行わず起動継続 (デバッグビルド / リローンチ失敗)
    RelaunchSkipped(String),
}

/// RedirectionGuard (Enforce=1) を検出した場合、explorer.exe 経由でセルフリローンチする。
/// リローンチする場合は内部で `exit(0)` するため戻らない。
pub fn relaunch_if_guarded() -> GuardOutcome {
    let marker_was_fresh = consume_marker();

    if !redirection_trust_enforced() {
        return if marker_was_fresh {
            GuardOutcome::RelaunchedPreviously
        } else {
            GuardOutcome::NotGuarded
        };
    }

    if marker_was_fresh {
        // 直近にリローンチ済みなのに Enforce=1 のまま (親の explorer 自体が
        // 緩和策持ち等)。再試行しても結果は変わらないため起動を継続する。
        return GuardOutcome::StillGuarded;
    }

    if cfg!(debug_assertions) {
        // `pnpm run tauri dev` 中に exit すると tauri CLI が dev server を落とし、
        // リローンチ後のプロセスが孤立するため、開発時は検出のみ行う。
        return GuardOutcome::RelaunchSkipped(
            "デバッグビルドのためリローンチをスキップ".to_string(),
        );
    }

    match write_marker_and_relaunch() {
        Ok(()) => std::process::exit(0),
        Err(reason) => GuardOutcome::RelaunchSkipped(reason),
    }
}

/// 自プロセスの RedirectionTrust ポリシーで EnforceRedirectionTrust (bit0) が
/// 立っているかを照会する。
fn redirection_trust_enforced() -> bool {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessMitigationPolicy, ProcessRedirectionTrustPolicy,
    };

    // PROCESS_MITIGATION_REDIRECTION_TRUST_POLICY は DWORD ビットフィールドの
    // union (4 bytes) のため u32 で照会する (windows-sys 0.59 に構造体定義が無い)。
    let mut flags: u32 = 0;
    let ok = unsafe {
        GetProcessMitigationPolicy(
            GetCurrentProcess(),
            ProcessRedirectionTrustPolicy,
            &mut flags as *mut u32 as *mut _,
            std::mem::size_of::<u32>(),
        )
    };
    // ポリシー未サポートの OS (Win11 22H2 未満など) では API が失敗する = 緩和策なし
    ok != 0 && (flags & 1) != 0
}

/// ループ防止マーカーのパス。explorer.exe 経由の起動では引数も環境変数も
/// 新プロセスへ渡せないため、一時ファイルで「リローンチ直後」を伝搬する。
fn marker_path() -> PathBuf {
    std::env::temp_dir().join("oretachi-redirection-guard-relaunch")
}

const MARKER_TTL: Duration = Duration::from_secs(60);

/// マーカーの mtime から「リローンチ直後」かどうかを判定する。
/// mtime が未来 (時計巻き戻し等) の場合はループ抑止を優先して fresh 扱いにする。
fn marker_is_fresh(mtime: SystemTime, now: SystemTime) -> bool {
    match now.duration_since(mtime) {
        Ok(elapsed) => elapsed <= MARKER_TTL,
        Err(_) => true,
    }
}

/// マーカーを読み取って削除し、fresh だったかを返す。
/// stale マーカーを残すと TTL 経過まで手動再起動でのリトライが効かなくなるため、
/// 結果によらず必ず削除する。
fn consume_marker() -> bool {
    let path = marker_path();
    let Ok(meta) = std::fs::metadata(&path) else {
        return false;
    };
    let fresh = meta
        .modified()
        .map(|mtime| marker_is_fresh(mtime, SystemTime::now()))
        .unwrap_or(true);
    let _ = std::fs::remove_file(&path);
    fresh
}

/// マーカーを書き込み、explorer.exe 経由で自 exe を起動する。
/// explorer は exe パス以外の引数を新プロセスへ渡せないため、コマンドライン引数は
/// 失われる (現状引数は未使用。deep link 等を導入する場合はここの再設計が必要)。
fn write_marker_and_relaunch() -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("自 exe パスの取得に失敗したためリローンチを中止: {e}"))?;

    std::fs::write(marker_path(), b"relaunch")
        .map_err(|e| format!("ループ防止マーカーの書き込みに失敗したためリローンチを中止: {e}"))?;

    // PATH 探索に依存しないよう %SystemRoot% からの絶対パスで起動する。
    // explorer は成功時にも終了コード 1 を返すことがあるため exit code は見ない。
    let explorer = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("explorer.exe"))
        .unwrap_or_else(|| PathBuf::from("explorer.exe"));

    match std::process::Command::new(explorer).arg(&exe).spawn() {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(marker_path());
            Err(format!("explorer.exe 経由のリローンチ起動に失敗: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_within_ttl_is_fresh() {
        let now = SystemTime::now();
        let mtime = now - Duration::from_secs(30);
        assert!(marker_is_fresh(mtime, now));
    }

    #[test]
    fn marker_past_ttl_is_stale() {
        let now = SystemTime::now();
        let mtime = now - Duration::from_secs(61);
        assert!(!marker_is_fresh(mtime, now));
    }

    #[test]
    fn marker_with_future_mtime_is_fresh() {
        let now = SystemTime::now();
        let mtime = now + Duration::from_secs(300);
        assert!(marker_is_fresh(mtime, now));
    }
}
