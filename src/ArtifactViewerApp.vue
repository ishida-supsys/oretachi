<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, message } from "@tauri-apps/plugin-dialog";
import Toast from "primevue/toast";
import type { ToastMessageOptions } from "primevue/toast";
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
  ArtifactStateChangedEvent,
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
let unlistenState: UnlistenFn | null = null;

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

/**
 * メモリーのリセット。iframe は初期値を srcdoc から同期で読むため、
 * 消しただけでは表示中のフォームが変わらない。キーを進めて作り直させる。
 */
const reactViewSeq = ref(0);

/** 外からのストア更新を iframe へ押し込むために使う（`pushMemory`） */
const reactViewRef = ref<InstanceType<typeof ArtifactReactView> | null>(null);

const selectedMemory = computed(() =>
  selectedId.value ? states.value[selectedId.value]?.memory : undefined,
);

const hasSelectedMemory = computed(() => {
  const memory = selectedMemory.value;
  return !!memory && Object.keys(memory).length > 0;
});

/**
 * 「新しいアーティファクトが追加された」トーストに載せる導線。
 * 表示中の画面は奪わないので、ここを踏まない限り選択は変わらない。
 */
interface CreatedToastData {
  artifactId: string;
}

/**
 * PrimeVue は `add()` に渡したオブジェクトをそのまま保持して `#message` スロットへ流すため、
 * 宣言外のフィールドも往復する。`ToastMessageOptions` に `data` が無いのでここで足す。
 */
type ArtifactToastMessage = ToastMessageOptions & { data?: CreatedToastData };

function createdToastArtifactId(toastMessage: ArtifactToastMessage): string | null {
  return toastMessage.data?.artifactId ?? null;
}

/** 既定の描画を差し替えた分、severity アイコンは自前で出す（PrimeVue の既定と同じ絵柄） */
const toastIcons: Record<string, string> = {
  success: "pi-check",
  info: "pi-info-circle",
  warn: "pi-exclamation-triangle",
  error: "pi-times-circle",
};

function toastIcon(severity: string | undefined): string {
  return toastIcons[severity ?? "info"] ?? "pi-info-circle";
}

/** トーストの「開く」。踏まれて初めて表示を切り替える */
async function openFromToast(toastMessage: ArtifactToastMessage) {
  const artifactId = createdToastArtifactId(toastMessage);
  toast.remove(toastMessage);
  if (!artifactId) return;
  // 押されるまでの間に消えている可能性がある（削除・ワークツリーごと破棄）
  if (!artifacts.value.some((a) => a.id === artifactId)) {
    toast.add({
      severity: "warn",
      summary: t("navigate.notFound"),
      detail: artifactId,
      life: 4000,
    });
    return;
  }
  await selectArtifact(artifactId);
}

/** メモリーの保存失敗。アーティファクト側は入力を受け付け続けるので必ず見せる */
function onMemoryError(msg: string) {
  toast.add({ severity: "error", summary: t("memory.saveFailed"), detail: msg, life: 6000 });
}

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

// ─── 表示中ロック ─────────────────────────────────────────────────────────────
//
// 「いまこのウィンドウで開いているアーティファクト」を Rust の in-memory レジストリへ
// 登録し続ける。ロックの成立は本体 JSON の `locked_while_open` との AND なので、
// フラグの立っていないアーティファクトを登録しても何も止まらない（判定は Rust 側）。
//
// 解除の取りこぼしは Rust 側で二重に手当てしてある（ウィンドウ破棄 + ハートビート TTL）
// ので、ここが送れなくてもロックが永久に残ることはない。

let lockTimer: ReturnType<typeof setInterval> | null = null;

/**
 * ハートビート間隔が Rust から取れなかった場合の保険。
 * 本来の値は Rust 側の `ARTIFACT_LOCK_HEARTBEAT_MS`（TTL と対で決まる）。
 */
const ARTIFACT_LOCK_HEARTBEAT_FALLBACK_MS = 10000;

/** `artifact-state-changed` の処理世代。後着ハンドラが先着を追い越した場合に捨てる */
let stateGeneration = 0;

async function touchLock() {
  const artifactId = selectedId.value;
  if (!artifactId) return;
  try {
    await invoke("artifact_lock_touch", { scope, scopeId, artifactId });
  } catch (e) {
    // ロックは付加機能なので、失敗しても閲覧は続行させる
    console.warn("artifact_lock_touch failed", e);
  }
}

async function releaseLock() {
  try {
    await invoke("artifact_lock_release");
  } catch (e) {
    console.warn("artifact_lock_release failed", e);
  }
}

/**
 * MCP ツール呼び出しの中継。ホワイトリストとスコープの強制は Rust 側が行う。
 * `artifactId` を渡すのは監査ログのためで、権限判定には使われない
 * （判定に使うのはこのビューアのスコープ = アーティファクトの置き場所）。
 */
function callMcpTool(artifactId: string, tool: string, params: Record<string, unknown>) {
  return invoke<string>("artifact_call_mcp_tool", {
    scope,
    scopeId,
    artifactId,
    tool,
    params,
  });
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

async function refreshSelected(artifactId: string, command: string, autoOpen = true) {
  // 一覧を更新する前に既知かどうかを見ておく。artifact_module の create は
  // 「既存アーティファクトへのモジュール追加」でも command="create" を emit するため、
  // 更新後の一覧では本当の新規追加と区別できなくなる
  const isNewArtifact = !artifacts.value.some((a) => a.id === artifactId);
  await loadList();
  // サイドカーは削除で消えるだけでなく、転送（同じ ID への上書き = command "create"）で
  // メモリーが差し替わる。据え置くと古い memory を初期値にした iframe が
  // 次の保存でディスク上の新しい値を潰すため、どの command でも読み直す
  await loadStates();
  if (command === "delete") {
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
    if (selectedId.value === artifactId) {
      // 同じ ID の再作成（上書き転送）では selectArtifact が早期 return するため、
      // 選択中の本文を捨てて読み直させる
      selectedArtifact.value = null;
      await selectArtifact(artifactId);
    } else if (
      selectedId.value === null &&
      // 絞り込みで隠れているものを本文にだけ出すと、サイドバーが「一致なし」なのに
      // 表示はある、という食い違いになる（delete 側のフォールバックと揃える）
      visibleArtifacts.value.some((a) => a.id === artifactId)
    ) {
      // まだ何も開いていないときだけ拾う（ビューアを開いた直後の初期表示）
      await selectArtifact(artifactId);
    } else if (isNewArtifact && autoOpen) {
      // 別のアーティファクトを閲覧中に、無関係な ID が作られた（AI の生成、転送など）。
      // ここで選択を奪うと iframe が作り直され、読んでいた位置や入力途中のフォームが
      // 飛ぶ。表示は据え置き、代わりに「開く」付きのトーストで導線だけ残す。
      //
      // autoOpen=false（フックによる URL 自動登録）は作業の副産物なので黙って取り込む。
      // 既知の ID（モジュール追加）も「追加されました」は嘘になるので出さない。
      const created = artifacts.value.find((a) => a.id === artifactId);
      const toastMessage: ArtifactToastMessage = {
        severity: "info",
        summary: t("created.summary"),
        detail: created?.title ?? artifactId,
        life: 6000,
        data: { artifactId },
      };
      toast.add(toastMessage);
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

// 選択が変わったらロックを張り替える。null になったら解除する
watch(selectedId, (id) => {
  if (id) void touchLock();
  else void releaseLock();
});

onMounted(async () => {
  // 既存ウィンドウ宛の遷移指示。Tauri のイベントにバッファリングは無く、一方で
  // 送信側の focusExisting は起動途中のウィンドウでも true を返すため、
  // loadList() などを待つ前に最優先で登録する（待つと取りこぼす）。
  // ウィンドウを跨ぐ遷移なので履歴は積まない。
  unlistenNavigate = await listen<ArtifactNavigateEvent>(ARTIFACT_NAVIGATE_EVENT, async (event) => {
    await navigateWithin(event.payload.artifactId, "replace");
  });

  // ロックのハートビート。後続の await（listen / loadList）が失敗しても
  // 「ウィンドウは開いたままロックだけ TTL で失効する」を避けるため、ここで先に張る。
  // ロック対象の登録自体は watch(selectedId) が行う
  const intervalMs = await invoke<number>("artifact_lock_heartbeat_interval").catch(
    () => ARTIFACT_LOCK_HEARTBEAT_FALLBACK_MS,
  );
  lockTimer = setInterval(() => void touchLock(), intervalMs);

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
      await refreshSelected(
        event.payload.artifactId,
        event.payload.command,
        event.payload.autoOpen !== false,
      );
    });
  }

  // MCP の artifact_store がストアを書き換えたら、サイドカーのキャッシュを取り直し、
  // 表示中の iframe にも押し込む。押し込まないと iframe は古いスナップショットを持ち続け、
  // 次の 1 入力で自分の状態を丸ごと書き戻して MCP 側の書き込みを消してしまう
  // （MCP 側は成功を返しているので、消えたことに誰も気づけない）
  unlistenState = await listen<ArtifactStateChangedEvent>("artifact-state-changed", async (event) => {
    if (event.payload.scope !== scope || event.payload.scopeId !== scopeId) return;
    // loadStates() の await を挟むので、短時間に複数回届くとハンドラの完了順が
    // 入れ替わり、古いスナップショットを iframe へ押し込みうる。世代で捨てる
    const generation = ++stateGeneration;
    await loadStates();
    if (generation !== stateGeneration) return;
    if (event.payload.artifactId !== selectedId.value) return;
    reactViewRef.value?.pushMemory(selectedMemory.value ?? {});
  });

});

onUnmounted(() => {
  unlisten?.();
  unlistenNavigate?.();
  unlistenState?.();
  if (lockTimer !== null) clearInterval(lockTimer);
  void releaseLock();
});
</script>

<template>
  <div class="artifact-viewer">
    <!-- 既定の描画をそのまま使うと「開く」を挿せないため、本文だけ自前で描く -->
    <Toast>
      <template #message="slotProps">
        <div class="toast-body">
          <i :class="`pi ${toastIcon(slotProps.message.severity)} toast-icon`" />
          <div class="toast-text">
            <div class="toast-summary">{{ slotProps.message.summary }}</div>
            <div v-if="slotProps.message.detail" class="toast-detail">
              {{ slotProps.message.detail }}
            </div>
          </div>
          <button
            v-if="createdToastArtifactId(slotProps.message)"
            class="toast-open"
            @click="openFromToast(slotProps.message)"
          >
            {{ t("created.open") }}
          </button>
        </div>
      </template>
    </Toast>
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
              <!-- リポジトリ保管庫には MCP からの書き込み経路が無く、フラグが効かないので出さない -->
              <span
                v-if="!isRepositoryScope && selectedArtifact.locked_while_open"
                class="locked-badge"
                :title="t('locked.tooltip')"
              >
                <i class="pi pi-lock" />{{ t("locked.label") }}
              </span>
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
            ref="reactViewRef"
            :key="`${selectedArtifact.id}:${reactViewSeq}`"
            :content="selectedArtifact.content"
            :modules="selectedArtifact.modules"
            :memory="selectedMemory"
            :save-memory="(m: Record<string, unknown>) => saveArtifactMemory(selectedArtifact!.id, m)"
            :call-tool="(tool: string, p: Record<string, unknown>) => callMcpTool(selectedArtifact!.id, tool, p)"
            @navigate="onNavigate"
            @memory-error="onMemoryError"
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

/* 表示中ロック。AI から書き込めない理由がユーザーに分かるように出す */
.locked-badge {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  margin-right: 6px;
  padding: 0 5px;
  border-radius: 3px;
  background: rgba(249, 226, 175, 0.12);
  color: #f9e2af;
  /* .content-type が monospace なので、バッジだけ本文用フォントへ戻す */
  font-family: system-ui, -apple-system, sans-serif;
}

.locked-badge i {
  font-size: 9px;
}

.content-body {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

/* トースト本文。既定の描画を #message で置き換えているので自前で組む */
.toast-body {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
}

.toast-icon {
  flex-shrink: 0;
  align-self: flex-start;
  margin-top: 2px;
}

.toast-text {
  flex: 1;
  min-width: 0;
}

.toast-summary {
  font-weight: 600;
}

.toast-detail {
  margin-top: 2px;
  font-size: 12px;
  /* 長いタイトルでトーストが横に伸びないよう省略する */
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toast-open {
  flex-shrink: 0;
  padding: 3px 10px;
  border: 1px solid currentColor;
  border-radius: 4px;
  background: transparent;
  color: inherit;
  font-size: 12px;
  cursor: pointer;
}

.toast-open:hover {
  background: rgba(255, 255, 255, 0.12);
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
    "created": {
      "summary": "New artifact added",
      "open": "Open"
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
    "locked": {
      "label": "locked while open",
      "tooltip": "While this window is open, the AI cannot overwrite this artifact via MCP. Close the window to allow writes."
    },
    "memory": {
      "resetLabel": "Reset memory",
      "resetTooltip": "Clear the saved form state of this React artifact",
      "resetTitle": "Reset memory",
      "resetConfirm": "Clear the saved form state of this artifact?",
      "resetDone": "Memory reset",
      "resetFailed": "Failed to reset memory",
      "saveFailed": "Failed to save memory"
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
    "created": {
      "summary": "アーティファクトが追加されました",
      "open": "開く"
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
    "locked": {
      "label": "表示中ロック",
      "tooltip": "このウィンドウを開いている間、AI は MCP からこのアーティファクトを書き換えられません。書き込ませるにはウィンドウを閉じてください。"
    },
    "memory": {
      "resetLabel": "メモリーをリセット",
      "resetTooltip": "この React アーティファクトに保存されたフォーム入力を消します",
      "resetTitle": "メモリーのリセット",
      "resetConfirm": "このアーティファクトに保存されたフォーム入力を消しますか？",
      "resetDone": "メモリーをリセットしました",
      "resetFailed": "メモリーのリセットに失敗しました",
      "saveFailed": "メモリーの保存に失敗しました"
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
