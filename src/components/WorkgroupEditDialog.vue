<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { AI_AGENT_LABELS, ALL_AGENT_KINDS } from "../constants/aiAgents";
import type { AiAgentKind, ClaudeCodeMode, Workgroup } from "../types/settings";

const { t } = useI18n();

const props = defineProps<{
  group: Workgroup;
}>();

const emit = defineEmits<{
  save: [patch: Partial<Workgroup>];
  remove: [];
  cancel: [];
}>();

// プリセット色（null = 無色）
const PRESET_COLORS: (string | null)[] = [
  null,
  "#f38ba8", "#fab387", "#f9e2af",
  "#a6e3a1", "#89b4fa", "#cba6f7", "#94e2d5",
];

const MODES: ClaudeCodeMode[] = ["plan", "manual", "acceptEdit", "auto"];

const name = ref(props.group.name ?? "");
const color = ref<string | null>(props.group.color ?? null);
const autoAssignHotkey = ref(props.group.autoAssignHotkey ?? false);
const taskAddAgent = ref<AiAgentKind | "">(props.group.taskAddAgent ?? "");
const claudeCodeMode = ref<ClaudeCodeMode>(props.group.claudeCodeMode ?? "plan");
const execPrompt = ref(props.group.execPrompt ?? "");

function save() {
  emit("save", {
    name: name.value.trim() || undefined,
    color: color.value ?? undefined,
    autoAssignHotkey: autoAssignHotkey.value,
    taskAddAgent: taskAddAgent.value || undefined,
    claudeCodeMode: claudeCodeMode.value,
    execPrompt: execPrompt.value.trim() || undefined,
  });
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title') }}</h3>

      <div class="field">
        <label class="label">{{ t('name') }}</label>
        <input v-model="name" class="text-in" type="text" :placeholder="t('namePlaceholder')" />
      </div>

      <div class="field">
        <label class="label">{{ t('color') }}</label>
        <div class="swatches">
          <button
            v-for="(c, i) in PRESET_COLORS"
            :key="i"
            class="swatch"
            :class="{ selected: color === c }"
            :style="{ background: c ?? 'transparent' }"
            :title="c ?? t('noColor')"
            @click="color = c"
          >{{ c ? '' : '∅' }}</button>
        </div>
      </div>

      <div class="divider" />

      <div class="field checkbox-field">
        <label class="checkbox-label">
          <input v-model="autoAssignHotkey" type="checkbox" />
          {{ t('autoAssignHotkey') }}
        </label>
      </div>

      <div class="field">
        <label class="label">{{ t('taskAddAgent') }}</label>
        <select v-model="taskAddAgent" class="select">
          <option value="">{{ t('notSet') }}</option>
          <option v-for="kind in ALL_AGENT_KINDS" :key="kind" :value="kind">{{ AI_AGENT_LABELS[kind] }}</option>
        </select>
      </div>

      <div v-if="taskAddAgent === 'claudeCode'" class="field">
        <label class="label">{{ t('claudeCodeMode') }}</label>
        <div class="mode-row">
          <button
            v-for="m in MODES"
            :key="m"
            class="mode-btn"
            :class="{ selected: claudeCodeMode === m }"
            @click="claudeCodeMode = m"
          >{{ t('mode.' + m) }}</button>
        </div>
      </div>

      <div class="field">
        <label class="label">{{ t('execPrompt') }}</label>
        <textarea v-model="execPrompt" class="textarea" :placeholder="t('execPromptPlaceholder')" rows="4" />
        <p class="hint">{{ t('execPromptHint') }}</p>
      </div>

      <div class="dialog-actions">
        <button class="btn-remove" @click="emit('remove')">{{ t('removeGroup') }}</button>
        <div class="spacer" />
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.cancel') }}</button>
        <button class="btn-primary" @click="save">{{ t('saveButton') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 10px;
  padding: 24px;
  width: 460px;
  max-width: 90vw;
  max-height: 88vh;
  overflow-y: auto;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
  margin: 0 0 16px;
}

.field {
  margin-bottom: 14px;
}

.label {
  display: block;
  font-size: 12px;
  color: #a6adc8;
  margin-bottom: 5px;
}

.text-in,
.select,
.textarea {
  width: 100%;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 10px;
  font-size: 13px;
  color: #cdd6f4;
  outline: none;
  box-sizing: border-box;
}

.textarea {
  resize: vertical;
  font-family: monospace;
  line-height: 1.5;
}

.select option {
  background: #313244;
}

.swatches {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.swatch {
  width: 30px;
  height: 30px;
  border-radius: 6px;
  border: 1px solid #45475a;
  cursor: pointer;
  color: #6c7086;
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.swatch.selected {
  border: 2px solid #cdd6f4;
  box-shadow: 0 0 0 1px #1e1e2e;
}

.divider {
  height: 1px;
  background: #313244;
  margin: 16px 0;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #cdd6f4;
  cursor: pointer;
  user-select: none;
}

.checkbox-label input[type="checkbox"] {
  cursor: pointer;
  accent-color: #cba6f7;
}

.mode-row {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}

.mode-btn {
  padding: 6px 12px;
  border-radius: 5px;
  border: 1px solid #45475a;
  background: #313244;
  color: #a6adc8;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.mode-btn.selected {
  background: #cba6f7;
  color: #1e1e2e;
  border-color: #cba6f7;
}

.hint {
  font-size: 11px;
  color: #6c7086;
  margin: 5px 0 0;
}

.dialog-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 20px;
}

.spacer {
  flex: 1;
}

.btn-cancel {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  cursor: pointer;
}

.btn-cancel:hover {
  background: #45475a;
}

.btn-primary {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-primary:hover {
  background: #b794e6;
}

.btn-remove {
  background: transparent;
  color: #f38ba8;
  border: 1px solid rgba(243, 139, 168, 0.4);
  border-radius: 4px;
  padding: 7px 14px;
  font-size: 12px;
  cursor: pointer;
}

.btn-remove:hover {
  background: rgba(243, 139, 168, 0.15);
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Edit workgroup",
    "name": "Name",
    "namePlaceholder": "Auto: Group (n) if empty",
    "color": "Color",
    "noColor": "No color",
    "autoAssignHotkey": "Auto-assign hotkeys",
    "taskAddAgent": "Task execution agent",
    "notSet": "Not set (use global)",
    "claudeCodeMode": "Claude Code mode",
    "mode": { "plan": "Plan", "manual": "Manual", "acceptEdit": "AcceptEdit", "auto": "Auto" },
    "execPrompt": "Execution prompt",
    "execPromptPlaceholder": "e.g. Work on the following task.\n\n{{PROMPT}}",
    "execPromptHint": "{{PROMPT}} is replaced with the task prompt. Empty = task prompt only.",
    "saveButton": "Save",
    "removeGroup": "Delete this group"
  },
  "ja": {
    "title": "ワークグループの編集",
    "name": "名称",
    "namePlaceholder": "空欄なら「グループ(番号)」を自動採番",
    "color": "色",
    "noColor": "無色",
    "autoAssignHotkey": "ホットキー自動割り当て",
    "taskAddAgent": "タスク実行エージェント",
    "notSet": "未設定（全体設定を使用）",
    "claudeCodeMode": "Claude Code モード",
    "mode": { "plan": "Plan", "manual": "Manual", "acceptEdit": "AcceptEdit", "auto": "Auto" },
    "execPrompt": "実行プロンプト",
    "execPromptPlaceholder": "例: 以下のタスクに取り組んでください。\n\n{{PROMPT}}",
    "execPromptHint": "{{PROMPT}} がタスク実行プロンプトに置換されます。未指定ならプロンプトのみと等価。",
    "saveButton": "保存",
    "removeGroup": "このグループを削除"
  }
}
</i18n>
