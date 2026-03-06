<script setup lang="ts">
import { ref } from "vue";
import type { HotkeyBinding } from "../types/settings";
import { formatHotkey, eventToBinding } from "../composables/useHotkeys";

defineProps<{
  modelValue: HotkeyBinding;
}>();

const emit = defineEmits<{
  "update:modelValue": [binding: HotkeyBinding];
}>();

const capturing = ref(false);

function startCapture() {
  capturing.value = true;
}

function onKeydown(event: KeyboardEvent) {
  if (!capturing.value) return;
  event.preventDefault();
  event.stopPropagation();

  if (event.key === "Escape") {
    capturing.value = false;
    return;
  }

  // 修飾キーのみは無視
  if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;

  const binding = eventToBinding(event);
  emit("update:modelValue", binding);
  capturing.value = false;
}

function onBlur() {
  capturing.value = false;
}
</script>

<template>
  <button
    class="hotkey-input"
    :class="{ 'hotkey-input--capturing': capturing }"
    tabindex="0"
    @click="startCapture"
    @keydown="onKeydown"
    @blur="onBlur"
  >
    {{ capturing ? 'キーを入力...' : formatHotkey(modelValue) }}
  </button>
</template>

<style scoped>
.hotkey-input {
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 13px;
  color: #cdd6f4;
  cursor: pointer;
  font-family: monospace;
  min-width: 140px;
  text-align: left;
  outline: none;
}

.hotkey-input:hover {
  background: #45475a;
}

.hotkey-input--capturing {
  border-color: #cba6f7;
  background: rgba(203, 166, 247, 0.1);
  color: #cba6f7;
}
</style>
