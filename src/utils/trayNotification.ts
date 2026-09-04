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
 * 解決順は「ワークツリー個別 > true」のみで、
 * バックエンドの `settings::resolve_tray_notification` と同じ規則。
 *
 * ワークグループの `trayNotification` は参照しない。あれは
 * `initialTrayNotification` が新規ワークツリー作成時に一度だけ焼き込む初期値であり、
 * 後からグループ設定を変えても既存ワークツリーには影響しない（#171）。
 */
export function resolveTrayNotification(
  worktree: Pick<WorktreeEntry, "trayNotification">,
): boolean {
  // `??` で繋ぐこと。settings.rs の Option フィールドには skip_serializing_if が無いため、
  // get_settings は未設定を `undefined` ではなく **`null`** で返す（既存 settings.json の
  // `"autoApproval": null` と同じ形）。`!== undefined` で判定すると null を
  // 「個別に設定済み」と誤認する。
  return worktree.trayNotification ?? true;
}

/** settings 全体から Map<worktreeId, 実効値> を組み立てる。 */
export function buildTrayNotificationMap(settings: AppSettings): Map<string, boolean> {
  const map = new Map<string, boolean>();
  for (const wt of settings.worktrees) {
    map.set(wt.id, resolveTrayNotification(wt));
  }
  return map;
}

/**
 * 新規ワークツリーへ焼き込む `trayNotification` の初期値（#171）。
 *
 * 所属ワークグループが**明示的に設定している場合のみ**その値を返す。グループ側が
 * 未設定（`null` / `undefined`）なら `undefined` を返し、呼び出し側はキー自体を
 * 書かない（未設定のまま = 実効値 `true`）。`worktreeDefaults.autoApproval` と同じ
 * 「作成時にコピーする既定値」の流儀。
 */
export function initialTrayNotification(
  worktree: Pick<WorktreeEntry, "workgroupId">,
  groupOf: GroupResolver,
): boolean | undefined {
  return groupOf(worktree)?.trayNotification ?? undefined;
}
