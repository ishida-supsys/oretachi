import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useSettings, syncRepositoryWorktrees, applyPluginConfig } from "./useSettings";
import { useWorktrees } from "./useWorktrees";

export type AddRepositoryResult = "added" | "cancelled" | "notARepo" | "alreadyRegistered";

/**
 * リポジトリ追加アクション。
 * ディレクトリ選択 → git リポジトリ検証 → 重複チェック → settings へ追加。
 * エラーメッセージの表示は呼び出し側 (i18n を持つコンポーネント) が結果コードで行う。
 */
export function useRepositoryActions() {
  const { settings, scheduleSave } = useSettings();
  const { syncWorktreesFromSettings } = useWorktrees();

  async function addRepository(): Promise<AddRepositoryResult> {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return "cancelled";

    // サブディレクトリを選ばれてもリポジトリのルートへ正規化する。
    // ルート以外を登録すると git worktree list が返すメインワークツリーと食い違い、
    // oretachi_import_worktree がメインワークツリーを「未登録」として拾ってしまう。
    let repoPath: string;
    try {
      const valid = await invoke<boolean>("git_validate_repo", { path: selected });
      if (!valid) return "notARepo";
      repoPath = await invoke<string>("git_repo_root", { path: selected });
    } catch {
      return "notARepo";
    }

    if (settings.value.repositories.some((r) => r.path === repoPath)) {
      return "alreadyRegistered";
    }

    const name = repoPath.split(/[/\\]/).pop() ?? repoPath;
    settings.value.repositories.push({
      id: repoPath,
      name,
      path: repoPath,
    });
    // リポジトリ擬似ワークツリーを生成してランタイム配列にも反映する。
    // 自ウィンドウ発の settings-changed は無視されるためここで明示的に同期しないと、
    // 追加直後のカード/タブにターミナル追加ボタンが出ない。
    syncRepositoryWorktrees();
    syncWorktreesFromSettings();
    scheduleSave();
    // リポジトリ root で開いたターミナルでも oretachi プラグイン (通知フック + MCP) を効かせる。
    // 失敗してもリポジトリ追加自体は成立させる（カードのメニューから再適用できる）。
    void applyPluginConfig(repoPath, name).catch((e) => {
      console.error("リポジトリのプラグイン設定書き込みに失敗:", e);
    });
    return "added";
  }

  return { addRepository };
}
