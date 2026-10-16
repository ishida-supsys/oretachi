<script setup lang="ts">
import { reactive, ref, onMounted, onBeforeUnmount } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import type { AppInstanceList, AppInstanceListItem } from "../types/appInstance";
import { buildPermissionView, formatGrantedAt } from "../utils/appPermissionView";
import {
  appRowBadges,
  appRowName,
  canOpenWindow,
  canStart,
  canStop,
  summarizeGrant,
} from "../utils/appGrantSummary";
import { useArtifactWindow } from "../composables/useArtifactWindow";

const { t } = useI18n();
const { openArtifactViewer } = useArtifactWindow();

const items = ref<AppInstanceListItem[]>([]);
const dangerousTools = ref<string[]>([]);
const loading = ref(false);
/** 一覧の読み込みエラーと、操作のエラーは別に持つ（再読み込みで操作のエラーを消さない） */
const loadError = ref("");
const actionError = ref("");
const expanded = reactive(new Set<string>());
/** 押下中（二重起動のエラーを出さない）。キーは keyOf */
const busy = reactive(new Set<string>());
let loadSeq = 0;
let unlisten: UnlistenFn | null = null;
let disposed = false;

const keyOf = (i: AppInstanceListItem) => `${i.worktreeId}/${i.artifactId}`;

async function load() {
  // app-instance-changed は連発するので、古い応答が新しい状態を上書きしないよう世代で捨てる
  const seq = ++loadSeq;
  loading.value = true;
  try {
    const res = await invoke<AppInstanceList>("list_app_instances");
    if (seq !== loadSeq || disposed) return;
    items.value = res.items;
    dangerousTools.value = res.dangerousTools;
    loadError.value = "";
  } catch (e) {
    if (seq !== loadSeq) return;
    loadError.value = String(e);
  } finally {
    if (seq === loadSeq) loading.value = false;
  }
}

function toggle(item: AppInstanceListItem) {
  if (!item.granted) return;
  const k = keyOf(item);
  if (expanded.has(k)) expanded.delete(k);
  else expanded.add(k);
}

async function start(item: AppInstanceListItem) {
  const k = keyOf(item);
  if (busy.has(k)) return;
  busy.add(k);
  actionError.value = "";
  try {
    // 承認は start の中で、これまでどおり承認ダイアログを通る（ここからバイパスしない）。
    // 承認待ちの間は invoke が返らないので、その間ボタンを押せなくする
    await invoke("app_instance_start", { worktreeId: item.worktreeId, artifactId: item.artifactId });
  } catch (e) {
    actionError.value = String(e);
  } finally {
    busy.delete(k);
  }
}

async function stop(item: AppInstanceListItem) {
  actionError.value = "";
  try {
    await invoke("app_instance_stop", { worktreeId: item.worktreeId, artifactId: item.artifactId });
  } catch (e) {
    actionError.value = String(e);
  }
}

async function openWindow(item: AppInstanceListItem) {
  try {
    await openArtifactViewer(item.worktreeId, item.artifactId);
  } catch (e) {
    actionError.value = String(e);
  }
}

async function revoke(item: AppInstanceListItem) {
  const ok = await ask(
    t("revokeConfirm", { name: appRowName(item), worktree: item.worktreeName }),
    { title: t("revokeTitle"), kind: "warning" },
  );
  if (!ok) return;
  actionError.value = "";
  try {
    await invoke("revoke_app_grant", { worktreeId: item.worktreeId, artifactId: item.artifactId });
  } catch (e) {
    actionError.value = String(e);
  }
  await load();
}

function summaryParts(item: AppInstanceListItem) {
  return item.resolved ? summarizeGrant(item.resolved) : [];
}

function detailRows(item: AppInstanceListItem) {
  return item.resolved ? buildPermissionView(item.resolved, null, dangerousTools.value).rows : [];
}

/** 展開行の 1 行を `種別 / 値` にする。値は承認ダイアログと同じ（解決済みの絶対パス・URL・maxBytes・越境先） */
function rowKind(r: ReturnType<typeof detailRows>[number]): string {
  switch (r.kind) {
    case "fs":
      return r.access === "write" ? t("kindFsWrite") : t("kindFsRead");
    case "db":
      return r.access === "write" ? t("kindDbWrite") : t("kindDbRead");
    default:
      return t("kind_" + r.kind);
  }
}

function rowValue(r: ReturnType<typeof detailRows>[number]): string {
  switch (r.kind) {
    case "target":
    case "fs":
    case "db":
    case "network":
      return r.label;
    case "memory":
      return t("memoryMax", { bytes: r.value });
    case "aiTerminal":
      return t("summaryAiTerminal", { spec: r.value });
    case "subWindow":
      return t("summarySubWindow", { spec: r.label });
    case "fsSensitive":
      return t("summaryFsSensitive");
    case "dbSensitive":
      return t("summaryDbSensitive");
    default:
      return r.value;
  }
}

onMounted(async () => {
  await load();
  // 起動・停止・承認・取り消しで状態が変わる
  const off = await listen("app-instance-changed", () => void load());
  if (disposed) off();
  else unlisten = off;
});

onBeforeUnmount(() => {
  disposed = true;
  unlisten?.();
});
</script>

<template>
  <div class="apps-table-wrapper">
    <p v-if="loadError" class="error">{{ loadError }}</p>
    <p v-if="actionError" class="error">{{ actionError }}</p>
    <div v-if="!loading && items.length === 0" class="empty">{{ t("empty") }}</div>
    <table v-else-if="items.length > 0" class="apps-table">
      <thead>
        <tr>
          <th class="col-expand"></th>
          <th>{{ t("colApp") }}</th>
          <th>{{ t("colState") }}</th>
          <th>{{ t("colWorktree") }}</th>
          <th>{{ t("colPerms") }}</th>
          <th>{{ t("colGranted") }}</th>
          <th class="col-actions"></th>
        </tr>
      </thead>
      <tbody>
        <template v-for="item in items" :key="keyOf(item)">
          <tr>
            <td class="cell-expand" :class="{ clickable: item.granted }" @click="toggle(item)">
              <i v-if="item.granted" :class="expanded.has(keyOf(item)) ? 'pi pi-chevron-down' : 'pi pi-chevron-right'" />
            </td>
            <td class="cell-name">{{ appRowName(item) }}</td>
            <td>
              <span class="badges">
                <span
                  v-for="b in appRowBadges(item)"
                  :key="b"
                  class="badge"
                  :class="'badge-' + b"
                  :title="b === 'manifestError' ? (item.manifestError ?? '') : ''"
                >{{ t("state_" + b) }}</span>
              </span>
            </td>
            <td class="cell-sub">{{ item.worktreeName }}</td>
            <td class="cell-perms">
              <template v-if="item.granted">
                <template v-for="(p, i) in summaryParts(item)" :key="p.key">
                  <span v-if="i > 0"> / </span>
                  <span :class="{ critical: p.critical }">{{ t(p.key, p.params ?? {}) }}</span>
                </template>
                <span v-if="summaryParts(item).length === 0">{{ t("summaryNone") }}</span>
              </template>
              <span v-else class="muted">—</span>
            </td>
            <td class="cell-sub">{{ formatGrantedAt(item.grantedAt) }}</td>
            <td class="cell-actions">
              <button v-if="canStop(item)" class="btn" @click="stop(item)">{{ t("stop") }}</button>
              <button v-else class="btn btn-start" :disabled="!canStart(item) || busy.has(keyOf(item))" @click="start(item)">{{ t("start") }}</button>
              <button class="btn btn-open" :disabled="!canOpenWindow(item)" @click="openWindow(item)">{{ t("openWindow") }}</button>
              <button class="btn btn-revoke" :disabled="!item.granted" @click="revoke(item)">{{ t("revoke") }}</button>
            </td>
          </tr>
          <tr v-if="item.granted && expanded.has(keyOf(item))" class="detail-row">
            <td></td>
            <td colspan="6">
              <div class="detail">
                <div v-if="detailRows(item).length === 0" class="muted">{{ t("summaryNone") }}</div>
                <div
                  v-for="r in detailRows(item)"
                  :key="r.kind + r.value"
                  class="detail-line"
                  :class="r.severity"
                >
                  <span class="detail-kind">{{ rowKind(r) }}</span>
                  <code class="detail-value">{{ rowValue(r) }}</code>
                </div>
              </div>
            </td>
          </tr>
        </template>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.apps-table-wrapper {
  width: 100%;
  overflow-x: auto;
}
.apps-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}
.apps-table thead tr {
  background: #181825;
  border-bottom: 1px solid #313244;
}
.apps-table th {
  padding: 8px 12px;
  text-align: left;
  font-size: 11px;
  font-weight: 600;
  color: #6c7086;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  white-space: nowrap;
}
.apps-table tbody tr {
  border-bottom: 1px solid #1e1e2e;
}
.apps-table tbody tr:not(.detail-row):hover {
  background: #181825;
}
.apps-table td {
  padding: 9px 12px;
  color: #cdd6f4;
  vertical-align: top;
}
.col-expand {
  width: 20px;
}
.cell-expand {
  color: #6c7086;
  font-size: 11px;
}
.cell-expand.clickable {
  cursor: pointer;
}
.cell-name {
  font-weight: 500;
}
.cell-sub {
  color: #a6adc8;
}
.cell-perms {
  font-size: 12px;
}
.critical {
  color: #f38ba8;
}
.muted {
  color: #6c7086;
}
.badges {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
}
.badge {
  padding: 0 8px;
  border-radius: 10px;
  font-size: 10px;
  white-space: nowrap;
  border: 1px solid transparent;
}
.badge-running {
  background: #1e3a2a;
  color: #a6e3a1;
}
.badge-starting {
  background: #3a3520;
  color: #f9e2af;
}
.badge-stopped {
  background: #313244;
  color: #6c7086;
}
.badge-ungranted {
  background: #3a3520;
  color: #f9e2af;
}
.badge-manifestError,
.badge-missing {
  background: #3a2030;
  color: #f38ba8;
}
.badge-reapproval {
  background: #3a2a20;
  color: #fab387;
}
.cell-actions {
  white-space: nowrap;
}
.btn {
  background: transparent;
  border: 1px solid #45475a;
  border-radius: 4px;
  color: #a6adc8;
  padding: 2px 10px;
  font-size: 12px;
  cursor: pointer;
  margin-right: 4px;
}
.btn:hover:not(:disabled) {
  background: #313244;
}
.btn:disabled {
  opacity: 0.35;
  cursor: default;
}
.btn-start {
  color: #a6e3a1;
}
.btn-open {
  color: #89b4fa;
}
.btn-revoke {
  color: #f38ba8;
}
.detail {
  background: #181825;
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 8px 10px;
}
.detail-line {
  display: flex;
  gap: 10px;
  font-size: 12px;
  padding: 2px 0;
}
.detail-line.danger .detail-value {
  color: #fab387;
}
.detail-line.critical .detail-value {
  color: #f38ba8;
}
.detail-kind {
  width: 130px;
  flex-shrink: 0;
  color: #6c7086;
}
.detail-value {
  color: #a6adc8;
  word-break: break-all;
}
.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 200px;
  color: #6c7086;
  font-size: 14px;
}
.error {
  margin: 0 0 8px;
  font-size: 12px;
  color: #f38ba8;
}
</style>

<i18n lang="json">
{
  "en": {
    "empty": "No app artifacts.",
    "colApp": "App",
    "colState": "State",
    "colWorktree": "Worktree",
    "colPerms": "Permissions",
    "colGranted": "Approved",
    "state_running": "running",
    "state_starting": "starting",
    "state_stopped": "stopped",
    "state_ungranted": "not approved",
    "state_reapproval": "re-approval pending",
    "state_manifestError": "invalid manifest",
    "state_missing": "artifact missing",
    "start": "Start",
    "stop": "Stop",
    "openWindow": "Open in window",
    "revoke": "Revoke",
    "revokeTitle": "Revoke approval",
    "revokeConfirm": "Revoke the approval for \"{name}\" in \"{worktree}\"? A running app will be stopped.",
    "kind_tool": "MCP tool",
    "kind_target": "Other worktree",
    "kind_memory": "Memory",
    "kind_aiTerminal": "AI terminal",
    "kind_subWindow": "Sub-window",
    "kind_subWindowFlag": "Sub-window flag",
    "kind_fsSensitive": "Sensitive files",
    "kind_dbSensitive": "Sensitive databases",
    "kind_network": "Network",
    "kindFsRead": "Files (read)",
    "kindFsWrite": "Files (write)",
    "kindDbRead": "Database (read-only)",
    "kindDbWrite": "Database (read-write)",
    "memoryMax": "up to {bytes} B",
    "summaryTools": "{count} MCP tool(s)",
    "summaryWildcard": "wildcard worktrees",
    "summaryTargets": "other worktrees",
    "summaryMemory": "memory {bytes} B",
    "summaryAiTerminal": "AI terminal {spec}",
    "summaryFs": "files (read {read} / write {write})",
    "summaryFsSensitive": "writes to hook/workflow files",
    "summaryDb": "databases (read-only {read} / read-write {write})",
    "summaryDbSensitive": "databases in hook/workflow locations",
    "summaryNetwork": "network ({count} URL prefixes)",
    "summarySubWindow": "sub-windows ({spec})",
    "summarySubWindowOverlay": "sub-windows that can overlay other windows ({spec})",
    "summaryNone": "no permissions"
  },
  "ja": {
    "empty": "アプリ形式のアーティファクトはありません。",
    "colApp": "アプリ",
    "colState": "状態",
    "colWorktree": "ワークツリー",
    "colPerms": "権限",
    "colGranted": "承認日時",
    "state_running": "起動中",
    "state_starting": "起動準備中",
    "state_stopped": "停止中",
    "state_ungranted": "未承認",
    "state_reapproval": "再承認待ち",
    "state_manifestError": "manifest 不正",
    "state_missing": "アーティファクトなし",
    "start": "起動",
    "stop": "停止",
    "openWindow": "ウィンドウで開く",
    "revoke": "取り消す",
    "revokeTitle": "承認の取り消し",
    "revokeConfirm": "「{worktree}」の「{name}」の承認を取り消しますか？起動中のアプリは停止します。",
    "kind_tool": "MCP ツール",
    "kind_target": "他ワークツリー",
    "kind_memory": "メモリー",
    "kind_aiTerminal": "AI ターミナル",
    "kind_subWindow": "サブウィンドウ",
    "kind_subWindowFlag": "サブウィンドウの属性",
    "kind_fsSensitive": "既定拒否パス",
    "kind_dbSensitive": "既定拒否パスの DB",
    "kind_network": "ネットワーク",
    "kindFsRead": "ファイル（読み取り）",
    "kindFsWrite": "ファイル（書き込み）",
    "kindDbRead": "データベース（読み取り専用）",
    "kindDbWrite": "データベース（読み書き）",
    "memoryMax": "最大 {bytes} B",
    "summaryTools": "MCP ツール {count} 件",
    "summaryWildcard": "ワイルドカードのワークツリー",
    "summaryTargets": "他ワークツリー",
    "summaryMemory": "メモリー {bytes} B",
    "summaryAiTerminal": "AI ターミナル {spec}",
    "summaryFs": "ファイル（読 {read} / 書 {write}）",
    "summaryFsSensitive": "フック・ワークフローのファイルへの書き込み",
    "summaryDb": "データベース（読み取り専用 {read} / 読み書き {write}）",
    "summaryDbSensitive": "フック・ワークフローの場所のデータベース",
    "summaryNetwork": "ネットワーク（URL プレフィックス {count} 件）",
    "summarySubWindow": "サブウィンドウ（{spec}）",
    "summarySubWindowOverlay": "他のウィンドウに重なれるサブウィンドウ（{spec}）",
    "summaryNone": "権限なし"
  }
}
</i18n>
