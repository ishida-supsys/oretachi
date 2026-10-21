//! heartbeat 向けの補助計測: WebView2 の子プロセス(browser / renderer / gpu-process / utility)の
//! CPU 時間を一定周期で差分計測し、ログに 1 行残す。
//!
//! 2026-10-21 のハング(PC 全体が操作不能)では、oretachi.exe の UI スレッド自体は軽く、
//! 負荷は WebView2 の renderer / gpu-process と DWM に乗っていた。`[heartbeat] pong` の
//! rtt や JS の blockedMs にはそれが出ないため、停止の数分前から負荷が上がっていたのかを
//! 後から追えるように、ここで別途記録する。
//!
//! 計測は専用スレッドで行う(全プロセス列挙を UI スレッドや tokio ワーカーに載せない)。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// 計測周期。heartbeat の ping(30 秒)と揃える。
const INTERVAL: Duration = Duration::from_secs(30);

/// 計測スレッドを起動する。アプリ起動時に 1 回呼ぶ。
pub fn start() {
    let spawned = std::thread::Builder::new()
        .name("webview-proc-stats".into())
        .spawn(run);
    if let Err(e) = spawned {
        log::warn!("[heartbeat] procs: 計測スレッドを起動できません: {}", e);
    }
}

fn run() {
    let mut sampler = Sampler::new();
    loop {
        std::thread::sleep(INTERVAL);
        if let Some(line) = sampler.sample() {
            log::debug!("[heartbeat] procs {}", line);
        }
    }
}

struct Sampler {
    sys: System,
    /// 前回の pid → 累積 CPU 時間(ms)
    prev: HashMap<Pid, u64>,
    prev_at: Option<Instant>,
}

/// 1 カテゴリぶんの集計。
#[derive(Default)]
struct Bucket {
    /// CPU 時間の増分(ms)
    cpu_ms: u64,
    /// プロセス数
    count: u32,
    /// 常駐メモリ合計(bytes)
    rss: u64,
}

impl Sampler {
    fn new() -> Self {
        Self { sys: System::new(), prev: HashMap::new(), prev_at: None }
    }

    /// 前回からの差分で各カテゴリの CPU 使用率(1 コア = 100%)を文字列にする。初回は `None`。
    fn sample(&mut self) -> Option<String> {
        // `refresh_processes` の既定はコマンドラインを取らない(`--type=` の判別に要る)ので明示する。
        // cmd は起動後に変わらないため一度取れれば以後は更新しない。
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu()
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        let now = Instant::now();
        let self_pid = Pid::from_u32(std::process::id());

        // 自プロセスの子孫を集める(WebView2 は browser → renderer/gpu の 2 段)
        let mut descendants: HashSet<Pid> = HashSet::new();
        descendants.insert(self_pid);
        loop {
            let before = descendants.len();
            for (pid, proc_) in self.sys.processes() {
                if let Some(parent) = proc_.parent() {
                    if descendants.contains(&parent) {
                        descendants.insert(*pid);
                    }
                }
            }
            if descendants.len() == before {
                break;
            }
        }

        let mut buckets: HashMap<&'static str, Bucket> = HashMap::new();
        let mut next: HashMap<Pid, u64> = HashMap::new();
        for (pid, proc_) in self.sys.processes() {
            // dwm.exe も見たいところだが、別ユーザー(DWM-1)のプロセスで非管理者からは
            // CPU 時間が読めない(sysinfo も PowerShell も 0 を返す)ので対象にしない。
            if !descendants.contains(pid) || !proc_.name().eq_ignore_ascii_case("msedgewebview2.exe") {
                continue;
            }
            let category = webview_kind(proc_.cmd().iter().map(|s| s.to_string_lossy()));
            let cpu = proc_.accumulated_cpu_time();
            next.insert(*pid, cpu);
            let bucket = buckets.entry(category).or_default();
            bucket.count += 1;
            bucket.rss += proc_.memory();
            if let Some(prev) = self.prev.get(pid) {
                bucket.cpu_ms += cpu.saturating_sub(*prev);
            }
        }

        let elapsed_ms = self.prev_at.map(|t| now.duration_since(t).as_millis() as u64);
        self.prev = next;
        self.prev_at = Some(now);
        let elapsed_ms = elapsed_ms.filter(|ms| *ms > 0)?;
        // WebView2 の子が居ない(Windows 以外など)なら何も出さない
        if buckets.is_empty() {
            return None;
        }

        let mut parts: Vec<String> = Vec::new();
        for key in ["browser", "renderer", "gpu", "utility", "other"] {
            let Some(b) = buckets.get(key) else { continue };
            let pct = b.cpu_ms * 100 / elapsed_ms;
            if b.count > 1 {
                parts.push(format!("{}={}%({}p,{}MB)", key, pct, b.count, b.rss / 1024 / 1024));
            } else {
                parts.push(format!("{}={}%({}MB)", key, pct, b.rss / 1024 / 1024));
            }
        }
        Some(parts.join(" "))
    }
}

/// Chromium 系の `--type=` 引数からプロセス種別を引く。無ければ browser プロセス。
fn webview_kind<'a>(args: impl Iterator<Item = std::borrow::Cow<'a, str>>) -> &'static str {
    for arg in args {
        if let Some(kind) = arg.strip_prefix("--type=") {
            return match kind {
                "renderer" => "renderer",
                "gpu-process" => "gpu",
                "utility" => "utility",
                _ => "other",
            };
        }
    }
    "browser"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn webview_kind_reads_type_arg() {
        let args = |v: &[&'static str]| v.iter().map(|s| Cow::Borrowed(*s)).collect::<Vec<_>>();
        assert_eq!(webview_kind(args(&["x.exe", "--type=renderer", "--lang=ja"]).into_iter()), "renderer");
        assert_eq!(webview_kind(args(&["x.exe", "--type=gpu-process"]).into_iter()), "gpu");
        assert_eq!(webview_kind(args(&["x.exe", "--type=utility"]).into_iter()), "utility");
        assert_eq!(webview_kind(args(&["x.exe", "--type=crashpad-handler"]).into_iter()), "other");
        assert_eq!(webview_kind(args(&["x.exe", "--embedded-browser-webview=1"]).into_iter()), "browser");
    }
}
