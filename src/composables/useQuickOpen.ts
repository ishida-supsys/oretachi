import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export function useQuickOpen(repoPath: string) {
  const isOpen = ref(false);
  const fileCache = ref<string[] | null>(null);
  const isLoading = ref(false);

  async function loadFiles(): Promise<string[]> {
    if (fileCache.value !== null) return fileCache.value;
    isLoading.value = true;
    try {
      const files = await invoke<string[]>("git_list_files", { repoPath });
      fileCache.value = files;
      return files;
    } finally {
      isLoading.value = false;
    }
  }

  function invalidateCache() {
    fileCache.value = null;
  }

  function open() {
    isOpen.value = true;
  }

  function close() {
    isOpen.value = false;
  }

  function toggle() {
    if (isOpen.value) {
      close();
    } else {
      open();
    }
  }

  let unlisten: (() => void) | null = null;
  onMounted(async () => {
    unlisten = await listen("codereview-fs-changed", invalidateCache);
  });
  onUnmounted(() => {
    unlisten?.();
  });

  return { isOpen, fileCache, isLoading, loadFiles, open, close, toggle, invalidateCache };
}
