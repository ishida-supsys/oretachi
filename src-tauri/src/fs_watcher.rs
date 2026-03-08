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
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Manager};

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

        let mut debouncer = new_debouncer(
            Duration::from_millis(500),
            move |res: DebounceEventResult| match res {
                Ok(events) => {
                    let relevant = events.iter().any(|e| is_relevant_path(&e.path));
                    if relevant {
                        if let Some(window) = app_handle
                            .get_webview_window(&format!("codereview-{}", worktree_id_clone))
                        {
                            use tauri::Emitter;
                            let _ = window.emit("codereview-fs-changed", ());
                        }
                    }
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

/// `.git/` 配下は `.git/index` のみ通す（ステージング検知用）。
/// それ以外の `.git/objects` 等は除外してノイズを減らす。
fn is_relevant_path(path: &Path) -> bool {
    let components: Vec<_> = path.components().collect();
    let git_idx = components.iter().position(|c| c.as_os_str() == ".git");
    match git_idx {
        None => true,
        Some(i) => {
            // .git/index のみ許可（.git 直下の index ファイル）
            i + 2 == components.len() && components[i + 1].as_os_str() == "index"
        }
    }
}
