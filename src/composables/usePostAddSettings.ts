import { ref } from "vue";
import { useSettings } from "./useSettings";
import type { NotificationHookEntry } from "../types/settings";

export function usePostAddSettings() {
  const { settings, scheduleSave } = useSettings();

  const showCopyDialog = ref(false);
  const copyDialogRepoId = ref("");
  const copyDialogRepoPath = ref("");
  const copyDialogCurrentTargets = ref<string[]>([]);
  const copyDialogCurrentPM = ref<string | undefined>(undefined);
  const copyDialogCurrentPMArgs = ref<string | undefined>(undefined);
  const copyDialogCurrentHooks = ref<NotificationHookEntry[]>([]);

  function openCopyDialog(repoId: string) {
    const repo = settings.value.repositories.find((r) => r.id === repoId);
    if (!repo) return;
    copyDialogRepoId.value = repoId;
    copyDialogRepoPath.value = repo.path;
    copyDialogCurrentTargets.value = repo.copyTargets ?? [];
    copyDialogCurrentPM.value = repo.packageManager;
    copyDialogCurrentPMArgs.value = repo.packageManagerArgs;
    copyDialogCurrentHooks.value = repo.notificationHooks ?? [];
    showCopyDialog.value = true;
  }

  function onDialogConfirm(
    targets: string[],
    packageManager: string | undefined,
    packageManagerArgs: string | undefined,
    notificationHooks: NotificationHookEntry[],
  ) {
    const repo = settings.value.repositories.find((r) => r.id === copyDialogRepoId.value);
    if (!repo) return;
    repo.copyTargets = targets.length > 0 ? targets : undefined;
    repo.packageManager = packageManager;
    repo.packageManagerArgs = packageManagerArgs || undefined;
    repo.notificationHooks = notificationHooks.length > 0 ? notificationHooks : undefined;
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
    copyDialogCurrentPMArgs,
    copyDialogCurrentHooks,
    openCopyDialog,
    onDialogConfirm,
    clearCopyTargets,
  };
}
