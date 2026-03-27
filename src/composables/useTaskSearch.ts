import { ref } from "vue";

export function useTaskSearch(search: (query: string) => Promise<void>) {
  const taskSearchInput = ref("");
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  function onSearchInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      search(taskSearchInput.value);
    }, 300);
  }

  function clearSearch() {
    taskSearchInput.value = "";
    search("");
  }

  return { taskSearchInput, onSearchInput, clearSearch };
}
