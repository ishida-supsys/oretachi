<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { open, message } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { useSettings } from "../composables/useSettings";
import { useRepositoryActions } from "../composables/useRepositoryActions";
import { usePostAddSettings } from "../composables/usePostAddSettings";
import { useArtifactWindow } from "../composables/useArtifactWindow";
import { useMasonryLayout } from "../composables/useMasonryLayout";
import { computeNaturalCardWidth } from "../utils/cardWidth";
import { isHomeWorktree } from "../utils/homeWorktree";
import { isRepositoryWorktree, makeRepositoryWorktreeId } from "../utils/repositoryWorktree";
import { extractUrlArtifacts } from "../utils/artifactUrl";
import type { RepoArtifactChangedEvent, UrlArtifactEntry } from "../types/artifact";
import type { Repository } from "../types/settings";
import type { Worktree } from "../types/worktree";
import PostAddSettingsDialog from "./PostAddSettingsDialog.vue";
import RepositoryCard from "./RepositoryCard.vue";
import WorktreeCard from "./WorktreeCard.vue";

const { t } = useI18n();
const { settings, scheduleSave } = useSettings();
const {
  showCopyDialog,
  copyDialogRepoPath,
  copyDialogCurrentTargets,
  copyDialogCurrentPM,
  copyDialogCurrentPMArgs,
  copyDialogCurrentHooks,
  copyDialogCurrentPullBeforeAdd,
  copyDialogCurrentBranchNamePattern,
  openCopyDialog,
  onDialogConfirm,
} = usePostAddSettings();

const { addRepository: addRepositoryAction } = useRepositoryActions();
const { openRepositoryArtifactViewer } = useArtifactWindow();

const props = defineProps<{
  worktrees: Worktree[];
  thumbnailUrls: Map<number, string>;
  /** ワークツリー単位のアーティファクト件数（ホームカード用） */
  artifactCounts: Map<string, number>;
  /** ワークツリー単位の URL アーティファクト（ホームカード用） */
  artifactUrls: Map<string, UrlArtifactEntry[]>;
  notifications: Map<string, number>;
  hotkeyChars: Map<string, string>;
  detachedWorktrees: Set<string>;
  autoApprovals: Map<string, boolean>;
  aiJudgingWorktrees: Set<string>;
  /** ホームカードの description / タスク一覧（ワークツリー一覧と同じ内容を出す） */
  cardTooltips?: Map<string, string | undefined>;
  descriptionOpens?: Map<string, boolean>;
  showAllDescriptions?: boolean;
}>();

const emit = defineEmits<{
  selectTerminal: [terminalId: number];
  addTerminal: [worktreeId: string];
  openInIde: [worktreeId: string];
  openArtifacts: [worktreeId: string];
  moveToSubWindow: [worktreeId: string];
  moveToMainWindow: [worktreeId: string];
  focusSubWindow: [worktreeId: string];
  setHotkeyChar: [worktreeId: string];
  toggleAutoApproval: [worktreeId: string];
  toggleDescription: [worktreeId: string];
  cancelAiJudging: [worktreeId: string];
  removeRepository: [repositoryId: string];
}>();

/** リポジトリ ID → 恒久保存アーティファクト件数 */
const repoArtifactCounts = ref(new Map<string, number>());
/** リポジトリ ID → 恒久保存の URL アーティファクト */
const repoArtifactUrls = ref(new Map<string, UrlArtifactEntry[]>());
let unlisten: UnlistenFn | null = null;

async function refreshArtifactCount(repoId: string) {
  try {
    const list = await invoke<unknown[]>("list_repo_artifacts", { repositoryId: repoId });
    repoArtifactCounts.value.set(repoId, list.length);
    repoArtifactUrls.value.set(repoId, extractUrlArtifacts(list));
    // Map の変異は追跡されないため、再代入して再描画させる
    repoArtifactCounts.value = new Map(repoArtifactCounts.value);
    repoArtifactUrls.value = new Map(repoArtifactUrls.value);
  } catch {
    /* 件数バッジは補助情報なので失敗しても無視 */
  }
}

async function refreshAllArtifactCounts() {
  await Promise.all(settings.value.repositories.map((r) => refreshArtifactCount(r.id)));
}

async function addRepository() {
  const result = await addRepositoryAction();
  if (result === "notARepo") {
    await message(t("error.notARepo"), { kind: "error" });
  } else if (result === "alreadyRegistered") {
    await message(t("error.alreadyRegistered"), { kind: "warning" });
  } else if (result === "added") {
    await refreshAllArtifactCounts();
  }
}

/**
 * 配下にワークツリーがあるか。
 * リポジトリ擬似ワークツリーは repositoryId が自分自身を指すので数えない
 * （数えると登録解除が永久にできなくなる）。
 */
function hasWorktrees(repoId: string): boolean {
  return props.worktrees.some((w) => !isRepositoryWorktree(w) && w.repositoryId === repoId);
}

async function openArtifacts(repoId: string) {
  const repo = settings.value.repositories.find((r) => r.id === repoId);
  if (!repo) return;
  await openRepositoryArtifactViewer(repo.id, repo.name);
}

async function selectExecScript(repoId: string) {
  const selected = await open({
    multiple: false,
    filters: [{ name: "Scripts", extensions: ["ps1", "sh"] }],
  });
  if (typeof selected !== "string") return;

  const repo = settings.value.repositories.find((r) => r.id === repoId);
  if (!repo) return;

  repo.execScript = selected;
  scheduleSave();
}

function clearExecScript(repoId: string) {
  const repo = settings.value.repositories.find((r) => r.id === repoId);
  if (!repo) return;

  repo.execScript = undefined;
  scheduleSave();
}

// ─── 一覧（ホームカード + リポジトリカード） ────────────────────────────────

type RepoListItem =
  | { key: string; kind: "home"; worktree: Worktree }
  | { key: string; kind: "repo"; repo: Repository; worktree?: Worktree };

const homeWorktree = computed(() => props.worktrees.find(isHomeWorktree));

/** ホームを先頭に、以降は settings.repositories の順で並べる */
const listItems = computed<RepoListItem[]>(() => {
  const byId = new Map(props.worktrees.map((w) => [w.id, w]));
  const items: RepoListItem[] = [];
  const home = homeWorktree.value;
  if (home) items.push({ key: home.id, kind: "home", worktree: home });
  for (const repo of settings.value.repositories) {
    items.push({
      key: repo.id,
      kind: "repo",
      repo,
      worktree: byId.get(makeRepositoryWorktreeId(repo.id)),
    });
  }
  return items;
});

const cardWidth = computed(() =>
  computeNaturalCardWidth(listItems.value.map((item) => item.worktree?.terminals.length ?? 0)),
);

// 並べ替え・削除アニメーションは持たない一覧なので、auto-animate / FLIP は使わない
const { containerRef, columns } = useMasonryLayout(listItems, { minColumnWidth: cardWidth, gap: 12 });

let disposed = false;

onMounted(async () => {
  // 件数取得を待ってから listen すると、その間に unmount された場合に
  // unlisten が呼ばれずリスナが残り続けるため、先に登録する
  const fn = await listen<RepoArtifactChangedEvent>("repo-artifact-changed", async (event) => {
    await refreshArtifactCount(event.payload.repositoryId);
  });
  if (disposed) {
    fn();
    return;
  }
  unlisten = fn;
  await refreshAllArtifactCounts();
});

onUnmounted(() => {
  disposed = true;
  unlisten?.();
  unlisten = null;
});

// ホームタブのヘッダーにある「+ 追加」ボタンから呼ばれる
defineExpose({ addRepository });
</script>

<template>
  <div class="repo-panel">
    <div ref="containerRef" class="repo-list">
      <div
        v-for="(col, colIndex) in columns"
        :key="colIndex"
        class="masonry-column"
        :style="{ maxWidth: cardWidth + 'px' }"
      >
        <template v-for="item in col" :key="item.key">
          <!-- ホームカードは WorktreeCard の isHome 分岐をそのまま使う -->
          <WorktreeCard
            v-if="item.kind === 'home'"
            :worktree="item.worktree"
            :thumbnail-urls="thumbnailUrls"
            :detached="detachedWorktrees.has(item.worktree.id)"
            :notification-count="notifications.get(item.worktree.id) ?? 0"
            :hotkey-char="hotkeyChars.get(item.worktree.id)"
            :artifact-count="artifactCounts.get(item.worktree.id) ?? 0"
            :artifact-urls="artifactUrls.get(item.worktree.id) ?? []"
            :auto-approval="autoApprovals.get(item.worktree.id) ?? false"
            :ai-judging="aiJudgingWorktrees.has(item.worktree.id)"
            :tooltip="cardTooltips?.get(item.worktree.id)"
            :description-open="showAllDescriptions || (descriptionOpens?.get(item.worktree.id) ?? false)"
            @toggle-description="emit('toggleDescription', $event)"
            @cancel-ai-judging="emit('cancelAiJudging', $event)"
            @select-terminal="emit('selectTerminal', $event)"
            @add-terminal="emit('addTerminal', $event)"
            @open-in-ide="emit('openInIde', $event)"
            @open-artifacts="emit('openArtifacts', $event)"
            @move-to-sub-window="emit('moveToSubWindow', $event)"
            @move-to-main-window="emit('moveToMainWindow', $event)"
            @focus-sub-window="emit('focusSubWindow', $event)"
            @set-hotkey-char="emit('setHotkeyChar', $event)"
            @toggle-auto-approval="emit('toggleAutoApproval', $event)"
          />
          <RepositoryCard
            v-else
            :repo="item.repo"
            :worktree="item.worktree"
            :thumbnail-urls="thumbnailUrls"
            :artifact-count="repoArtifactCounts.get(item.repo.id) ?? 0"
            :artifact-urls="repoArtifactUrls.get(item.repo.id) ?? []"
            :notification-count="item.worktree ? notifications.get(item.worktree.id) ?? 0 : 0"
            :hotkey-char="item.worktree ? hotkeyChars.get(item.worktree.id) : undefined"
            :detached="item.worktree ? detachedWorktrees.has(item.worktree.id) : false"
            :auto-approval="item.worktree ? autoApprovals.get(item.worktree.id) ?? false : false"
            :has-worktrees="hasWorktrees(item.repo.id)"
            @select-terminal="emit('selectTerminal', $event)"
            @add-terminal="emit('addTerminal', $event)"
            @open-in-ide="emit('openInIde', $event)"
            @open-artifacts="openArtifacts"
            @move-to-sub-window="emit('moveToSubWindow', $event)"
            @move-to-main-window="emit('moveToMainWindow', $event)"
            @focus-sub-window="emit('focusSubWindow', $event)"
            @set-hotkey-char="emit('setHotkeyChar', $event)"
            @toggle-auto-approval="emit('toggleAutoApproval', $event)"
            @configure-post-add="openCopyDialog"
            @select-exec-script="selectExecScript"
            @clear-exec-script="clearExecScript"
            @remove-repository="emit('removeRepository', $event)"
          />
        </template>
      </div>
    </div>

    <div v-if="settings.repositories.length === 0" class="empty-state">
      {{ t("repositories.empty") }}
    </div>

    <PostAddSettingsDialog
      v-if="showCopyDialog"
      :repo-path="copyDialogRepoPath"
      :current-targets="copyDialogCurrentTargets"
      :current-package-manager="copyDialogCurrentPM"
      :current-package-manager-args="copyDialogCurrentPMArgs"
      :current-notification-hooks="copyDialogCurrentHooks"
      :current-pull-before-add="copyDialogCurrentPullBeforeAdd"
      :current-branch-name-pattern="copyDialogCurrentBranchNamePattern"
      @confirm="onDialogConfirm"
      @cancel="showCopyDialog = false"
    />
  </div>
</template>

<style scoped>
.repo-panel {
  display: flex;
  flex-direction: column;
}

/* ワークツリー一覧 (.worktree-list) と同じ masonry レイアウト */
.repo-list {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  align-items: flex-start;
}

.masonry-column {
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex: 1 1 0;
  min-width: 0;
}

.empty-state {
  padding: 16px;
  text-align: center;
  color: #6c7086;
  font-size: 13px;
}
</style>

<i18n lang="json">
{
  "en": {
    "repositories": {
      "empty": "No repositories registered"
    },
    "error": {
      "notARepo": "The selected folder is not a git repository.",
      "alreadyRegistered": "This repository is already registered."
    }
  },
  "ja": {
    "repositories": {
      "empty": "リポジトリが登録されていません"
    },
    "error": {
      "notARepo": "選択したフォルダは git リポジトリではありません。",
      "alreadyRegistered": "このリポジトリはすでに登録されています。"
    }
  }
}
</i18n>
