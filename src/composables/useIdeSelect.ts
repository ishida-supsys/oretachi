import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import type { IdeInfo } from "../types/ide";

/**
 * IDE検出・選択・起動ロジック。
 * App.vue / SubWindowApp.vue で共通利用する。
 */
export function useIdeSelect() {
  const showIdeDialog = ref(false);
  const detectedIdes = ref<IdeInfo[]>([]);
  const ideTargetPath = ref("");

  async function openInIde(path: string): Promise<void> {
    const ides = await invoke<IdeInfo[]>("detect_ides");

    if (ides.length === 0) {
      await message(
        "Cursor、VS Code、Antigravity のいずれもインストールされていません。",
        { title: "IDE が見つかりません", kind: "warning" }
      );
      return;
    }

    if (ides.length === 1) {
      await invoke("open_in_ide", { command: ides[0].command, path });
      return;
    }

    detectedIdes.value = ides;
    ideTargetPath.value = path;
    showIdeDialog.value = true;
  }

  async function onIdeSelected(ide: IdeInfo): Promise<void> {
    showIdeDialog.value = false;
    try {
      await invoke("open_in_ide", { command: ide.command, path: ideTargetPath.value });
    } catch (e) {
      await message(`IDE の起動に失敗しました: ${e}`, { kind: "error" });
    }
  }

  return { showIdeDialog, detectedIdes, ideTargetPath, openInIde, onIdeSelected };
}
