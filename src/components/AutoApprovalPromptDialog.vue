<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{
  worktreeId: string;
  worktreeName: string;
  currentPrompt: string;
  lastCommand: string;
}>();

const emit = defineEmits<{
  save: [worktreeId: string, prompt: string];
  cancel: [];
}>();

const promptText = ref(props.currentPrompt);

watch(() => props.currentPrompt, (val) => {
  promptText.value = val;
});

function insertLastCommand() {
  if (!props.lastCommand) return;
  promptText.value += (promptText.value ? "\n" : "") + props.lastCommand;
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title') }}</h3>
      <p class="dialog-sub">{{ worktreeName }}</p>

      <label class="field-label">{{ t('label') }}</label>
      <textarea
        v-model="promptText"
        class="prompt-textarea"
        :placeholder="t('placeholder')"
      />

      <button
        v-if="lastCommand"
        class="btn-insert"
        @click="insertLastCommand"
      >
        <span class="pi pi-plus" style="font-size: 10px" />
        {{ t('insertLastCommand') }}: <span class="command-chip">{{ lastCommand }}</span>
      </button>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('cancel') }}</button>
        <button class="btn-save" @click="emit('save', worktreeId, promptText)">{{ t('save') }}</button>
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
  width: 480px;
  max-width: 90vw;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #a6e3a1;
  margin: 0;
}

.dialog-sub {
  font-size: 13px;
  color: #a6adc8;
  margin: 0;
}

.field-label {
  font-size: 12px;
  color: #a6adc8;
}

.prompt-textarea {
  width: 100%;
  height: 120px;
  background: #181825;
  color: #cdd6f4;
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 8px;
  font-size: 13px;
  font-family: monospace;
  resize: vertical;
  box-sizing: border-box;
}

.prompt-textarea:focus {
  outline: none;
  border-color: #a6e3a1;
}

.btn-insert {
  display: flex;
  align-items: center;
  gap: 6px;
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 5px 10px;
  font-size: 12px;
  cursor: pointer;
  align-self: flex-start;
  max-width: 100%;
  overflow: hidden;
}

.btn-insert:hover {
  background: #45475a;
}

.command-chip {
  color: #a6e3a1;
  font-family: monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 280px;
  display: inline-block;
  vertical-align: middle;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 4px;
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

.btn-save {
  background: rgba(166, 227, 161, 0.2);
  color: #a6e3a1;
  border: 1px solid rgba(166, 227, 161, 0.4);
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-save:hover {
  background: rgba(166, 227, 161, 0.3);
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Auto Approval - Additional Prompt",
    "label": "Additional instructions for AI judgment:",
    "placeholder": "e.g., Always approve npm test, Never approve commands with sudo...",
    "insertLastCommand": "Insert last judged command",
    "cancel": "Cancel",
    "save": "Save"
  },
  "ja": {
    "title": "自動承認 - 追加プロンプト",
    "label": "AI判定時の追加指示:",
    "placeholder": "例: npm test は常に承認する、sudo を含むコマンドは承認しない...",
    "insertLastCommand": "直前の判定コマンドを挿入",
    "cancel": "キャンセル",
    "save": "保存"
  }
}
</i18n>
