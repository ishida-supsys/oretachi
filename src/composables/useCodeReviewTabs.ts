import { ref, reactive } from "vue";

export interface CodeReviewTab {
  id: string;
  type: "file" | "diff" | "review";
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

  function openReviewTab(): void {
    const existing = tabs.find((t) => t.type === "review");
    if (existing) {
      activeTabId.value = existing.id;
      return;
    }
    const id = `tab-${++tabCounter}`;
    tabs.push({ id, type: "review", label: "Review Session", filePath: "" });
    activeTabId.value = id;
  }

  function closeReviewTab(): void {
    const tab = tabs.find((t) => t.type === "review");
    if (tab) closeTab(tab.id);
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

  function updateFileTab(filePath: string, content: string): void {
    const tab = tabs.find((t) => t.type === "file" && t.filePath === filePath);
    if (tab) {
      tab.content = content;
    }
  }

  function updateDiffTab(filePath: string, staged: boolean, oldContent: string, newContent: string): void {
    const key = `${filePath}:${staged ? "staged" : "unstaged"}`;
    const tab = tabs.find((t) => t.type === "diff" && t.filePath === key);
    if (tab) {
      tab.oldContent = oldContent;
      tab.newContent = newContent;
    }
  }

  function getOpenTabs(): Array<{ filePath: string; type: "file" | "diff"; staged?: boolean }> {
    return tabs.map((t) => {
      if (t.type === "file") {
        return { filePath: t.filePath, type: "file" as const };
      } else {
        const colonIdx = t.filePath.lastIndexOf(":");
        const actualPath = t.filePath.slice(0, colonIdx);
        const staged = t.filePath.slice(colonIdx + 1) === "staged";
        return { filePath: actualPath, type: "diff" as const, staged };
      }
    });
  }

  return { tabs, activeTabId, openFileTab, openDiffTab, openReviewTab, closeReviewTab, closeTab, switchTab, activeTab, updateFileTab, updateDiffTab, getOpenTabs };
}
