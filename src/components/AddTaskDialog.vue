<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = withDefaults(defineProps<{
  initialPrompt?: string;
  mode?: "add" | "rerun";
  showRemoteExec?: boolean;
  initialRemoteExec?: boolean;
}>(), {
  initialPrompt: "",
  mode: "add",
  showRemoteExec: false,
  initialRemoteExec: false,
});

const emit = defineEmits<{
  confirm: [prompt: string, remoteExec: boolean];
  cancel: [];
}>();

const promptText = ref(props.initialPrompt);
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const showConfirm = ref(false);
const remoteExec = ref(props.initialRemoteExec);

const isDirty = computed(() => promptText.value.trim() !== props.initialPrompt.trim());

onMounted(() => {
  textareaRef.value?.focus();
});

function confirm() {
  const trimmed = promptText.value.trim();
  if (!trimmed) return;
  emit("confirm", trimmed, remoteExec.value);
}

function tryCancel() {
  if (isDirty.value) {
    showConfirm.value = true;
    return;
  }
  emit('cancel');
}

function confirmCancel() {
  emit('cancel');
}

function dismissConfirm() {
  showConfirm.value = false;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    confirm();
  }
  if (e.key === "Escape") {
    e.preventDefault();
    if (showConfirm.value) {
      dismissConfirm();
    } else {
      tryCancel();
    }
  }
}
</script>

<template>
  <div class="dialog-overlay">
    <div class="dialog">
      <h3 class="dialog-title">{{ mode === 'rerun' ? t('rerunTitle') : t('addTitle') }}</h3>

      <div class="field">
        <label class="label">{{ t('prompt') }}</label>
        <textarea
          ref="textareaRef"
          v-model="promptText"
          class="textarea"
          :placeholder="t('promptPlaceholder')"
          rows="5"
          @keydown="onKeydown"
        />
        <p class="hint">{{ t('submitHint') }}</p>
      </div>

      <div v-if="showRemoteExec" class="field checkbox-field">
        <label class="checkbox-label">
          <input type="checkbox" v-model="remoteExec" />
          <span>{{ t('remoteExec') }}</span>
        </label>
      </div>

      <div v-if="showConfirm" class="confirm-bar">
        <span class="confirm-message">{{ t('confirmClose') }}</span>
        <div class="confirm-actions">
          <button class="btn-back" @click="dismissConfirm">{{ t('back') }}</button>
          <button class="btn-confirm-close" @click="confirmCancel">{{ t('closeAnyway') }}</button>
        </div>
      </div>

      <div v-else class="dialog-actions">
        <button class="btn-cancel" @click="tryCancel">{{ t('common.cancel') }}</button>
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

.confirm-bar {
  margin-top: 20px;
  padding: 12px 14px;
  background: #2a1f3d;
  border: 1px solid #cba6f7;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.confirm-message {
  font-size: 12px;
  color: #cba6f7;
  flex: 1;
}

.confirm-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.btn-confirm-close {
  background: #f38ba8;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 5px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.btn-confirm-close:hover {
  background: #eba0ac;
}

.btn-back {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 5px 12px;
  font-size: 12px;
  cursor: pointer;
}

.btn-back:hover {
  background: #45475a;
}

.checkbox-field {
  margin-bottom: 0;
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
  accent-color: #cba6f7;
  width: 14px;
  height: 14px;
  cursor: pointer;
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
    "remoteExec": "Start in web session",
    "add": "Add",
    "rerun": "Rerun",
    "confirmClose": "You have unsaved input. Are you sure you want to close?",
    "closeAnyway": "Close",
    "back": "Back"
  },
  "ja": {
    "addTitle": "タスクを追加",
    "rerunTitle": "タスクを再実行",
    "prompt": "プロンプト",
    "promptPlaceholder": "例: https://github.com/owner/repo/issues/123 を実装してください",
    "submitHint": "Ctrl+Enter で送信",
    "remoteExec": "Webセッションで開始",
    "add": "追加",
    "rerun": "再実行",
    "confirmClose": "入力内容が失われますが、閉じてもよろしいですか？",
    "closeAnyway": "閉じる",
    "back": "戻る"
  }
}
</i18n>
