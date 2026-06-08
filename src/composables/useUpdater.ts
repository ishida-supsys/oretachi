import { ref } from "vue";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { logError, logInfo } from "../utils/log";
import { invoke } from "@tauri-apps/api/core";

export function useUpdater() {
  const isChecking = ref(false);
  const isDownloading = ref(false);

  async function checkForUpdate() {
    if (isChecking.value) return null;
    isChecking.value = true;
    try {
      const update = await check();
      if (update) {
        logInfo(`アップデート利用可能: ${update.version}`);
        return update;
      }
    } catch (e) {
      logError(`アップデート確認エラー: ${e}`);
    } finally {
      isChecking.value = false;
    }
    return null;
  }

  async function downloadAndInstall(update: Awaited<ReturnType<typeof check>>) {
    if (!update) return;
    isDownloading.value = true;
    try {
      await update.downloadAndInstall();
      await invoke("prepare_for_relaunch");
      await relaunch();
    } catch (e) {
      logError(`アップデートインストールエラー: ${e}`);
    } finally {
      isDownloading.value = false;
    }
  }

  return {
    isChecking,
    isDownloading,
    checkForUpdate,
    downloadAndInstall,
  };
}
