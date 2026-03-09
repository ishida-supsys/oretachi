<script setup lang="ts">
import type { HotkeyBinding } from "../../types/settings";
import { formatHotkey } from "../../composables/useHotkeys";

defineProps<{
  buttonPos: { top: number; left: number; height: number } | null;
  filePath?: string;
  hotkey?: HotkeyBinding;
}>();

defineEmits<{ click: [] }>();
</script>

<template>
  <button
    v-if="buttonPos && filePath"
    class="absolute z-10 flex items-center gap-1 px-2 py-0.5 text-xs bg-primary-600 hover:bg-primary-500 text-white rounded shadow-lg transition-colors pointer-events-auto"
    :style="{ top: (buttonPos.top + buttonPos.height) + 'px', left: buttonPos.left + 'px' }"
    @click="$emit('click')"
    @mousedown.prevent
  >
    <i class="pi pi-comments text-[10px]" />
    Chat<template v-if="hotkey"> {{ formatHotkey(hotkey) }}</template>
  </button>
</template>
