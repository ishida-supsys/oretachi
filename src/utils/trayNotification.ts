import type { AppSettings, TrayNotificationMode, WorktreeEntry, Workgroup } from "../types/settings";

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

const TRAY_NOTIFICATION_MODES: readonly TrayNotificationMode[] = ["all", "need_input", "off"];

/**
 * `trayNotification` の生値を正規化する。バックエンド（`get_settings`）は読み込み時に
 * 旧 bool 値を `all`/`off` へ正規化して返すため通常は文字列しか来ないが、後方互換のため
 * bool もここで吸収する。未知の値・`null`/`undefined` は「未設定」として `undefined`。
 */
function normalizeTrayNotificationMode(
  value: TrayNotificationMode | boolean | null | undefined,
): TrayNotificationMode | undefined {
  if (value === true) return "all";
  if (value === false) return "off";
  if (value == null) return undefined;
  return TRAY_NOTIFICATION_MODES.includes(value) ? value : undefined;
}

/**
 * フック由来通知をトレイ通知として出すモードの実効値。
 * 解決順は「ワークツリー個別 > all」のみで、
 * バックエンドの `settings::resolve_tray_notification_mode` と同じ規則。
 *
 * ワークグループの `trayNotification` は参照しない。あれは
 * `initialTrayNotification` が新規ワークツリー作成時に一度だけ焼き込む初期値であり、
 * 後からグループ設定を変えても既存ワークツリーには影響しない（#171）。
 */
export function resolveTrayNotificationMode(
  worktree: Pick<WorktreeEntry, "trayNotification">,
): TrayNotificationMode {
  return normalizeTrayNotificationMode(worktree.trayNotification) ?? "all";
}

/** settings 全体から Map<worktreeId, 実効モード> を組み立てる。 */
export function buildTrayNotificationModeMap(settings: AppSettings): Map<string, TrayNotificationMode> {
  const map = new Map<string, TrayNotificationMode>();
  for (const wt of settings.worktrees) {
    map.set(wt.id, resolveTrayNotificationMode(wt));
  }
  return map;
}

/**
 * 新規ワークツリーへ焼き込む `trayNotification` の初期値（#171）。
 *
 * 所属ワークグループが**明示的に設定している場合のみ**その値を返す。グループ側が
 * 未設定（`null` / `undefined`）なら `undefined` を返し、呼び出し側はキー自体を
 * 書かない（未設定のまま = 実効値 `all`）。`worktreeDefaults.autoApproval` と同じ
 * 「作成時にコピーする既定値」の流儀。
 */
export function initialTrayNotification(
  worktree: Pick<WorktreeEntry, "workgroupId">,
  groupOf: GroupResolver,
): TrayNotificationMode | undefined {
  return normalizeTrayNotificationMode(groupOf(worktree)?.trayNotification);
}
