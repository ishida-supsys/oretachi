<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "vue-i18n";
import type { NotificationHookEntry } from "../types/settings";

const props = defineProps<{
  repoPath: string;
  currentTargets: string[];
  currentPackageManager?: string;
  currentNotificationHooks?: NotificationHookEntry[];
}>();

const emit = defineEmits<{
  confirm: [targets: string[], packageManager: string | undefined, notificationHooks: NotificationHookEntry[]];
  cancel: [];
}>();

const { t } = useI18n();

const entries = ref<string[]>([]);
const selected = ref<Set<string>>(new Set(props.currentTargets));
const loading = ref(true);
const detectedPMs = ref<string[]>([]);
const selectedPM = ref<string | undefined>(props.currentPackageManager);

const ALL_PMS = ["npm", "pnpm", "yarn", "bun"];

const HOOK_EVENTS = ["Stop", "Notification", "SubagentStop", "PreToolUse", "PostToolUse", "PermissionRequest"] as const;
const NOTIFY_KINDS = ["completed", "approval", "general"] as const;
// event → { enabled, kind }
const hookState = ref<Map<string, { enabled: boolean; kind: string }>>(new Map());

// 初期化: 既存設定またはデフォルト推奨設定
{
  const DEFAULTS: Record<string, string> = { Stop: "completed", Notification: "approval", PermissionRequest: "approval" };
  for (const ev of HOOK_EVENTS) {
    const existing = props.currentNotificationHooks?.find((h) => h.event === ev);
    if (existing) {
      hookState.value.set(ev, { enabled: true, kind: existing.kind });
    } else {
      hookState.value.set(ev, { enabled: ev in DEFAULTS, kind: DEFAULTS[ev] ?? "completed" });
    }
  }
}

onMounted(async () => {
  try {
    const [gitignoreResult, detectedResult] = await Promise.all([
      invoke<string[]>("read_gitignore", { repoPath: props.repoPath }),
      invoke<string[]>("detect_package_manager", { repoPath: props.repoPath }),
    ]);
    entries.value = gitignoreResult;
    detectedPMs.value = detectedResult;
    // 未設定かつ検出結果があれば自動選択
    if (selectedPM.value === undefined && detectedResult.length > 0) {
      selectedPM.value = detectedResult[0];
    }
  } catch (e) {
    console.warn("PostAddSettingsDialog init failed:", e);
  } finally {
    loading.value = false;
  }
});

function toggle(entry: string) {
  if (selected.value.has(entry)) {
    selected.value.delete(entry);
  } else {
    selected.value.add(entry);
  }
}

function selectAll() {
  selected.value = new Set(entries.value);
}

function deselectAll() {
  selected.value = new Set();
}

function onConfirm() {
  const hooks: NotificationHookEntry[] = [];
  for (const ev of HOOK_EVENTS) {
    const state = hookState.value.get(ev);
    if (state?.enabled) {
      hooks.push({ event: ev as NotificationHookEntry["event"], kind: state.kind as NotificationHookEntry["kind"] });
    }
  }
  emit("confirm", Array.from(selected.value), selectedPM.value || undefined, hooks);
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t("title") }}</h3>

      <!-- パッケージマネージャーセクション -->
      <div class="section">
        <div class="section-header">{{ t("pkgManager.sectionLabel") }}</div>
        <div class="pm-row">
          <span class="pm-hint" v-if="detectedPMs.length > 0">
            {{ t("pkgManager.detected", { pms: detectedPMs.join(", ") }) }}
          </span>
          <span class="pm-hint" v-else-if="!loading">{{ t("pkgManager.notDetected") }}</span>
          <select
            class="pm-select"
            :value="selectedPM ?? ''"
            @change="selectedPM = ($event.target as HTMLSelectElement).value || undefined"
          >
            <option value="">{{ t("pkgManager.none") }}</option>
            <option
              v-for="pm in ALL_PMS"
              :key="pm"
              :value="pm"
            >{{ pm }} install{{ detectedPMs.includes(pm) ? " ✓" : "" }}</option>
          </select>
        </div>
      </div>

      <!-- Claude Code通知フックセクション -->
      <div class="section">
        <div class="section-header">{{ t("hooks.sectionLabel") }}</div>
        <p class="section-description">{{ t("hooks.description") }}</p>
        <div class="hook-list">
          <div
            v-for="ev in HOOK_EVENTS"
            :key="ev"
            class="hook-row"
          >
            <label class="hook-checkbox-label">
              <input
                type="checkbox"
                :checked="hookState.get(ev)?.enabled"
                @change="hookState.get(ev)!.enabled = ($event.target as HTMLInputElement).checked"
              />
              <span class="hook-event-name">{{ ev }}</span>
            </label>
            <select
              class="hook-kind-select"
              :disabled="!hookState.get(ev)?.enabled"
              :value="hookState.get(ev)?.kind"
              @change="hookState.get(ev)!.kind = ($event.target as HTMLSelectElement).value"
            >
              <option v-for="kind in NOTIFY_KINDS" :key="kind" :value="kind">{{ kind }}</option>
            </select>
          </div>
        </div>
      </div>

      <!-- gitignore コピーセクション -->
      <div class="section">
        <div class="section-header">{{ t("copy.sectionLabel") }}</div>
        <p class="section-description">{{ t("copy.description") }}</p>

        <div class="list-header">
          <button class="btn-text" @click="selectAll">{{ t("selectAll") }}</button>
          <button class="btn-text" @click="deselectAll">{{ t("deselectAll") }}</button>
        </div>

        <div class="entry-list">
          <div v-if="loading" class="empty-state">{{ t("loading") }}</div>
          <div v-else-if="entries.length === 0" class="empty-state">{{ t("empty") }}</div>
          <label
            v-for="entry in entries"
            :key="entry"
            class="checkbox-label"
          >
            <input
              type="checkbox"
              :checked="selected.has(entry)"
              @change="toggle(entry)"
            />
            <span class="entry-text">{{ entry }}</span>
          </label>
        </div>
      </div>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t("cancel") }}</button>
        <button class="btn-confirm" @click="onConfirm">{{ t("confirm") }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  z-index: 100;
  display: flex;
  align-items: center;
  justify-content: center;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 10px;
  padding: 24px;
  width: 440px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  max-height: 80vh;
  overflow-y: auto;
}

.dialog-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
}

.section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.section-header {
  font-size: 11px;
  font-weight: 600;
  color: #6c7086;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  padding-bottom: 6px;
  border-bottom: 1px solid #313244;
}

.section-description {
  margin: 0;
  font-size: 12px;
  color: #6c7086;
  line-height: 1.5;
}

.pm-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.pm-hint {
  font-size: 11px;
  color: #a6adc8;
  white-space: nowrap;
}

.pm-select {
  flex: 1;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 5px 8px;
  font-size: 12px;
  color: #cdd6f4;
  outline: none;
  cursor: pointer;
}

.pm-select:focus {
  border-color: #cba6f7;
}

.list-header {
  display: flex;
  gap: 8px;
}

.btn-text {
  background: transparent;
  border: none;
  color: #89b4fa;
  font-size: 12px;
  cursor: pointer;
  padding: 0;
}

.btn-text:hover {
  text-decoration: underline;
}

.entry-list {
  max-height: 220px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 8px;
  background: #181825;
}

.empty-state {
  color: #6c7086;
  font-size: 12px;
  text-align: center;
  padding: 16px;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  user-select: none;
  padding: 4px 6px;
  border-radius: 4px;
}

.checkbox-label:hover {
  background: #313244;
}

.checkbox-label input[type="checkbox"] {
  accent-color: #cba6f7;
  cursor: pointer;
}

.entry-text {
  font-size: 12px;
  color: #cdd6f4;
  font-family: monospace;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.btn-cancel {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 16px;
  font-size: 13px;
  cursor: pointer;
}

.btn-cancel:hover {
  background: #45475a;
}

.btn-confirm {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 6px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-confirm:hover {
  background: #b4befe;
}

.hook-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.hook-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.hook-checkbox-label {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  user-select: none;
  flex: 1;
}

.hook-checkbox-label input[type="checkbox"] {
  accent-color: #cba6f7;
  cursor: pointer;
}

.hook-event-name {
  font-size: 12px;
  color: #cdd6f4;
  font-family: monospace;
}

.hook-kind-select {
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 3px 6px;
  font-size: 12px;
  color: #cdd6f4;
  outline: none;
  cursor: pointer;
}

.hook-kind-select:focus {
  border-color: #cba6f7;
}

.hook-kind-select:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Post-add settings",
    "pkgManager": {
      "sectionLabel": "Package install",
      "detected": "Detected: {pms}",
      "notDetected": "No lock file found",
      "none": "None"
    },
    "hooks": {
      "sectionLabel": "Claude Code notification hooks",
      "description": "Configure which hooks trigger notifications via oretachi. Enabled hooks will write to .claude/settings.local.json on worktree creation."
    },
    "copy": {
      "sectionLabel": "Gitignore copy",
      "description": "Select files/directories from .gitignore to copy from the root repository to new worktrees."
    },
    "selectAll": "Select all",
    "deselectAll": "Deselect all",
    "loading": "Loading...",
    "empty": "No .gitignore found or no entries",
    "confirm": "Save",
    "cancel": "Cancel"
  },
  "ja": {
    "title": "追加後設定",
    "pkgManager": {
      "sectionLabel": "パッケージインストール",
      "detected": "検出: {pms}",
      "notDetected": "ロックファイルが見つかりません",
      "none": "なし"
    },
    "hooks": {
      "sectionLabel": "Claude Code 通知フック",
      "description": "oretachi経由で通知するフックを設定します。有効なフックはワークツリー追加時に .claude/settings.local.json に書き込まれます。"
    },
    "copy": {
      "sectionLabel": "gitignore コピー",
      "description": ".gitignoreのファイル/ディレクトリをルートリポジトリから新しいワークツリーにコピーする対象を選択します。"
    },
    "selectAll": "全選択",
    "deselectAll": "全解除",
    "loading": "読み込み中...",
    "empty": ".gitignoreが見つからないかエントリがありません",
    "confirm": "保存",
    "cancel": "キャンセル"
  }
}
</i18n>
