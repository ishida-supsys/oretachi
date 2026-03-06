<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";

const props = defineProps<{
  worktreeId: string;
  worktreeName: string;
  currentChar?: string;
  usedChars: Set<string>; // 他のワークツリーに割り当て済みの文字
}>();

const emit = defineEmits<{
  confirm: [worktreeId: string, char: string];
  clear: [worktreeId: string];
  cancel: [];
}>();

const error = ref("");

function onKeydown(event: KeyboardEvent) {
  event.preventDefault();
  event.stopPropagation();

  if (event.key === "Escape") {
    emit("cancel");
    return;
  }

  const key = event.key.toLowerCase();
  if (!/^[a-z0-9]$/.test(key)) return;

  if (props.usedChars.has(key)) {
    error.value = `「${key}」は別のワークツリーに割り当て済みです`;
    return;
  }

  emit("confirm", props.worktreeId, key);
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown, true);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown, true);
});
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">ホットキー割り当て</h3>
      <p class="dialog-sub">{{ worktreeName }}</p>

      <div class="current-info" v-if="currentChar">
        現在の割り当て: <span class="key-badge">Alt+{{ currentChar }}</span>
      </div>

      <p class="hint">英数字キーを押してください</p>
      <p class="hint-sub">割り当てると <strong>Alt+[文字]</strong> でこのワークツリーにフォーカスできます</p>

      <p v-if="error" class="error-msg">{{ error }}</p>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">キャンセル</button>
        <button
          v-if="currentChar"
          class="btn-clear"
          @click="emit('clear', worktreeId)"
        >
          割り当て解除
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
  width: 380px;
  max-width: 90vw;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
  margin: 0 0 4px;
}

.dialog-sub {
  font-size: 13px;
  color: #a6adc8;
  margin: 0 0 16px;
}

.current-info {
  font-size: 13px;
  color: #cdd6f4;
  margin-bottom: 16px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.key-badge {
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 2px 8px;
  font-family: monospace;
  font-size: 13px;
  color: #cba6f7;
}

.hint {
  font-size: 14px;
  font-weight: 600;
  color: #cdd6f4;
  margin: 0 0 6px;
  text-align: center;
}

.hint-sub {
  font-size: 12px;
  color: #6c7086;
  margin: 0 0 16px;
  text-align: center;
}

.error-msg {
  font-size: 12px;
  color: #f38ba8;
  margin: 0 0 16px;
  text-align: center;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 8px;
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

.btn-clear {
  background: transparent;
  color: #f38ba8;
  border: 1px solid rgba(243, 139, 168, 0.4);
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  cursor: pointer;
}

.btn-clear:hover {
  background: rgba(243, 139, 168, 0.1);
}
</style>
