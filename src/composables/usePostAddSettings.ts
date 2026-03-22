import { ref } from "vue";
import { useSettings } from "./useSettings";

export function usePostAddSettings() {
  const { settings, scheduleSave } = useSettings();

  const showCopyDialog = ref(false);
  const copyDialogRepoId = ref("");
  const copyDialogRepoPath = ref("");
  const copyDialogCurrentTargets = ref<string[]>([]);
  const copyDialogCurrentPM = ref<string | undefined>(undefined);

  function openCopyDialog(repoId: string) {
    const repo = settings.value.repositories.find((r) => r.id === repoId);
    if (!repo) return;
    copyDialogRepoId.value = repoId;
    copyDialogRepoPath.value = repo.path;
    copyDialogCurrentTargets.value = repo.copyTargets ?? [];
    copyDialogCurrentPM.value = repo.packageManager;
    showCopyDialog.value = true;
  }

  function onDialogConfirm(targets: string[], packageManager: string | undefined) {
    const repo = settings.value.repositories.find((r) => r.id === copyDialogRepoId.value);
    if (!repo) return;
    repo.copyTargets = targets.length > 0 ? targets : undefined;
    repo.packageManager = packageManager;
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
    copyDialogCurrentPM,
    openCopyDialog,
    onDialogConfirm,
    clearCopyTargets,
  };
}
