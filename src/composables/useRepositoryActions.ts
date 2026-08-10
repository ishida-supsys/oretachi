import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useSettings, syncRepositoryWorktrees } from "./useSettings";
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

    try {
      const valid = await invoke<boolean>("git_validate_repo", { path: selected });
      if (!valid) return "notARepo";
    } catch {
      return "notARepo";
    }

    if (settings.value.repositories.some((r) => r.path === selected)) {
      return "alreadyRegistered";
    }

    const name = selected.split(/[/\\]/).pop() ?? selected;
    settings.value.repositories.push({
      id: selected,
      name,
      path: selected,
    });
    // リポジトリ擬似ワークツリーを生成してランタイム配列にも反映する。
    // 自ウィンドウ発の settings-changed は無視されるためここで明示的に同期しないと、
    // 追加直後のカード/タブにターミナル追加ボタンが出ない。
    syncRepositoryWorktrees();
    syncWorktreesFromSettings();
    scheduleSave();
    return "added";
  }

  return { addRepository };
}
