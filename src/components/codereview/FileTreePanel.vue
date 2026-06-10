<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Tree from "primevue/tree";
import type { TreeNode } from "primevue/treenode";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{ repoPath: string }>();
const emit = defineEmits<{ (e: "open-file", filePath: string): void }>();

interface DirEntry {
  name: string;
  path: string;
  isDir: boolean;
}

const nodes = ref<TreeNode[]>([]);
const selectedKey = ref<Record<string, boolean>>({});
const expandedKeys = ref<Record<string, boolean>>({});
const loading = ref(false);
const error = ref("");

function toNode(entry: DirEntry): TreeNode {
  if (entry.isDir) {
    // children を未設定にしておき、展開時に遅延取得する
    return {
      key: entry.path,
      label: entry.name,
      icon: "pi pi-folder",
      data: null,
      leaf: false,
      loading: false,
    };
  }
  return {
    key: entry.path,
    label: entry.name,
    icon: "pi pi-file",
    data: entry.path,
    leaf: true,
  };
}

async function fetchEntries(relPath: string): Promise<TreeNode[]> {
  const entries = await invoke<DirEntry[]>("list_dir_entries", {
    repoPath: props.repoPath,
    relPath,
  });
  return entries.map(toNode);
}

async function loadRoot() {
  if (!props.repoPath) return;
  loading.value = true;
  error.value = "";
  try {
    nodes.value = await fetchEntries("");
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function onNodeExpand(node: TreeNode) {
  // ディレクトリ以外、ロード済み、またはロード中ならスキップ（再入による二重フェッチ防止）
  if (node.leaf || Array.isArray(node.children) || node.loading) return;
  node.loading = true;
  try {
    node.children = await fetchEntries(node.key as string);
  } catch {
    node.children = [];
  } finally {
    node.loading = false;
  }
}

function findNode(list: TreeNode[], key: string): TreeNode | null {
  for (const n of list) {
    if (n.key === key) return n;
    if (Array.isArray(n.children)) {
      const found = findNode(n.children, key);
      if (found) return found;
    }
  }
  return null;
}

// FS 変更時: ルートを再取得し、展開中だったディレクトリを浅い順に再ロードして展開状態を保つ
async function refresh() {
  const expanded = Object.keys(expandedKeys.value).filter((k) => expandedKeys.value[k]);
  await loadRoot();
  expanded.sort((a, b) => a.split("/").length - b.split("/").length);
  for (const key of expanded) {
    const node = findNode(nodes.value, key);
    if (node && !node.leaf) await onNodeExpand(node);
  }
}

function onNodeSelect(node: TreeNode) {
  if (node.leaf && node.data) {
    emit("open-file", node.data as string);
  }
}

let unlistenFsChanged: (() => void) | null = null;
let debounceTimer: ReturnType<typeof setTimeout> | null = null;

onMounted(async () => {
  loadRoot();
  unlistenFsChanged = await listen("codereview-fs-changed", () => {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      refresh();
    }, 500);
  });
});

onUnmounted(() => {
  if (debounceTimer !== null) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
  }
  unlistenFsChanged?.();
});
</script>

<template>
  <div class="h-full overflow-auto p-1">
    <div v-if="loading" class="flex items-center justify-center p-4 text-surface-400 text-sm">
      <i class="pi pi-spin pi-spinner mr-2" />{{ t("loading") }}
    </div>
    <div v-else-if="error" class="p-3 text-red-400 text-xs">{{ error }}</div>
    <Tree
      v-else
      :value="nodes"
      v-model:selection-keys="selectedKey"
      v-model:expanded-keys="expandedKeys"
      selection-mode="single"
      loading-mode="icon"
      class="text-sm w-full border-none bg-transparent"
      @node-select="onNodeSelect"
      @node-expand="onNodeExpand"
    />
  </div>
</template>

<style scoped>
:deep(.p-tree-node-children) {
  padding-left: 1rem;
}
</style>

<i18n lang="json">
{
  "en": { "loading": "Loading files..." },
  "ja": { "loading": "ファイルを読み込み中..." }
}
</i18n>
