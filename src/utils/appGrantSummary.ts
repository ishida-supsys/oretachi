import type { AppInstanceListItem, AppResolvedPermissions } from "../types/appInstance";
import { buildPermissionView } from "./appPermissionView";

/** 権限の要約 1 件。`key` は AppsTable の i18n キー（`summary*`）、`params` は補間値 */
export interface SummaryPart {
  key: string;
  params?: Record<string, string | number>;
  /** 越境先のワイルドカード・FS/DB の既定拒否パス解除・AI ターミナルなど、最も強い警告の対象か */
  critical: boolean;
}

/**
 * 承認済みの権限を、一覧の 1 セル向けに要約する。表示ロジックは承認ダイアログと同じ
 * `buildPermissionView` の行から数えるので、ダイアログに出る権限が要約から漏れない。
 * 解決済みの絶対パス・URL・maxBytes などの詳細は、行の展開側（同じ行ビュー）が出す。
 */
export function summarizeGrant(resolved: AppResolvedPermissions): SummaryPart[] {
  const v = buildPermissionView(resolved, null, []);
  const parts: SummaryPart[] = [];
  const count = (kind: string, access?: "read" | "write") =>
    v.rows.filter((r) => r.kind === kind && (access === undefined || r.access === access)).length;

  const tools = count("tool");
  if (tools > 0) parts.push({ key: "summaryTools", params: { count: tools }, critical: false });
  // 越境先はワイルドカードか名指しか。ワイルドカードは承認後に作られるワークツリーにも当たる
  if (v.rows.some((r) => r.wildcard)) parts.push({ key: "summaryWildcard", critical: true });
  else if (count("target") > 0) parts.push({ key: "summaryTargets", critical: false });
  const mem = v.rows.find((r) => r.kind === "memory");
  if (mem) parts.push({ key: "summaryMemory", params: { bytes: mem.value }, critical: false });
  const fsRead = count("fs", "read");
  const fsWrite = count("fs", "write");
  if (fsRead + fsWrite > 0) parts.push({ key: "summaryFs", params: { read: fsRead, write: fsWrite }, critical: false });
  if (count("fsSensitive") > 0) parts.push({ key: "summaryFsSensitive", critical: true });
  const ai = v.rows.find((r) => r.kind === "aiTerminal");
  if (ai) parts.push({ key: "summaryAiTerminal", params: { spec: ai.value }, critical: true });
  const dbRead = count("db", "read");
  const dbWrite = count("db", "write");
  if (dbRead + dbWrite > 0) parts.push({ key: "summaryDb", params: { read: dbRead, write: dbWrite }, critical: false });
  if (count("dbSensitive") > 0) parts.push({ key: "summaryDbSensitive", critical: true });
  const net = count("network");
  if (net > 0) parts.push({ key: "summaryNetwork", params: { count: net }, critical: false });
  const sw = v.rows.find((r) => r.kind === "subWindow");
  if (sw) {
    const overlay = v.rows.some((r) => r.kind === "subWindowFlag" && r.severity === "danger");
    parts.push({
      key: overlay ? "summarySubWindowOverlay" : "summarySubWindow",
      params: { spec: sw.label },
      critical: overlay,
    });
  }
  return parts;
}

export type AppRowBadge = "running" | "starting" | "stopped" | "ungranted" | "reapproval";

/** 状態バッジ。実行状態に、未承認・再承認待ちを足して返す */
export function appRowBadges(item: AppInstanceListItem): AppRowBadge[] {
  const badges: AppRowBadge[] = [
    item.state === "running" ? "running" : item.state === "starting" ? "starting" : "stopped",
  ];
  if (!item.granted) badges.push("ungranted");
  if (item.reapprovalPending) badges.push("reapproval");
  return badges;
}

/** 行の表示名。title が空（アーティファクトが消えて承認だけ残っている）なら appName、id の順 */
export function appRowName(item: AppInstanceListItem): string {
  return item.artifactTitle || item.appName || item.artifactId;
}

/** 起動できるか。アーティファクトが消えた行（appName 空）は起動できない。承認は起動時のダイアログで行う */
export function canStart(item: AppInstanceListItem): boolean {
  return item.state === null && item.appName !== "";
}

/** 停止できるか（起動処理中＝承認ダイアログ待ちも止められる） */
export function canStop(item: AppInstanceListItem): boolean {
  return item.state !== null;
}

/** アーティファクトウィンドウで開けるか（アーティファクトが残っている行だけ） */
export function canOpenWindow(item: AppInstanceListItem): boolean {
  return item.appName !== "";
}
