<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{
  worktreeName: string;
  branchName: string;
  /** 未コミットファイル件数（0 なら警告非表示） */
  dirtyCount: number;
  /** 表示中が最後の1件か。確定ボタンのラベル切替に使う */
  isLast: boolean;
}>();

const emit = defineEmits<{
  confirm: [options: { deleteBranch: boolean }];
  cancel: [];
}>();

// マージ先の指定は持たない（トレイは高速レビュー動線。マージ先を選びたいときは
// メインの削除ダイアログでやる）。ブランチ削除は常に OFF から始める。
const deleteBranch = ref(false);

// トレイは狭くオーバーレイの余白が小さいため、クリック以外の離脱手段を用意する。
// TrayPopupApp の useHotkeyListener が window capture で先に発火するため、
// こちらも capture フェーズで受けて stopPropagation する。
function onKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  event.preventDefault();
  event.stopPropagation();
  emit("cancel");
}

onMounted(() => window.addEventListener("keydown", onKeydown, true));
onUnmounted(() => window.removeEventListener("keydown", onKeydown, true));
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title', { name: props.worktreeName }) }}</h3>

      <div class="branch-info">
        <span class="pi pi-code-branch" style="font-size: 10px" />
        <span class="branch-name">{{ props.branchName }}</span>
      </div>

      <label class="checkbox-label">
        <input v-model="deleteBranch" type="checkbox" />
        {{ t('deleteBranch') }}
      </label>

      <p v-if="props.dirtyCount > 0" class="warn warn-dirty">
        {{ t('dirtyFilesWarning', { count: props.dirtyCount }) }}
      </p>
      <p class="warn">{{ t('removeWarning') }}</p>
      <p v-if="deleteBranch" class="warn warn-force">{{ t('forceDeleteWarning') }}</p>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.cancel') }}</button>
        <button class="btn-danger" @click="emit('confirm', { deleteBranch })">
          {{ props.isLast ? t('confirmDone') : t('confirmNext') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* スタイルは RemoveWorktreeDialog.vue に準拠。ただしトレイは高さが小さいことがあるため
   全体をコンパクトにし、ダイアログ自体をスクロール可能にしている */
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 200;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 8px;
  padding: 16px;
  width: 340px;
  max-width: calc(100vw - 32px);
  max-height: calc(100vh - 24px);
  overflow-y: auto;
  box-sizing: border-box;
}

.dialog-title {
  font-size: 14px;
  font-weight: 600;
  color: #f38ba8;
  margin: 0 0 10px;
  overflow-wrap: anywhere;
}

.branch-info {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 12px;
  font-size: 12px;
  color: #9399b2;
}

.branch-name {
  font-family: monospace;
  color: #cdd6f4;
  background: #313244;
  padding: 1px 6px;
  border-radius: 4px;
  overflow-wrap: anywhere;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: #cdd6f4;
  cursor: pointer;
  user-select: none;
  margin-bottom: 12px;
}

.checkbox-label input[type="checkbox"] {
  cursor: pointer;
  accent-color: #f38ba8;
}

/* .warn を先に定義してから警告の種類ごとに色を上書きする（同詳細度のため後勝ち） */
.warn {
  font-size: 11px;
  color: #f9e2af;
  margin: 0 0 6px;
  line-height: 1.5;
}

.warn-dirty {
  color: #fab387;
}

.warn-force {
  color: #f38ba8;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 10px;
}

.btn-cancel {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  cursor: pointer;
}

.btn-cancel:hover {
  background: #45475a;
}

.btn-danger {
  background: #f38ba8;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.btn-danger:hover {
  background: #eb6f8e;
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Archive \"{name}\"",
    "deleteBranch": "Delete branch",
    "removeWarning": "⚠ The worktree will be saved to the archive and removed (git worktree remove)",
    "forceDeleteWarning": "⚠ No merge target: git branch -D will force-delete",
    "dirtyFilesWarning": "⚠ {count} uncommitted file(s) will be lost",
    "confirmNext": "Archive & next",
    "confirmDone": "Archive & done"
  },
  "ja": {
    "title": "「{name}」をアーカイブ",
    "deleteBranch": "ブランチを削除する",
    "removeWarning": "⚠ アーカイブに保存して git worktree remove が実行されます",
    "forceDeleteWarning": "⚠ マージ先未指定のため git branch -D で強制削除されます",
    "dirtyFilesWarning": "⚠ {count} 件の未コミットファイルが失われます",
    "confirmNext": "アーカイブ化して次へ",
    "confirmDone": "アーカイブ化して完了"
  }
}
</i18n>
