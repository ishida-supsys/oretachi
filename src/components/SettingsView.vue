<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open, message } from "@tauri-apps/plugin-dialog";
import { useSettings } from "../composables/useSettings";
import HotkeyInput from "./HotkeyInput.vue";
import type { AiAgentKind } from "../types/settings";
import { useI18n } from "vue-i18n";
import { setLocale } from "../i18n";

const { t } = useI18n();

const { settings, scheduleSave } = useSettings();

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
    mcpStatus.value = await invoke<McpStatus>("restart_mcp_server");
  } catch (e) {
    console.error("restart_mcp_server failed:", e);
    await fetchMcpStatus();
  } finally {
    restarting.value = false;
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

async function addRepository() {
  const selected = await open({ directory: true, multiple: false });
  if (typeof selected !== "string") return;

  try {
    const valid = await invoke<boolean>("git_validate_repo", { path: selected });
    if (!valid) {
      await message(t("error.notARepo"), { kind: "error" });
      return;
    }
  } catch {
    await message(t("error.notARepo"), { kind: "error" });
    return;
  }

  const name = selected.split(/[/\\]/).pop() ?? selected;

  // 重複チェック
  if (settings.value.repositories.some((r) => r.path === selected)) {
    await message(t("error.alreadyRegistered"), { kind: "warning" });
    return;
  }

  settings.value.repositories.push({
    id: selected, // id にパスをそのまま使用
    name,
    path: selected,
  });
  scheduleSave();
}

function removeRepository(id: string) {
  settings.value.repositories = settings.value.repositories.filter(
    (r) => r.id !== id
  );
  scheduleSave();
}

function hasWorktrees(repoId: string): boolean {
  return settings.value.worktrees.some((w) => w.repositoryId === repoId);
}

async function selectExecScript(repoId: string) {
  const selected = await open({
    multiple: false,
    filters: [{ name: "Scripts", extensions: ["ps1", "sh"] }],
  });
  if (typeof selected !== "string") return;
  const repo = settings.value.repositories.find((r) => r.id === repoId);
  if (!repo) return;
  repo.execScript = selected;
  scheduleSave();
}

function clearExecScript(repoId: string) {
  const repo = settings.value.repositories.find((r) => r.id === repoId);
  if (!repo) return;
  repo.execScript = undefined;
  scheduleSave();
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
    </div>

    <!-- ホットキー設定 -->
    <div class="field-group">
      <label class="field-label">{{ t('hotkeys.label') }}</label>
      <div class="row-input row-input--inline">
        <input
          id="auto-assign-hotkey"
          type="checkbox"
          class="toggle-checkbox"
          :checked="settings.autoAssignHotkey"
          @change="(e) => { settings.autoAssignHotkey = (e.target as HTMLInputElement).checked; scheduleSave(); }"
        />
        <label for="auto-assign-hotkey" class="inline-label toggle-label">{{ t('hotkeys.autoAssign') }}</label>
      </div>
      <table v-if="settings.hotkeys" class="hotkey-table">
        <thead>
          <tr>
            <th class="hotkey-th">{{ t('hotkeys.action') }}</th>
            <th class="hotkey-th">{{ t('hotkeys.key') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.trayPopup') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.globalTrayPopup"
                @update:model-value="(v) => { settings.hotkeys.globalTrayPopup = v; scheduleSave(); }"
              />
            </td>
          </tr>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.terminalNext') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.terminalNext"
                @update:model-value="(v) => { settings.hotkeys.terminalNext = v; scheduleSave(); }"
              />
            </td>
          </tr>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.terminalPrev') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.terminalPrev"
                @update:model-value="(v) => { settings.hotkeys.terminalPrev = v; scheduleSave(); }"
              />
            </td>
          </tr>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.terminalAdd') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.terminalAdd"
                @update:model-value="(v) => { settings.hotkeys.terminalAdd = v; scheduleSave(); }"
              />
            </td>
          </tr>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.terminalClose') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.terminalClose"
                @update:model-value="(v) => { settings.hotkeys.terminalClose = v; scheduleSave(); }"
              />
            </td>
          </tr>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.trayNext') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.trayNext"
                @update:model-value="(v) => { settings.hotkeys.trayNext = v; scheduleSave(); }"
              />
            </td>
          </tr>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.homeTab') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.homeTab"
                @update:model-value="(v) => { settings.hotkeys.homeTab = v; scheduleSave(); }"
              />
            </td>
          </tr>
          <tr>
            <td class="hotkey-td-label">{{ t('hotkeys.addTask') }}</td>
            <td class="hotkey-td-input">
              <HotkeyInput
                :model-value="settings.hotkeys.addTask"
                @update:model-value="(v) => { settings.hotkeys.addTask = v; scheduleSave(); }"
              />
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- リポジトリ一覧 -->
    <div class="field-group">
      <div class="field-header">
        <label class="field-label">{{ t('repositories.label') }}</label>
        <button class="btn-primary" @click="addRepository">{{ t('repositories.add') }}</button>
      </div>
      <div class="repo-list">
        <div
          v-if="settings.repositories.length === 0"
          class="empty-state"
        >
          {{ t('repositories.empty') }}
        </div>
        <div
          v-for="repo in settings.repositories"
          :key="repo.id"
          class="repo-item"
        >
          <div class="repo-row-main">
            <span class="repo-name">{{ repo.name }}</span>
            <span class="repo-path">{{ repo.path }}</span>
            <button class="btn-remove" :disabled="hasWorktrees(repo.id)" :title="hasWorktrees(repo.id) ? t('repositories.hasWorktrees') : undefined" @click="removeRepository(repo.id)">×</button>
          </div>
          <div class="repo-row-script">
            <span class="script-label">{{ t('repositories.execScript') }}</span>
            <input
              class="text-input script-input"
              :value="repo.execScript ?? ''"
              readonly
              :placeholder="t('common.notConfigured')"
            />
            <button class="btn-secondary" @click="selectExecScript(repo.id)">{{ t('worktreeBaseDir.select') }}</button>
            <button
              v-if="repo.execScript"
              class="btn-secondary"
              @click="clearExecScript(repo.id)"
            >{{ t('common.clear') }}</button>
          </div>
        </div>
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
      "restarting": "Restarting..."
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
      "notDetected": " (Not detected)"
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
      "enableAutoApproval": "Enable auto approval"
    },
    "hotkeys": {
      "label": "Hotkeys",
      "autoAssign": "Auto-assign hotkeys",
      "action": "Action",
      "key": "Key",
      "trayPopup": "Show Tray Popup (global)",
      "terminalNext": "Switch terminal: next",
      "terminalPrev": "Switch terminal: prev",
      "terminalAdd": "Add terminal",
      "terminalClose": "Close terminal",
      "trayNext": "Next notification (tray)",
      "homeTab": "Home tab",
      "addTask": "Add task"
    },
    "repositories": {
      "label": "Repositories",
      "add": "+ Add",
      "empty": "No repositories registered",
      "hasWorktrees": "Cannot delete: worktrees exist",
      "execScript": "Exec script"
    },
    "error": {
      "notARepo": "The selected folder is not a git repository.",
      "alreadyRegistered": "This repository is already registered."
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
      "restarting": "再起動中..."
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
      "notDetected": " (未検出)"
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
      "enableAutoApproval": "自動承認を有効にする"
    },
    "hotkeys": {
      "label": "ホットキー",
      "autoAssign": "ホットキー自動割り当て",
      "action": "操作",
      "key": "キー",
      "trayPopup": "Tray Popup 表示 (グローバル)",
      "terminalNext": "ターミナル切り替え: 次",
      "terminalPrev": "ターミナル切り替え: 前",
      "terminalAdd": "ターミナル追加",
      "terminalClose": "ターミナルを閉じる",
      "trayNext": "次の通知へ (トレイ)",
      "homeTab": "ホームタブへ戻る",
      "addTask": "タスク追加"
    },
    "repositories": {
      "label": "リポジトリ一覧",
      "add": "+ 追加",
      "empty": "リポジトリが登録されていません",
      "hasWorktrees": "ワークツリーが存在するため削除できません",
      "execScript": "実行スクリプト"
    },
    "error": {
      "notARepo": "選択したフォルダは git リポジトリではありません。",
      "alreadyRegistered": "このリポジトリはすでに登録されています。"
    }
  }
}
</i18n>
