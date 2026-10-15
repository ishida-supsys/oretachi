<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import type { AppGrantListItem } from "../../types/appInstance";
import { buildPermissionView, formatGrantedAt } from "../../utils/appPermissionView";

const { t } = useI18n();

const items = ref<AppGrantListItem[]>([]);
const loading = ref(false);
const error = ref("");
let unlisten: UnlistenFn | null = null;
let disposed = false;

async function load() {
  loading.value = true;
  try {
    items.value = await invoke<AppGrantListItem[]>("list_app_grants");
    error.value = "";
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function revoke(item: AppGrantListItem) {
  const ok = await ask(
    t("revokeConfirm", { name: item.artifactTitle || item.artifactId, worktree: item.worktreeName }),
    { title: t("revokeTitle"), kind: "warning" },
  );
  if (!ok) return;
  try {
    await invoke("revoke_app_grant", { worktreeId: item.worktreeId, artifactId: item.artifactId });
  } catch (e) {
    error.value = String(e);
  }
  await load();
}

function summary(item: AppGrantListItem): string {
  const v = buildPermissionView(item.resolved, null, []);
  const parts: string[] = [];
  const tools = v.rows.filter((r) => r.kind === "tool").length;
  if (tools > 0) parts.push(t("summaryTools", { count: tools }));
  // critical は越境先のワイルドカード・FS の既定拒否パス解除・専用 AI ターミナルの 3 種類あるので、要因ごとに出す
  if (v.rows.some((r) => r.wildcard)) parts.push(t("summaryWildcard"));
  else if (v.rows.some((r) => r.kind === "target")) parts.push(t("summaryTargets"));
  const mem = v.rows.find((r) => r.kind === "memory");
  if (mem) parts.push(t("summaryMemory", { bytes: mem.value }));
  const fsRead = v.rows.filter((r) => r.kind === "fs" && r.access === "read").length;
  const fsWrite = v.rows.filter((r) => r.kind === "fs" && r.access === "write").length;
  if (fsRead + fsWrite > 0) parts.push(t("summaryFs", { read: fsRead, write: fsWrite }));
  if (v.rows.some((r) => r.kind === "fsSensitive")) parts.push(t("summaryFsSensitive"));
  const ai = v.rows.find((r) => r.kind === "aiTerminal");
  if (ai) parts.push(t("summaryAiTerminal", { spec: ai.value }));
  const dbRead = v.rows.filter((r) => r.kind === "db" && r.access === "read").length;
  const dbWrite = v.rows.filter((r) => r.kind === "db" && r.access === "write").length;
  if (dbRead + dbWrite > 0) parts.push(t("summaryDb", { read: dbRead, write: dbWrite }));
  if (v.rows.some((r) => r.kind === "dbSensitive")) parts.push(t("summaryDbSensitive"));
  const net = v.rows.filter((r) => r.kind === "network").length;
  if (net > 0) parts.push(t("summaryNetwork", { count: net }));
  return parts.length > 0 ? parts.join(" / ") : t("summaryNone");
}

onMounted(async () => {
  await load();
  // 起動・停止・承認の取り消しで一覧の「起動中」表示が変わる
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
  <div class="field-group">
    <label class="field-label">{{ t("label") }}</label>
    <p class="note">{{ t("note") }}</p>
    <p v-if="error" class="error">{{ error }}</p>
    <p v-if="!loading && items.length === 0" class="empty">{{ t("empty") }}</p>
    <table v-else-if="items.length > 0" class="grant-table">
      <thead>
        <tr>
          <th>{{ t("colApp") }}</th>
          <th>{{ t("colWorktree") }}</th>
          <th>{{ t("colPerms") }}</th>
          <th>{{ t("colGranted") }}</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="item in items" :key="item.worktreeId + '/' + item.artifactId">
          <td>
            {{ item.artifactTitle || item.artifactId }}
            <span v-if="item.running" class="running">{{ t("running") }}</span>
          </td>
          <td>{{ item.worktreeName }}</td>
          <td>{{ summary(item) }}</td>
          <td>{{ formatGrantedAt(item.grantedAt) }}</td>
          <td><button class="btn-revoke" @click="revoke(item)">{{ t("revoke") }}</button></td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.note {
  margin: 0 0 8px;
  font-size: 12px;
  color: #6c7086;
}
.empty {
  margin: 0;
  font-size: 12px;
  color: #6c7086;
}
.error {
  margin: 0 0 8px;
  font-size: 12px;
  color: #f38ba8;
}
.grant-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}
.grant-table th {
  text-align: left;
  font-weight: 500;
  color: #6c7086;
  padding: 4px 8px;
  border-bottom: 1px solid #313244;
}
.grant-table td {
  padding: 6px 8px;
  border-bottom: 1px solid #262637;
  color: #cdd6f4;
}
.running {
  margin-left: 6px;
  padding: 0 6px;
  border-radius: 8px;
  background: #1e3a2a;
  color: #a6e3a1;
  font-size: 10px;
}
.btn-revoke {
  background: transparent;
  border: 1px solid #45475a;
  border-radius: 4px;
  color: #f38ba8;
  padding: 2px 10px;
  font-size: 12px;
  cursor: pointer;
}
.btn-revoke:hover {
  background: #313244;
}
</style>

<i18n lang="json">
{
  "en": {
    "label": "Approved apps",
    "note": "App artifacts you have allowed to run. Revoking asks for approval again the next time the app starts (and stops it if it is running).",
    "empty": "No apps have been approved.",
    "colApp": "App",
    "colWorktree": "Worktree",
    "colPerms": "Permissions",
    "colGranted": "Approved",
    "running": "running",
    "revoke": "Revoke",
    "revokeTitle": "Revoke approval",
    "revokeConfirm": "Revoke the approval for \"{name}\" in \"{worktree}\"? A running app will be stopped.",
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
    "summaryNone": "no permissions"
  },
  "ja": {
    "label": "承認済みのアプリ",
    "note": "実行を許可したアプリ形式のアーティファクトです。取り消すと、次に起動するときにもう一度承認を求めます（起動中なら停止します）。",
    "empty": "承認済みのアプリはありません。",
    "colApp": "アプリ",
    "colWorktree": "ワークツリー",
    "colPerms": "権限",
    "colGranted": "承認日時",
    "running": "起動中",
    "revoke": "取り消す",
    "revokeTitle": "承認の取り消し",
    "revokeConfirm": "「{worktree}」の「{name}」の承認を取り消しますか？起動中のアプリは停止します。",
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
    "summaryNone": "権限なし"
  }
}
</i18n>
