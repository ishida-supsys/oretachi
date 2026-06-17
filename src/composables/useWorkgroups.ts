import { computed } from "vue";
import { useSettings } from "./useSettings";
import { useWorktrees } from "./useWorktrees";
import { i18n } from "../i18n";
import type { Workgroup } from "../types/settings";

const { settings, scheduleSave } = useSettings();
const { worktrees } = useWorktrees();

function genId(): string {
  return `wg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/** ワークグループ一覧（settings.workgroups の参照） */
const groups = computed<Workgroup[]>(() => settings.value.workgroups ?? []);

/** 選択中のワークグループID。未設定/存在しない場合は先頭グループにフォールバック */
const activeWorkgroupId = computed<string>({
  get() {
    const list = groups.value;
    const cur = settings.value.activeWorkgroupId;
    if (cur && list.some((g) => g.id === cur)) return cur;
    return list[0]?.id ?? "";
  },
  set(id: string) {
    settings.value.activeWorkgroupId = id;
    scheduleSave();
  },
});

/** worktree が実際に属するグループID（未設定/不明なら先頭グループにフォールバック） */
function resolvedGroupId(workgroupId: string | undefined): string {
  const list = groups.value;
  if (workgroupId && list.some((g) => g.id === workgroupId)) return workgroupId;
  return list[0]?.id ?? "";
}

/** worktree が属するグループを返す（フォールバック込み） */
function groupOf(worktree: { workgroupId?: string }): Workgroup | undefined {
  const id = resolvedGroupId(worktree.workgroupId);
  return groups.value.find((g) => g.id === id);
}

/** グループの表示名（未設定なら「グループ(番号)」を自動生成） */
function displayName(group: Workgroup): string {
  if (group.name && group.name.trim()) return group.name;
  const idx = groups.value.findIndex((g) => g.id === group.id);
  return i18n.global.t("workgroup.autoName", { n: idx + 1 });
}

/** グループに属するワークツリー数（フォールバック込み） */
function worktreeCount(groupId: string): number {
  return settings.value.worktrees.filter(
    (w) => resolvedGroupId(w.workgroupId) === groupId,
  ).length;
}

/** 新しいグループを追加して選択状態にする */
function addWorkgroup(): Workgroup {
  if (!settings.value.workgroups) settings.value.workgroups = [];
  const group: Workgroup = {
    id: genId(),
    autoAssignHotkey: settings.value.autoAssignHotkey ?? false,
    claudeCodeMode: "plan",
  };
  settings.value.workgroups.push(group);
  settings.value.activeWorkgroupId = group.id;
  scheduleSave();
  return group;
}

/** グループの属性を更新する */
function updateWorkgroup(id: string, patch: Partial<Workgroup>): void {
  const group = settings.value.workgroups?.find((g) => g.id === id);
  if (!group) return;
  Object.assign(group, patch);
  scheduleSave();
}

/** グループレコードを削除する（所属ワークツリーの削除は呼び出し側で済ませておくこと） */
function deleteWorkgroupRecord(id: string): void {
  if (!settings.value.workgroups) return;
  settings.value.workgroups = settings.value.workgroups.filter((g) => g.id !== id);
  if (settings.value.activeWorkgroupId === id) {
    settings.value.activeWorkgroupId = settings.value.workgroups[0]?.id;
  }
  scheduleSave();
}

/** グループの並び替え（fromId を toId の位置へ） */
function reorderWorkgroup(fromId: string, toId: string): void {
  if (fromId === toId || !settings.value.workgroups) return;
  const list = settings.value.workgroups;
  const fromIdx = list.findIndex((g) => g.id === fromId);
  const toIdx = list.findIndex((g) => g.id === toId);
  if (fromIdx === -1 || toIdx === -1) return;
  const [item] = list.splice(fromIdx, 1);
  list.splice(toIdx, 0, item);
  scheduleSave();
}

/** ワークツリーを別グループへ移動する */
function moveWorktreeToWorkgroup(worktreeId: string, groupId: string): void {
  const entry = settings.value.worktrees.find((w) => w.id === worktreeId);
  if (entry) entry.workgroupId = groupId;
  const wt = worktrees.value.find((w) => w.id === worktreeId);
  if (wt) wt.workgroupId = groupId;
  scheduleSave();
}

export function useWorkgroups() {
  return {
    groups,
    activeWorkgroupId,
    resolvedGroupId,
    groupOf,
    displayName,
    worktreeCount,
    addWorkgroup,
    updateWorkgroup,
    deleteWorkgroupRecord,
    reorderWorkgroup,
    moveWorktreeToWorkgroup,
  };
}
