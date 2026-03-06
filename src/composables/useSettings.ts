import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { platform } from "@tauri-apps/plugin-os";
import type { AppSettings, HotkeyBinding, HotkeySettings } from "../types/settings";

const isMac = platform() === "macos";

const defaultHotkeys = () => ({
  globalTrayPopup: isMac ? { meta: true, shift: true, key: "o" } : { ctrl: true, shift: true, key: "o" },
  terminalNext: { ctrl: true, key: "Tab" },
  terminalPrev: { ctrl: true, shift: true, key: "Tab" },
  terminalAdd: isMac ? { meta: true, key: "t" } : { ctrl: true, key: "t" },
  terminalClose: isMac ? { meta: true, key: "w" } : { ctrl: true, key: "w" },
  trayNext: isMac ? { meta: true, key: "n" } : { ctrl: true, key: "n" },
});

function migrateHotkeys(hotkeys: HotkeySettings): boolean {
  let changed = false;
  const isOldDefault = (b: HotkeyBinding, key: string, shift = false) =>
    !!b.ctrl && !b.alt && !b.meta && !!b.shift === shift && b.key === key;

  // 全プラットフォーム: Ctrl+Q → Ctrl+W / ⌘+W
  if (isOldDefault(hotkeys.terminalClose, "q")) {
    hotkeys.terminalClose = isMac ? { meta: true, key: "w" } : { ctrl: true, key: "w" };
    changed = true;
  }
  // macOS: Ctrl → ⌘ 変換 (Tab 切替は除外)
  if (isMac) {
    if (isOldDefault(hotkeys.terminalAdd, "t")) {
      hotkeys.terminalAdd = { meta: true, key: "t" };
      changed = true;
    }
    if (isOldDefault(hotkeys.trayNext, "n")) {
      hotkeys.trayNext = { meta: true, key: "n" };
      changed = true;
    }
    if (isOldDefault(hotkeys.globalTrayPopup, "o", true)) {
      hotkeys.globalTrayPopup = { meta: true, shift: true, key: "o" };
      changed = true;
    }
  }
  return changed;
}

const settings = ref<AppSettings>({
  repositories: [],
  worktreeBaseDir: "",
  worktrees: [],
  terminal: { fontSize: 14 },
  hotkeys: defaultHotkeys(),
  alwaysOnTop: false,
});

let saveTimer: ReturnType<typeof setTimeout> | null = null;

async function loadSettings() {
  const loaded = await invoke<AppSettings>("get_settings");
  // 古い設定ファイルに hotkeys がない場合のデフォルト補完
  if (!loaded.hotkeys) {
    loaded.hotkeys = defaultHotkeys();
  } else {
    const def = defaultHotkeys();
    loaded.hotkeys = { ...def, ...loaded.hotkeys };
    // 旧フォーマット移行: globalTrayPopup が文字列だった場合はデフォルトに置換
    if (typeof loaded.hotkeys.globalTrayPopup === "string") {
      loaded.hotkeys.globalTrayPopup = def.globalTrayPopup;
    }
  }
  if (loaded.alwaysOnTop === undefined) {
    loaded.alwaysOnTop = false;
  }
  // ホットキーマイグレーション (冪等)
  if (migrateHotkeys(loaded.hotkeys)) {
    try {
      await invoke("save_settings", { settings: loaded });
    } catch (e) {
      console.error("マイグレーション保存に失敗:", e);
    }
  }
  settings.value = loaded;
}

function scheduleSave() {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await invoke("save_settings", { settings: settings.value });
      await emit("settings-changed");
    } catch (e) {
      console.error("設定の保存に失敗:", e);
    }
  }, 500);
}

export function useSettings() {
  return { settings, loadSettings, scheduleSave };
}
