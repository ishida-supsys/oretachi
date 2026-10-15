<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { ArtifactData } from "../../types/artifact";
import type {
  AppGrant,
  AppInstanceStatus,
  AppLaunchApi,
  AppLogEntry,
} from "../../types/appInstance";
import { buildPermissionView, formatGrantedAt } from "../../utils/appPermissionView";

const { t } = useI18n();

const props = defineProps<{
  artifact: ArtifactData;
  /** 起動操作。Web ビューア / リポジトリ保管庫では渡されない（起動不可と表示する） */
  api?: AppLaunchApi;
  /** 起動できない理由。api が無いときの文言を切り替える */
  unsupportedReason?: "web" | "repository";
}>();

const status = ref<AppInstanceStatus | null>(null);
const logs = ref<AppLogEntry[]>([]);
const grant = ref<AppGrant | null>(null);
const busy = ref(false);
const errorText = ref("");
const logBox = ref<HTMLElement | null>(null);

// manifest は AI が書いたもの。名前などの表示にだけ使い、権限の根拠にはしない
const manifest = computed(() => props.artifact.app ?? {});
const appName = computed(() => manifest.value.name || props.artifact.title);

const isStarting = computed(() => status.value?.state === "starting");
const isRunning = computed(() => status.value?.running === true);
const stateLabel = computed(() => {
  if (isStarting.value) return t("state.starting");
  if (isRunning.value) return t("state.running");
  return t("state.stopped");
});

const permissionView = computed(() =>
  grant.value ? buildPermissionView(grant.value.resolved, null, []) : null,
);

async function refresh() {
  if (!props.api) return;
  const id = props.artifact.id;
  try {
    const [s, l, g] = await Promise.all([props.api.status(id), props.api.log(id), props.api.grant(id)]);
    status.value = s;
    logs.value = l;
    grant.value = g;
    await scrollLogToEnd();
  } catch (e) {
    errorText.value = String(e);
  }
}

async function scrollLogToEnd() {
  await nextTick();
  const el = logBox.value;
  if (el) el.scrollTop = el.scrollHeight;
}

async function run(action: () => Promise<unknown>) {
  if (busy.value) return;
  busy.value = true;
  errorText.value = "";
  try {
    await action();
  } catch (e) {
    errorText.value = String(e);
  } finally {
    busy.value = false;
    await refresh();
  }
}

/**
 * 起動は承認ダイアログの応答まで（最大 10 分）戻らない。`busy` に含めると、承認待ちの間
 * 停止・承認の取り消しまで無効になって UI から取り下げられなくなるので、起動だけは別扱いにする
 * （二重起動は status の starting / running と Rust 側の予約で止まる）。
 */
const startPending = ref(false);
async function onStart() {
  if (startPending.value || isRunning.value || isStarting.value) return;
  startPending.value = true;
  errorText.value = "";
  try {
    await props.api!.start(props.artifact.id);
  } catch (e) {
    errorText.value = String(e);
  } finally {
    startPending.value = false;
    await refresh();
  }
}
const onStop = () => run(() => props.api!.stop(props.artifact.id));
const onToggleVisible = () => run(() => props.api!.toggleVisible(props.artifact.id));
const onRevoke = () => run(() => props.api!.revoke(props.artifact.id));

const unlistens: Array<() => void> = [];
let disposed = false;

onMounted(async () => {
  if (!props.api) return;
  await refresh();
  const id = props.artifact.id;
  const offChanged = await props.api.onChanged((s) => {
    if (s.artifactId !== id) return;
    status.value = s;
    // 承認の保存・取り消しは status の granted だけでは中身が分からないので取り直す
    void props.api!.grant(id).then((g) => (grant.value = g));
  });
  const offLog = await props.api.onLog(({ artifactId, entry }) => {
    if (artifactId !== id) return;
    // refresh() のスナップショットとイベントが重なると同じ行が 2 回入る（実機で確認）
    if (logs.value.some((l) => l.ts === entry.ts && l.message === entry.message)) return;
    logs.value = [...logs.value, entry].slice(-500);
  });
  if (disposed) {
    offChanged();
    offLog();
    return;
  }
  unlistens.push(offChanged, offLog);
});

onBeforeUnmount(() => {
  disposed = true;
  for (const off of unlistens) off();
});

watch(logs, () => void scrollLogToEnd());

function hhmmss(ms: number): string {
  return new Date(ms).toLocaleTimeString();
}
</script>

<template>
  <div class="app-panel">
    <header class="app-header">
      <span class="pi pi-th-large app-icon" />
      <div class="app-title">
        <span class="app-name">{{ appName }}</span>
        <span v-if="manifest.version" class="app-version">v{{ manifest.version }}</span>
      </div>
      <span v-if="api" class="state-chip" :class="{ running: isRunning, starting: isStarting }">
        {{ stateLabel }}
      </span>
    </header>

    <p v-if="manifest.description" class="app-description">{{ manifest.description }}</p>
    <p v-if="manifest.source?.skill || manifest.source?.version" class="app-source">
      {{ t("source") }}: {{ manifest.source?.skill ?? "—" }}
      <template v-if="manifest.source?.version"> (v{{ manifest.source.version }})</template>
    </p>

    <div v-if="!api" class="unsupported">
      <span class="pi pi-ban" />
      {{ unsupportedReason === "repository" ? t("unsupported.repository") : t("unsupported.web") }}
    </div>

    <template v-else>
      <p class="note">{{ t("note") }}</p>

      <div class="actions">
        <button
          class="btn primary"
          :disabled="busy || startPending || isRunning || isStarting"
          @click="onStart"
        >
          <span class="pi pi-play" /> {{ t("start") }}
        </button>
        <button class="btn" :disabled="busy || !(isRunning || isStarting)" @click="onStop">
          <span class="pi pi-stop" /> {{ t("stop") }}
        </button>
        <button class="btn" :disabled="busy || !isRunning" @click="onToggleVisible">
          <span class="pi pi-window-maximize" />
          {{ status?.visible ? t("hideWindow") : t("showWindow") }}
        </button>
        <button class="btn danger" :disabled="busy || !grant" @click="onRevoke">
          <span class="pi pi-times" /> {{ t("revoke") }}
        </button>
      </div>

      <p v-if="isStarting" class="info">{{ t("waitingApproval") }}</p>
      <p v-if="status?.reapprovalPending" class="warn">{{ t("reapprovalPending") }}</p>
      <p v-if="status?.memoryOverCap" class="warn">
        {{ t("memoryOverCap", { bytes: status.memoryBytes ?? 0, max: status.memoryMaxBytes ?? 0 }) }}
      </p>
      <p v-if="isRunning && status && status.reloadCount > 0" class="info">
        {{ t("reloaded", { count: status.reloadCount, time: formatGrantedAt((status.lastReloadAt ?? 0) / 1000) }) }}
      </p>
      <p v-if="errorText" class="error">{{ errorText }}</p>

      <section class="section">
        <h4>{{ t("granted.title") }}</h4>
        <template v-if="grant && permissionView">
          <p class="granted-at">{{ t("granted.at", { time: formatGrantedAt(grant.grantedAt) }) }}</p>
          <ul class="perm-list">
            <li v-for="r in permissionView.rows" :key="r.kind + r.value" :class="r.severity">
              <template v-if="r.kind === 'tool'">{{ t("granted.tool") }}: <code>{{ r.value }}</code></template>
              <template v-else-if="r.kind === 'target'">
                {{ t("granted.target") }}: {{ r.label }}
                <span v-if="r.wildcard" class="tag">{{ t("granted.wildcard") }}</span>
              </template>
              <template v-else-if="r.kind === 'aiTerminal'">
                {{ t("granted.aiTerminal", { spec: r.value }) }}
                <span class="tag">{{ t("granted.aiTerminalTag") }}</span>
              </template>
              <template v-else>{{ t("granted.memory", { bytes: r.value }) }}</template>
            </li>
            <li v-if="permissionView.rows.length === 0" class="none">{{ t("granted.none") }}</li>
            <li v-if="permissionView.ownWorktreeOnly && permissionView.rows.some((r) => r.kind === 'tool')" class="none">
              {{ t("granted.ownOnly") }}
            </li>
          </ul>
        </template>
        <p v-else class="none">{{ t("granted.notYet") }}</p>
      </section>

      <section class="section log-section">
        <h4>{{ t("log.title") }}</h4>
        <div ref="logBox" class="log">
          <div v-if="logs.length === 0" class="none">{{ t("log.empty") }}</div>
          <div v-for="(l, i) in logs" :key="i" class="log-line" :class="l.level">
            <span class="ts">{{ hhmmss(l.ts) }}</span>
            <span class="msg">{{ l.message }}</span>
          </div>
        </div>
      </section>
    </template>
  </div>
</template>

<style scoped>
.app-panel {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 20px 24px;
  overflow-y: auto;
  color: #cdd6f4;
}
.app-header {
  display: flex;
  align-items: center;
  gap: 10px;
}
.app-icon {
  font-size: 20px;
  color: #cba6f7;
}
.app-title {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex: 1;
  min-width: 0;
}
.app-name {
  font-size: 18px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.app-version {
  font-size: 12px;
  color: #6c7086;
}
.state-chip {
  font-size: 11px;
  padding: 2px 10px;
  border-radius: 10px;
  background: #313244;
  color: #a6adc8;
}
.state-chip.running {
  background: #1e3a2a;
  color: #a6e3a1;
}
.state-chip.starting {
  background: #3a331e;
  color: #f9e2af;
}
.app-description,
.app-source {
  margin: 0;
  font-size: 13px;
  color: #a6adc8;
}
.app-source {
  font-size: 12px;
  color: #6c7086;
}
.note {
  margin: 0;
  font-size: 12px;
  color: #6c7086;
}
.unsupported {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 14px;
  border: 1px solid #45475a;
  border-radius: 6px;
  color: #f9e2af;
  font-size: 13px;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 14px;
  border: 1px solid #45475a;
  border-radius: 4px;
  background: #313244;
  color: #cdd6f4;
  font-size: 13px;
  cursor: pointer;
}
.btn:hover:not(:disabled) {
  background: #45475a;
}
.btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.btn.primary {
  background: #89b4fa;
  border-color: #89b4fa;
  color: #1e1e2e;
  font-weight: 600;
}
.btn.primary:hover:not(:disabled) {
  background: #74a8f7;
}
.btn.danger {
  color: #f38ba8;
}
.info {
  margin: 0;
  font-size: 12px;
  color: #f9e2af;
}
.warn {
  margin: 0;
  padding: 8px 12px;
  border-radius: 4px;
  background: #3a331e;
  color: #f9e2af;
  font-size: 12px;
}
.error {
  margin: 0;
  padding: 8px 12px;
  border-radius: 4px;
  background: #3a1e26;
  color: #f38ba8;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-word;
}
.section h4 {
  margin: 0 0 6px;
  font-size: 12px;
  font-weight: 600;
  color: #a6adc8;
}
.granted-at {
  margin: 0 0 4px;
  font-size: 11px;
  color: #6c7086;
}
.perm-list {
  margin: 0;
  padding-left: 18px;
  font-size: 12px;
}
.perm-list li.danger {
  color: #fab387;
}
.perm-list li.critical {
  color: #f38ba8;
}
.perm-list code {
  font-family: monospace;
}
.tag {
  margin-left: 6px;
  padding: 0 6px;
  border-radius: 3px;
  background: #3a1e26;
  color: #f38ba8;
  font-size: 10px;
}
.none {
  color: #6c7086;
  font-size: 12px;
  margin: 0;
}
.log-section {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 140px;
}
.log {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  background: #181825;
  border: 1px solid #313244;
  border-radius: 4px;
  padding: 6px 8px;
  font-family: monospace;
  font-size: 11px;
}
.log-line {
  display: flex;
  gap: 8px;
  line-height: 1.5;
}
.log-line .ts {
  color: #6c7086;
  flex-shrink: 0;
}
.log-line .msg {
  white-space: pre-wrap;
  word-break: break-word;
}
.log-line.warn .msg {
  color: #f9e2af;
}
.log-line.error .msg {
  color: #f38ba8;
}
</style>

<i18n lang="json">
{
  "en": {
    "state": { "stopped": "Stopped", "starting": "Waiting for approval", "running": "Running" },
    "source": "Created from",
    "note": "This manifest was written by an AI. It has no permissions until you approve it in the confirmation dialog shown when you start the app.",
    "start": "Start",
    "stop": "Stop",
    "showWindow": "Show run window",
    "hideWindow": "Hide run window",
    "revoke": "Revoke approval",
    "waitingApproval": "Waiting for you to approve the permissions in the main window…",
    "reapprovalPending": "The manifest's permissions changed and need your approval in the main window. Until then the app keeps running with the old permissions; calls that need the new ones are rejected.",
    "memoryOverCap": "The saved memory ({bytes} bytes) exceeds the approved maximum ({max} bytes). Nothing was deleted. Writes that do not fit are rejected until it fits; reads still work.",
    "reloaded": "Reloaded {count} time(s) after code updates (last: {time}). Approval and memory are kept.",
    "granted": {
      "title": "Approved permissions",
      "at": "Approved {time}",
      "notYet": "Not approved yet. You will be asked when you start the app.",
      "tool": "MCP tool",
      "target": "Other worktrees",
      "wildcard": "includes worktrees created later",
      "memory": "Memory up to {bytes} bytes",
      "aiTerminal": "Dedicated AI terminals ({spec})",
      "aiTerminalTag": "same authority as you",
      "none": "No permissions",
      "ownOnly": "MCP calls reach this worktree only"
    },
    "log": { "title": "Debug log", "empty": "No entries yet" },
    "unsupported": {
      "web": "App artifacts cannot be started from the web viewer. Open it in the oretachi app.",
      "repository": "App artifacts in a repository vault cannot be started. Start it from the worktree it belongs to."
    }
  },
  "ja": {
    "state": { "stopped": "停止中", "starting": "承認待ち", "running": "起動中" },
    "source": "生成元",
    "note": "この manifest は AI が書いたもので、アプリを起動するときの確認ダイアログで許可するまで何の権限も持ちません。",
    "start": "起動",
    "stop": "停止",
    "showWindow": "実行画面を開く",
    "hideWindow": "実行画面を隠す",
    "revoke": "承認を取り消す",
    "waitingApproval": "メインウィンドウで権限の承認を待っています…",
    "reapprovalPending": "manifest の権限が変わり、メインウィンドウでの再承認を待っています。承認までは旧権限のまま動作し、新しい権限が要る呼び出しは拒否されます。",
    "memoryOverCap": "保存済みのメモリー（{bytes} バイト）が承認済みの最大サイズ（{max} バイト）を超えています。データは消していません。収まるまで、収まらない書き込みは拒否されます（読み取りはできます）。",
    "reloaded": "コードの更新で {count} 回リロードしました（最終: {time}）。承認とメモリーは維持されています。",
    "granted": {
      "title": "承認済みの権限",
      "at": "{time} に承認",
      "notYet": "まだ承認されていません。起動するときに確認ダイアログが出ます。",
      "tool": "MCP ツール",
      "target": "他ワークツリー",
      "wildcard": "今後作られるワークツリーも含む",
      "memory": "メモリー 最大 {bytes} バイト",
      "aiTerminal": "専用 AI ターミナル（{spec}）",
      "aiTerminalTag": "あなたと同等の権限",
      "none": "権限なし",
      "ownOnly": "MCP 呼び出しはこのワークツリーだけに届きます"
    },
    "log": { "title": "デバッグログ", "empty": "まだ記録がありません" },
    "unsupported": {
      "web": "アプリ形式のアーティファクトは Web ビューアからは起動できません。oretachi アプリで開いてください。",
      "repository": "リポジトリ保管庫のアプリは起動できません。置かれていたワークツリーから起動してください。"
    }
  }
}
</i18n>
