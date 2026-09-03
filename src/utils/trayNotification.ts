import type { AppSettings, WorktreeEntry, Workgroup } from "../types/settings";

/**
 * ワークツリーの所属ワークグループを引く関数。`useWorkgroups.groupOf` を渡す想定。
 *
 * 「workgroupId が未設定 / 不明なら先頭グループ」というフォールバック規則をここで
 * 再実装せず呼び出し側の確立済み実装に委ねることで、Rust の `settings::resolve_workgroup`
 * とのズレを防ぐ（useWorkgroups はモジュールシングルトンで settings を直接引くため、
 * この純粋関数からは import せず引数で受け取る）。
 */
export type GroupResolver = (
  worktree: Pick<WorktreeEntry, "workgroupId">,
) => Workgroup | undefined;

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
  groupOf: GroupResolver,
): boolean {
  // `??` で繋ぐこと。settings.rs の Option フィールドには skip_serializing_if が無いため、
  // get_settings は未設定を `undefined` ではなく **`null`** で返す（既存 settings.json の
  // `"autoApproval": null` と同じ形）。`!== undefined` で判定すると null を
  // 「個別に設定済み」と誤認し、ワークグループ既定値へ落ちなくなる。
  return worktree.trayNotification ?? groupOf(worktree)?.trayNotification ?? true;
}

/** settings 全体から Map<worktreeId, 実効値> を組み立てる。 */
export function buildTrayNotificationMap(
  settings: AppSettings,
  groupOf: GroupResolver,
): Map<string, boolean> {
  const map = new Map<string, boolean>();
  for (const wt of settings.worktrees) {
    map.set(wt.id, resolveTrayNotification(wt, groupOf));
  }
  return map;
}
