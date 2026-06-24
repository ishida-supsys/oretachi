//! システム全体の CPU/メモリ/ネットワーク使用状況を周期取得し、
//! `system-metrics` イベントでフロントエンド（猫ターミナル）へ通知する。
//!
//! 猫ターミナルはホーム画面の装飾オーバーレイであり特定の PTY セッションに
//! 紐づかないため、ここではシステム全体の指標を扱う。
//! CPU 使用率とネットワークレートは前回 refresh からの差分で算出されるため、
//! `System` / `Networks` をポーリングスレッド内で生かし続ける。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sysinfo::{Networks, System};
use tauri::{AppHandle, Emitter};

/// ポーリング間隔（秒）。sysinfo の MINIMUM_CPU_UPDATE_INTERVAL より十分長い。
const POLL_INTERVAL_SECS: u64 = 2;

/// 将来 disk 等の指標を増やせるよう、各指標をフラットに並べた構造体。
#[derive(Clone, serde::Serialize)]
pub struct SystemMetrics {
    /// システム全体の CPU 使用率（%）
    pub cpu_percent: f32,
    /// 使用中メモリ（バイト）
    pub mem_used: u64,
    /// 総メモリ（バイト）
    pub mem_total: u64,
    /// 受信レート（バイト/秒）
    pub net_rx_per_sec: u64,
    /// 送信レート（バイト/秒）
    pub net_tx_per_sec: u64,
}

/// システムメトリクスのポーリング制御を保持する managed state。
pub struct SystemMetricsState {
    /// ポーリングスレッドが稼働すべきか（stop で false）
    running: Arc<AtomicBool>,
    /// 起動世代。start のたびに増加させ、古いスレッドを確実に終了させる。
    epoch: Arc<AtomicU64>,
}

impl SystemMetricsState {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 2秒周期でシステムメトリクスを emit するスレッドを起動する。
    /// 世代チェックにより、同時に稼働するスレッドは最新の1本のみとなる。
    pub fn start(&self, app_handle: AppHandle) {
        let my_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.running.store(true, Ordering::SeqCst);

        let running = self.running.clone();
        let epoch = self.epoch.clone();

        std::thread::spawn(move || {
            let mut sys = System::new();
            let mut networks = Networks::new_with_refreshed_list();

            // 初回 CPU サンプル（差分計算の基準。この時点の値は 0 のため捨てる）
            sys.refresh_cpu_all();

            while running.load(Ordering::SeqCst) && epoch.load(Ordering::SeqCst) == my_epoch {
                std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
                if !running.load(Ordering::SeqCst) || epoch.load(Ordering::SeqCst) != my_epoch {
                    break;
                }

                sys.refresh_cpu_all();
                sys.refresh_memory();
                networks.refresh(false);

                // refresh 以降の差分（= POLL_INTERVAL_SECS 間の受信/送信量）を全 IF で合算
                let mut rx = 0u64;
                let mut tx = 0u64;
                for (_, data) in &networks {
                    rx += data.received();
                    tx += data.transmitted();
                }

                let metrics = SystemMetrics {
                    cpu_percent: sys.global_cpu_usage(),
                    mem_used: sys.used_memory(),
                    mem_total: sys.total_memory(),
                    net_rx_per_sec: rx / POLL_INTERVAL_SECS,
                    net_tx_per_sec: tx / POLL_INTERVAL_SECS,
                };

                // ウィンドウ破棄後などで emit に失敗したら終了
                if app_handle.emit("system-metrics", &metrics).is_err() {
                    break;
                }
            }
        });
    }

    /// ポーリングを停止する（次の世代開始まで再開しない）。
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}
