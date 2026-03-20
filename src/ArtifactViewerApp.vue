<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Toast from "primevue/toast";
import ArtifactCodeView from "./components/artifact/ArtifactCodeView.vue";
import ArtifactMarkdownView from "./components/artifact/ArtifactMarkdownView.vue";
import ArtifactHtmlView from "./components/artifact/ArtifactHtmlView.vue";
import ArtifactSvgView from "./components/artifact/ArtifactSvgView.vue";
import ArtifactMermaidView from "./components/artifact/ArtifactMermaidView.vue";
import type { ArtifactMeta, ArtifactData, ArtifactChangedEvent } from "./types/artifact";

const { t } = useI18n();

const params = new URLSearchParams(window.location.search);
const worktreeId = params.get("worktreeId") ?? "";
const worktreeName = params.get("worktreeName") ?? "";

const artifacts = ref<ArtifactMeta[]>([]);
const selectedId = ref<string | null>(null);
const selectedArtifact = ref<ArtifactData | null>(null);
const loading = ref(false);

let unlisten: UnlistenFn | null = null;

const typeIcons: Record<string, string> = {
  "application/vnd.ant.code": "pi-code",
  "text/markdown": "pi-file-edit",
  "text/html": "pi-globe",
  "image/svg+xml": "pi-image",
  "application/vnd.ant.mermaid": "pi-sitemap",
  "application/vnd.ant.react": "pi-code",
};

function typeIcon(contentType: string): string {
  return typeIcons[contentType] ?? "pi-file";
}

function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleString();
}

// JSONの "type" フィールドを content_type にマッピングする
// (Rust側は serde(rename="type") でJSONに保存するため)
function mapMeta(raw: any): ArtifactMeta {
  return { ...raw, content_type: raw.type ?? raw.content_type };
}
function mapArtifact(raw: any): ArtifactData {
  return { ...raw, content_type: raw.type ?? raw.content_type };
}

async function loadList() {
  try {
    const list = await invoke<any[]>("list_artifacts", { worktreeId });
    artifacts.value = list.map(mapMeta);
  } catch (e) {
    console.error("list_artifacts failed", e);
  }
}

async function selectArtifact(id: string) {
  if (selectedId.value === id && selectedArtifact.value) return;
  selectedId.value = id;
  loading.value = true;
  try {
    const raw = await invoke<string>("read_artifact", { worktreeId, artifactId: id });
    selectedArtifact.value = mapArtifact(JSON.parse(raw));
  } catch (e) {
    console.error("read_artifact failed", e);
    selectedArtifact.value = null;
  } finally {
    loading.value = false;
  }
}

async function refreshSelected(artifactId: string, command: string) {
  await loadList();
  if (command === "delete") {
    if (selectedId.value === artifactId) {
      selectedId.value = null;
      selectedArtifact.value = null;
      if (artifacts.value.length > 0) {
        await selectArtifact(artifacts.value[0].id);
      }
    }
  } else if (command === "create") {
    await selectArtifact(artifactId);
  } else if (selectedId.value === artifactId) {
    try {
      const raw = await invoke<string>("read_artifact", { worktreeId, artifactId });
      selectedArtifact.value = mapArtifact(JSON.parse(raw));
    } catch { /* ignore */ }
  }
}

onMounted(async () => {
  await loadList();
  if (artifacts.value.length > 0) {
    await selectArtifact(artifacts.value[0].id);
  }

  unlisten = await listen<ArtifactChangedEvent>("artifact-changed", async (event) => {
    if (event.payload.worktreeId !== worktreeId) return;
    await refreshSelected(event.payload.artifactId, event.payload.command);
  });
});

onUnmounted(() => {
  unlisten?.();
});
</script>

<template>
  <div class="artifact-viewer">
    <Toast />
    <div class="sidebar">
      <div class="sidebar-header">
        <span class="pi pi-box sidebar-icon" />
        <span class="sidebar-title">{{ worktreeName }}</span>
      </div>
      <div v-if="artifacts.length === 0" class="empty-list">
        {{ t("emptyList") }}
      </div>
      <div
        v-for="artifact in artifacts"
        :key="artifact.id"
        class="artifact-item"
        :class="{ selected: selectedId === artifact.id }"
        @click="selectArtifact(artifact.id)"
      >
        <span :class="`pi ${typeIcon(artifact.content_type)} artifact-icon`" />
        <div class="artifact-item-info">
          <span class="artifact-title">{{ artifact.title }}</span>
          <span class="artifact-meta">{{ formatDate(artifact.updated_at) }}</span>
        </div>
      </div>
    </div>

    <div class="main-content">
      <div v-if="!selectedArtifact && !loading" class="empty-main">
        <span class="pi pi-box empty-icon" />
        <span>{{ t("selectPrompt") }}</span>
      </div>

      <div v-else-if="loading" class="loading-main">
        <span class="pi pi-spin pi-spinner" />
      </div>

      <template v-else-if="selectedArtifact">
        <div class="content-header">
          <span :class="`pi ${typeIcon(selectedArtifact.content_type)} type-icon`" />
          <div class="content-title-area">
            <span class="content-title">{{ selectedArtifact.title }}</span>
            <span class="content-type">{{ selectedArtifact.content_type }}</span>
          </div>
        </div>
        <div class="content-body">
          <ArtifactCodeView
            v-if="selectedArtifact.content_type === 'application/vnd.ant.code'"
            :content="selectedArtifact.content"
            :language="selectedArtifact.language"
          />
          <ArtifactMarkdownView
            v-else-if="selectedArtifact.content_type === 'text/markdown'"
            :content="selectedArtifact.content"
          />
          <ArtifactHtmlView
            v-else-if="selectedArtifact.content_type === 'text/html'"
            :content="selectedArtifact.content"
          />
          <ArtifactSvgView
            v-else-if="selectedArtifact.content_type === 'image/svg+xml'"
            :content="selectedArtifact.content"
          />
          <ArtifactMermaidView
            v-else-if="selectedArtifact.content_type === 'application/vnd.ant.mermaid'"
            :content="selectedArtifact.content"
          />
          <!-- application/vnd.ant.react やその他はコードとして表示 -->
          <ArtifactCodeView
            v-else
            :content="selectedArtifact.content"
            :language="selectedArtifact.language"
          />
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.artifact-viewer {
  display: flex;
  height: 100vh;
  background: #1e1e2e;
  color: #cdd6f4;
  font-family: sans-serif;
  overflow: hidden;
}

/* ── サイドバー ── */
.sidebar {
  width: 240px;
  min-width: 180px;
  max-width: 320px;
  background: #181825;
  border-right: 1px solid #313244;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.sidebar-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 14px 14px 10px;
  border-bottom: 1px solid #313244;
  font-weight: 600;
  font-size: 13px;
}

.sidebar-icon {
  color: #cba6f7;
  font-size: 14px;
}

.sidebar-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: #cdd6f4;
}

.empty-list {
  padding: 16px;
  font-size: 12px;
  color: #6c7086;
  text-align: center;
}

.artifact-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 10px 12px;
  cursor: pointer;
  border-bottom: 1px solid #1e1e2e;
  transition: background 0.12s;
}

.artifact-item:hover {
  background: #313244;
}

.artifact-item.selected {
  background: #313244;
  border-left: 2px solid #cba6f7;
  padding-left: 10px;
}

.artifact-icon {
  color: #89b4fa;
  font-size: 13px;
  margin-top: 2px;
  flex-shrink: 0;
}

.artifact-item-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.artifact-title {
  font-size: 13px;
  color: #cdd6f4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.artifact-meta {
  font-size: 10px;
  color: #6c7086;
}

/* ── メイン領域 ── */
.main-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.empty-main,
.loading-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: #6c7086;
  font-size: 13px;
}

.empty-icon {
  font-size: 32px;
  color: #45475a;
}

.content-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid #313244;
  background: #181825;
  flex-shrink: 0;
}

.type-icon {
  color: #89b4fa;
  font-size: 16px;
}

.content-title-area {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.content-title {
  font-size: 14px;
  font-weight: 600;
  color: #cdd6f4;
}

.content-type {
  font-size: 11px;
  color: #6c7086;
  font-family: monospace;
}

.content-body {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
</style>

<i18n lang="json">
{
  "en": {
    "emptyList": "No artifacts",
    "selectPrompt": "Select an artifact to view"
  },
  "ja": {
    "emptyList": "アーティファクトがありません",
    "selectPrompt": "アーティファクトを選択してください"
  }
}
</i18n>
