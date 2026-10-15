//! アプリ形式アーティファクトの承認（grant）ストア。
//!
//! **host 側のファイル（`app_data/app-grants.json`）にだけ持つ。** アーティファクト JSON には置かない
//! （`artifact` / `artifact_module` で AI が書き換えられるため）。キーは `<worktreeId>/<artifactId>`。
//! 破損・欠落は「承認なし」として扱う（再承認を求める側＝fail closed）。

use crate::app_manifest::AppGrant;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FILE_NAME: &str = "app-grants.json";

pub fn grant_key(worktree_id: &str, artifact_id: &str) -> String {
    format!("{}/{}", worktree_id, artifact_id)
}

fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('/')
}

#[derive(Default)]
pub struct AppGrantStore {
    path: Mutex<Option<PathBuf>>,
    grants: Mutex<HashMap<String, AppGrant>>,
}

impl AppGrantStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// `data_dir` は `app_data_dir()`。読めない・壊れているときは空で始める（再承認になるだけ）。
    pub fn init(&self, data_dir: &Path) {
        let path = data_dir.join(FILE_NAME);
        let loaded: HashMap<String, AppGrant> = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                log::warn!("[app-grant] {} を解析できません（承認なしとして扱います）: {}", FILE_NAME, e);
                HashMap::new()
            }),
            Err(_) => HashMap::new(),
        };
        if let Ok(mut p) = self.path.lock() {
            *p = Some(path);
        }
        if let Ok(mut g) = self.grants.lock() {
            *g = loaded;
        }
    }

    pub fn get(&self, worktree_id: &str, artifact_id: &str) -> Option<AppGrant> {
        self.grants
            .lock()
            .ok()
            .and_then(|g| g.get(&grant_key(worktree_id, artifact_id)).cloned())
    }

    /// `(worktreeId, artifactId, grant)` の一覧。
    pub fn list(&self) -> Vec<(String, String, AppGrant)> {
        let Ok(g) = self.grants.lock() else { return Vec::new() };
        let mut out: Vec<(String, String, AppGrant)> = g
            .iter()
            .filter_map(|(k, v)| split_key(k).map(|(w, a)| (w.to_string(), a.to_string(), v.clone())))
            .collect();
        out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        out
    }

    pub fn put(&self, worktree_id: &str, artifact_id: &str, grant: AppGrant) -> Result<(), String> {
        let mut g = self.grants.lock().map_err(|_| "grant ストアのロックが壊れています".to_string())?;
        g.insert(grant_key(worktree_id, artifact_id), grant);
        self.persist_locked(&g)
    }

    /// 承認を取り消す。無ければ `Ok(false)`。
    pub fn remove(&self, worktree_id: &str, artifact_id: &str) -> Result<bool, String> {
        let mut g = self.grants.lock().map_err(|_| "grant ストアのロックが壊れています".to_string())?;
        let existed = g.remove(&grant_key(worktree_id, artifact_id)).is_some();
        if existed {
            self.persist_locked(&g)?;
        }
        Ok(existed)
    }

    /// ワークツリー丸ごとの削除（`artifacts/<worktreeId>/` ごと消える経路）で呼ぶ。消した件数を返す。
    pub fn remove_worktree(&self, worktree_id: &str) -> Result<usize, String> {
        let mut g = self.grants.lock().map_err(|_| "grant ストアのロックが壊れています".to_string())?;
        let prefix = format!("{}/", worktree_id);
        let before = g.len();
        g.retain(|k, _| !k.starts_with(&prefix));
        let removed = before - g.len();
        if removed > 0 {
            self.persist_locked(&g)?;
        }
        Ok(removed)
    }

    /// 一時ファイルへ書いてから rename する（途中で落ちても壊れたファイルを残さない）。
    fn persist_locked(&self, grants: &HashMap<String, AppGrant>) -> Result<(), String> {
        let path = self
            .path
            .lock()
            .map_err(|_| "grant ストアのパスのロックが壊れています".to_string())?
            .clone()
            .ok_or_else(|| "grant ストアが初期化されていません".to_string())?;
        let json = serde_json::to_string_pretty(grants).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("grant の書き込みに失敗: {}", e))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("grant の置き換えに失敗: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_manifest::{MemoryPermission, RequestedPermissions, ResolvedPermissions};

    fn grant(max: u64) -> AppGrant {
        AppGrant {
            granted_at: 1,
            requested: RequestedPermissions { mcp: None, memory: Some(MemoryPermission { max_bytes: max }), ai_terminal: None, fs: None, db: None, network: None, sub_window: None },
            resolved: ResolvedPermissions { mcp: None, memory: Some(MemoryPermission { max_bytes: max }), ai_terminal: None, fs: None, db: None, network: None, sub_window: None },
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oretachi-grant-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn persists_and_reloads() {
        let dir = temp_dir("reload");
        let s = AppGrantStore::new();
        s.init(&dir);
        s.put("wt1", "a1", grant(10)).unwrap();
        let s2 = AppGrantStore::new();
        s2.init(&dir);
        assert_eq!(s2.get("wt1", "a1"), Some(grant(10)));
        assert_eq!(s2.get("wt1", "other"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_and_remove_worktree() {
        let dir = temp_dir("remove");
        let s = AppGrantStore::new();
        s.init(&dir);
        s.put("wt1", "a1", grant(1)).unwrap();
        s.put("wt1", "a2", grant(2)).unwrap();
        s.put("wt2", "a1", grant(3)).unwrap();
        assert!(s.remove("wt1", "a1").unwrap());
        assert!(!s.remove("wt1", "a1").unwrap());
        assert_eq!(s.remove_worktree("wt1").unwrap(), 1);
        assert_eq!(s.get("wt2", "a1"), Some(grant(3)));
        assert_eq!(s.list().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worktree_prefix_does_not_match_longer_id() {
        let dir = temp_dir("prefix");
        let s = AppGrantStore::new();
        s.init(&dir);
        s.put("wt1", "a", grant(1)).unwrap();
        s.put("wt10", "a", grant(2)).unwrap();
        s.remove_worktree("wt1").unwrap();
        assert_eq!(s.get("wt10", "a"), Some(grant(2)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_means_no_grants() {
        let dir = temp_dir("corrupt");
        std::fs::write(dir.join(FILE_NAME), "{ not json").unwrap();
        let s = AppGrantStore::new();
        s.init(&dir);
        assert!(s.list().is_empty());
        // 壊れたあとでも書ける
        s.put("w", "a", grant(1)).unwrap();
        assert_eq!(s.get("w", "a"), Some(grant(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
