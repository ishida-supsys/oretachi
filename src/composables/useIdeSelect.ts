import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import type { IdeInfo } from "../types/ide";
import { useCodeReviewWindow } from "./useCodeReviewWindow";
import type { CodeReviewOrigin } from "./useCodeReviewLineChat";

export const CODE_REVIEWER_IDE: IdeInfo = {
  id: "code-reviewer",
  name: "Code Reviewer",
  command: "",
};

export interface IdeSelectOptions {
  /** worktreeId を渡す（Code Reviewer ウィンドウ識別に使用） */
  worktreeId?: string;
  worktreeName?: string;
  /** Code Reviewerを開いたウィンドウの種別 */
  origin?: CodeReviewOrigin;
}

/**
 * IDE検出・選択・起動ロジック。
 * App.vue / SubWindowApp.vue で共通利用する。
 */
export function useIdeSelect() {
  const showIdeDialog = ref(false);
  const detectedIdes = ref<IdeInfo[]>([]);
  const ideTargetPath = ref("");
  const ideTargetOptions = ref<IdeSelectOptions>({});

  const { openCodeReview } = useCodeReviewWindow();

  async function openInIde(path: string, options: IdeSelectOptions = {}): Promise<void> {
    const ides = await invoke<IdeInfo[]>("detect_ides");

    if (ides.length === 1) {
      // IDE が1つだけ検出された場合は直接起動
      try {
        await invoke("open_in_ide", { command: ides[0].command, path });
      } catch (e) {
        await message(`IDE の起動に失敗しました: ${e}`, { kind: "error" });
      }
      return;
    }

    // IDE が0件、または2件以上の場合はダイアログ表示
    // Code Reviewer は末尾に配置（実IDEを優先）
    detectedIdes.value = [...ides, CODE_REVIEWER_IDE];
    ideTargetPath.value = path;
    ideTargetOptions.value = options;
    showIdeDialog.value = true;
  }

  async function onIdeSelected(ide: IdeInfo): Promise<void> {
    showIdeDialog.value = false;

    if (ide.id === "code-reviewer") {
      await openCodeReview(
        ideTargetOptions.value.worktreeId ?? ideTargetPath.value,
        ideTargetOptions.value.worktreeName ?? ideTargetPath.value,
        ideTargetPath.value,
        ideTargetOptions.value.origin,
      );
      return;
    }

    try {
      await invoke("open_in_ide", { command: ide.command, path: ideTargetPath.value });
    } catch (e) {
      await message(`IDE の起動に失敗しました: ${e}`, { kind: "error" });
    }
  }

  return { showIdeDialog, detectedIdes, ideTargetPath, openInIde, onIdeSelected };
}
