import type {
  AppProcessResolvedArg,
  AppProcessResolvedCommand,
  AppResolvedFs,
  AppResolvedPermissions,
  AppSchedulerAction,
} from "../types/appInstance";

/**
 * 承認ダイアログに並べる権限の行を組み立てる純関数（描画は呼び出し側）。
 *
 * - `critical`: 最も強い警告。ワイルドカードの越境先（今後作られるワークツリーも含み、メインの
 *   クローンの端末にも届く）、FS / DB の既定拒否パス解除（`.git/hooks` などへ書けるので任意コード実行になる）、
 *   専用 AI ターミナル（ユーザーと同じ権限で動くエージェントを起動できる）、マイクの音声入力
 *   （ユーザーの発話を聞ける）、コマンドの起動（`process`。ユーザーの権限で、承認した絶対パスの実行ファイルを動かせる）。
 * - `danger`: ユーザーと同等の権限（任意コマンド実行）になりうるツールと、ファイル・DB の書き込み。
 * - `normal`: それ以外。
 *
 * 再承認のときは、直前に有効だった権限との差分（追加された行・外れた行）を出す。
 * **見せるのは解決済みの形**（manifest の生文字列ではない）。
 */

export type PermissionSeverity = "normal" | "danger" | "critical";

export const PERMISSION_ROW_KINDS = [
  "tool",
  "target",
  "memory",
  "aiTerminal",
  "fs",
  "fsSensitive",
  "db",
  "dbSensitive",
  "network",
  "subWindow",
  "subWindowFlag",
  "audio",
  "mediaRead",
  "mediaNet",
  "scheduler",
  "process",
] as const;

export type PermissionRowKind = (typeof PERMISSION_ROW_KINDS)[number];

/** 描画側が分岐で扱える種別か。false の行は種別名と値をそのまま出す（別種別の文言に落とさない） */
export function isKnownRowKind(kind: string): kind is PermissionRowKind {
  return (PERMISSION_ROW_KINDS as readonly string[]).includes(kind);
}

export interface PermissionRow {
  /**
   * `mediaRead`: メディア再生のローカルのスコープ 1 件（`fs` と同じ形で、常に read）。
   * `mediaNet`: メディア再生の URL プレフィックス 1 件。
   * `fs` / `db`: FS・DB のスコープ 1 件（`access` と、解決済みの絶対パス `label`。DB の read は読み取り専用）。
   * `fsSensitive` / `dbSensitive`: 既定拒否パスの解除
   */
  kind: PermissionRowKind;
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
  /** `scheduler` 行: ジョブの cron 式 / 実行内容の種別 / 実行内容の 1 行表示（ツール名または initialPrompt の冒頭） */
  cron?: string;
  action?: "tool" | "aiTerminal";
  detail?: string;
  /**
   * `scheduler` 行: 実行内容の**全文**（ツールは名前と params の整形 JSON、AI ターミナルは initialPrompt /
   * systemPrompt / model / cwd。切り詰めない）。承認ダイアログはこれを出す。無人で動くので、承認する人が
   * 何が実行されるかをすべて見られなければならない（`detail` は一覧用の 1 行表示）
   */
  fullText?: string;
  /**
   * `process` 行（コマンド 1 件。`label` がコマンド名）: 承認時に解決した絶対パスの実行ファイル・引数の雛形
   * （固定の引数はそのまま、パラメータは `{名前: 型 …}`）・cwd・継承する環境変数・上限の表示。
   * manifest の生文字列ではなく**解決済みの形**を出す
   */
  procExec?: string;
  procArgs?: string[];
  procCwd?: string;
  procEnv?: string[];
  procLimits?: string;
  /**
   * `process` 行: `under` が、このアプリ自身の書き込みスコープ（`fs` / `db`）と重なっている `path` パラメータの名前。
   * アプリが自分で書いたファイルをそのままコマンドへ渡せる（それをコードとして実行する実行ファイル —
   * インタプリタにスクリプトのパスを渡す形など — なら、アプリが任意のコードを実行できる）。
   * manifest の検査は「exec・固定のパス引数・cwd・先頭の path パラメータ」だけを塞ぐ限定的なもので、
   * これはそこから漏れる形を人に見せるための警告（空・無しなら出さない）
   */
  procWritableParams?: string[];
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
  // マイクの音声入力: ユーザーの発話を聞ける権限なので最も強い警告。最大秒数とレートは解決済みの値のまま出す
  if (perm.audio) {
    const value = `${perm.audio.maxSeconds}s · ${perm.audio.sampleRates.join("/")}Hz`;
    rows.push({ kind: "audio", value, label: value, severity: "critical", wildcard: false, added: false });
  }
  // ネットワーク: Rust プロキシが通す URL プレフィックス 1 件ごとに 1 行（ws:// / wss:// は WebSocket）。
  // 平文の http / ws は盗聴・改ざんを受けうるので danger（https / wss は normal）。
  // ループバック等は常に拒否されるので、ここでは出さない
  for (const prefix of perm.network?.allow ?? []) {
    rows.push({
      kind: "network",
      value: prefix,
      label: prefix,
      severity: prefix.startsWith("http://") || prefix.startsWith("ws://") ? "danger" : "normal",
      wildcard: false,
      added: false,
    });
  }
  // メディア再生（#402）: ローカルのスコープ（読み取り専用）と URL プレフィックスを別の行にする。
  // どちらも「見る・聞く」だけだが、平文の http は盗聴・改ざんを受けうるので danger（network と同じ）
  for (const s of perm.media?.fs?.scopes ?? []) {
    rows.push({
      kind: "mediaRead",
      value: `read:${s.scheme}:${s.path}`,
      label: displayFsPath(s.path),
      access: "read",
      scheme: s.scheme,
      severity: "normal",
      wildcard: false,
      added: false,
    });
  }
  for (const prefix of perm.media?.allow ?? []) {
    rows.push({
      kind: "mediaNet",
      value: prefix,
      label: prefix,
      severity: prefix.startsWith("http://") ? "danger" : "normal",
      wildcard: false,
      added: false,
    });
  }
  // スケジューラー（#413）: 無人で動くジョブ 1 件ごとに 1 行。value に実行内容の全体を入れるので、
  // cron・ツール・params・プロンプトのどれが変わっても再承認の差分（追加・外れ）に出る。
  // AI ターミナルを開くジョブと、危険なツールを呼ぶジョブは、人が見ていない所で動くので critical
  for (const job of perm.scheduler?.jobs ?? []) {
    const a = job.action;
    const isAi = a.type === "aiTerminal";
    rows.push({
      kind: "scheduler",
      value: JSON.stringify([job.id, job.cron, a]),
      label: job.id,
      cron: job.cron,
      action: a.type,
      detail: isAi ? truncate(a.initialPrompt, 200) : a.tool,
      fullText: schedulerFullText(a),
      severity: isAi || dangerous.has(a.tool) ? "critical" : "normal",
      wildcard: false,
      added: false,
    });
  }
  // コマンドの起動（#428）: コマンド 1 件ごとに 1 行。常に critical（ユーザーの権限でコマンドを実行できる。
  // exec やスクリプトを書き換えられる相手も、この承認を通して任意のコードを実行できる）。
  // value にコマンドの解決済みの全体を入れるので、exec の絶対パス・引数・cwd・環境変数・上限のどれが
  // 変わっても再承認の差分（追加・外れ）に出る
  const writable = writableRoots(perm);
  for (const [name, cmd] of Object.entries(perm.process?.commands ?? {})) {
    const writableParams = cmd.args.flatMap((a) =>
      a.type === "path" && a.fs.scopes.some((s) => writable.some((w) => pathsOverlap(w, normalizePath(s.path))))
        ? [a.param]
        : [],
    );
    rows.push({
      kind: "process",
      value: JSON.stringify([name, cmd]),
      label: name,
      procExec: displayFsPath(cmd.exec),
      procArgs: cmd.args.map(processArgTemplate),
      procCwd: displayFsPath(cmd.cwd),
      procEnv: cmd.envInherit,
      procLimits: processLimits(cmd),
      ...(writableParams.length > 0 ? { procWritableParams: writableParams } : {}),
      severity: "critical",
      wildcard: false,
      added: false,
    });
  }
  // サブウィンドウ: 数と大きさの上限を 1 行、申請した見た目・位置の権限を 1 件ごとに 1 行。
  // 透明・枠なし・常に前面は、他のウィンドウの上に重なって操作を横取りできるので danger。位置の指定は normal
  if (perm.subWindow) {
    const sw = perm.subWindow;
    // 大きさの上限を省略した軸は「上限なし」= ∞
    const size = `${sw.maxCount}×${sw.maxWidth ?? "∞"}x${sw.maxHeight ?? "∞"}`;
    rows.push({
      kind: "subWindow",
      value: size,
      label: size,
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

/** 引数の雛形 1 件。固定の引数はそのまま、パラメータは `{名前: 型 …}`（`path` は解決済みの `under` の絶対パス付き） */
export function processArgTemplate(a: AppProcessResolvedArg): string {
  switch (a.type) {
    case "fixed":
      return /[\s"]/.test(a.value) ? JSON.stringify(a.value) : a.value;
    case "path": {
      const under = a.fs.scopes[0] ? displayFsPath(a.fs.scopes[0].path) : a.under;
      return `{${a.param}: path under ${under}}`;
    }
    case "token":
      return `{${a.param}: token}`;
    case "enum":
      return `{${a.param}: ${a.values.join(" | ")}}`;
  }
}

/** 比較用のパス（表示と同じく拡張長パスの接頭辞を外し、区切りを `/` に、大文字小文字は無視、末尾の区切りは落とす） */
function normalizePath(p: string): string {
  return displayFsPath(p).replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}

function pathsOverlap(a: string, b: string): boolean {
  return a === b || a.startsWith(b + "/") || b.startsWith(a + "/");
}

/** このアプリ自身が書き込めるスコープ（`fs` / `db` の write）の絶対パス */
function writableRoots(perm: AppResolvedPermissions): string[] {
  return [...(perm.fs?.scopes ?? []), ...(perm.db?.scopes ?? [])].filter((s) => s.write).map((s) => normalizePath(s.path));
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${+(n / 1024 / 1024 / 1024).toFixed(1)} GiB`;
  if (n >= 1024 * 1024) return `${+(n / 1024 / 1024).toFixed(1)} MiB`;
  if (n >= 1024) return `${+(n / 1024).toFixed(1)} KiB`;
  return `${n} B`;
}

/** 同時実行数・実行時間・出力量の上限の 1 行表示（承認した値のまま） */
function processLimits(c: AppProcessResolvedCommand): string {
  const time = c.timeoutSec == null ? "no timeout" : `timeout ${c.timeoutSec}s`;
  return `max ${c.maxConcurrent} · ${time} · output ≤ ${formatBytes(c.maxOutputBytes)}`;
}

function schedulerFullText(a: AppSchedulerAction): string {
  if (a.type === "tool") return `${a.tool}\n${JSON.stringify(a.params, null, 2)}`;
  const parts: string[] = [];
  if (a.title) parts.push(`title: ${a.title}`);
  if (a.model) parts.push(`model: ${a.model}`);
  if (a.cwd) parts.push(`cwd: ${a.cwd}`);
  if (a.systemPrompt) parts.push(`systemPrompt:\n${a.systemPrompt}`);
  parts.push(`initialPrompt:\n${a.initialPrompt}`);
  return parts.join("\n\n");
}

function truncate(s: string, max: number): string {
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > max ? t.slice(0, max) + "…" : t;
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
