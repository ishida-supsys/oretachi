import { ref, reactive } from "vue";

export interface CodeReviewTab {
  id: string;
  type: "file" | "diff";
  label: string;
  filePath: string;
  content?: string;
  oldContent?: string;
  newContent?: string;
  language?: string;
}

let tabCounter = 0;

export function useCodeReviewTabs() {
  const tabs = reactive<CodeReviewTab[]>([]);
  const activeTabId = ref<string>("");

  function openFileTab(filePath: string, content: string, language?: string): void {
    const existing = tabs.find((t) => t.type === "file" && t.filePath === filePath);
    if (existing) {
      activeTabId.value = existing.id;
      return;
    }
    const id = `tab-${++tabCounter}`;
    const label = filePath.split("/").pop() ?? filePath;
    tabs.push({ id, type: "file", label, filePath, content, language });
    activeTabId.value = id;
  }

  function openDiffTab(filePath: string, oldContent: string, newContent: string, staged: boolean): void {
    const key = `${filePath}:${staged ? "staged" : "unstaged"}`;
    const existing = tabs.find((t) => t.type === "diff" && t.filePath === key);
    if (existing) {
      activeTabId.value = existing.id;
      return;
    }
    const id = `tab-${++tabCounter}`;
    const label = `${filePath.split("/").pop() ?? filePath} (${staged ? "staged" : "changes"})`;
    tabs.push({ id, type: "diff", label, filePath: key, oldContent, newContent });
    activeTabId.value = id;
  }

  function closeTab(id: string): void {
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx === -1) return;
    tabs.splice(idx, 1);
    if (activeTabId.value === id) {
      activeTabId.value = tabs[Math.max(0, idx - 1)]?.id ?? "";
    }
  }

  function switchTab(id: string): void {
    activeTabId.value = id;
  }

  const activeTab = () => tabs.find((t) => t.id === activeTabId.value);

  return { tabs, activeTabId, openFileTab, openDiffTab, closeTab, switchTab, activeTab };
}
