import { ref } from "vue";

export interface TabItem {
  id: number;
  title: string;
}

let counter = 0;

export function useTabs() {
  const tabs = ref<TabItem[]>([]);
  const activeTabId = ref<number | null>(null);

  function addTab(): number {
    counter++;
    const id = counter;
    tabs.value.push({ id, title: `Tab ${id}` });
    activeTabId.value = id;
    return id;
  }

  function removeTab(id: number) {
    const index = tabs.value.findIndex((t) => t.id === id);
    if (index === -1) return;

    tabs.value.splice(index, 1);

    if (tabs.value.length === 0) {
      // 最後のタブが閉じられた場合は新タブを自動作成
      addTab();
      return;
    }

    if (activeTabId.value === id) {
      // 閉じたタブがアクティブだった場合、隣のタブをアクティブにする
      const newIndex = Math.min(index, tabs.value.length - 1);
      activeTabId.value = tabs.value[newIndex].id;
    }
  }

  function setActiveTab(id: number) {
    activeTabId.value = id;
  }

  return { tabs, activeTabId, addTab, removeTab, setActiveTab };
}
