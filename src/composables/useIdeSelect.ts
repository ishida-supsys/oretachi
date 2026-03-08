import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import type { IdeInfo } from "../types/ide";
import { useCodeReviewWindow } from "./useCodeReviewWindow";

export const CODE_REVIEWER_IDE: IdeInfo = {
  id: "code-reviewer",
  name: "Code Reviewer",
  command: "",
};

export interface IdeSelectOptions {
  /** worktreeId を渡す（Code Reviewer ウィンドウ識別に使用） */
  worktreeId?: string;
  worktreeName?: string;
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
    const allIdes = [CODE_REVIEWER_IDE, ...ides];

    if (allIdes.length === 1) {
      // Code Reviewer のみ（他IDEなし）
      await openCodeReview(
        options.worktreeId ?? path,
        options.worktreeName ?? path,
        path,
      );
      return;
    }

    detectedIdes.value = allIdes;
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
