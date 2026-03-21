import { ref } from "vue";
import { useSettings } from "./useSettings";

export function usePostAddSettings() {
  const { settings, scheduleSave } = useSettings();

  const showCopyDialog = ref(false);
  const copyDialogRepoId = ref("");
  const copyDialogRepoPath = ref("");
  const copyDialogCurrentTargets = ref<string[]>([]);

  function openCopyDialog(repoId: string) {
    const repo = settings.value.repositories.find((r) => r.id === repoId);
    if (!repo) return;
    copyDialogRepoId.value = repoId;
    copyDialogRepoPath.value = repo.path;
    copyDialogCurrentTargets.value = repo.copyTargets ?? [];
    showCopyDialog.value = true;
  }

  function onCopyDialogConfirm(targets: string[]) {
    const repo = settings.value.repositories.find((r) => r.id === copyDialogRepoId.value);
    if (!repo) return;
    repo.copyTargets = targets.length > 0 ? targets : undefined;
    scheduleSave();
    showCopyDialog.value = false;
  }

  function clearCopyTargets(repoId: string) {
    const repo = settings.value.repositories.find((r) => r.id === repoId);
    if (!repo) return;
    repo.copyTargets = undefined;
    scheduleSave();
  }

  return {
    showCopyDialog,
    copyDialogRepoPath,
    copyDialogCurrentTargets,
    openCopyDialog,
    onCopyDialogConfirm,
    clearCopyTargets,
  };
}
