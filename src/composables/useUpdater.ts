import { ref } from "vue";
import { check } from "@tauri-apps/plugin-updater";
import { logError, logInfo } from "../utils/log";
import { invoke } from "@tauri-apps/api/core";

export function useUpdater() {
  const isChecking = ref(false);
  const isDownloading = ref(false);

  // 更新なしは null を返す。確認自体に失敗した場合は例外を投げ、呼び出し側で表示する。
  async function checkForUpdate() {
    if (isChecking.value) return null;
    isChecking.value = true;
    try {
      const update = await check();
      if (update) {
        logInfo(`アップデート利用可能: ${update.version}`);
        return update;
      }
      return null;
    } catch (e) {
      logError(`アップデート確認エラー: ${e}`);
      throw e;
    } finally {
      isChecking.value = false;
    }
  }

  // 失敗時は例外を投げ、呼び出し側で表示する（従来は無反応だった）。
  async function downloadAndInstall(update: Awaited<ReturnType<typeof check>>) {
    if (!update) return;
    isDownloading.value = true;
    try {
      // チェック・ダウンロード・インストールを Rust 側コマンドで実施する。
      // Windows ではインストーラ起動直前に Job の KILL_ON_JOB_CLOSE を解除する
      // 必要があり、プラグインの downloadAndInstall では process::exit され
      // JS に戻らないためフックを差し込めない（巻き込み終了でアップデート不発）。
      await invoke("download_and_install_update");
      // Windows ではプロセスが置き換わるためここに戻らない。
    } catch (e) {
      logError(`アップデートインストールエラー: ${e}`);
      throw e;
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
