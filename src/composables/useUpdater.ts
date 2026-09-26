import { ref } from "vue";
import { check } from "@tauri-apps/plugin-updater";
import { logError, logInfo } from "../utils/log";
import { invoke } from "@tauri-apps/api/core";

// ダウンロード完了後・インストール直前に一度だけ呼ばれるフック。Windows では
// インストール実行時に process::exit(0) されアプリの通常終了経路
// （onCloseRequested）を通らないため、セッション保存等をここに登録して確実に
// 実行させる。ダウンロードより前に呼ぶとダウンロード中に進んだセッション変更が
// 保存されないため、Rust 側の処理を download/install の2コマンドに分けている。
let beforeInstallHook: (() => Promise<void>) | null = null;

/** アップデートインストール直前に実行する処理を登録する（呼び出し側は1つのみ想定）。 */
export function setBeforeInstallHook(hook: (() => Promise<void>) | null) {
  beforeInstallHook = hook;
}

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
      // チェック・ダウンロードを Rust 側コマンドで実施する（download_update）。
      // ここで得た Update とダウンロード済みバイト列は Rust 側で保持し、
      // install_downloaded_update に引き継ぐ。
      const hasUpdate = await invoke<boolean>("download_update");
      if (!hasUpdate) return; // ダウンロード時点で更新が無くなっていた場合

      // インストール直前フック（セッション保存等）。失敗しても更新は続行する。
      if (beforeInstallHook) {
        try {
          await beforeInstallHook();
        } catch (e) {
          logError(`アップデート前処理エラー: ${e}`);
        }
      }

      // インストールを実施する。Windows ではインストーラ起動直前に Job の
      // KILL_ON_JOB_CLOSE を解除する必要があり、プラグインの downloadAndInstall
      // では process::exit され JS に戻らないためフックを差し込めない
      // （巻き込み終了でアップデート不発）。そのため独自コマンドを使う。
      await invoke("install_downloaded_update");
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
    setBeforeInstallHook,
  };
}
