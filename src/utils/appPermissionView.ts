import type { AppResolvedPermissions } from "../types/appInstance";

/**
 * 承認ダイアログに並べる権限の行を組み立てる純関数（描画は呼び出し側）。
 *
 * - `critical`: 最も強い警告。ワイルドカードの越境先（今後作られるワークツリーも含み、メインの
 *   クローンの端末にも届く）、FS の既定拒否パス解除（`.git/hooks` などへ書けるので任意コード実行になる）、
 *   専用 AI ターミナル（ユーザーと同じ権限で動くエージェントを起動できる）。
 * - `danger`: ユーザーと同等の権限（任意コマンド実行）になりうるツールと、ファイルの書き込み。
 * - `normal`: それ以外。
 *
 * 再承認のときは、直前に有効だった権限との差分（追加された行・外れた行）を出す。
 * **見せるのは解決済みの形**（manifest の生文字列ではない）。
 */

export type PermissionSeverity = "normal" | "danger" | "critical";

export interface PermissionRow {
  /** `fs`: FS のスコープ 1 件（`access` と、解決済みの絶対パス `label`）。`fsSensitive`: 既定拒否パスの解除 */
  kind: "tool" | "target" | "memory" | "aiTerminal" | "fs" | "fsSensitive";
  /** ツール名 / 越境先の `stored` / `maxBytes` の文字列 / `claude×N`（件数の変更も差分に出る）/ FS は `<read|write>:<scheme>:<絶対パス>` */
  value: string;
  /** 越境先のときの表示名 / FS のときの表示用の絶対パス */
  label: string;
  /** `fs` 行のアクセス種別 */
  access?: "read" | "write";
  /** `fs` 行のスキーム（`worktree://` / `artifact://` のどちらの領域か） */
  scheme?: "worktree" | "artifact";
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

const VERBATIM_UNC = "\\\\?\\UNC\\";
const VERBATIM = "\\\\?\\";

/**
 * Windows の `canonicalize` が返す拡張長パス（`\\?\C:\x` / `\\?\UNC\srv\x`）を、人が読む形へ戻す。
 * **表示専用**（判定には使わない。Rust は保存済みの形をそのまま比較する）。
 */
export function displayFsPath(p: string): string {
  if (p.startsWith(VERBATIM_UNC)) return "\\\\" + p.slice(VERBATIM_UNC.length);
  if (p.startsWith(VERBATIM)) return p.slice(VERBATIM.length);
  return p;
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
  if (perm.aiTerminal) {
    const value = `${perm.aiTerminal.agents.join("+")}×${perm.aiTerminal.maxConcurrent}`;
    rows.push({ kind: "aiTerminal", value, label: value, severity: "critical", wildcard: false, added: false });
  }
  if (perm.fs) {
    for (const s of perm.fs.scopes) {
      const shown = displayFsPath(s.path);
      rows.push({
        kind: "fs",
        value: `${s.write ? "write" : "read"}:${s.scheme}:${s.path}`,
        label: shown,
        access: s.write ? "write" : "read",
        scheme: s.scheme,
        severity: s.write ? "danger" : "normal",
        wildcard: false,
        added: false,
      });
    }
    if (perm.fs.allowSensitivePaths) {
      rows.push({
        kind: "fsSensitive",
        value: "allowSensitivePaths",
        label: "allowSensitivePaths",
        severity: "critical",
        wildcard: false,
        added: false,
      });
    }
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
