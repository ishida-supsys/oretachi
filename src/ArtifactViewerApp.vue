<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, message } from "@tauri-apps/plugin-dialog";
import Toast from "primevue/toast";
import ArtifactCodeView from "./components/artifact/ArtifactCodeView.vue";
import ArtifactMarkdownView from "./components/artifact/ArtifactMarkdownView.vue";
import ArtifactHtmlView from "./components/artifact/ArtifactHtmlView.vue";
import ArtifactSvgView from "./components/artifact/ArtifactSvgView.vue";
import ArtifactMermaidView from "./components/artifact/ArtifactMermaidView.vue";
import ArtifactReactView from "./components/artifact/ArtifactReactView.vue";
import ArtifactTableView from "./components/artifact/ArtifactTableView.vue";
import { isTableContentType } from "./utils/csvArtifact";
import type {
  ArtifactMeta,
  ArtifactData,
  ArtifactChangedEvent,
  RepoArtifactChangedEvent,
  CopyArtifactResult,
} from "./types/artifact";

const { t } = useI18n();
const toast = useToast();

const params = new URLSearchParams(window.location.search);
// scope 未指定は従来どおり worktree スコープとして扱う（古い URL 互換）
const scope = params.get("scope") === "repository" ? "repository" : "worktree";
const worktreeId = params.get("worktreeId") ?? "";
const worktreeName = params.get("worktreeName") ?? "";
const repositoryId = params.get("repositoryId") ?? "";
const repositoryName = params.get("repositoryName") ?? "";

const isRepositoryScope = scope === "repository";
const headerTitle = isRepositoryScope ? repositoryName : worktreeName;

const artifacts = ref<ArtifactMeta[]>([]);
const selectedId = ref<string | null>(null);
const selectedArtifact = ref<ArtifactData | null>(null);
const loading = ref(false);
const transferring = ref(false);

let unlisten: UnlistenFn | null = null;

const typeIcons: Record<string, string> = {
  "application/vnd.ant.code": "pi-code",
  "text/markdown": "pi-file-edit",
  "text/html": "pi-globe",
  "image/svg+xml": "pi-image",
  "application/vnd.ant.mermaid": "pi-sitemap",
  "application/vnd.ant.react": "pi-play",
  "text/csv": "pi-table",
  "text/tab-separated-values": "pi-table",
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

// スコープごとの Tauri コマンド差分を吸収する薄いラッパ
function invokeList(): Promise<any[]> {
  return isRepositoryScope
    ? invoke<any[]>("list_repo_artifacts", { repositoryId })
    : invoke<any[]>("list_artifacts", { worktreeId });
}

function invokeRead(artifactId: string): Promise<string> {
  return isRepositoryScope
    ? invoke<string>("read_repo_artifact", { repositoryId, artifactId })
    : invoke<string>("read_artifact", { worktreeId, artifactId });
}

async function loadList() {
  try {
    const list = await invokeList();
    artifacts.value = list.map(mapMeta);
  } catch (e) {
    console.error("list artifacts failed", e);
  }
}

async function selectArtifact(id: string) {
  if (selectedId.value === id && selectedArtifact.value) return;
  selectedId.value = id;
  loading.value = true;
  try {
    const raw = await invokeRead(id);
    selectedArtifact.value = mapArtifact(JSON.parse(raw));
  } catch (e) {
    console.error("read artifact failed", e);
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
    // 同じ ID の再作成（上書き転送）では selectArtifact が早期 return するため、
    // 選択中の本文を捨てて読み直させる
    const isSelected = selectedId.value === artifactId;
    if (isSelected) selectedArtifact.value = null;
    // リポジトリ側は「保管庫を眺める」用途なので、他ウィンドウからの転送で
    // 閲覧中の表示を奪わない。ワークツリー側は生成直後に見せる従来動作を維持する。
    if (isSelected || !isRepositoryScope || selectedId.value === null) {
      await selectArtifact(artifactId);
    }
  } else if (selectedId.value === artifactId) {
    try {
      const raw = await invokeRead(artifactId);
      selectedArtifact.value = mapArtifact(JSON.parse(raw));
    } catch { /* ignore */ }
  }
}

/**
 * 表示中のアーティファクトを、このワークツリーの元リポジトリへコピーする。
 * 転送先はバックエンドが worktreeId から解決するため、ここでは指定しない。
 */
async function transferToRepository() {
  const artifactId = selectedId.value;
  if (!artifactId || transferring.value) return;

  transferring.value = true;
  try {
    let result = await invoke<CopyArtifactResult>("copy_artifact_to_repository", {
      worktreeId,
      artifactId,
      overwrite: false,
    });

    if (result.status === "exists") {
      const confirmed = await ask(
        t("transfer.overwriteConfirm", { repository: result.repositoryName }),
        { title: t("transfer.overwriteTitle"), kind: "warning" },
      );
      if (!confirmed) return;
      result = await invoke<CopyArtifactResult>("copy_artifact_to_repository", {
        worktreeId,
        artifactId,
        overwrite: true,
      });
    }

    toast.add({
      severity: "success",
      summary: t("transfer.done", { repository: result.repositoryName }),
      life: 3000,
    });
  } catch (e) {
    console.error("copy_artifact_to_repository failed", e);
    await message(String(e), { title: t("transfer.failed"), kind: "error" });
  } finally {
    transferring.value = false;
  }
}

/** リポジトリスコープでのみ使う、恒久保存アーティファクトの個別削除 */
async function deleteRepoArtifact() {
  const artifactId = selectedId.value;
  if (!artifactId) return;

  const confirmed = await ask(
    t("delete.confirm", { title: selectedArtifact.value?.title ?? artifactId }),
    { title: t("delete.title"), kind: "warning" },
  );
  if (!confirmed) return;

  try {
    await invoke("delete_repo_artifact", { repositoryId, artifactId });
  } catch (e) {
    console.error("delete_repo_artifact failed", e);
    await message(String(e), { title: t("delete.failed"), kind: "error" });
  }
}

onMounted(async () => {
  await loadList();
  if (artifacts.value.length > 0) {
    await selectArtifact(artifacts.value[0].id);
  }

  if (isRepositoryScope) {
    unlisten = await listen<RepoArtifactChangedEvent>("repo-artifact-changed", async (event) => {
      if (event.payload.repositoryId !== repositoryId) return;
      await refreshSelected(event.payload.artifactId, event.payload.command);
    });
  } else {
    unlisten = await listen<ArtifactChangedEvent>("artifact-changed", async (event) => {
      if (event.payload.worktreeId !== worktreeId) return;
      await refreshSelected(event.payload.artifactId, event.payload.command);
    });
  }
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
        <span :class="isRepositoryScope ? 'pi pi-folder sidebar-icon' : 'pi pi-box sidebar-icon'" />
        <span class="sidebar-title">{{ headerTitle }}</span>
      </div>
      <div class="artifact-list">
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
            <span class="content-type">
              {{ selectedArtifact.content_type }}
              <template v-if="isRepositoryScope && selectedArtifact.source_worktree_id">
                · {{ t("source", { worktreeId: selectedArtifact.source_worktree_id }) }}
              </template>
            </span>
          </div>
          <div class="header-actions">
            <button
              v-if="!isRepositoryScope"
              class="btn-header btn-transfer"
              :disabled="transferring"
              :title="t('transfer.tooltip')"
              @click="transferToRepository"
            >
              <i :class="transferring ? 'pi pi-spin pi-spinner' : 'pi pi-upload'" />
              <span>{{ repositoryName ? t("transfer.labelNamed", { repository: repositoryName }) : t("transfer.label") }}</span>
            </button>
            <button
              v-else
              class="btn-header btn-delete"
              :title="t('delete.title')"
              @click="deleteRepoArtifact"
            >
              <i class="pi pi-trash" />
              <span>{{ t("delete.label") }}</span>
            </button>
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
          <ArtifactReactView
            v-else-if="selectedArtifact.content_type === 'application/vnd.ant.react'"
            :content="selectedArtifact.content"
            :modules="selectedArtifact.modules"
          />
          <ArtifactTableView
            v-else-if="isTableContentType(selectedArtifact.content_type)"
            :key="selectedArtifact.id"
            :content="selectedArtifact.content"
            :content-type="selectedArtifact.content_type"
          />
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

.artifact-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
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
  flex: 1;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.btn-header {
  display: flex;
  align-items: center;
  gap: 6px;
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.btn-header:hover:not(:disabled) {
  background: #45475a;
}

.btn-header:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-transfer {
  border-color: #cba6f7;
  color: #cba6f7;
}

.btn-delete:hover {
  border-color: #f38ba8;
  color: #f38ba8;
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
    "selectPrompt": "Select an artifact to view",
    "source": "from worktree {worktreeId}",
    "transfer": {
      "label": "Transfer to repository",
      "labelNamed": "Transfer to {repository}",
      "tooltip": "Copy this artifact to the repository so it survives worktree deletion",
      "overwriteTitle": "Overwrite?",
      "overwriteConfirm": "An artifact with the same ID already exists in {repository}. Overwrite it?",
      "done": "Transferred to {repository}",
      "failed": "Transfer failed"
    },
    "delete": {
      "label": "Delete",
      "title": "Delete artifact",
      "confirm": "Delete \"{title}\" from this repository?",
      "failed": "Delete failed"
    }
  },
  "ja": {
    "emptyList": "アーティファクトがありません",
    "selectPrompt": "アーティファクトを選択してください",
    "source": "転送元 worktree {worktreeId}",
    "transfer": {
      "label": "リポジトリへ転送",
      "labelNamed": "{repository} へ転送",
      "tooltip": "リポジトリへコピーして、ワークツリー削除後も残るようにします",
      "overwriteTitle": "上書きしますか？",
      "overwriteConfirm": "{repository} に同じ ID のアーティファクトが既にあります。上書きしますか？",
      "done": "{repository} に転送しました",
      "failed": "転送に失敗しました"
    },
    "delete": {
      "label": "削除",
      "title": "アーティファクトの削除",
      "confirm": "「{title}」をこのリポジトリから削除しますか？",
      "failed": "削除に失敗しました"
    }
  }
}
</i18n>
