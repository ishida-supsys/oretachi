import { ref, computed, nextTick } from "vue";
import { SearchAddon } from "@xterm/addon-search";
import type { Terminal } from "@xterm/xterm";
import { i18n } from "../i18n";

const SEARCH_DECORATIONS = {
  matchBackground: "#585b70",
  matchBorder: "#585b70",
  matchOverviewRuler: "#585b70",
  activeMatchBackground: "#f9e2af",
  activeMatchBorder: "#f9e2af",
  activeMatchColorOverviewRuler: "#f9e2af",
} as const;

export function useTerminalSearch(getTerminal: () => Terminal | null) {
  let searchAddon: SearchAddon | null = null;

  const showSearchBar = ref(false);
  const searchQuery = ref("");
  const searchResultIndex = ref(-1);
  const searchResultCount = ref(0);
  const searchInputRef = ref<HTMLInputElement | null>(null);

  const searchCountText = computed(() => {
    const t = i18n.global.t;
    if (!searchQuery.value) return "";
    if (searchResultCount.value === 0) return t("search.noResults");
    if (searchResultIndex.value < 0) return t("search.results", { count: searchResultCount.value });
    return t("search.position", { current: searchResultIndex.value + 1, total: searchResultCount.value });
  });

  function loadAddon(terminal: Terminal): void {
    searchAddon = new SearchAddon();
    terminal.loadAddon(searchAddon);
  }

  function dispose(): void {
    searchAddon?.dispose();
    searchAddon = null;
  }

  function toggleSearchBar(): void {
    if (showSearchBar.value) {
      closeSearchBar();
    } else {
      showSearchBar.value = true;
      nextTick(() => {
        searchInputRef.value?.focus();
      });
    }
  }

  function closeSearchBar(): void {
    showSearchBar.value = false;
    searchQuery.value = "";
    searchResultIndex.value = -1;
    searchResultCount.value = 0;
    searchAddon?.clearDecorations();
    getTerminal()?.focus();
  }

  function findNext(): void {
    if (!searchAddon || !searchQuery.value) return;
    const found = searchAddon.findNext(searchQuery.value, {
      decorations: SEARCH_DECORATIONS,
      incremental: false,
    });
    if (!found) {
      searchResultCount.value = 0;
    }
  }

  function findPrevious(): void {
    if (!searchAddon || !searchQuery.value) return;
    searchAddon.findPrevious(searchQuery.value, {
      decorations: SEARCH_DECORATIONS,
      incremental: false,
    });
  }

  function onSearchInput(): void {
    if (!searchAddon) return;
    if (!searchQuery.value) {
      searchAddon.clearDecorations();
      searchResultCount.value = 0;
      searchResultIndex.value = -1;
      return;
    }
    searchAddon.findNext(searchQuery.value, {
      decorations: SEARCH_DECORATIONS,
      incremental: true,
    });
  }

  function onSearchKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      closeSearchBar();
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (event.shiftKey) {
        findPrevious();
      } else {
        findNext();
      }
    }
  }

  return {
    showSearchBar,
    searchQuery,
    searchCountText,
    searchInputRef,
    loadAddon,
    dispose,
    toggleSearchBar,
    closeSearchBar,
    findNext,
    findPrevious,
    onSearchInput,
    onSearchKeydown,
  };
}
