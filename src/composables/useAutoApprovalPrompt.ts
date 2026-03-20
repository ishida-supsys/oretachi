import { reactive, ref } from "vue";
import { emitTo } from "@tauri-apps/api/event";
import type { AppSettings } from "../types/settings";
import type { Ref } from "vue";

export function useAutoApprovalPrompt(
  settings: Ref<AppSettings>,
  scheduleSave: () => void,
  isDetached: (worktreeId: string) => boolean,
) {
  // ワークツリー ID → 追加プロンプト
  const autoApprovalPromptMap = reactive(new Map<string, string>());

  // ワークツリー ID → 直前のAI判定コマンド
  const lastJudgedCommandMap = reactive(new Map<string, string>());

  // ダイアログ状態
  const showAutoApprovalPromptDialog = ref(false);
  const autoApprovalPromptTargetId = ref("");

  /** 設定ファイルから追加プロンプトを復元 */
  function restoreFromSettings() {
    for (const wt of settings.value.worktrees) {
      if (wt.autoApprovalPrompt) {
        autoApprovalPromptMap.set(wt.id, wt.autoApprovalPrompt);
      }
    }
  }

  /** 自動承認バッジクリック → ダイアログを開く */
  function onClickAutoApproval(worktreeId: string) {
    autoApprovalPromptTargetId.value = worktreeId;
    showAutoApprovalPromptDialog.value = true;
  }

  /** 追加プロンプト保存 */
  async function onSaveAutoApprovalPrompt(worktreeId: string, prompt: string) {
    const trimmed = prompt.trim();
    if (trimmed) {
      autoApprovalPromptMap.set(worktreeId, trimmed);
    } else {
      autoApprovalPromptMap.delete(worktreeId);
    }

    const wtEntry = settings.value.worktrees.find((w) => w.id === worktreeId);
    if (wtEntry) {
      wtEntry.autoApprovalPrompt = trimmed || undefined;
      scheduleSave();
    }

    if (isDetached(worktreeId)) {
      await emitTo(`sub-${worktreeId}`, "sub-set-auto-approval-prompt", { prompt: trimmed });
    }

    showAutoApprovalPromptDialog.value = false;
  }

  return {
    autoApprovalPromptMap,
    lastJudgedCommandMap,
    showAutoApprovalPromptDialog,
    autoApprovalPromptTargetId,
    restoreFromSettings,
    onClickAutoApproval,
    onSaveAutoApprovalPrompt,
  };
}
