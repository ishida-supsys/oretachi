<script setup lang="ts">
import "./monaco-workers";
import { ref, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";
import Toast from "primevue/toast";
import FileTreePanel from "./components/codereview/FileTreePanel.vue";
import GitPanel from "./components/codereview/GitPanel.vue";
import CodeReviewTabs from "./components/codereview/CodeReviewTabs.vue";
import MonacoFileViewer from "./components/codereview/MonacoFileViewer.vue";
import MonacoDiffViewer from "./components/codereview/MonacoDiffViewer.vue";
import ReviewSessionView from "./components/codereview/ReviewSessionView.vue";
import CommitDiffView from "./components/codereview/CommitDiffView.vue";
import CodeReviewSettingsView from "./components/codereview/CodeReviewSettingsView.vue";
import { useCodeReviewTabs } from "./composables/useCodeReviewTabs";
import { useReviewSession } from "./composables/useReviewSession";
import { useSettings } from "./composables/useSettings";
import { useCodeReviewSettings } from "./composables/useCodeReviewSettings";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCodeReviewChat, type CodeReviewOrigin } from "./composables/useCodeReviewLineChat";

const { t } = useI18n();
const toast = useToast();

const params = new URLSearchParams(window.location.search);
const worktreeId = params.get("worktreeId") ?? "";
const worktreePath = params.get("worktreePath") ?? "";
const worktreeName = params.get("worktreeName") ?? "";
const origin = (params.get("origin") as CodeReviewOrigin | null) ?? "main";

type Panel = "files" | "git";
const activePanel = ref<Panel>("files");

const sidebarWidth = ref(250);
const isResizing = ref(false);

const { tabs, activeTabId, openFileTab, openDiffTab, openReviewTab, closeReviewTab, openCommitDiffTab, openSettingsTab, closeTab, switchTab, activeTab, updateFileTab, updateDiffTab, getOpenTabs } =
  useCodeReviewTabs();

const { isReviewMode, startReview, endReview, refreshReviewFiles } = useReviewSession();
const { loadSettings } = useSettings();
const { resolved: crSettings } = useCodeReviewSettings();

function handleStartReview() {
  startReview(worktreePath);
  openReviewTab();
}

function handleCloseTab(id: string) {
  const tab = tabs.find((t) => t.id === id);
  if (tab?.type === "review") {
    endReview();
  }
  closeTab(id);
}

function handleSelectCommit(payload: { hash: string; shortHash: string; message: string }) {
  openCommitDiffTab(payload.hash, payload.shortHash, payload.message);
}

function handleReviewClose() {
  endReview();
  closeReviewTab();
  emit("codereview-fs-changed");
}

// ─── サイドバーリサイズ ───────────────────────────────────────────────────────
function startResize(e: MouseEvent) {
  isResizing.value = true;
  const startX = e.clientX;
  const startWidth = sidebarWidth.value;

  function onMove(ev: MouseEvent) {
    const delta = ev.clientX - startX;
    sidebarWidth.value = Math.max(160, Math.min(600, startWidth + delta));
  }
  function onUp() {
    isResizing.value = false;
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
  }
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}

// ─── ファイルを開く ──────────────────────────────────────────────────────────
async function handleOpenFile(filePath: string) {
  try {
    const content = await invoke<string>("git_read_file", {
      repoPath: worktreePath,
      filePath,
      revision: null,
    });
    const ext = filePath.split(".").pop() ?? "";
    openFileTab(filePath, content, ext);
  } catch (e) {
    toast.add({ severity: "error", summary: t("error.fileOpen"), detail: String(e), life: 4000 });
  }
}

// ─── Diff を開く ────────────────────────────────────────────────────────────
async function handleOpenDiff(payload: { filePath: string; staged: boolean }) {
  try {
    const diff = await invoke<{ old_content: string; new_content: string; is_binary: boolean }>("git_get_file_diff", {
      repoPath: worktreePath,
      filePath: payload.filePath,
      staged: payload.staged,
    });
    openDiffTab(payload.filePath, diff.old_content, diff.new_content, payload.staged, diff.is_binary);
  } catch (e) {
    toast.add({ severity: "error", summary: t("error.diffOpen"), detail: String(e), life: 4000 });
  }
}

// ─── タブ自動リフレッシュ ─────────────────────────────────────────────────────
async function refreshOpenTabs() {
  const openTabs = getOpenTabs();
  await Promise.allSettled(
    openTabs.map(async (tab) => {
      if (tab.type === "file") {
        try {
          const content = await invoke<string>("git_read_file", {
            repoPath: worktreePath,
            filePath: tab.filePath,
            revision: null,
          });
          updateFileTab(tab.filePath, content);
        } catch { /* 無視 */ }
      } else {
        try {
          const diff = await invoke<{ old_content: string; new_content: string }>("git_get_file_diff", {
            repoPath: worktreePath,
            filePath: tab.filePath,
            staged: tab.staged,
          });
          updateDiffTab(tab.filePath, tab.staged!, diff.old_content, diff.new_content);
        } catch { /* 無視 */ }
      }
    }),
  );
}

let refreshTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleRefresh() {
  if (refreshTimer !== null) clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    refreshOpenTabs();
    if (isReviewMode.value) {
      refreshReviewFiles(worktreePath);
    }
  }, 300);
}

let unlistenFsChanged: (() => void) | null = null;

onMounted(async () => {
  document.title = `Code Review - ${worktreeName}`;

  await loadSettings();

  // FS ウォッチャー起動
  if (worktreeId && worktreePath) {
    try {
      await invoke("start_fs_watch", { worktreeId, worktreePath });
      unlistenFsChanged = await listen("codereview-fs-changed", scheduleRefresh);
    } catch (e) {
      console.warn("start_fs_watch failed:", e);
    }
  }

  // 自動レビュータブオープン
  if (worktreePath && crSettings.value.autoOpenReviewOnDiff) {
    try {
      const statusEntries = await invoke<{ path: string }[]>("git_get_status", { repoPath: worktreePath });
      if (statusEntries.length > 0) {
        handleStartReview();
      }
    } catch {
      // 無視
    }
  }

  // ウィンドウフォーカス時にも再取得（フォールバック）
  window.addEventListener("focus", scheduleRefresh);

  const appWindow = getCurrentWindow();
  appWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    cleanup();
    await appWindow.destroy();
  });
});

function cleanup() {
  if (refreshTimer !== null) {
    clearTimeout(refreshTimer);
    refreshTimer = null;
  }
  window.removeEventListener("focus", scheduleRefresh);
  unlistenFsChanged?.();
  unlistenFsChanged = null;
  if (worktreeId) {
    invoke("stop_fs_watch", { worktreeId }).catch(() => {});
  }
}

onUnmounted(cleanup);

const { handleChatWithAgent } = useCodeReviewChat(worktreeId, origin);
</script>

<template>
  <Toast />
  <div class="flex h-screen w-screen overflow-hidden bg-surface-900 text-surface-100">
    <!-- アイコンメニューバー -->
    <div class="flex flex-col items-center gap-3 w-12 bg-surface-950 py-3 shrink-0">
      <button
        class="p-1 rounded hover:bg-surface-700 transition-colors"
        :class="activePanel === 'files' ? 'text-primary-400' : 'text-surface-400'"
        :title="t('panel.files')"
        @click="activePanel = 'files'"
      >
        <i class="pi pi-folder text-lg" />
      </button>
      <button
        class="p-1 rounded hover:bg-surface-700 transition-colors"
        :class="activePanel === 'git' ? 'text-primary-400' : 'text-surface-400'"
        :title="t('panel.git')"
        @click="activePanel = 'git'"
      >
        <i class="pi pi-share-alt text-lg" />
      </button>
      <div class="flex-1" />
      <button
        class="p-1 rounded hover:bg-surface-700 transition-colors"
        :class="activeTab?.type === 'settings' ? 'text-primary-400' : 'text-surface-400'"
        :title="t('panel.settings')"
        @click="openSettingsTab()"
      >
        <i class="pi pi-cog text-lg" />
      </button>
    </div>

    <!-- サイドバー -->
    <div
      class="flex flex-col overflow-hidden shrink-0 bg-surface-800 border-r border-surface-700"
      :style="{ width: sidebarWidth + 'px' }"
    >
      <div class="px-3 py-2 text-xs font-semibold text-surface-400 uppercase tracking-wider border-b border-surface-700 shrink-0">
        {{ activePanel === "files" ? t("panel.files") : t("panel.git") }}
      </div>
      <div class="flex-1 overflow-hidden">
        <FileTreePanel
          v-if="activePanel === 'files'"
          :repo-path="worktreePath"
          @open-file="handleOpenFile"
        />
        <GitPanel
          v-else
          :repo-path="worktreePath"
          @open-diff="handleOpenDiff"
          @start-review="handleStartReview"
          @select-commit="handleSelectCommit"
        />
      </div>
    </div>

    <!-- リサイズハンドル -->
    <div
      class="w-1 cursor-col-resize hover:bg-primary-500 transition-colors shrink-0"
      :class="isResizing ? 'bg-primary-500' : 'bg-surface-700'"
      @mousedown="startResize"
    />

    <!-- メインエリア -->
    <div class="flex-1 flex flex-col overflow-hidden">
      <CodeReviewTabs
        :tabs="tabs"
        :active-tab-id="activeTabId"
        @switch="switchTab"
        @close="handleCloseTab"
      />
      <div class="flex-1 overflow-hidden">
        <template v-if="activeTab">
          <CodeReviewSettingsView
            v-if="activeTab.type === 'settings'"
          />
          <ReviewSessionView
            v-else-if="activeTab.type === 'review'"
            :repo-path="worktreePath"
            @close="handleReviewClose"
            @chat="handleChatWithAgent"
          />
          <CommitDiffView
            v-else-if="activeTab.type === 'commit-diff'"
            :repo-path="worktreePath"
            :commit-hash="activeTab.commitHash!"
            @chat="handleChatWithAgent"
          />
          <MonacoFileViewer
            v-else-if="activeTab.type === 'file'"
            :content="activeTab.content ?? ''"
            :language="activeTab.language"
            :file-path="activeTab.filePath"
            @chat="handleChatWithAgent"
          />
          <div
            v-else-if="activeTab.isBinary"
            class="flex items-center justify-center h-full text-surface-500 text-sm"
          >
            <i class="pi pi-ban mr-2" />{{ t("editor.binaryFile") }}
          </div>
          <MonacoDiffViewer
            v-else
            :old-content="activeTab.oldContent ?? ''"
            :new-content="activeTab.newContent ?? ''"
            :file-path="activeTab.filePath.replace(/:(?:staged|unstaged)$/, '')"
            @chat="handleChatWithAgent"
          />
        </template>
        <div
          v-else
          class="flex items-center justify-center h-full text-surface-500 text-sm"
        >
          {{ t("editor.noFileOpen") }}
        </div>
      </div>
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "panel": { "files": "Files", "git": "Git", "settings": "Settings" },
    "editor": { "noFileOpen": "Select a file to view", "binaryFile": "Binary file (diff not shown)" },
    "error": { "fileOpen": "Failed to open file", "diffOpen": "Failed to open diff" }
  },
  "ja": {
    "panel": { "files": "ファイル", "git": "Git", "settings": "設定" },
    "editor": { "noFileOpen": "ファイルを選択してください", "binaryFile": "バイナリファイル（差分を表示できません）" },
    "error": { "fileOpen": "ファイルを開けませんでした", "diffOpen": "Diffを開けませんでした" }
  }
}
</i18n>
