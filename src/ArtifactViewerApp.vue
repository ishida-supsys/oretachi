<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
import { sortArtifacts, filterArtifacts } from "./utils/artifactList";
import { parseArtifactLink } from "./utils/artifactLink";
import {
  useArtifactWindow,
  ARTIFACT_NAVIGATE_EVENT,
  type ArtifactNavigateEvent,
} from "./composables/useArtifactWindow";
import { useArtifactHistory } from "./composables/useArtifactHistory";
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
const repositoryId = params.get("repositoryId") ?? "";
/** 起動時に選択しておくアーティファクト（リンクから新規ウィンドウで開かれたとき） */
const initialArtifactId = params.get("artifactId") ?? "";

const isRepositoryScope = scope === "repository";
const scopeId = isRepositoryScope ? repositoryId : worktreeId;

// URL には ID しか載らない（リンクを書く側は遷移先の名前を知らないため）。
// 名前は resolve_artifact_scope で settings から解決する。解決前・失敗時は ID を出す。
const headerTitle = ref(scopeId);
const repositoryName = ref("");

const { openArtifactViewer, openRepositoryArtifactViewer } = useArtifactWindow();

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

let unlisten: UnlistenFn | null = null;
let unlistenNavigate: UnlistenFn | null = null;

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
  } catch (e) {
    console.error("list artifacts failed", e);
  }
}

async function loadStates() {
  try {
    states.value = await invoke<Record<string, ArtifactState>>("list_artifact_states", {
      scope,
      scopeId,
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
      scope,
      scopeId,
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

/**
 * React アーティファクトのメモリー（フォーム入力などの復元用 JSON ストア）を保存する。
 * `memory` が null ならリセット（サイドカーからキーごと削除）。
 * 反映は成功後にだけ行い、保存に失敗した値が次回の初期値にならないようにする。
 */
async function saveArtifactMemory(
  artifactId: string,
  memory: Record<string, unknown> | null,
): Promise<void> {
  await invoke("set_artifact_memory", { scope, scopeId, artifactId, memory });
  const next = { ...(states.value[artifactId] ?? {}) };
  if (memory) next.memory = memory;
  else delete next.memory;
  states.value = { ...states.value, [artifactId]: next };
}

/** iframe 内の setMemory から呼ばれる。呼び出し時点の選択に紐づける */
async function saveSelectedMemory(memory: Record<string, unknown>): Promise<void> {
  const artifactId = selectedId.value;
  if (!artifactId) throw new Error("no artifact selected");
  await saveArtifactMemory(artifactId, memory);
}

/**
 * メモリーのリセット。iframe は初期値を srcdoc から同期で読むため、
 * 消しただけでは表示中のフォームが変わらない。キーを進めて作り直させる。
 */
const reactViewSeq = ref(0);

const selectedMemory = computed(() =>
  selectedId.value ? states.value[selectedId.value]?.memory : undefined,
);

const hasSelectedMemory = computed(() => {
  const memory = selectedMemory.value;
  return !!memory && Object.keys(memory).length > 0;
});

async function resetMemory() {
  const artifactId = selectedId.value;
  if (!artifactId) return;

  const confirmed = await ask(t("memory.resetConfirm"), {
    title: t("memory.resetTitle"),
    kind: "warning",
  });
  if (!confirmed) return;

  try {
    await saveArtifactMemory(artifactId, null);
    reactViewSeq.value += 1;
    toast.add({ severity: "success", summary: t("memory.resetDone"), life: 3000 });
  } catch (e) {
    console.error("set_artifact_memory failed", e);
    await message(String(e), { title: t("memory.resetFailed"), kind: "error" });
  }
}

const history = useArtifactHistory();
const { canGoBack, canGoForward } = history;

/** 本文の読み込みのみ。履歴は触らない */
async function loadArtifact(id: string) {
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

async function selectArtifact(id: string, mode: "push" | "replace" = "push") {
  if (selectedId.value === id && selectedArtifact.value) return;
  if (mode === "push") history.push(id);
  else history.replace(id);
  await loadArtifact(id);
}

/**
 * 履歴を1つ前/後ろへ動かす。ワークツリーごと削除された場合など、削除イベントで
 * 拾いきれず履歴に残った死んだエントリは、その場で取り除いて隣を試す
 * （そうしないと同じエントリで永久に足止めされる）。
 */
async function stepHistory(delta: -1 | 1) {
  let prunedAny = false;
  while (delta < 0 ? canGoBack.value : canGoForward.value) {
    const nextIndex = history.index.value + delta;
    const id = history.entries.value[nextIndex];
    if (artifacts.value.some((a) => a.id === id)) {
      history.moveTo(nextIndex);
      await loadArtifact(id);
      return;
    }
    history.prune(id);
    prunedAny = true;
  }
  if (prunedAny) {
    toast.add({ severity: "warn", summary: t("navigate.notFound"), life: 4000 });
  }
}

async function goBack() {
  await stepHistory(-1);
}

async function goForward() {
  await stepHistory(1);
}

// ── artifact: リンクの遷移 ──

/** 同一スコープ内の遷移。存在しなければトーストで知らせ、表示は今のまま維持する */
async function navigateWithin(artifactId: string, mode: "push" | "replace") {
  // 一覧が古いだけの可能性があるので、無いときは取り直してから判定する
  if (!artifacts.value.some((a) => a.id === artifactId)) {
    await loadList();
  }
  if (!artifacts.value.some((a) => a.id === artifactId)) {
    toast.add({
      severity: "warn",
      summary: t("navigate.notFound"),
      detail: artifactId,
      life: 4000,
    });
    return;
  }
  await selectArtifact(artifactId, mode);
}

/** ビュー（markdown / html / react）から上がってきた artifact: リンクを処理する */
async function onNavigate(href: string) {
  const target = parseArtifactLink(href);
  if (!target) {
    // artifact: と書いてあるのに解析できない = リンクの書き間違い。
    // 黙って無反応にすると書き手が typo に気づけないので知らせる
    toast.add({ severity: "warn", summary: t("navigate.invalidLink"), detail: href, life: 4000 });
    return;
  }

  const isSameScope =
    target.scope === null ||
    (target.scope === "worktree" && !isRepositoryScope && target.id === worktreeId) ||
    (target.scope === "repository" && isRepositoryScope && target.id === repositoryId);

  if (isSameScope) {
    await navigateWithin(target.artifactId, "push");
    return;
  }

  try {
    if (target.scope === "worktree") {
      await openArtifactViewer(target.id ?? "", target.artifactId);
    } else {
      await openRepositoryArtifactViewer(target.id ?? "", target.artifactId);
    }
  } catch (e) {
    console.error("open artifact viewer failed", e);
    toast.add({ severity: "error", summary: t("navigate.openFailed"), detail: href, life: 4000 });
  }
}

async function refreshSelected(artifactId: string, command: string) {
  await loadList();
  if (command === "delete") {
    // 本体と一緒にサイドカーも消えるので、ピン止め状態を読み直す
    await loadStates();
    history.prune(artifactId);
    if (selectedId.value === artifactId) {
      selectedId.value = null;
      selectedArtifact.value = null;
      // prune が現在位置を1つ前へ下げているので、その指す先をそのまま表示する
      // （履歴は既に動かし終えているため loadArtifact で積み直さない）
      const fallback = history.current.value;
      if (fallback) {
        await loadArtifact(fallback);
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

/** ヘッダー・ウィンドウタイトルに出す名前を settings から解決する */
async function resolveScopeName() {
  try {
    const info = await invoke<{ displayName: string; repositoryName: string | null }>(
      "resolve_artifact_scope",
      { scope, id: scopeId },
    );
    headerTitle.value = info.displayName;
    repositoryName.value = info.repositoryName ?? "";
  } catch (e) {
    // 削除済みワークツリーのビューアが残っている場合など。ID 表示のまま続行する
    console.warn("resolve_artifact_scope failed", e);
  }
  try {
    await getCurrentWindow().setTitle(`Artifacts - ${headerTitle.value}`);
  } catch (e) {
    console.warn("setTitle failed", e);
  }
}

onMounted(async () => {
  // 既存ウィンドウ宛の遷移指示。Tauri のイベントにバッファリングは無く、一方で
  // 送信側の focusExisting は起動途中のウィンドウでも true を返すため、
  // loadList() などを待つ前に最優先で登録する（待つと取りこぼす）。
  // ウィンドウを跨ぐ遷移なので履歴は積まない。
  unlistenNavigate = await listen<ArtifactNavigateEvent>(ARTIFACT_NAVIGATE_EVENT, async (event) => {
    await navigateWithin(event.payload.artifactId, "replace");
  });

  void resolveScopeName();
  await loadList();
  // ピン止めはソート順に効くので、先頭を選ぶ前に読む
  await loadStates();
  // リンクから開かれた場合は指定のアーティファクトを、無ければ先頭を選ぶ
  if (initialArtifactId) {
    if (artifacts.value.some((a) => a.id === initialArtifactId)) {
      await selectArtifact(initialArtifactId);
    } else {
      toast.add({
        severity: "warn",
        summary: t("navigate.notFound"),
        detail: initialArtifactId,
        life: 4000,
      });
    }
  }
  if (!selectedId.value && sortedArtifacts.value.length > 0) {
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
  unlistenNavigate?.();
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
      <!-- 読み込み失敗で本文が消えても履歴は残るため、分岐の外に出して行き止まりを作らない -->
      <div class="nav-bar">
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
            <!-- リポジトリスコープにはメニューが無いので、リセットはヘッダーに直接出す
                 （転送でメモリーを引き継ぐため、転送先でもリセットは必要） -->
            <button
              v-if="isRepositoryScope && selectedArtifact.content_type === 'application/vnd.ant.react'"
              class="btn-header"
              :disabled="!hasSelectedMemory"
              :title="t('memory.resetTooltip')"
              @click="resetMemory"
            >
              <i class="pi pi-eraser" />
              <span>{{ t("memory.resetLabel") }}</span>
            </button>
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
            <button
              v-if="selectedArtifact.content_type === 'application/vnd.ant.react'"
              class="popup-item"
              :disabled="!hasSelectedMemory"
              :title="t('memory.resetTooltip')"
              @click="withMenuHidden(resetMemory)"
            >
              <span class="pi pi-eraser" />
              {{ t("memory.resetLabel") }}
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

        <div class="content-body">
          <ArtifactCodeView
            v-if="selectedArtifact.content_type === 'application/vnd.ant.code'"
            :content="selectedArtifact.content"
            :language="selectedArtifact.language"
          />
          <ArtifactMarkdownView
            v-else-if="selectedArtifact.content_type === 'text/markdown'"
            :content="selectedArtifact.content"
            @navigate="onNavigate"
          />
          <ArtifactHtmlView
            v-else-if="selectedArtifact.content_type === 'text/html'"
            :content="selectedArtifact.content"
            @navigate="onNavigate"
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
            :key="`${selectedArtifact.id}:${reactViewSeq}`"
            :content="selectedArtifact.content"
            :modules="selectedArtifact.modules"
            :memory="selectedMemory"
            :save-memory="saveSelectedMemory"
            @navigate="onNavigate"
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

.nav-bar {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 6px 10px;
  background: #181825;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}

.btn-nav {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: #cdd6f4;
  cursor: pointer;
  font-size: 12px;
}

.btn-nav:hover:not(:disabled) {
  background: #313244;
  border-color: #45475a;
}

.btn-nav:disabled {
  color: #45475a;
  cursor: default;
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
    "navigate": {
      "notFound": "Artifact not found",
      "invalidLink": "Invalid artifact link",
      "openFailed": "Failed to open the linked artifact"
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
    "memory": {
      "resetLabel": "Reset memory",
      "resetTooltip": "Clear the saved form state of this React artifact",
      "resetTitle": "Reset memory",
      "resetConfirm": "Clear the saved form state of this artifact?",
      "resetDone": "Memory reset",
      "resetFailed": "Failed to reset memory"
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
    "navigate": {
      "notFound": "アーティファクトが見つかりません",
      "invalidLink": "アーティファクトリンクの書式が不正です",
      "openFailed": "リンク先のアーティファクトを開けませんでした"
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
    "memory": {
      "resetLabel": "メモリーをリセット",
      "resetTooltip": "この React アーティファクトに保存されたフォーム入力を消します",
      "resetTitle": "メモリーのリセット",
      "resetConfirm": "このアーティファクトに保存されたフォーム入力を消しますか？",
      "resetDone": "メモリーをリセットしました",
      "resetFailed": "メモリーのリセットに失敗しました"
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
