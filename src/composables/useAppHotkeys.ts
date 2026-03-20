import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { useHotkeyListener, useAltCharKeyListener, bindingToAccelerator } from "./useHotkeys";
import type { Ref } from "vue";
import type { Worktree } from "../types/worktree";
import type { AppSettings } from "../types/settings";
import type { WorktreeFrameBundle } from "./useWorktreeFrameBundles";

interface UseAppHotkeysDeps {
  settings: Ref<AppSettings>;
  loadSettings: () => Promise<void>;
  activeWorktreeId: Ref<string | null>;
  worktreeFrameBundles: Map<string, WorktreeFrameBundle>;
  viewMode: Ref<string>;
  worktrees: Ref<Worktree[]>;
  isDetached: (id: string) => boolean;
  switchToWorktree: (id: string) => void;
  focusSubWindow: (id: string) => Promise<void>;
  onAddTerminal: (id: string) => Promise<void>;
  showAddTaskDialog: Ref<boolean>;
  goHome: () => void;
  onTrayButtonClick: () => Promise<void>;
}

export function useAppHotkeys(deps: UseAppHotkeysDeps) {
  let globalShortcutRegistered = false;
  let registeredAccelerator: string | null = null;

  async function registerGlobalShortcut() {
    const binding = deps.settings.value.hotkeys?.globalTrayPopup;
    if (!binding) return;
    const accelerator = bindingToAccelerator(binding);
    try {
      if (globalShortcutRegistered && registeredAccelerator) {
        await unregister(registeredAccelerator);
        globalShortcutRegistered = false;
        registeredAccelerator = null;
      }
      await register(accelerator, () => {
        deps.onTrayButtonClick();
      });
      globalShortcutRegistered = true;
      registeredAccelerator = accelerator;
    } catch (e) {
      console.error("[GlobalShortcut] 登録失敗:", e);
    }
  }

  function focusWorktreeByChar(char: string) {
    const wt = deps.worktrees.value.find((w) => {
      const entry = deps.settings.value.worktrees.find((e) => e.id === w.id);
      return entry?.hotkeyChar === char;
    });
    if (!wt) return;

    if (deps.isDetached(wt.id)) {
      deps.focusSubWindow(wt.id);
    } else if (wt.terminals.length > 0) {
      deps.switchToWorktree(wt.id);
    } else {
      deps.onAddTerminal(wt.id);
    }
  }

  // Alt+[char] ワークツリーフォーカス（setup時登録）
  useAltCharKeyListener((char, event) => {
    const homeTabBinding = deps.settings.value.hotkeys?.homeTab;
    if (homeTabBinding) {
      const { alt, key } = homeTabBinding;
      if (alt && key.length === 1 && key.toLowerCase() === char) return;
    }
    const wt = deps.worktrees.value.find((w) => {
      const entry = deps.settings.value.worktrees.find((e) => e.id === w.id);
      return entry?.hotkeyChar === char;
    });
    if (!wt) return;
    event.preventDefault();
    event.stopPropagation();
    focusWorktreeByChar(char);
  });

  // ホットキーリスナー登録（setup時）
  useHotkeyListener(() => {
    const hk = deps.settings.value.hotkeys;
    if (!hk) return [];

    const actions = [];
    const activeBundle = () =>
      deps.activeWorktreeId.value
        ? deps.worktreeFrameBundles.get(deps.activeWorktreeId.value)
        : undefined;

    actions.push({ binding: hk.terminalNext, handler: () => activeBundle()?.frame.switchNextTerminal() });
    actions.push({ binding: hk.terminalPrev, handler: () => activeBundle()?.frame.switchPrevTerminal() });

    actions.push({
      binding: hk.terminalAdd,
      handler: () => {
        const worktreeId =
          deps.activeWorktreeId.value ??
          deps.worktrees.value.find((w) => !deps.isDetached(w.id))?.id;
        if (worktreeId) deps.onAddTerminal(worktreeId);
      },
    });

    actions.push({
      binding: hk.terminalClose,
      handler: () => {
        if (deps.viewMode.value !== "terminal") return;
        activeBundle()?.frame.closeActiveTerminal();
      },
    });

    if (hk.addTask) {
      actions.push({
        binding: hk.addTask,
        handler: () => {
          deps.showAddTaskDialog.value = true;
        },
      });
    }

    if (hk.homeTab) {
      actions.push({
        binding: hk.homeTab,
        handler: () => {
          deps.goHome();
        },
      });
    }

    return actions;
  });

  async function init() {
    await registerGlobalShortcut();

    await listen("settings-changed", async () => {
      await deps.loadSettings();
      await getCurrentWindow().setAlwaysOnTop(deps.settings.value.alwaysOnTop);
      await registerGlobalShortcut();
    });

    await listen<{ char: string }>("sub-alt-char-focus", (event) => {
      focusWorktreeByChar(event.payload.char);
    });
  }

  return { registerGlobalShortcut, init };
}
