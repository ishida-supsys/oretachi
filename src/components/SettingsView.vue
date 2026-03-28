<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { open, ask, message } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useSettings } from "../composables/useSettings";
import { useUpdater } from "../composables/useUpdater";
import ColorPicker from "primevue/colorpicker";
import type { AiAgentKind } from "../types/settings";
import { useI18n } from "vue-i18n";
import { setLocale } from "../i18n";
import { playNotificationSound } from "../utils/notificationSound";
import type { NotificationKind } from "../composables/useNotifications";
import SettingsHotkeySection from "./settings/SettingsHotkeySection.vue";
import SettingsRepositoriesSection from "./settings/SettingsRepositoriesSection.vue";

const { t } = useI18n();

const { settings, scheduleSave, flushSave } = useSettings();

const { isChecking, checkForUpdate, downloadAndInstall } = useUpdater();

const appVersion = ref("");

async function onCheckUpdate() {
  const update = await checkForUpdate();
  if (update) {
    const yes = await ask(
      t("update.available", { version: update.version }),
      { title: t("update.title"), kind: "info" }
    );
    if (yes) await downloadAndInstall(update);
  } else {
    await message(t("about.upToDate"), { title: t("update.title"), kind: "info" });
  }
}

// ─── AIエージェント ───────────────────────────────────────────────────────────

interface AiAgentInfo {
  kind: AiAgentKind;
  name: string;
  command: string;
}

const AI_AGENT_LABELS: Record<AiAgentKind, string> = {
  claudeCode: "Claude Code",
  geminiCli: "Gemini CLI",
  codexCli: "Codex CLI",
  clineCli: "Cline CLI",
};

const ALL_AGENT_KINDS: AiAgentKind[] = ["claudeCode", "geminiCli", "codexCli", "clineCli"];

const detectedAgents = ref<AiAgentInfo[]>([]);

onMounted(async () => {
  try {
    detectedAgents.value = await invoke<AiAgentInfo[]>("detect_ai_agents");
  } catch (e) {
    console.error("detect_ai_agents failed:", e);
  }
  appVersion.value = await getVersion();
});

function isAgentDetected(kind: AiAgentKind): boolean {
  return detectedAgents.value.some((a) => a.kind === kind);
}

// ─── MCP サーバー ─────────────────────────────────────────────────────────────

interface McpStatus {
  running: boolean;
  port: number | null;
}

const mcpStatus = ref<McpStatus | null>(null);
const restarting = ref(false);

async function fetchMcpStatus() {
  try {
    mcpStatus.value = await invoke<McpStatus>("get_mcp_status");
  } catch (e) {
    console.error("get_mcp_status failed:", e);
  }
}

async function restartMcp() {
  restarting.value = true;
  try {
    await flushSave();
    mcpStatus.value = await invoke<McpStatus>("restart_mcp_server");
  } catch (e) {
    console.error("restart_mcp_server failed:", e);
    await fetchMcpStatus();
  } finally {
    restarting.value = false;
  }
}

const regenerating = ref(false);

async function regenerateApiKey() {
  const yes = await ask(t('mcp.regenerateConfirm'), { title: t('mcp.label'), kind: 'warning' });
  if (!yes) return;
  regenerating.value = true;
  try {
    const newKey = await invoke<string>('regenerate_mcp_api_key');
    settings.value.mcpApiKey = newKey;
    mcpStatus.value = await invoke<McpStatus>('restart_mcp_server');
  } catch (e) {
    console.error('regenerate_mcp_api_key failed:', e);
  } finally {
    regenerating.value = false;
  }
}

async function copyApiKey() {
  if (settings.value.mcpApiKey) {
    await writeText(settings.value.mcpApiKey);
  }
}

onMounted(fetchMcpStatus);

async function selectWorktreeBaseDir() {
  const selected = await open({ directory: true, multiple: false });
  if (typeof selected === "string") {
    settings.value.worktreeBaseDir = selected;
    scheduleSave();
  }
}

// ─── 外観 ─────────────────────────────────────────────────────────────────────

const gamingBorderThemes = [
  { value: "gaming",     labelKey: "appearance.gamingBorderThemes.gaming" },
  { value: "snow-white", labelKey: "appearance.gamingBorderThemes.snowWhite" },
  { value: "gold",       labelKey: "appearance.gamingBorderThemes.gold" },
  { value: "flame",      labelKey: "appearance.gamingBorderThemes.flame" },
  { value: "aqua",       labelKey: "appearance.gamingBorderThemes.aqua" },
  { value: "dark",       labelKey: "appearance.gamingBorderThemes.dark" },
  { value: "nature",     labelKey: "appearance.gamingBorderThemes.nature" },
];

function applyAcrylicEffect() {
  const app = settings.value.appearance;
  const hex = (app?.acrylicColor ?? "#121212").replace("#", "");
  const r = parseInt(hex.substring(0, 2), 16) || 18;
  const g = parseInt(hex.substring(2, 4), 16) || 18;
  const b = parseInt(hex.substring(4, 6), 16) || 18;
  const a = app?.acrylicOpacity ?? 125;
  invoke("apply_acrylic_effect", { r, g, b, a }).catch(() => {});
}

// ─── 通知音 ───────────────────────────────────────────────────────────────────

const systemSounds = ref<string[]>([]);

onMounted(async () => {
  try {
    systemSounds.value = await invoke<string[]>("list_system_sounds");
  } catch (e) {
    console.error("list_system_sounds failed:", e);
  }
});

function ensureNotificationSound() {
  if (!settings.value.notificationSound) {
    settings.value.notificationSound = { volume: 80 };
  }
}

function getSoundForKind(kind: NotificationKind): string {
  return settings.value.notificationSound?.[kind] ?? "";
}

async function setSoundForKind(kind: NotificationKind, value: string) {
  ensureNotificationSound();
  if (value === "__pick_custom__") {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Audio", extensions: ["wav", "ogg", "mp3"] }],
    });
    if (typeof selected !== "string") {
      // キャンセルされた場合は元の値のままにする（リセットしない）
      return;
    }
    try {
      const filename = await invoke<string>("copy_custom_sound", { sourcePath: selected });
      settings.value.notificationSound![kind] = `custom:${filename}`;
    } catch (e) {
      console.error("copy_custom_sound failed:", e);
      return;
    }
  } else {
    settings.value.notificationSound![kind] = value || null;
  }
  scheduleSave();
}

function getVolumeForSound(): number {
  return settings.value.notificationSound?.volume ?? 80;
}

function setVolume(value: number) {
  ensureNotificationSound();
  settings.value.notificationSound!.volume = value;
  scheduleSave();
}

async function previewSound(kind: NotificationKind) {
  const sound = getSoundForKind(kind);
  if (!sound) return;
  const volume = getVolumeForSound();
  await playNotificationSound(sound, volume).catch(() => {});
}

function getSoundLabel(sound: string | null | undefined): string {
  if (!sound) return "";
  if (sound.startsWith("custom:")) return sound.slice("custom:".length);
  return sound;
}
</script>

<template>
  <div class="settings-view">
    <h2 class="section-title">{{ t('title') }}</h2>

    <!-- Language / 言語 -->
    <div class="field-group">
      <label class="field-label">Language / 言語</label>
      <div class="row-input row-input--inline">
        <select
          class="text-input select-input"
          :value="settings.locale ?? 'en'"
          @change="(e) => {
            const v = (e.target as HTMLSelectElement).value;
            settings.locale = v;
            setLocale(v as 'en' | 'ja');
            scheduleSave();
          }"
        >
          <option value="en">English</option>
          <option value="ja">日本語</option>
        </select>
      </div>
    </div>

    <!-- MCP サーバー -->
    <div class="field-group">
      <label class="field-label">{{ t('mcp.label') }}</label>
      <div class="mcp-row">
        <span
          class="mcp-badge"
          :class="mcpStatus?.running ? 'badge--running' : 'badge--stopped'"
        >
          {{ mcpStatus === null ? t('mcp.loading') : mcpStatus.running ? t('mcp.running') : t('mcp.stopped') }}
        </span>
        <span v-if="mcpStatus?.running && mcpStatus?.port" class="mcp-port">
          {{ t('mcp.port', { port: mcpStatus.port }) }}
        </span>
        <button class="btn-secondary" :disabled="restarting" @click="restartMcp">
          {{ restarting ? t('mcp.restarting') : t('mcp.restart') }}
        </button>
      </div>
      <div class="row-input row-input--inline">
        <label for="mcp-port" class="inline-label">{{ t('mcp.fixedPort') }}</label>
        <input
          id="mcp-port"
          type="number"
          class="number-input"
          :value="settings.mcpPort ?? 0"
          min="0"
          max="65535"
          @change="(e) => {
            const v = parseInt((e.target as HTMLInputElement).value) || 0;
            settings.mcpPort = Math.max(0, Math.min(65535, v));
            scheduleSave();
          }"
        />
        <span class="unit-label">{{ t('mcp.fixedPortHint') }}</span>
      </div>
      <div v-if="settings.mcpApiKey" class="row-input row-input--inline mcp-api-key-row">
        <label class="inline-label">{{ t('mcp.apiKey') }}</label>
        <code class="api-key-display">{{ settings.mcpApiKey }}</code>
        <button class="btn-secondary btn-small" @click="copyApiKey">{{ t('mcp.copy') }}</button>
        <button class="btn-secondary btn-small" :disabled="regenerating" @click="regenerateApiKey">
          {{ regenerating ? t('mcp.regenerating') : t('mcp.regenerate') }}
        </button>
      </div>
    </div>

    <!-- ウィンドウ設定 -->
    <div class="field-group">
      <label class="field-label">{{ t('window.label') }}</label>
      <div class="row-input row-input--inline">
        <input
          id="always-on-top"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.alwaysOnTop"
          @change="(e) => { settings.alwaysOnTop = (e.target as HTMLInputElement).checked; scheduleSave(); }"
        />
        <label for="always-on-top" class="inline-label toggle-label">{{ t('window.alwaysOnTop') }}</label>
      </div>
      <div class="row-input row-input--inline mt-8">
        <input
          id="enable-os-notification"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.enableOsNotification"
          @change="(e) => { settings.enableOsNotification = (e.target as HTMLInputElement).checked; scheduleSave(); }"
        />
        <label for="enable-os-notification" class="inline-label toggle-label">{{ t('window.osNotification') }}</label>
      </div>
      <div class="row-input row-input--inline mt-8">
        <input
          id="focus-main-on-empty-tray"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.focusMainOnEmptyTray"
          @change="(e) => { settings.focusMainOnEmptyTray = (e.target as HTMLInputElement).checked; scheduleSave(); }"
        />
        <label for="focus-main-on-empty-tray" class="inline-label toggle-label">{{ t('window.focusMainOnEmptyTray') }}</label>
      </div>
    </div>

    <!-- 通知音 -->
    <div class="field-group">
      <label class="field-label">{{ t('notificationSound.label') }}</label>
      <div class="row-input mt-8">
        <span class="inline-label">{{ t('notificationSound.volume') }}</span>
        <input
          type="range"
          min="0"
          max="100"
          class="sound-volume-slider"
          :value="getVolumeForSound()"
          @change="(e) => setVolume(Number((e.target as HTMLInputElement).value))"
        />
        <span class="volume-label">{{ getVolumeForSound() }}</span>
      </div>
      <div v-for="kind in (['approval', 'completed', 'general'] as const)" :key="kind" class="row-input mt-8 sound-row">
        <span class="inline-label sound-kind-label">{{ t(`notificationSound.kind.${kind}`) }}</span>
        <select
          class="text-input select-input sound-select"
          :value="getSoundForKind(kind)"
          @change="(e) => setSoundForKind(kind, (e.target as HTMLSelectElement).value)"
        >
          <option value="">{{ t('notificationSound.none') }}</option>
          <optgroup :label="t('notificationSound.systemSounds')" v-if="systemSounds.length > 0">
            <option
              v-for="s in systemSounds"
              :key="s"
              :value="`system:${s}`"
            >{{ s }}</option>
          </optgroup>
          <option value="__pick_custom__">{{ t('notificationSound.pickCustom') }}</option>
          <option
            v-if="getSoundForKind(kind).startsWith('custom:')"
            :value="getSoundForKind(kind)"
          >{{ getSoundLabel(getSoundForKind(kind)) }}</option>
        </select>
        <button
          class="preview-btn"
          :disabled="!getSoundForKind(kind)"
          @click="previewSound(kind)"
          :title="t('notificationSound.preview')"
        >▶</button>
      </div>
    </div>

    <!-- 自動承認 -->
    <div class="field-group">
      <label class="field-label">{{ t('autoApproval.label') }}</label>
      <div class="row-input row-input--inline">
        <span class="inline-label">{{ t('autoApproval.approvalAgent') }}</span>
        <select
          class="text-input select-input"
          :value="settings.aiAgent?.approvalAgent ?? ''"
          @change="(e) => {
            const v = (e.target as HTMLSelectElement).value;
            if (!settings.aiAgent) settings.aiAgent = {};
            settings.aiAgent.approvalAgent = v ? (v as AiAgentKind) : undefined;
            scheduleSave();
          }"
        >
          <option value="">{{ t('autoApproval.notSet') }}</option>
          <option
            v-for="kind in ALL_AGENT_KINDS"
            :key="kind"
            :value="kind"
            :disabled="!isAgentDetected(kind)"
          >{{ AI_AGENT_LABELS[kind] }}{{ !isAgentDetected(kind) ? t('autoApproval.notDetected') : '' }}</option>
        </select>
      </div>
      <div class="row-input row-input--inline mt-8">
        <span class="inline-label">{{ t('autoApproval.taskAddAgent') }}</span>
        <select
          class="text-input select-input"
          :value="settings.aiAgent?.taskAddAgent ?? ''"
          @change="(e) => {
            const v = (e.target as HTMLSelectElement).value;
            if (!settings.aiAgent) settings.aiAgent = {};
            settings.aiAgent.taskAddAgent = v ? (v as AiAgentKind) : undefined;
            scheduleSave();
          }"
        >
          <option value="">{{ t('autoApproval.notSet') }}</option>
          <option
            v-for="kind in ALL_AGENT_KINDS"
            :key="kind"
            :value="kind"
            :disabled="!isAgentDetected(kind)"
          >{{ AI_AGENT_LABELS[kind] }}{{ !isAgentDetected(kind) ? t('autoApproval.notDetected') : '' }}</option>
        </select>
      </div>
      <div class="row-input row-input--inline mt-8">
        <span class="inline-label">{{ t('autoApproval.aiTimeout') }}</span>
        <input
          class="text-input number-input"
          type="number"
          :value="settings.aiTimeoutSecs ?? 120"
          min="10"
          max="600"
          @change="(e) => {
            settings.aiTimeoutSecs = Number((e.target as HTMLInputElement).value) || 120;
            scheduleSave();
          }"
        />
        <span class="unit-label">{{ t('autoApproval.seconds') }}</span>
      </div>
    </div>

    <!-- ターミナル設定 -->
    <div class="field-group">
      <label class="field-label">{{ t('terminal.label') }}</label>
      <div class="row-input row-input--inline">
        <span class="inline-label">{{ t('terminal.fontSize') }}</span>
        <input
          class="text-input number-input"
          type="number"
          :value="settings.terminal.fontSize"
          min="8"
          max="32"
          @change="(e) => { settings.terminal.fontSize = Number((e.target as HTMLInputElement).value); scheduleSave(); }"
        />
        <span class="unit-label">px</span>
      </div>
      <div class="row-input row-input--inline mt-8">
        <span class="inline-label">{{ t('terminal.defaultShell') }}</span>
        <input
          class="text-input shell-input"
          :value="settings.terminal.shell ?? ''"
          :placeholder="t('terminal.shellPlaceholder')"
          @change="(e) => { const v = (e.target as HTMLInputElement).value.trim(); settings.terminal.shell = v || undefined; scheduleSave(); }"
        />
      </div>
    </div>

    <!-- ワークツリー追加先ディレクトリ -->
    <div class="field-group">
      <label class="field-label">{{ t('worktreeBaseDir.label') }}</label>
      <div class="row-input">
        <input
          class="text-input"
          :value="settings.worktreeBaseDir"
          readonly
          :placeholder="t('common.notConfigured')"
        />
        <button class="btn-secondary" @click="selectWorktreeBaseDir">{{ t('worktreeBaseDir.select') }}</button>
      </div>
    </div>

    <!-- ワークツリー追加時のデフォルト動作 -->
    <div class="field-group">
      <label class="field-label">{{ t('worktreeDefaults.label') }}</label>
      <div class="row-input row-input--inline">
        <input
          id="worktree-default-subwindow"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.worktreeDefaults?.openInSubWindow"
          @change="(e) => {
            if (!settings.worktreeDefaults) settings.worktreeDefaults = {};
            settings.worktreeDefaults.openInSubWindow = (e.target as HTMLInputElement).checked;
            scheduleSave();
          }"
        />
        <label for="worktree-default-subwindow" class="inline-label toggle-label">{{ t('worktreeDefaults.openInSubWindow') }}</label>
      </div>
      <div class="row-input row-input--inline mt-8">
        <input
          id="worktree-default-auto-approval"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.worktreeDefaults?.autoApproval"
          @change="(e) => {
            if (!settings.worktreeDefaults) settings.worktreeDefaults = {};
            settings.worktreeDefaults.autoApproval = (e.target as HTMLInputElement).checked;
            scheduleSave();
          }"
        />
        <label for="worktree-default-auto-approval" class="inline-label toggle-label">{{ t('worktreeDefaults.enableAutoApproval') }}</label>
      </div>
      <div class="row-input row-input--inline mt-8">
        <input
          id="worktree-default-auto-open-artifact"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.worktreeDefaults?.autoOpenArtifact ?? true"
          @change="(e) => {
            if (!settings.worktreeDefaults) settings.worktreeDefaults = {};
            settings.worktreeDefaults.autoOpenArtifact = (e.target as HTMLInputElement).checked;
            scheduleSave();
          }"
        />
        <label for="worktree-default-auto-open-artifact" class="inline-label toggle-label">{{ t('worktreeDefaults.autoOpenArtifact') }}</label>
      </div>
    </div>

    <!-- ホットキー設定 -->
    <SettingsHotkeySection />

    <!-- リポジトリ一覧 -->
    <SettingsRepositoriesSection />

    <!-- 外観設定 -->
    <div class="field-group">
      <label class="field-label">{{ t('appearance.label') }}</label>
      <div class="row-input row-input--inline">
        <input
          id="appearance-enable-acrylic"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.appearance?.enableAcrylic ?? true"
          @change="(e) => {
            if (!settings.appearance) settings.appearance = {};
            settings.appearance.enableAcrylic = (e.target as HTMLInputElement).checked;
            scheduleSave();
          }"
        />
        <label for="appearance-enable-acrylic" class="inline-label toggle-label">{{ t('appearance.enableAcrylic') }}</label>
      </div>
      <p class="appearance-note">{{ t('appearance.restartNote') }}</p>
      <div v-if="settings.appearance?.enableAcrylic ?? true" class="appearance-acrylic-controls">
        <div class="row-input row-input--inline">
          <label class="inline-label">{{ t('appearance.opacity') }}</label>
          <input
            type="range"
            min="0"
            max="255"
            class="acrylic-opacity-slider"
            :value="settings.appearance?.acrylicOpacity ?? 125"
            @input="(e) => {
              if (!settings.appearance) settings.appearance = {};
              settings.appearance.acrylicOpacity = Number((e.target as HTMLInputElement).value);
              scheduleSave();
              applyAcrylicEffect();
            }"
          />
          <span class="acrylic-opacity-value">{{ settings.appearance?.acrylicOpacity ?? 125 }}</span>
        </div>
        <div class="row-input row-input--inline">
          <label class="inline-label">{{ t('appearance.color') }}</label>
          <ColorPicker
            :model-value="(settings.appearance?.acrylicColor ?? '#121212').replace('#', '')"
            format="hex"
            @update:model-value="(val: string) => {
              if (!settings.appearance) settings.appearance = {};
              settings.appearance.acrylicColor = '#' + val;
              scheduleSave();
              applyAcrylicEffect();
            }"
          />
        </div>
      </div>
      <div class="row-input row-input--inline mt-8">
        <input
          id="appearance-enable-gaming-border"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.appearance?.enableGamingBorder ?? false"
          @change="(e) => {
            if (!settings.appearance) settings.appearance = {};
            settings.appearance.enableGamingBorder = (e.target as HTMLInputElement).checked;
            scheduleSave();
          }"
        />
        <label for="appearance-enable-gaming-border" class="inline-label toggle-label">{{ t('appearance.enableGamingBorder') }}</label>
      </div>
      <div v-if="settings.appearance?.enableGamingBorder" class="row-input mt-8">
        <label class="row-label">{{ t('appearance.gamingBorderTheme') }}</label>
        <select
          class="text-input select-input"
          :value="settings.appearance?.gamingBorderTheme ?? 'gaming'"
          @change="(e) => {
            if (!settings.appearance) settings.appearance = {};
            settings.appearance.gamingBorderTheme = (e.target as HTMLSelectElement).value;
            scheduleSave();
          }"
        >
          <option v-for="theme in gamingBorderThemes" :key="theme.value" :value="theme.value">{{ t(theme.labelKey) }}</option>
        </select>
      </div>
    </div>

    <!-- ターミナル猫 -->
    <div class="field-group">
      <label class="field-label">{{ t('homeCat.label') }}</label>
      <div class="row-input row-input--inline">
        <input
          id="enableHomeCat"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.enableHomeCat === true"
          @change="(e) => { settings.enableHomeCat = (e.target as HTMLInputElement).checked; scheduleSave(); }"
        />
        <label for="enableHomeCat" class="inline-label toggle-label">{{ t('homeCat.enable') }}</label>
      </div>
    </div>

    <!-- バージョン情報 -->
    <div class="field-group">
      <label class="field-label">{{ t('about.label') }}</label>
      <div class="row-input">
        <span class="version-text">v{{ appVersion }}</span>
        <button class="btn-secondary" :disabled="isChecking" @click="onCheckUpdate">
          {{ isChecking ? t('about.checking') : t('about.checkUpdate') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-view {
  width: 100%;
  height: 100%;
  overflow-y: auto;
  padding: 24px;
  background: transparent;
  box-sizing: border-box;
  color: #cdd6f4;
}

.section-title {
  font-size: 18px;
  font-weight: 600;
  margin: 0 0 24px;
  color: #cba6f7;
}

.field-group {
  margin-bottom: 24px;
}

.field-label {
  display: block;
  font-size: 13px;
  color: #a6adc8;
  margin-bottom: 8px;
}

.field-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.row-input {
  display: flex;
  gap: 8px;
}

.text-input {
  flex: 1;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 10px;
  font-size: 13px;
  color: #cdd6f4;
  outline: none;
}

.btn-primary {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  cursor: pointer;
  font-weight: 600;
  white-space: nowrap;
}

.btn-primary:hover {
  background: #b4befe;
}

.btn-secondary {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.btn-secondary:hover {
  background: #45475a;
}

.repo-list {
  border: 1px solid #313244;
  border-radius: 6px;
  overflow: hidden;
}

.repo-item {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border-bottom: 1px solid #313244;
  background: #181825;
}

.repo-row-main {
  display: flex;
  align-items: center;
  gap: 12px;
}

.repo-row-script {
  display: flex;
  align-items: center;
  gap: 8px;
}

.script-label {
  font-size: 11px;
  color: #6c7086;
  white-space: nowrap;
  min-width: 80px;
}

.script-input {
  font-size: 12px;
}

.repo-item:last-child {
  border-bottom: none;
}

.repo-name {
  font-size: 13px;
  font-weight: 600;
  color: #cdd6f4;
  min-width: 120px;
}

.repo-path {
  font-size: 12px;
  color: #6c7086;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.btn-remove {
  background: transparent;
  color: #6c7086;
  border: none;
  font-size: 16px;
  cursor: pointer;
  padding: 0 4px;
  line-height: 1;
}

.btn-remove:hover {
  color: #f38ba8;
}

.btn-remove:disabled {
  color: #313244;
  cursor: not-allowed;
}

.empty-state {
  padding: 16px;
  text-align: center;
  color: #6c7086;
  font-size: 13px;
  background: #181825;
}

.row-input--inline {
  align-items: center;
}

.inline-label {
  font-size: 13px;
  color: #a6adc8;
  white-space: nowrap;
}

.number-input {
  flex: unset;
  width: 72px;
  text-align: right;
}

.unit-label {
  font-size: 13px;
  color: #6c7086;
}

.mcp-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.mcp-badge {
  font-size: 12px;
  font-weight: 600;
  padding: 3px 10px;
  border-radius: 10px;
  white-space: nowrap;
}

.badge--running {
  background: rgba(166, 227, 161, 0.15);
  color: #a6e3a1;
  border: 1px solid rgba(166, 227, 161, 0.4);
}

.badge--stopped {
  background: rgba(243, 139, 168, 0.15);
  color: #f38ba8;
  border: 1px solid rgba(243, 139, 168, 0.4);
}

.mcp-port {
  font-size: 12px;
  color: #a6adc8;
  font-family: monospace;
}

.mcp-api-key-row {
  margin-top: 8px;
  flex-wrap: wrap;
  gap: 8px;
}

.api-key-display {
  font-family: monospace;
  font-size: 11px;
  background: #1e1e2e;
  padding: 4px 8px;
  border-radius: 4px;
  color: #a6adc8;
  user-select: all;
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.btn-small {
  font-size: 12px;
  padding: 3px 10px;
}

.hotkey-table {
  border-collapse: collapse;
  width: 100%;
}

.hotkey-th {
  text-align: left;
  font-size: 12px;
  color: #6c7086;
  font-weight: 500;
  padding: 4px 8px 8px;
  border-bottom: 1px solid #313244;
}

.hotkey-td-label {
  font-size: 13px;
  color: #a6adc8;
  white-space: nowrap;
  padding: 6px 8px;
}

.hotkey-td-input {
  padding: 4px 8px;
}

.toggle-checkbox {
  width: 16px;
  height: 16px;
  accent-color: #cba6f7;
  cursor: pointer;
  flex: unset;
}

.toggle-label {
  cursor: pointer;
}

.mt-8 {
  margin-top: 8px;
}

.shell-input {
  font-size: 12px;
  font-family: monospace;
}

.select-input {
  flex: unset;
  min-width: 180px;
  cursor: pointer;
}

.appearance-note {
  margin: 6px 0 0;
  font-size: 11px;
  color: #6c7086;
}

.appearance-acrylic-controls {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.acrylic-opacity-slider {
  flex: 1;
  min-width: 80px;
  accent-color: #89b4fa;
}

.acrylic-opacity-value {
  min-width: 28px;
  text-align: right;
  font-size: 12px;
  color: #a6adc8;
}

.sound-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.sound-kind-label {
  min-width: 90px;
  flex-shrink: 0;
}

.sound-select {
  flex: 1;
  min-width: 0;
}

.sound-volume-slider {
  flex: 1;
  min-width: 80px;
  accent-color: #89b4fa;
}

.volume-label {
  min-width: 28px;
  text-align: right;
  font-size: 12px;
  color: #a6adc8;
}

.preview-btn {
  background: transparent;
  border: 1px solid #45475a;
  border-radius: 4px;
  color: #a6adc8;
  cursor: pointer;
  padding: 2px 6px;
  font-size: 11px;
  flex-shrink: 0;
}

.preview-btn:hover:not(:disabled) {
  background: #313244;
  color: #cdd6f4;
}

.preview-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Settings",
    "mcp": {
      "label": "MCP Server",
      "loading": "Loading...",
      "running": "Running",
      "stopped": "Stopped",
      "port": "Port: {port}",
      "restart": "Restart",
      "restarting": "Restarting...",
      "fixedPort": "Fixed port",
      "fixedPortHint": "0 = auto assign. Changes take effect after restart.",
      "apiKey": "API Key",
      "copy": "Copy",
      "regenerate": "Regenerate",
      "regenerating": "Regenerating...",
      "regenerateConfirm": "Regenerating the API key will disconnect all MCP clients. Continue?"
    },
    "window": {
      "label": "Window",
      "alwaysOnTop": "Always on top",
      "osNotification": "Show OS desktop notification on notify",
      "focusMainOnEmptyTray": "Focus main window with tray hotkey when no notifications"
    },
    "autoApproval": {
      "label": "Auto Approval",
      "approvalAgent": "AI Agent (Auto Approval, Commit Msg, Task Gen)",
      "taskAddAgent": "AI Agent (Task Execution)",
      "notSet": "(Not set)",
      "notDetected": " (Not detected)",
      "aiTimeout": "AI Timeout",
      "seconds": "sec"
    },
    "terminal": {
      "label": "Terminal",
      "fontSize": "Font size",
      "defaultShell": "Default shell",
      "shellPlaceholder": "Empty = system default"
    },
    "worktreeBaseDir": {
      "label": "Worktree base directory",
      "select": "Select"
    },
    "worktreeDefaults": {
      "label": "Default worktree behavior",
      "openInSubWindow": "Open in sub window",
      "enableAutoApproval": "Enable auto approval",
      "autoOpenArtifact": "Auto-open artifact viewer on new artifact"
    },
    "appearance": {
      "label": "Appearance",
      "enableAcrylic": "Enable Acrylic / LiquidGlass effect",
      "enableGamingBorder": "Enable outline effect",
      "gamingBorderTheme": "Theme color",
      "gamingBorderThemes": {
        "gaming": "Gaming",
        "snowWhite": "Snow White",
        "gold": "Gold",
        "flame": "Flame",
        "aqua": "Aqua",
        "dark": "Dark",
        "nature": "Nature"
      },
      "restartNote": "Enabling/disabling the effect requires restarting the app.",
      "opacity": "Opacity",
      "color": "Tint color"
    },
    "notificationSound": {
      "label": "Notification Sound",
      "volume": "Volume",
      "none": "None",
      "systemSounds": "System Sounds",
      "pickCustom": "Choose custom file...",
      "preview": "Preview",
      "kind": {
        "approval": "Approval",
        "completed": "Completed",
        "general": "General"
      }
    },
    "homeCat": {
      "label": "Terminal Cat (Experimental)",
      "enable": "Show cat on home screen"
    }
  },
  "ja": {
    "title": "設定",
    "mcp": {
      "label": "MCP サーバー",
      "loading": "取得中...",
      "running": "稼働中",
      "stopped": "停止",
      "port": "ポート: {port}",
      "restart": "再起動",
      "restarting": "再起動中...",
      "fixedPort": "固定ポート",
      "fixedPortHint": "0 = 自動割り当て。変更は再起動後に反映されます。",
      "apiKey": "APIキー",
      "copy": "コピー",
      "regenerate": "再生成",
      "regenerating": "再生成中...",
      "regenerateConfirm": "APIキーを再生成すると、接続中のMCPクライアントは切断されます。続行しますか？"
    },
    "window": {
      "label": "ウィンドウ",
      "alwaysOnTop": "常に手前に表示",
      "osNotification": "通知時にOSのデスクトップ通知を表示",
      "focusMainOnEmptyTray": "通知がない時にトレイホットキーでメインウィンドウにフォーカス"
    },
    "autoApproval": {
      "label": "自動承認 / AI エージェント",
      "approvalAgent": "自動承認・コミットメッセージ・タスク生成",
      "taskAddAgent": "タスク追加コード実行",
      "notSet": "(未設定)",
      "notDetected": " (未検出)",
      "aiTimeout": "AI タイムアウト",
      "seconds": "秒"
    },
    "terminal": {
      "label": "ターミナル",
      "fontSize": "文字サイズ",
      "defaultShell": "デフォルトシェル",
      "shellPlaceholder": "空欄 = システムデフォルト"
    },
    "worktreeBaseDir": {
      "label": "ワークツリーの追加先ディレクトリ",
      "select": "選択"
    },
    "worktreeDefaults": {
      "label": "ワークツリー追加時のデフォルト動作",
      "openInSubWindow": "サブウィンドウで開く",
      "enableAutoApproval": "自動承認を有効にする",
      "autoOpenArtifact": "アーティファクト追加時にビューアを自動で開く"
    },
    "appearance": {
      "label": "外観",
      "enableAcrylic": "Acrylic / LiquidGlass エフェクトを有効にする",
      "enableGamingBorder": "アウトラインエフェクト有効",
      "gamingBorderTheme": "テーマカラー",
      "gamingBorderThemes": {
        "gaming": "ゲーミング",
        "snowWhite": "スノーホワイト",
        "gold": "ゴールド",
        "flame": "フレイム",
        "aqua": "アクア",
        "dark": "ダーク",
        "nature": "ネイチャー"
      },
      "restartNote": "エフェクトの有効/無効はアプリ再起動後に反映されます。",
      "opacity": "不透明度",
      "color": "背景色"
    },
    "notificationSound": {
      "label": "通知音",
      "volume": "音量",
      "none": "なし",
      "systemSounds": "システムサウンド",
      "pickCustom": "カスタムファイルを選択...",
      "preview": "プレビュー",
      "kind": {
        "approval": "承認待ち",
        "completed": "作業完了",
        "general": "汎用"
      }
    },
    "homeCat": {
      "label": "ターミナル猫（試験的）",
      "enable": "ホーム画面に猫を表示する"
    }
  }
}
</i18n>
