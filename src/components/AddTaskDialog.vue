<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = withDefaults(defineProps<{
  initialPrompt?: string;
  mode?: "add" | "rerun";
}>(), {
  initialPrompt: "",
  mode: "add",
});

const emit = defineEmits<{
  confirm: [prompt: string];
  cancel: [];
}>();

const promptText = ref(props.initialPrompt);

function confirm() {
  const trimmed = promptText.value.trim();
  if (!trimmed) return;
  emit("confirm", trimmed);
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    confirm();
  }
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ mode === 'rerun' ? t('rerunTitle') : t('addTitle') }}</h3>

      <div class="field">
        <label class="label">{{ t('prompt') }}</label>
        <textarea
          v-model="promptText"
          class="textarea"
          :placeholder="t('promptPlaceholder')"
          rows="5"
          autofocus
          @keydown="onKeydown"
        />
        <p class="hint">{{ t('submitHint') }}</p>
      </div>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.cancel') }}</button>
        <button
          class="btn-confirm"
          :disabled="!promptText.trim()"
          @click="confirm"
        >
          {{ mode === 'rerun' ? t('rerun') : t('add') }}
        </button>
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
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
  margin: 0 0 20px;
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

.textarea {
  width: 100%;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 8px 10px;
  font-size: 13px;
  color: #cdd6f4;
  outline: none;
  box-sizing: border-box;
  resize: vertical;
  font-family: inherit;
  line-height: 1.5;
}

.textarea:focus {
  border-color: #cba6f7;
}

.hint {
  margin: 4px 0 0;
  font-size: 11px;
  color: #6c7086;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 20px;
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

.btn-confirm {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-confirm:hover:not(:disabled) {
  background: #b4befe;
}

.btn-confirm:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

<i18n lang="json">
{
  "en": {
    "addTitle": "Add Task",
    "rerunTitle": "Rerun Task",
    "prompt": "Prompt",
    "promptPlaceholder": "e.g. Implement https://github.com/owner/repo/issues/123",
    "submitHint": "Ctrl+Enter to submit",
    "add": "Add",
    "rerun": "Rerun"
  },
  "ja": {
    "addTitle": "タスクを追加",
    "rerunTitle": "タスクを再実行",
    "prompt": "プロンプト",
    "promptPlaceholder": "例: https://github.com/owner/repo/issues/123 を実装してください",
    "submitHint": "Ctrl+Enter で送信",
    "add": "追加",
    "rerun": "再実行"
  }
}
</i18n>
