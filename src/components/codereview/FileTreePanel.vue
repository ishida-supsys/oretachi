<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import Tree from "primevue/tree";
import type { TreeNode } from "primevue/treenode";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{ repoPath: string }>();
const emit = defineEmits<{ (e: "open-file", filePath: string): void }>();

const nodes = ref<TreeNode[]>([]);
const selectedKey = ref<Record<string, boolean>>({});
const loading = ref(false);
const error = ref("");

function buildTree(files: string[]): TreeNode[] {
  const root: Map<string, TreeNode> = new Map();

  for (const file of files) {
    const parts = file.split("/");
    let currentMap = root;
    let currentPath = "";

    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      currentPath = currentPath ? `${currentPath}/${part}` : part;
      const isFile = i === parts.length - 1;

      if (!currentMap.has(part)) {
        const node: TreeNode = {
          key: currentPath,
          label: part,
          icon: isFile ? "pi pi-file" : "pi pi-folder",
          data: isFile ? currentPath : null,
          leaf: isFile,
          children: isFile ? undefined : [],
        };
        currentMap.set(part, node);
      }

      if (!isFile) {
        const parentNode = currentMap.get(part)!;
        // 子ノード用 Map を作成（再利用）
        if (!parentNode._childMap) {
          (parentNode as any)._childMap = new Map<string, TreeNode>();
          parentNode.children = [];
        }
        currentMap = (parentNode as any)._childMap;
      }
    }
  }

  // Map を配列に変換（ディレクトリ優先ソート）
  function mapToNodes(map: Map<string, TreeNode>): TreeNode[] {
    const arr = Array.from(map.values());
    arr.sort((a, b) => {
      if (!a.leaf && b.leaf) return -1;
      if (a.leaf && !b.leaf) return 1;
      return (a.label ?? "").localeCompare(b.label ?? "");
    });
    for (const node of arr) {
      if (!node.leaf && (node as any)._childMap) {
        node.children = mapToNodes((node as any)._childMap);
        delete (node as any)._childMap;
      }
    }
    return arr;
  }

  return mapToNodes(root);
}

async function loadFiles() {
  if (!props.repoPath) return;
  loading.value = true;
  error.value = "";
  try {
    const files = await invoke<string[]>("git_list_files", { repoPath: props.repoPath });
    nodes.value = buildTree(files);
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

function onNodeSelect(node: TreeNode) {
  if (node.leaf && node.data) {
    emit("open-file", node.data as string);
  }
}

onMounted(loadFiles);
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
      selection-mode="single"
      class="text-sm w-full border-none bg-transparent"
      @node-select="onNodeSelect"
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
