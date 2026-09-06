<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, message } from "@tauri-apps/plugin-dialog";
import Toast from "primevue/toast";
import Popover from "primevue/popover";
import ArtifactCodeView from "./components/artifact/ArtifactCodeView.vue";
import ArtifactMarkdownView from "./components/artifact/ArtifactMarkdownView.vue";
import ArtifactHtmlView from "./components/artifact/ArtifactHtmlView.vue";
import ArtifactSvgView from "./components/artifact/ArtifactSvgView.vue";
import ArtifactMermaidView from "./components/artifact/ArtifactMermaidView.vue";
import ArtifactReactView from "./components/artifact/ArtifactReactView.vue";
import ArtifactTableView from "./components/artifact/ArtifactTableView.vue";
import ArtifactUrlView from "./components/artifact/ArtifactUrlView.vue";
import { isTableContentType } from "./utils/csvArtifact";
import {
  sortArtifacts,
  filterArtifacts,
  pushHistory as pushHistoryEntry,
  pruneHistory as pruneHistoryEntries,
  canGoBack as historyCanGoBack,
  canGoForward as historyCanGoForward,
  createHistory,
} from "./utils/artifactList";
import { URL_ARTIFACT_CONTENT_TYPE } from "./types/artifact";
import type {
  ArtifactMeta,
  ArtifactData,
  ArtifactState,
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

// 状態サイドカーは両スコープ共通のコマンドを使うので、スコープ指定をここで固定しておく
const stateScope = isRepositoryScope ? "repository" : "worktree";
const stateScopeId = isRepositoryScope ? repositoryId : worktreeId;

const artifacts = ref<ArtifactMeta[]>([]);
const states = ref<Record<string, ArtifactState>>({});
const searchQuery = ref("");
const selectedId = ref<string | null>(null);
const selectedArtifact = ref<ArtifactData | null>(null);
const loading = ref(false);
const transferring = ref(false);
const menuRef = ref<InstanceType<typeof Popover> | null>(null);
/** ピン止め更新が飛んでいる最中の ID（連打による UI とディスクの食い違いを防ぐ） */
const pinningIds = ref<Set<string>>(new Set());

// 選択履歴はこのウィンドウ内に閉じたスタック。別ウィンドウの遷移とは連結しない
// （連結すると「戻る」で別ウィンドウにフォーカスが飛ぶ挙動になる）
const history = ref(createHistory());

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
  [URL_ARTIFACT_CONTENT_TYPE]: "pi-link",
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
    pruneHistory();
  } catch (e) {
    console.error("list artifacts failed", e);
  }
}

async function loadStates() {
  try {
    states.value = await invoke<Record<string, ArtifactState>>("list_artifact_states", {
      scope: stateScope,
      scopeId: stateScopeId,
    });
  } catch (e) {
    // サイドカーは補助情報なので、読めなくても一覧の表示は続ける
    console.error("list_artifact_states failed", e);
  }
}

function isPinned(id: string): boolean {
  return states.value[id]?.pinned === true;
}

const sortedArtifacts = computed(() => sortArtifacts(artifacts.value, isPinned));
const visibleArtifacts = computed(() => filterArtifacts(sortedArtifacts.value, searchQuery.value));

async function togglePin(artifactId: string) {
  // 連打で pinned=true / false が同時に飛ぶと、楽観更新した UI とディスクが食い違う。
  // Rust 側でも直列化しているが、ここで弾いておかないと最後の応答が勝つとは限らない
  if (pinningIds.value.has(artifactId)) return;

  const pinned = !isPinned(artifactId);
  const previous = states.value[artifactId];
  // 先に反映して即座に並び替える（失敗したらこの ID だけ元の値へ戻す）
  states.value = { ...states.value, [artifactId]: { ...previous, pinned } };
  pinningIds.value = new Set(pinningIds.value).add(artifactId);
  try {
    await invoke("set_artifact_pinned", {
      scope: stateScope,
      scopeId: stateScopeId,
      artifactId,
      pinned,
    });
  } catch (e) {
    console.error("set_artifact_pinned failed", e);
    // 他の ID の未確定な楽観更新を巻き添えにしないよう、失敗した 1 キーだけ差し戻す
    const rolledBack = { ...states.value };
    if (previous) {
      rolledBack[artifactId] = previous;
    } else {
      delete rolledBack[artifactId];
    }
    states.value = rolledBack;
  } finally {
    const next = new Set(pinningIds.value);
    next.delete(artifactId);
    pinningIds.value = next;
  }
}

// ─── 選択履歴 ───────────────────────────────────────────────────────────────

const canGoBack = computed(() => historyCanGoBack(history.value));
const canGoForward = computed(() => historyCanGoForward(history.value));

/** 削除済み ID を履歴から取り除き、戻る / 進むがそこで止まらないようにする */
function pruneHistory() {
  history.value = pruneHistoryEntries(history.value, artifacts.value.map((a) => a.id));
}

async function goBack() {
  if (!canGoBack.value) return;
  const index = history.value.index - 1;
  history.value = { ...history.value, index };
  await selectArtifact(history.value.entries[index], false);
}

async function goForward() {
  if (!canGoForward.value) return;
  const index = history.value.index + 1;
  history.value = { ...history.value, index };
  await selectArtifact(history.value.entries[index], false);
}

async function selectArtifact(id: string, push = true) {
  if (selectedId.value === id && selectedArtifact.value) return;
  if (push) history.value = pushHistoryEntry(history.value, id);
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
    // 本体と一緒にサイドカーも消えるので、ピン止め状態を読み直す
    await loadStates();
    if (selectedId.value === artifactId) {
      selectedId.value = null;
      selectedArtifact.value = null;
      // 直前の選択が履歴に残っていればそこへ戻る（タブを閉じたときと同じ感覚）。
      // 履歴のカーソルが指す先をそのまま表示するので、積み直さない
      const fallback = history.value.entries[history.value.index];
      if (fallback) {
        await selectArtifact(fallback, false);
      } else {
        // 戻り先が無いので一覧の先頭を新しい起点にする。絞り込み中は絞り込み結果から
        // 選び、サイドバーが「一致なし」なのに本文だけ出る食い違いを避ける
        const next = visibleArtifacts.value[0];
        if (next) await selectArtifact(next.id);
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

/**
 * ワークツリースコープの個別削除。
 * 一覧の更新と再選択は `artifact-changed`(command=delete) 経由の既存ロジックに任せる。
 */
async function deleteWorktreeArtifact() {
  const artifactId = selectedId.value;
  if (!artifactId) return;

  const confirmed = await ask(
    t("delete.confirmWorktree", { title: selectedArtifact.value?.title ?? artifactId }),
    { title: t("delete.title"), kind: "warning" },
  );
  if (!confirmed) return;

  try {
    await invoke("delete_artifact", { worktreeId, artifactId });
  } catch (e) {
    console.error("delete_artifact failed", e);
    await message(String(e), { title: t("delete.failed"), kind: "error" });
  }
}

function withMenuHidden<T>(fn: () => T): T {
  menuRef.value?.hide();
  return fn();
}

onMounted(async () => {
  await loadList();
  await loadStates();
  if (sortedArtifacts.value.length > 0) {
    await selectArtifact(sortedArtifacts.value[0].id);
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
      <div class="sidebar-search">
        <span class="pi pi-search search-icon" />
        <input
          v-model="searchQuery"
          class="search-input"
          type="text"
          :placeholder="t('search.placeholder')"
        />
        <button
          v-if="searchQuery"
          class="search-clear"
          :title="t('search.clear')"
          @click="searchQuery = ''"
        >
          <i class="pi pi-times" />
        </button>
      </div>
      <div class="artifact-list">
        <div v-if="artifacts.length === 0" class="empty-list">
          {{ t("emptyList") }}
        </div>
        <div v-else-if="visibleArtifacts.length === 0" class="empty-list">
          {{ t("search.noMatch") }}
        </div>
        <div
          v-for="artifact in visibleArtifacts"
          :key="artifact.id"
          class="artifact-item"
          :class="{ selected: selectedId === artifact.id, pinned: isPinned(artifact.id) }"
          @click="selectArtifact(artifact.id)"
        >
          <span :class="`pi ${typeIcon(artifact.content_type)} artifact-icon`" />
          <div class="artifact-item-info">
            <span class="artifact-title">{{ artifact.title }}</span>
            <span class="artifact-meta">{{ formatDate(artifact.updated_at) }}</span>
          </div>
          <button
            class="pin-button"
            :class="{ active: isPinned(artifact.id) }"
            :title="isPinned(artifact.id) ? t('pin.unpin') : t('pin.pin')"
            @click.stop="togglePin(artifact.id)"
          >
            <i :class="isPinned(artifact.id) ? 'pi pi-star-fill' : 'pi pi-star'" />
          </button>
        </div>
      </div>
    </div>

    <div class="main-content">
      <!--
        ヘッダーとメニューは読み込み中・未選択でも出しっぱなしにする。
        本文と一緒に v-if で出し入れすると、遷移のたびに戻る / 進むボタンが
        アンマウントされて点滅し、読み込みに失敗すると押せなくなる。
      -->
      <div class="content-header">
        <div class="nav-buttons">
          <button
            class="btn-nav"
            :disabled="!canGoBack"
            :title="t('nav.back')"
            @click="goBack"
          >
            <i class="pi pi-arrow-left" />
          </button>
          <button
            class="btn-nav"
            :disabled="!canGoForward"
            :title="t('nav.forward')"
            @click="goForward"
          >
            <i class="pi pi-arrow-right" />
          </button>
        </div>
        <template v-if="selectedArtifact">
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
              class="btn-header"
              :title="t('menu.tooltip')"
              @click="menuRef?.toggle($event)"
            >
              <i :class="transferring ? 'pi pi-spin pi-spinner' : 'pi pi-ellipsis-h'" />
              <span>{{ t("menu.label") }}</span>
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
        </template>
      </div>

      <Popover v-if="!isRepositoryScope" ref="menuRef">
        <div class="popup-menu">
          <button
            class="popup-item"
            :disabled="transferring"
            :title="t('transfer.tooltip')"
            @click="withMenuHidden(transferToRepository)"
          >
            <span class="pi pi-upload" />
            {{ repositoryName ? t("transfer.labelNamed", { repository: repositoryName }) : t("transfer.label") }}
          </button>
          <div class="popup-divider" />
          <button
            class="popup-item popup-item-danger"
            @click="withMenuHidden(deleteWorktreeArtifact)"
          >
            <span class="pi pi-trash" />
            {{ t("delete.label") }}
          </button>
        </div>
      </Popover>

      <div v-if="!selectedArtifact && !loading" class="empty-main">
        <span class="pi pi-box empty-icon" />
        <span>{{ t("selectPrompt") }}</span>
      </div>

      <div v-else-if="loading" class="loading-main">
        <span class="pi pi-spin pi-spinner" />
      </div>

      <template v-else-if="selectedArtifact">
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
          <ArtifactUrlView
            v-else-if="selectedArtifact.content_type === URL_ARTIFACT_CONTENT_TYPE"
            :content="selectedArtifact.content"
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

.sidebar-search {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 10px;
  padding: 5px 8px;
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 4px;
}

.sidebar-search:focus-within {
  border-color: #cba6f7;
}

.search-icon {
  color: #6c7086;
  font-size: 11px;
  flex-shrink: 0;
}

.search-input {
  flex: 1;
  min-width: 0;
  background: none;
  border: none;
  outline: none;
  color: #cdd6f4;
  font-size: 12px;
  font-family: inherit;
}

.search-input::placeholder {
  color: #6c7086;
}

.search-clear {
  background: none;
  border: none;
  padding: 0;
  color: #6c7086;
  font-size: 10px;
  cursor: pointer;
  flex-shrink: 0;
}

.search-clear:hover {
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
  flex: 1;
}

.pin-button {
  background: none;
  border: none;
  padding: 0 2px;
  color: #6c7086;
  font-size: 11px;
  cursor: pointer;
  flex-shrink: 0;
  margin-top: 2px;
  /* ピン止め済み以外はホバーするまで出さず、一覧のノイズを増やさない */
  visibility: hidden;
}

.artifact-item:hover .pin-button,
.pin-button.active {
  visibility: visible;
}

.pin-button:hover {
  color: #cdd6f4;
}

.pin-button.active {
  color: #f9e2af;
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

.btn-delete:hover {
  border-color: #f38ba8;
  color: #f38ba8;
}

.nav-buttons {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

.btn-nav {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  background: none;
  border: none;
  border-radius: 4px;
  color: #cdd6f4;
  font-size: 12px;
  cursor: pointer;
}

.btn-nav:hover:not(:disabled) {
  background: #313244;
}

.btn-nav:disabled {
  opacity: 0.3;
  cursor: not-allowed;
}

/* ── ヘッダーメニュー（他カードのポップアップメニューと見た目を揃える） ── */
.popup-menu {
  display: flex;
  flex-direction: column;
  min-width: 200px;
}

.popup-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  background: none;
  border: none;
  color: var(--p-text-color);
  font-size: 13px;
  cursor: pointer;
  border-radius: 4px;
  text-align: left;
  width: 100%;
}

.popup-item:hover:not(:disabled) {
  background: var(--p-content-hover-background);
}

.popup-item:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.popup-item-danger {
  color: var(--p-red-400);
}

.popup-item-danger:not(:disabled):hover {
  background: color-mix(in srgb, var(--p-red-400) 15%, transparent);
}

.popup-divider {
  height: 1px;
  background: var(--p-content-border-color);
  margin: 4px 0;
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
    "search": {
      "placeholder": "Filter by title / type",
      "clear": "Clear filter",
      "noMatch": "No matching artifacts"
    },
    "pin": {
      "pin": "Pin to top",
      "unpin": "Unpin"
    },
    "nav": {
      "back": "Back",
      "forward": "Forward"
    },
    "menu": {
      "label": "Actions",
      "tooltip": "Transfer or delete this artifact"
    },
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
      "confirmWorktree": "Delete \"{title}\" from this worktree? Transferred copies in the repository are not affected.",
      "failed": "Delete failed"
    }
  },
  "ja": {
    "emptyList": "アーティファクトがありません",
    "selectPrompt": "アーティファクトを選択してください",
    "source": "転送元 worktree {worktreeId}",
    "search": {
      "placeholder": "タイトル / 種別で絞り込み",
      "clear": "絞り込みを解除",
      "noMatch": "一致するアーティファクトがありません"
    },
    "pin": {
      "pin": "ピン止めする",
      "unpin": "ピン止めを外す"
    },
    "nav": {
      "back": "戻る",
      "forward": "進む"
    },
    "menu": {
      "label": "操作",
      "tooltip": "このアーティファクトを転送 / 削除する"
    },
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
      "confirmWorktree": "「{title}」をこのワークツリーから削除しますか？ リポジトリへ転送済みのコピーは残ります。",
      "failed": "削除に失敗しました"
    }
  }
}
</i18n>
