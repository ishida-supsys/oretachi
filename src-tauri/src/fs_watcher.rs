use notify_debouncer_mini::{
    new_debouncer,
    notify::RecursiveMode,
    DebounceEventResult,
    Debouncer,
};
use notify_debouncer_mini::notify::RecommendedWatcher;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(2000);

struct ThrottleState {
    last_emit: Option<Instant>,
    pending: bool,
}

struct WatcherHandle {
    _debouncer: Debouncer<RecommendedWatcher>,
}

pub struct FsWatcherManager {
    watchers: Mutex<HashMap<String, WatcherHandle>>,
}

impl FsWatcherManager {
    pub fn new() -> Self {
        Self {
            watchers: Mutex::new(HashMap::new()),
        }
    }

    pub fn start_watching(
        &self,
        app_handle: AppHandle,
        worktree_id: String,
        worktree_path: String,
    ) -> Result<(), String> {
        let mut watchers = self
            .watchers
            .lock()
            .map_err(|e| format!("lock error: {}", e))?;

        // 既存のウォッチャーは停止してから再起動
        watchers.remove(&worktree_id);

        let worktree_id_clone = worktree_id.clone();
        let throttle_state = Arc::new(Mutex::new(ThrottleState {
            last_emit: None,
            pending: false,
        }));

        let mut debouncer = new_debouncer(
            Duration::from_millis(500),
            move |res: DebounceEventResult| match res {
                Ok(events) => {
                    let relevant = events.iter().any(|e| is_relevant_path(&e.path));
                    if !relevant {
                        return;
                    }

                    let now = Instant::now();
                    let mut state = match throttle_state.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };

                    let elapsed = state.last_emit
                        .map(|t| now.duration_since(t))
                        .unwrap_or(Duration::MAX);

                    if elapsed >= MIN_EMIT_INTERVAL {
                        // 前回emitから十分時間が経過 → 即emit
                        state.last_emit = Some(now);
                        drop(state);
                        if let Some(window) = app_handle
                            .get_webview_window(&format!("codereview-{}", worktree_id_clone))
                        {
                            use tauri::Emitter;
                            let _ = window.emit("codereview-fs-changed", ());
                        }
                    } else if !state.pending {
                        // スロットル中かつ pending なし → trailing emit をスケジュール
                        state.pending = true;
                        let remaining = MIN_EMIT_INTERVAL - elapsed;
                        drop(state);

                        let app_handle2 = app_handle.clone();
                        let worktree_id2 = worktree_id_clone.clone();
                        let throttle_state2 = throttle_state.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(remaining).await;
                            if let Ok(mut s) = throttle_state2.lock() {
                                s.pending = false;
                                s.last_emit = Some(Instant::now());
                            }
                            if let Some(window) = app_handle2
                                .get_webview_window(&format!("codereview-{}", worktree_id2))
                            {
                                use tauri::Emitter;
                                let _ = window.emit("codereview-fs-changed", ());
                            }
                        });
                    }
                    // pending が既にある場合は何もしない（重複防止）
                }
                Err(e) => {
                    log::warn!("fs_watcher error for {}: {:?}", worktree_id_clone, e);
                }
            },
        )
        .map_err(|e| format!("debouncer create error: {}", e))?;

        debouncer
            .watcher()
            .watch(Path::new(&worktree_path), RecursiveMode::Recursive)
            .map_err(|e| format!("watch error: {}", e))?;

        log::info!(
            "fs_watcher: started watching '{}' for worktree '{}'",
            worktree_path,
            worktree_id
        );

        watchers.insert(worktree_id, WatcherHandle { _debouncer: debouncer });
        Ok(())
    }

    pub fn stop_watching(&self, worktree_id: &str) -> Result<(), String> {
        let mut watchers = self
            .watchers
            .lock()
            .map_err(|e| format!("lock error: {}", e))?;
        watchers.remove(worktree_id);
        log::info!("fs_watcher: stopped watching worktree '{}'", worktree_id);
        Ok(())
    }

    pub fn stop_all(&self) {
        if let Ok(mut watchers) = self.watchers.lock() {
            watchers.clear();
            log::info!("fs_watcher: stopped all watchers");
        }
    }
}

/// `.git/` 配下は必要なファイルのみ通す（ノイズ削減）。
/// - `.git/index`: ステージング検知
/// - `.git/HEAD`: ブランチ切り替え検知
/// - `.git/*_HEAD`: MERGE_HEAD, REBASE_HEAD 等
/// - `.git/refs/**`: コミット・タグ・リモート追跡ブランチ検知
fn is_relevant_path(path: &Path) -> bool {
    let components: Vec<_> = path.components().collect();
    let git_idx = components.iter().position(|c| c.as_os_str() == ".git");
    match git_idx {
        None => true,
        Some(i) => {
            let remaining = components.len() - i - 1;
            if remaining == 0 {
                return false;
            }
            let first_under_git = components[i + 1].as_os_str();
            if remaining == 1 {
                // .git/index, .git/HEAD, .git/MERGE_HEAD 等
                return first_under_git == "index"
                    || first_under_git == "HEAD"
                    || first_under_git.to_string_lossy().ends_with("_HEAD");
            }
            // .git/refs/**
            first_under_git == "refs"
        }
    }
}
