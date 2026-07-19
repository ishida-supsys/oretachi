import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useSettings } from "./useSettings";

export type AddRepositoryResult = "added" | "cancelled" | "notARepo" | "alreadyRegistered";

/**
 * リポジトリ追加アクション。
 * ディレクトリ選択 → git リポジトリ検証 → 重複チェック → settings へ追加。
 * エラーメッセージの表示は呼び出し側 (i18n を持つコンポーネント) が結果コードで行う。
 */
export function useRepositoryActions() {
  const { settings, scheduleSave } = useSettings();

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
    scheduleSave();
    return "added";
  }

  return { addRepository };
}
