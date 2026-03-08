<script setup lang="ts">
import type { TaskItem } from "../types/task";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

defineProps<{
  task: TaskItem;
}>();

const emit = defineEmits<{
  remove: [taskId: string];
}>();

function stepLabel(code: TaskItem["steps"][number]["code"]): string {
  if (code.type === "add_worktree") {
    return `${t("stepAddWorktree")}: ${code.repository}/${code.branch}`;
  }
  return `${t("stepAgent")}: ${code.repository}/${code.branch}`;
}

function statusLabel(status: TaskItem["status"]): string {
  return t(`status.${status}`);
}
</script>

<template>
  <div class="task-card">
    <div class="task-header">
      <span class="task-prompt" :title="task.prompt">{{ task.prompt }}</span>
      <div class="task-header-right">
        <span
          class="task-badge"
          :class="`badge-${task.status}`"
        >{{ statusLabel(task.status) }}</span>
        <button class="btn-remove" :title="t('removeTitle')" @click="emit('remove', task.id)">
          <i class="pi pi-times" />
        </button>
      </div>
    </div>

    <div class="task-body">
      <!-- 生成中 -->
      <div v-if="task.status === 'generating'" class="generating">
        <i class="pi pi-spinner pi-spin" />
        <span>{{ t('generating') }}</span>
      </div>

      <!-- エラー (ステップなし) -->
      <div v-else-if="task.status === 'error' && task.steps.length === 0" class="error-msg">
        <i class="pi pi-exclamation-circle" />
        <span>{{ task.error ?? t('defaultError') }}</span>
      </div>

      <!-- ステップ一覧 -->
      <ul v-else-if="task.steps.length > 0" class="steps-list">
        <li v-for="(step, i) in task.steps" :key="i" class="step-item" :class="`step-${step.status}`">
          <span class="step-icon">
            <i v-if="step.status === 'done'" class="pi pi-check" />
            <i v-else-if="step.status === 'running'" class="pi pi-spinner pi-spin" />
            <i v-else-if="step.status === 'error'" class="pi pi-times" />
            <i v-else class="pi pi-circle" />
          </span>
          <span class="step-label">{{ stepLabel(step.code) }}</span>
          <span v-if="step.status === 'error' && step.error" class="step-error">{{ step.error }}</span>
        </li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.task-card {
  background: #181825;
  border: 1px solid #313244;
  border-radius: 8px;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.task-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
}

.task-prompt {
  font-size: 13px;
  color: #cdd6f4;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.task-header-right {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.task-badge {
  font-size: 11px;
  padding: 2px 7px;
  border-radius: 10px;
  font-weight: 600;
  white-space: nowrap;
}

.badge-generating {
  background: #45475a;
  color: #a6adc8;
}

.badge-executing {
  background: #1e3a5f;
  color: #89b4fa;
}

.badge-completed {
  background: #1e3a2f;
  color: #a6e3a1;
}

.badge-error {
  background: #3a1e1e;
  color: #f38ba8;
}

.btn-remove {
  background: transparent;
  border: none;
  color: #45475a;
  cursor: pointer;
  padding: 2px 4px;
  font-size: 11px;
  border-radius: 3px;
  line-height: 1;
}

.btn-remove:hover {
  color: #f38ba8;
  background: #313244;
}

.task-body {
  font-size: 12px;
}

.generating {
  display: flex;
  align-items: center;
  gap: 6px;
  color: #a6adc8;
}

.generating .pi-spinner {
  font-size: 12px;
}

.error-msg {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  color: #f38ba8;
}

.steps-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.step-item {
  display: flex;
  align-items: center;
  gap: 6px;
  color: #a6adc8;
}

.step-pending .step-icon { color: #45475a; }
.step-running .step-icon { color: #89b4fa; }
.step-done .step-icon { color: #a6e3a1; }
.step-error .step-icon { color: #f38ba8; }

.step-icon {
  font-size: 11px;
  width: 14px;
  flex-shrink: 0;
}

.step-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.step-error {
  font-size: 11px;
  color: #f38ba8;
  margin-left: 4px;
}
</style>

<i18n lang="json">
{
  "en": {
    "removeTitle": "Remove",
    "generating": "Generating task code...",
    "defaultError": "An error occurred",
    "stepAddWorktree": "Add WT",
    "stepAgent": "Agent",
    "status": {
      "generating": "Generating",
      "executing": "Executing",
      "completed": "Completed",
      "error": "Error"
    }
  },
  "ja": {
    "removeTitle": "削除",
    "generating": "タスク処理コード生成中...",
    "defaultError": "エラーが発生しました",
    "stepAddWorktree": "WT追加",
    "stepAgent": "エージェント",
    "status": {
      "generating": "生成中",
      "executing": "実行中",
      "completed": "完了",
      "error": "エラー"
    }
  }
}
</i18n>
