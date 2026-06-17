import { useSettings } from "./useSettings";
import { useWorkgroups } from "./useWorkgroups";

const HOTKEY_POOL = [
  '1','2','3','4','5','6','7','8','9',
  'a','b','c','d','e','f','g','h','i','j','k','l','m',
  'n','o','p','q','r','s','t','u','v','w','x','y','z',
];

export function useAutoHotkey() {
  const { settings, scheduleSave } = useSettings();
  const { groupOf } = useWorkgroups();

  function tryAutoAssignHotkey(worktreeId: string) {
    const worktree = settings.value.worktrees.find((w) => w.id === worktreeId);
    if (!worktree) return;

    // 自動割り当ての有効/無効は所属グループ単位（未設定時はグローバル設定にフォールバック）
    const group = groupOf(worktree);
    const enabled = group?.autoAssignHotkey ?? settings.value.autoAssignHotkey ?? false;
    if (!enabled) return;

    // ホットキーのユニーク性は全ワークツリー横断で担保する（仕様: 変化しない）
    const used = new Set(
      settings.value.worktrees
        .filter((w) => w.hotkeyChar)
        .map((w) => w.hotkeyChar as string)
    );

    const freeChar = HOTKEY_POOL.find((c) => !used.has(c));
    if (!freeChar) return;

    worktree.hotkeyChar = freeChar;
    scheduleSave();
  }

  return { tryAutoAssignHotkey };
}
