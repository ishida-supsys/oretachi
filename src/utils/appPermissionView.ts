import type { AppResolvedFs, AppResolvedPermissions } from "../types/appInstance";

/**
 * 承認ダイアログに並べる権限の行を組み立てる純関数（描画は呼び出し側）。
 *
 * - `critical`: 最も強い警告。ワイルドカードの越境先（今後作られるワークツリーも含み、メインの
 *   クローンの端末にも届く）、FS / DB の既定拒否パス解除（`.git/hooks` などへ書けるので任意コード実行になる）、
 *   専用 AI ターミナル（ユーザーと同じ権限で動くエージェントを起動できる）。
 * - `danger`: ユーザーと同等の権限（任意コマンド実行）になりうるツールと、ファイル・DB の書き込み。
 * - `normal`: それ以外。
 *
 * 再承認のときは、直前に有効だった権限との差分（追加された行・外れた行）を出す。
 * **見せるのは解決済みの形**（manifest の生文字列ではない）。
 */

export type PermissionSeverity = "normal" | "danger" | "critical";

export interface PermissionRow {
  /**
   * `fs` / `db`: FS・DB のスコープ 1 件（`access` と、解決済みの絶対パス `label`。DB の read は読み取り専用）。
   * `fsSensitive` / `dbSensitive`: 既定拒否パスの解除
   */
  kind:
    | "tool"
    | "target"
    | "memory"
    | "aiTerminal"
    | "fs"
    | "fsSensitive"
    | "db"
    | "dbSensitive"
    | "network"
    | "subWindow"
    | "subWindowFlag";
  /** サブウィンドウは `<最大数>×<幅>x<高さ>`、`subWindowFlag` は `transparent` / `frameless` / `alwaysOnTop` / `position`。ツール名 / 越境先の `stored` / `maxBytes` の文字列 / `claude×N`（件数の変更も差分に出る）/ FS・DB は `<read|write>:<scheme>:<絶対パス>` / ネットワークは正規化済みの URL プレフィックス */
  value: string;
  /** 越境先のときの表示名 / FS・DB のときの表示用の絶対パス */
  label: string;
  /** `fs` / `db` 行のアクセス種別 */
  access?: "read" | "write";
  /** `fs` / `db` 行のスキーム（`worktree://` / `artifact://` のどちらの領域か） */
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
  // ネットワーク: Rust プロキシが通す URL プレフィックス 1 件ごとに 1 行。平文 http は盗聴・改ざんを
  // 受けうるので danger（https は normal）。ループバック等は常に拒否されるので、ここでは出さない
  for (const prefix of perm.network?.allow ?? []) {
    rows.push({
      kind: "network",
      value: prefix,
      label: prefix,
      severity: prefix.startsWith("http://") ? "danger" : "normal",
      wildcard: false,
      added: false,
    });
  }
  // サブウィンドウ: 数と大きさの上限を 1 行、申請した見た目・位置の権限を 1 件ごとに 1 行。
  // 透明・枠なし・常に前面は、他のウィンドウの上に重なって操作を横取りできるので danger。位置の指定は normal
  if (perm.subWindow) {
    const sw = perm.subWindow;
    rows.push({
      kind: "subWindow",
      value: `${sw.maxCount}×${sw.maxWidth}x${sw.maxHeight}`,
      label: `${sw.maxCount}×${sw.maxWidth}x${sw.maxHeight}`,
      severity: "normal",
      wildcard: false,
      added: false,
    });
    const flags: [string, boolean, PermissionSeverity][] = [
      ["transparent", sw.transparent, "danger"],
      ["frameless", sw.frameless, "danger"],
      ["alwaysOnTop", sw.alwaysOnTop, "danger"],
      ["position", sw.position, "normal"],
    ];
    for (const [flag, on, severity] of flags) {
      if (on) rows.push({ kind: "subWindowFlag", value: flag, label: flag, severity, wildcard: false, added: false });
    }
  }
  if (perm.fs) rows.push(...scopeRows(perm.fs, "fs", "fsSensitive"));
  if (perm.db) rows.push(...scopeRows(perm.db, "db", "dbSensitive"));
  return rows;
}

/** FS / DB のスコープの行（両者は同じ形で、種別だけ違う） */
function scopeRows(
  scoped: AppResolvedFs,
  kind: "fs" | "db",
  sensitiveKind: "fsSensitive" | "dbSensitive",
): PermissionRow[] {
  const rows: PermissionRow[] = scoped.scopes.map((s) => ({
    kind,
    value: `${s.write ? "write" : "read"}:${s.scheme}:${s.path}`,
    label: displayFsPath(s.path),
    access: s.write ? "write" : "read",
    scheme: s.scheme,
    severity: s.write ? "danger" : "normal",
    wildcard: false,
    added: false,
  }));
  if (scoped.allowSensitivePaths) {
    rows.push({
      kind: sensitiveKind,
      value: "allowSensitivePaths",
      label: "allowSensitivePaths",
      severity: "critical",
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
