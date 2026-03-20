<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";

const { t } = useI18n();

const props = defineProps<{
  worktreeName: string;
  branchName: string;
  hotkeyChar?: string;
  autoApproval: boolean;
  aiJudging: boolean;
  isWindowFocused: boolean;
  showWindowControls?: boolean;
}>();

defineEmits<{
  "open-in-ide": [];
  "cancel-ai-judging": [];
  "click-auto-approval": [];
}>();

function onHeaderDrag(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('button')) return;
  getCurrentWindow().startDragging();
}

async function minimize() {
  await getCurrentWindow().minimize();
}

async function toggleMaximize() {
  await getCurrentWindow().toggleMaximize();
}

async function closeWindow() {
  await getCurrentWindow().close();
}
</script>

<template>
  <div
    class="flex items-center justify-between border-b shrink-0 pl-4 pr-[2px] py-1 transition-colors duration-200"
    :class="
      props.isWindowFocused
        ? 'border-[#cba6f7]/50'
        : 'opacity-80 border-[#313244]'
    "
    @mousedown.left="props.showWindowControls ? onHeaderDrag($event) : undefined"
  >
    <div class="flex items-center gap-2">
      <span
        class="text-sm font-semibold transition-colors duration-200"
        :class="props.isWindowFocused ? 'text-[#cba6f7]' : 'text-[#6c7086]'"
      >
        {{ props.worktreeName }}
      </span>
      <span class="flex items-center gap-1 text-xs font-mono text-[#9399b2]">
        <span class="pi pi-code-branch" style="font-size: 10px" />
        {{ props.branchName }}
      </span>
      <span
        v-if="props.hotkeyChar"
        class="text-[10px] px-1.5 py-0.5 rounded font-mono font-medium"
        style="background: rgba(203,166,247,0.15); color: #cba6f7; border: 1px solid rgba(203,166,247,0.3)"
      >Alt+{{ props.hotkeyChar.toUpperCase() }}</span>
      <button
        v-if="props.autoApproval"
        class="text-[10px] px-1.5 py-0.5 rounded font-medium cursor-pointer border-none"
        style="background: rgba(166, 227, 161, 0.15); color: #a6e3a1; border: 1px solid rgba(166, 227, 161, 0.3)"
        :title="t('editAutoApprovalPrompt')"
        @click="$emit('click-auto-approval')"
      >{{ t('autoApprovalBadge') }}</button>
      <button
        v-if="props.aiJudging"
        class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-semibold cursor-pointer border-none"
        style="background: #f9e2af; color: #1e1e2e"
        @click="$emit('cancel-ai-judging')"
      >
        <span class="pi pi-spin pi-spinner" style="font-size: 9px" />
        {{ t('aiJudgingBadge') }}
      </button>
    </div>
    <div class="flex items-center">
      <button
        class="flex items-center justify-center w-7 h-7 rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition-colors"
        :class="props.showWindowControls ? 'mr-4' : ''"
        :title="t('openInIde')"
        @click="$emit('open-in-ide')"
      >
        <span class="pi pi-code text-sm" />
      </button>
      <template v-if="props.showWindowControls">
        <button
          class="flex items-center justify-center h-8 hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4] transition-colors"
          style="width: 42px; margin: 0 1px;"
          :title="t('minimize')"
          @click="minimize"
        >
          <span class="pi pi-minus text-xs" />
        </button>
        <button
          class="flex items-center justify-center h-8 hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4] transition-colors"
          style="width: 42px; margin: 0 1px;"
          :title="t('maximize')"
          @click="toggleMaximize"
        >
          <span class="pi pi-stop text-xs" />
        </button>
        <button
          class="flex items-center justify-center h-8 hover:bg-[#c0392b] hover:text-white text-[#6c7086] transition-colors"
          style="width: 42px; margin: 0 1px;"
          :title="t('close')"
          @click="closeWindow"
        >
          <span class="pi pi-times text-xs" />
        </button>
      </template>
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "autoApprovalBadge": "Auto approval",
    "aiJudgingBadge": "AI judging",
    "openInIde": "Open in IDE",
    "minimize": "Minimize",
    "maximize": "Maximize",
    "close": "Close",
    "editAutoApprovalPrompt": "Edit additional prompt"
  },
  "ja": {
    "autoApprovalBadge": "自動承認",
    "aiJudgingBadge": "AI判定中",
    "openInIde": "IDE で開く",
    "minimize": "最小化",
    "maximize": "最大化",
    "close": "閉じる",
    "editAutoApprovalPrompt": "追加プロンプトを編集"
  }
}
</i18n>
