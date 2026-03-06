import { useSettings } from "./useSettings";

const HOTKEY_POOL = [
  '1','2','3','4','5','6','7','8','9',
  'a','b','c','d','e','f','g','h','i','j','k','l','m',
  'n','o','p','q','r','s','t','u','v','w','x','y','z',
];

export function useAutoHotkey() {
  const { settings, scheduleSave } = useSettings();

  function tryAutoAssignHotkey(worktreeId: string) {
    if (!settings.value.autoAssignHotkey) return;

    const used = new Set(
      settings.value.worktrees
        .filter((w) => w.hotkeyChar)
        .map((w) => w.hotkeyChar as string)
    );

    const freeChar = HOTKEY_POOL.find((c) => !used.has(c));
    if (!freeChar) return;

    const worktree = settings.value.worktrees.find((w) => w.id === worktreeId);
    if (!worktree) return;

    worktree.hotkeyChar = freeChar;
    scheduleSave();
  }

  return { tryAutoAssignHotkey };
}
