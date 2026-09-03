import type { AppSettings, WorktreeEntry, Workgroup } from "../types/settings";

/**
 * フック由来通知をトレイ通知として出すかの実効値。
 * 解決順は「ワークツリー個別 > 所属ワークグループの既定値 > true」で、
 * バックエンドの `settings::resolve_tray_notification` と同じ規則。
 *
 * 値はコピーせず参照時に毎回解決するため、ワークグループ既定値を変えると
 * 個別未設定のワークツリーへ再起動なしで反映される。
 */
export function resolveTrayNotification(
  worktree: Pick<WorktreeEntry, "trayNotification" | "workgroupId">,
  workgroups: Workgroup[] | undefined,
): boolean {
  if (worktree.trayNotification !== undefined) return worktree.trayNotification;
  if (worktree.workgroupId) {
    const group = workgroups?.find((g) => g.id === worktree.workgroupId);
    if (group?.trayNotification !== undefined) return group.trayNotification;
  }
  return true;
}

/** settings 全体から Map<worktreeId, 実効値> を組み立てる。 */
export function buildTrayNotificationMap(settings: AppSettings): Map<string, boolean> {
  const map = new Map<string, boolean>();
  for (const wt of settings.worktrees) {
    map.set(wt.id, resolveTrayNotification(wt, settings.workgroups));
  }
  return map;
}
