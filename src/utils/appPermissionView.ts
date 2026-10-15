import type { AppResolvedPermissions } from "../types/appInstance";

/**
 * 承認ダイアログに並べる権限の行を組み立てる純関数（描画は呼び出し側）。
 *
 * - `critical`: 最も強い警告（ワイルドカードの越境先）。今後作られるワークツリーも含み、
 *   メインのクローンの端末にも届く。
 * - `danger`: ユーザーと同等の権限（任意コマンド実行）になりうるツール。
 * - `normal`: それ以外。
 *
 * 再承認のときは、直前に有効だった権限との差分（追加された行・外れた行）を出す。
 * **見せるのは解決済みの形**（manifest の生文字列ではない）。
 */

export type PermissionSeverity = "normal" | "danger" | "critical";

export interface PermissionRow {
  kind: "tool" | "target" | "memory";
  /** ツール名 / 越境先の `stored` / `maxBytes` の文字列 */
  value: string;
  /** 越境先のときの表示名 */
  label: string;
  severity: PermissionSeverity;
  /** 越境先がワイルドカードか（承認後に作られるワークツリーにも当たる） */
  wildcard: boolean;
  /** 再承認で、直前の権限に無かった行 */
  added: boolean;
}

export interface PermissionView {
  rows: PermissionRow[];
  /** 再承認で外れた行（旧セットにあって新セットに無い） */
  removed: PermissionRow[];
  hasCritical: boolean;
  hasDanger: boolean;
  /** 越境先が空 = 自ワークツリーのみ */
  ownWorktreeOnly: boolean;
}

function rowsOf(perm: AppResolvedPermissions, dangerous: ReadonlySet<string>): PermissionRow[] {
  const rows: PermissionRow[] = [];
  for (const tool of perm.mcp?.tools ?? []) {
    rows.push({
      kind: "tool",
      value: tool,
      label: tool,
      severity: dangerous.has(tool) ? "danger" : "normal",
      wildcard: false,
      added: false,
    });
  }
  for (const target of perm.mcp?.targets ?? []) {
    rows.push({
      kind: "target",
      value: target.stored,
      label: target.label,
      severity: target.wildcard ? "critical" : "normal",
      wildcard: target.wildcard,
      added: false,
    });
  }
  if (perm.memory) {
    rows.push({
      kind: "memory",
      value: String(perm.memory.maxBytes),
      label: String(perm.memory.maxBytes),
      severity: "normal",
      wildcard: false,
      added: false,
    });
  }
  return rows;
}

const keyOf = (r: PermissionRow) => `${r.kind}\u0000${r.value}`;

export function buildPermissionView(
  next: AppResolvedPermissions,
  previous: AppResolvedPermissions | null,
  dangerousTools: readonly string[],
): PermissionView {
  const dangerous = new Set(dangerousTools);
  const rows = rowsOf(next, dangerous);
  let removed: PermissionRow[] = [];

  if (previous) {
    const before = rowsOf(previous, dangerous);
    const beforeKeys = new Set(before.map(keyOf));
    const afterKeys = new Set(rows.map(keyOf));
    for (const r of rows) r.added = !beforeKeys.has(keyOf(r));
    removed = before.filter((r) => !afterKeys.has(keyOf(r)));
  }

  return {
    rows,
    removed,
    hasCritical: rows.some((r) => r.severity === "critical"),
    hasDanger: rows.some((r) => r.severity === "danger"),
    ownWorktreeOnly: (next.mcp?.targets.length ?? 0) === 0,
  };
}

/** UNIX 秒 → 表示用の日時（ロケール依存。表示専用） */
export function formatGrantedAt(sec: number | null | undefined): string {
  if (!sec) return "—";
  return new Date(sec * 1000).toLocaleString();
}
