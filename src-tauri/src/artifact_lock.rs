//! アーティファクトの「表示中ロック」の実行時レジストリ。
//!
//! ロックは **永続フラグ（本体 JSON の `locked_while_open`）× 実行時の開閉状態** の AND で
//! 決まる。ここが後者を持つ。ファイルには一切書かないので、アプリを起動し直せば
//! ロックは必ず空から始まる（= 事故った状態が永続しない）。
//!
//! ## なぜ三重に手当てするのか
//!
//! webview がハングして JS が止まる既知事象（`oretachi-hang-triage`）があるため、
//! 「ビューアが close を送る」だけに頼るとロックが永久に解けなくなる。
//!
//! 1. **アプリ起動時クリア**: レジストリが in-memory なので自動的に達成される
//! 2. **ウィンドウ破棄時の解除**: `WindowEvent::Destroyed` で `release_window`
//!    （JS が動かなくなっていてもネイティブ側から確実に消える）
//! 3. **ハートビート TTL**: ビューアは開いている間 `touch` を打ち続け、
//!    `LOCK_TTL` を過ぎたエントリは失効扱いにする（ウィンドウは生きているが
//!    JS だけ止まった変種の救済）
//!
//! ## 粒度
//!
//! エントリはウィンドウラベル単位で 1 件。1 つのビューアウィンドウが同時に表示できる
//! アーティファクトは 1 件だけなので、同じラベルからの `touch` は上書きでよい。
//!
//! **つまりロックが守るのは「そのウィンドウでいま表示している 1 件」だけで、ウィンドウが
//! 開いたままでもユーザーが別のアーティファクトへ切り替えれば前のロックは外れる。**
//! ロックの意味を「いま見ているもの」に揃える意図的な選択（#203 でユーザーと合意済み）。
//! ウィンドウ単位の集合にして「一度開いたものを閉じるまで全部ロック」にもできるが、
//! バッジは表示中のアーティファクトにしか出せないため、ユーザーから見えないロックが
//! 積み上がって「エラーになるのに何が掴んでいるのか分からない」状態を作りやすい。
//!
//! 残る穴は `artifact_store` の write で、ユーザーが別ページを見ている間はフォーム入力を
//! 上書きできる（`artifact` の update / rewrite はコードだけでストアは残るので実害が小さい）。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// ハートビートが途切れたエントリを失効させるまでの猶予。
///
/// Chromium は非表示・最小化されたページのタイマーを間引くため、ビューア側の送信間隔
/// （`crate::ARTIFACT_LOCK_HEARTBEAT_MS` = 30 秒）は最大で 1 分程度まで引き伸ばされうる。
/// TTL をそこに近づけると「最小化しただけでロックが外れる」ので、3 倍の余裕を取る。
/// 逆に長すぎると webview ハング時の待ち時間になるが、ウィンドウを閉じれば
/// `WindowEvent::Destroyed` で即座に解除されるので実害は小さい。
pub const LOCK_TTL: Duration = Duration::from_secs(180);

/// ビューアが 1 枚のウィンドウで表示しているアーティファクト。
#[derive(Debug, Clone)]
struct OpenEntry {
    scope: String,
    scope_id: String,
    artifact_id: String,
    last_seen: Instant,
}

/// 「いまビューアで開かれているアーティファクト」の集合。キーはウィンドウラベル。
#[derive(Default)]
pub struct ArtifactOpenRegistry(Mutex<HashMap<String, OpenEntry>>);

impl ArtifactOpenRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定ウィンドウが表示中のアーティファクトを登録・更新する（ハートビート兼用）。
    pub fn touch(&self, window_label: &str, scope: &str, scope_id: &str, artifact_id: &str) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(
                window_label.to_string(),
                OpenEntry {
                    scope: scope.to_string(),
                    scope_id: scope_id.to_string(),
                    artifact_id: artifact_id.to_string(),
                    last_seen: Instant::now(),
                },
            );
        }
    }

    /// ウィンドウが表示をやめた（閉じた・別のアーティファクトを選んだ）ことを登録する。
    pub fn release_window(&self, window_label: &str) {
        if let Ok(mut map) = self.0.lock() {
            map.remove(window_label);
        }
    }

    /// 指定アーティファクトがロックを主張できる状態で開かれているか。
    /// 判定のついでに失効エントリを掃除する（ハングしたビューアのゴミを溜めない）。
    pub fn is_open(&self, scope: &str, scope_id: &str, artifact_id: &str) -> bool {
        let Ok(mut map) = self.0.lock() else {
            // ロックが毒されている場合は「開いていない」に倒す。
            // ここで true を返すと MCP 側の書き込みが恒久的に塞がれてしまう
            return false;
        };
        let now = Instant::now();
        map.retain(|_, e| now.duration_since(e.last_seen) < LOCK_TTL);
        map.values()
            .any(|e| e.scope == scope && e.scope_id == scope_id && e.artifact_id == artifact_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_and_release() {
        let reg = ArtifactOpenRegistry::new();
        assert!(!reg.is_open("worktree", "wt-1", "a"));
        reg.touch("artifact-wt-1", "worktree", "wt-1", "a");
        assert!(reg.is_open("worktree", "wt-1", "a"));
        // スコープ / ID が違えばロックしない
        assert!(!reg.is_open("repository", "wt-1", "a"));
        assert!(!reg.is_open("worktree", "wt-2", "a"));
        assert!(!reg.is_open("worktree", "wt-1", "b"));
        reg.release_window("artifact-wt-1");
        assert!(!reg.is_open("worktree", "wt-1", "a"));
    }

    /// 同じウィンドウが別のアーティファクトへ切り替えたら、前のロックは残らない
    #[test]
    fn touch_replaces_previous_artifact_of_same_window() {
        let reg = ArtifactOpenRegistry::new();
        reg.touch("artifact-wt-1", "worktree", "wt-1", "a");
        reg.touch("artifact-wt-1", "worktree", "wt-1", "b");
        assert!(!reg.is_open("worktree", "wt-1", "a"));
        assert!(reg.is_open("worktree", "wt-1", "b"));
    }

    /// 別ウィンドウが同じアーティファクトを開いていれば、片方を閉じてもロックは残る
    #[test]
    fn other_window_keeps_lock() {
        let reg = ArtifactOpenRegistry::new();
        reg.touch("artifact-wt-1", "worktree", "wt-1", "a");
        reg.touch("sub-1", "worktree", "wt-1", "a");
        reg.release_window("artifact-wt-1");
        assert!(reg.is_open("worktree", "wt-1", "a"));
    }

    /// ハートビートが途切れたエントリは TTL 経過で失効する（JS だけ止まった webview の救済）
    #[test]
    fn stale_entry_expires() {
        let reg = ArtifactOpenRegistry::new();
        {
            let mut map = reg.0.lock().expect("lock");
            map.insert(
                "artifact-wt-1".to_string(),
                OpenEntry {
                    scope: "worktree".to_string(),
                    scope_id: "wt-1".to_string(),
                    artifact_id: "a".to_string(),
                    last_seen: Instant::now() - LOCK_TTL - Duration::from_secs(1),
                },
            );
        }
        assert!(!reg.is_open("worktree", "wt-1", "a"));
        // 失効エントリは掃除されている
        assert!(reg.0.lock().expect("lock").is_empty());
    }
}
