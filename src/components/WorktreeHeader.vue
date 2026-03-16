<script setup lang="ts">
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{
  worktreeName: string;
  branchName: string;
  hotkeyChar?: string;
  autoApproval: boolean;
  aiJudging: boolean;
  isWindowFocused: boolean;
}>();

defineEmits<{
  "open-in-ide": [];
  "cancel-ai-judging": [];
}>();
</script>

<template>
  <div
    class="flex items-center justify-between border-b shrink-0 px-4 py-1 transition-colors duration-200"
    :class="
      props.isWindowFocused
        ? 'bg-gradient-to-r from-[#181825] via-[#2a2a3f] to-[#181825] animate-gradient-x border-[#cba6f7]/50'
        : 'bg-[#11111b] opacity-80 border-[#313244]'
    "
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
      <span
        v-if="props.autoApproval"
        class="text-[10px] px-1.5 py-0.5 rounded font-medium"
        style="background: rgba(166, 227, 161, 0.15); color: #a6e3a1; border: 1px solid rgba(166, 227, 161, 0.3)"
      >{{ t('autoApprovalBadge') }}</span>
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
    <button
      class="flex items-center justify-center w-7 h-7 rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition-colors"
      :title="t('openInIde')"
      @click="$emit('open-in-ide')"
    >
      <span class="pi pi-code text-sm" />
    </button>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "autoApprovalBadge": "Auto approval",
    "aiJudgingBadge": "AI judging",
    "openInIde": "Open in IDE"
  },
  "ja": {
    "autoApprovalBadge": "自動承認",
    "aiJudgingBadge": "AI判定中",
    "openInIde": "IDE で開く"
  }
}
</i18n>
