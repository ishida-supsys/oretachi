<script setup lang="ts">
import { ref, reactive, onMounted, onUnmounted, nextTick, computed } from "vue";
import { emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import TerminalView from "./components/TerminalView.vue";
import FrameContainer from "./components/FrameContainer.vue";
import IdeSelectDialog from "./components/IdeSelectDialog.vue";
import AutoApprovalPromptDialog from "./components/AutoApprovalPromptDialog.vue";
import TrayArchiveConfirmDialog from "./components/TrayArchiveConfirmDialog.vue";
import { useWorktreeFrame } from "./composables/useWorktreeFrame";
import { useSettings } from "./composables/useSettings";
import { useHotkeyListener } from "./composables/useHotkeys";
import { useIdeSelect } from "./composables/useIdeSelect";
import { useArtifactWindow } from "./composables/useArtifactWindow";
import ArtifactUrlHoverMenu from "./components/ArtifactUrlHoverMenu.vue";
import ArtifactIcon from "./components/ArtifactIcon.vue";
import { extractUrlArtifacts } from "./utils/artifactUrl";
import type { UrlArtifactEntry } from "./types/artifact";
import { invoke } from "@tauri-apps/api/core";
import type { TrayWorktreeData } from "./composables/useTrayPopup";
import { useWorktreeTaskMap } from "./composables/useWorktreeTaskMap";
import { initTerminalUnread, terminalUnread } from "./composables/useEventSubscriptions";
import { collectUnreadByTab } from "./utils/terminalUnread";
import type { FrameNode } from "./types/frame";
import type { TrayTerminalEntry } from "./types/terminal";
import { useI18n } from "vue-i18n";
import { cssPxToLogical } from "./utils/uiScale";
import { useUiZoom, applyUiZoom } from "./composables/useUiZoom";
import MacTrafficLights from "./components/MacTrafficLights.vue";
import { isMac } from "./composables/usePlatform";

const { t } = useI18n();
const { getTooltipText: getWorktreeTaskTooltip } = useWorktreeTaskMap();

// ヘッダー ref（ウィンドウサイズ補正用）
const headerRef = ref<HTMLDivElement | null>(null);
// フッター ref（ウィンドウサイズ補正用）
const footerRef = ref<HTMLDivElement | null>(null);
// description 情報バー ref（ウィンドウサイズ補正用）
const descBarRef = ref<HTMLDivElement | null>(null);

// 全ワークツリーデータ
const allWorktrees = ref<TrayWorktreeData[]>([]);
const currentIndex = ref(0);
const initialized = ref(false);

// ターミナルエントリ（現在のワークツリーのみ）
const terminalEntries = reactive(new Map<number, TrayTerminalEntry>());

// TerminalView ref 管理
const terminalRefs = reactive(new Map<number, InstanceType<typeof TerminalView>>());

// タブ（フロントの terminalId）→ 未 ack のワークツリーイベント件数（#120 §7 / #130）。
// バックエンドは pty の session_id で数えるので詰め替える。terminalEntries は表示中
// ワークツリーの分だけなので、他ワークツリーの未読が混ざることはない。
// トレイはトーストを出さない（メイン / サブウィンドウと二重になるため）。バッジのみ。
const terminalUnreadByTab = computed(() =>
  collectUnreadByTab(terminalUnread.value, terminalEntries),
);

// 閉鎖処理の再入防止フラグ（フッターボタンの disabled にも使うため ref）
const closing = ref(false);
// 遷移中フラグ（連打・ホットキー二重発火の抑止 + ボタンの disabled 用）
const navigating = ref(false);
// 遷移の世代。await の後で自分が最新でなければ以降の副作用を捨てる
// （destroy 済みウィンドウへの setSize や、打ち切った遷移の描画を防ぐ）
let navToken = 0;

// destroy の二重実行防止（メイン経由と保険タイマーの両方から呼ばれる）
let destroyed = false;
// destroy をメインに委ねたのに閉じてもらえなかった場合の保険（メインが落ちている等）
const DEFERRED_DESTROY_FALLBACK_MS = 3000;

// 「次へ ▾」ドロップダウンの開閉
const menuOpen = ref(false);
// アーカイブ確認ダイアログ
const showArchiveConfirm = ref(false);
const archiveDirtyCount = ref(0);

// フレームレイアウト（useWorktreeFrameで共通化）
const {
  root,
  initLayout,
  lastFocusedLeafId,
  setTerminalRef,
  returnAllToOffscreen,
  mountTerminalsToHosts,
  getAllLeafs,
  switchTerminal,
  closeTerminal,
  handleTerminalExit,
  onSplitRequest,
  onTabDrop,
  onTabEdgeDrop,
  onTabReorder,
  switchNextTerminal,
  switchPrevTerminal,
  closeActiveTerminal,
  updateContainerSizes,
} = useWorktreeFrame({
  terminalEntries,
  terminalRefs,
  // noResize=true のためタブ切替時に PTY サイズを手動適用
  async onAfterSwitch(_leafId, terminalId) {
    const entry = terminalEntries.get(terminalId);
    const term = terminalRefs.get(terminalId);
    if (entry && term) {
      const termObj = term.getTerminal();
      if (termObj) {
        termObj.resize(entry.cols, entry.rows);
        termObj.refresh(0, termObj.rows - 1);
        termObj.scrollToBottom();
      }
    }
  },
});

// 自動承認ダイアログ状態
const showAutoApprovalPromptDialog = ref(false);

// ────────────────────────────────────────────────
// 現在ワークツリーの表示
// ────────────────────────────────────────────────

// ウィンドウサイズをサブウィンドウに合わせる
// isDetached=true: windowSize はサブウィンドウ全体のサイズ → フッターのみ加算
// isDetached=false: windowSize はメインウィンドウのフレーム領域 → ヘッダー + フッター加算
// windowSize は常に論理px (DIP)。offsetHeight は CSS px のため実適用ズームで換算してから加算
async function applyWindowSize(data: TrayWorktreeData) {
  const win = getCurrentWindow();
  const zoom = appliedZoom.value;
  const footerH = cssPxToLogical(footerRef.value?.offsetHeight ?? 0, zoom);
  const headerH = data.isDetached ? 0 : cssPxToLogical(headerRef.value?.offsetHeight ?? 0, zoom);
  // 情報バー(description)分の高さを加算する。トレイはターミナルを縮める権限が無い(no-resize)ため、
  // コンテンツ領域を削らずウィンドウ外形を伸ばして確保する。description が無ければ 0。
  const descH = cssPxToLogical(descBarRef.value?.offsetHeight ?? 0, zoom);
  const width = data.windowSize?.width ?? 900;
  const height = (data.windowSize?.height ?? 600) + footerH + headerH + descH;
  await win.setSize(new LogicalSize(width, height));
}

async function showWorktree(data: TrayWorktreeData) {
  terminalEntries.clear();
  terminalRefs.clear();
  // 前のワークツリーの URL が残ったままにならないよう先にクリアしてから、
  // 表示切り替えを待たせないよう取得自体は投げっぱなしにする
  artifactUrls.value = [];
  void refreshArtifactUrls(data.worktreeId);

  // 情報バー(description)が現在の worktree の内容で描画されてから高さを計測する。
  // currentWorktree (= allWorktrees[currentIndex]) は呼び出し側で更新済みのため nextTick で DOM 反映を待つ。
  await nextTick();
  await applyWindowSize(data);

  for (const t of data.terminals) {
    terminalEntries.set(t.id, { ...t });
  }

  const ids = data.terminals.map((t) => t.id);

  // レイアウト復元: layout があればそのまま設定（detached/non-detached 両対応）
  if (data.layout) {
    root.value = data.layout as FrameNode;
  } else {
    initLayout(ids);
  }

  // 最初のリーフを lastFocusedLeafId に設定
  const leafs = getAllLeafs();
  if (leafs.length > 0) {
    lastFocusedLeafId.value = leafs[0].id;
  }

  await nextTick();
  // OSレベルのウィンドウリサイズがレイアウトに反映されるのを待つ
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve());
    });
  });

  // 情報バーの高さは横幅依存(line-clamp:2 + word-break)。117行目の初回計測は setSize 前の
  // 旧横幅で行われるため、横幅が確定した今この時点で再計測して高さを補正する
  // (新横幅で2行に折り返した場合のターミナルのクリップを防ぐ)。description がある時のみ。
  if (data.description?.trim()) {
    await applyWindowSize(data);
  }

  mountTerminalsToHosts();

  // 全リーフのアクティブターミナルをリサイズ+リフレッシュ
  for (const leaf of leafs) {
    if (leaf.activeTerminalId == null) continue;
    const term = terminalRefs.get(leaf.activeTerminalId);
    if (!term) continue;
    await term.handleTabActivated();
    // PTYサイズに合わせてxterm.jsをリサイズ（noResize=trueのためfit()は呼ばれない）
    const entry = terminalEntries.get(leaf.activeTerminalId);
    if (entry) {
      const termObj = term.getTerminal();
      if (termObj) {
        termObj.resize(entry.cols, entry.rows);
        termObj.refresh(0, termObj.rows - 1);
        termObj.scrollToBottom();
      }
    }
  }

  // 最初のリーフにフォーカス
  const firstActiveId = leafs[0]?.activeTerminalId;
  if (firstActiveId != null) {
    terminalRefs.get(firstActiveId)?.focus();
  }
}

// ────────────────────────────────────────────────
// イベントハンドラ
// ────────────────────────────────────────────────

// ────────────────────────────────────────────────
// ナビゲーション
// ────────────────────────────────────────────────

const currentWorktree = computed(() => allWorktrees.value[currentIndex.value] ?? null);
const isLast = computed(() => currentIndex.value >= allWorktrees.value.length - 1);
const isFirst = computed(() => currentIndex.value <= 0);
/** ナビゲーション系ボタンを止めるべき状態 */
const navBusy = computed(() => navigating.value || closing.value);

// IDE で開く
const { showIdeDialog, detectedIdes, openInIde, onIdeSelected } = useIdeSelect();

// アーティファクト
const { openArtifactViewer } = useArtifactWindow();

async function onOpenArtifacts() {
  if (!currentWorktree.value) return;
  await openArtifactViewer(currentWorktree.value.worktreeId, currentWorktree.value.worktreeName);
}

/** 表示中ワークツリーの URL アーティファクト（アイコン隣のドロップダウン用） */
const artifactUrls = ref<UrlArtifactEntry[]>([]);

async function refreshArtifactUrls(worktreeId: string) {
  try {
    const list = await invoke<unknown[]>("list_artifacts", { worktreeId });
    // 取得中に別ワークツリーへ切り替わっていたら破棄する（表示中と一覧の不一致を防ぐ）
    if (currentWorktree.value?.worktreeId !== worktreeId) return;
    artifactUrls.value = extractUrlArtifacts(list);
  } catch {
    if (currentWorktree.value?.worktreeId !== worktreeId) return;
    artifactUrls.value = [];
  }
}

async function onOpenInIde() {
  const wt = currentWorktree.value;
  if (!wt) return;
  await openInIde(wt.worktreePath, { worktreeId: wt.worktreeId, worktreeName: wt.worktreeName, origin: "tray" });
}

/** 現在のターミナルを detach する（遷移・閉鎖の共通前処理） */
async function detachCurrentTerminals() {
  // 「前へ」で戻ったときに tray-init 時点の古いスナップショットで再アタッチされないよう、
  // 離脱直前の画面内容を allWorktrees 側へ書き戻しておく。TerminalView の initialSnapshot は
  // onMounted 時のみ参照されるため、次回マウント時にこの値が使われる。
  // 制約: 離脱中に pty が exit しても再アタッチでは exit を受け取れない（既存挙動と同じ）。
  const wt = currentWorktree.value;
  if (wt) {
    for (const term of wt.terminals) {
      // serializeBuffer は内部で batcher.flush() してから serialize するので取りこぼさない
      const snapshot = terminalRefs.get(term.id)?.serializeBuffer(300);
      if (snapshot) term.snapshot = snapshot;
    }
  }
  returnAllToOffscreen();
  for (const [, ref] of terminalRefs) {
    ref.detach();
  }
  await nextTick();
}

type GoToOptions = {
  /**
   * 離れる側の通知を既読にする（既定 true）。アーカイブ導線では main 側の
   * archiveWorktree が内部で clearNotification するため false にする。
   */
  clearLeaving?: boolean;
  /** 呼び出し側で detachCurrentTerminals() を済ませている場合 true */
  alreadyDetached?: boolean;
};

/**
 * index のワークツリーを表示する。前へ / 次へ / アーカイブ後の再表示で共有する。
 * - 離れる側の副作用: detach（+ 任意で通知クリア）
 * - 入る側の副作用: tray-current-worktree-changed → showWorktree
 * アーカイブ後は splice 済みで index が据え置きになるため、同一 index への goTo も
 * 再表示として成立させる（同一 index を弾かない）。
 */
async function goTo(index: number, options: GoToOptions = {}): Promise<void> {
  const { clearLeaving = true, alreadyDetached = false } = options;
  if (closing.value || navigating.value) return;
  if (index < 0 || index >= allWorktrees.value.length) return;

  const token = ++navToken;
  navigating.value = true;
  try {
    const leaving = currentWorktree.value;
    if (!alreadyDetached) await detachCurrentTerminals();
    if (clearLeaving && leaving) {
      await emitTo("main", "tray-clear-notification", { worktreeId: leaving.worktreeId });
    }
    // await 中に閉鎖 / 別遷移が始まっていたら以降は行わない
    if (token !== navToken || closing.value) return;

    currentIndex.value = index;
    const entering = allWorktrees.value[index];
    await emitTo("main", "tray-current-worktree-changed", { worktreeId: entering.worktreeId });
    if (token !== navToken || closing.value) return;
    await showWorktree(entering);
  } finally {
    if (token === navToken) navigating.value = false;
  }
}

async function onNext() {
  if (!isLast.value) await goTo(currentIndex.value + 1);
}

async function onPrev() {
  // 「見終わった」の意味づけは方向に依らないので、前へでも離脱側は既読にする（冪等）
  if (!isFirst.value) await goTo(currentIndex.value - 1);
}

/** tray-closing を出してこのウィンドウを破棄する。保険タイマーからも呼ぶので冪等にする */
async function destroySelf(): Promise<void> {
  if (destroyed) return;
  destroyed = true;
  await emitTo("main", "tray-closing", {});
  await getCurrentWindow().destroy();
}

/**
 * ポップアップを閉じる共通処理。
 * 順序: detach → (通知クリア) → extraEmit → tray-closing → destroy
 * destroy() は close() ではないので onUnmounted が走らない。emit はすべて destroy より
 * 前に await して、IPC が落ちる前に届かせる。
 */
async function closePopup(options: {
  clearCurrentNotification?: boolean;
  /** tray-closing の直前に流す追加イベント */
  extraEmit?: (worktree: TrayWorktreeData) => Promise<void>;
  /**
   * destroy をメイン側に任せる。「ウィンドウで開く」で必要。
   * フォアグラウンドだったトレイが自分で先に消えるとプロセスがフォアグラウンド権を
   * 失い、後から走るメイン側の setFocus が OS に拒否される（メインが前面に出ず、
   * Z オーダー次第で別アプリがアクティブになる）。
   */
  deferDestroy?: boolean;
} = {}): Promise<void> {
  if (closing.value) return;
  closing.value = true;
  navToken++; // 進行中の goTo の続きを打ち切る
  try {
    const wt = currentWorktree.value;
    await detachCurrentTerminals();
    if (wt && (options.clearCurrentNotification ?? true)) {
      await emitTo("main", "tray-clear-notification", { worktreeId: wt.worktreeId });
    }
    if (wt && options.extraEmit) await options.extraEmit(wt);
    if (options.deferDestroy) {
      // メインが閉じてくれなかったときだけ自分で閉じる
      setTimeout(() => { void destroySelf(); }, DEFERRED_DESTROY_FALLBACK_MS);
      return;
    }
    await destroySelf();
  } catch (e) {
    // emit 失敗でボタンが永久に死ぬのを避ける（既存挙動を維持）
    closing.value = false;
    throw e;
  }
}

async function onDone() {
  await closePopup();
}

async function onClose() {
  await closePopup();
}

/**
 * 表示中ワークツリーをメイン / サブウィンドウで開いてトレイを閉じる。
 * トレイはターミナルを offscreen にマウントして掴んでいるため、detach で手放さないと
 * メイン / サブ側で表示できない。かつメインが前面に出るのにトレイが残っても邪魔なだけ。
 */
async function onShowInWindow() {
  await closePopup({
    extraEmit: (wt) => emitTo("main", "tray-show-worktree", { worktreeId: wt.worktreeId }),
    // メインが前面化を終えてから main 側にこのウィンドウを destroy させる
    deferDestroy: true,
  });
}

/** ▾ メニューの「アーカイブ化して…」→ 確認ダイアログを開く */
async function onClickArchive() {
  menuOpen.value = false;
  const wt = currentWorktree.value;
  if (!wt || !wt.canArchive || navBusy.value) return;
  // 未コミット件数は警告表示用。取得に失敗しても確認自体は出す（0 件扱い）
  archiveDirtyCount.value = await invoke<{ path: string }[]>("git_get_status", { repoPath: wt.worktreePath })
    .then((files) => files.length)
    .catch(() => 0);
  // 取得中に別ワークツリーへ切り替わっていたら開かない
  if (currentWorktree.value?.worktreeId !== wt.worktreeId) return;
  showArchiveConfirm.value = true;
}

/** 確認ダイアログ確定 → main へアーカイブを依頼し、トレイ側は先に進む */
async function onArchiveConfirmed(options: { deleteBranch: boolean }) {
  showArchiveConfirm.value = false;
  const wt = currentWorktree.value;
  if (!wt || closing.value) return;

  const index = currentIndex.value;
  const wasLast = isLast.value;

  // main 側のターミナル kill / git worktree remove とトレイの attach が競合しないよう、
  // アーカイブ依頼より前にトレイ側のターミナルを必ず切り離す
  await detachCurrentTerminals();

  // アーカイブは数秒〜数十秒かかり、失敗時のエラーダイアログは main 側に出る。
  // トレイは完了を待たずに次へ進む。通知クリアも archiveWorktree 内の clearNotification
  // が行うので tray-clear-notification は出さない。
  await emitTo("main", "tray-archive-worktree", {
    worktreeId: wt.worktreeId,
    deleteBranch: options.deleteBranch,
  });

  if (wasLast) {
    // 「アーカイブ化して完了」: 表示中が最後の1件 → ポップアップを閉じる
    await closePopup({ clearCurrentNotification: false });
    return;
  }

  // 「アーカイブ化して次へ」: 一覧から取り除くと後続が詰まるので、同じ index を再表示する。
  // wasLast === false なので splice 後も index <= length - 1 が保証される。
  allWorktrees.value.splice(index, 1);
  await goTo(index, { clearLeaving: false, alreadyDetached: true });
}

function onHeaderDrag(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('button')) return
  getCurrentWindow().startDragging()
}

function onClickAutoApproval() {
  if (!currentWorktree.value) return;
  showAutoApprovalPromptDialog.value = true;
}

async function onSaveAutoApprovalPrompt(wid: string, prompt: string) {
  showAutoApprovalPromptDialog.value = false;
  await emitTo("main", "tray-save-auto-approval-prompt", { worktreeId: wid, prompt: prompt.trim() });
}

async function onCancelAiJudging() {
  const wt = currentWorktree.value;
  if (!wt) return;
  await emitTo("main", "tray-cancel-ai-judging", { worktreeId: wt.worktreeId });
}

// ────────────────────────────────────────────────
// ライフサイクル
// ────────────────────────────────────────────────

const { settings, loadSettings } = useSettings();
const { appliedZoom } = useUiZoom();

// TrayPopup のホットキーリスナー
useHotkeyListener(() => {
  const hk = settings.value.hotkeys;
  if (!hk || !initialized.value) return [];
  // ダイアログ表示中はナビゲーション系を止める。useHotkeyListener は window capture で
  // 登録されるため、ダイアログ側の stopPropagation より先に発火してしまう。
  if (showArchiveConfirm.value || showIdeDialog.value || showAutoApprovalPromptDialog.value) return [];
  return [
    {
      binding: hk.trayNext,
      handler: () => {
        menuOpen.value = false;
        if (navBusy.value) return;
        if (isLast.value) {
          onDone();
        } else {
          onNext();
        }
      },
    },
    { binding: hk.terminalNext, handler: switchNextTerminal },
    { binding: hk.terminalPrev, handler: switchPrevTerminal },
    { binding: hk.terminalClose, handler: closeActiveTerminal },
  ];
});

let unlistenInit: UnlistenFn | null = null;
let unlistenSettings: UnlistenFn | null = null;
let unlistenArtifact: UnlistenFn | null = null;
let unlistenUnread: UnlistenFn | null = null;

/** ▾ メニューを Escape で閉じる。ダイアログ側は各コンポーネントが自前で処理する */
function onWindowKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape" || !menuOpen.value) return;
  e.preventDefault();
  e.stopPropagation();
  menuOpen.value = false;
}

onMounted(async () => {
  window.addEventListener("keydown", onWindowKeydown, true);
  await loadSettings();

  // タブ未読バッジの同期（#130）。event-inbox-changed は全 webview に届くので中継は不要。
  unlistenUnread = await initTerminalUnread();

  // 表示中ワークツリーにアーティファクトが登録されたら URL 一覧を追従させる
  unlistenArtifact = await listen<{ worktreeId: string }>("artifact-changed", async (event) => {
    if (event.payload.worktreeId !== currentWorktree.value?.worktreeId) return;
    await refreshArtifactUrls(event.payload.worktreeId);
  });

  // トレイ表示中の uiScale / ターミナル設定変更に追従
  unlistenSettings = await listen("settings-changed", async () => {
    await loadSettings();
    // main.ts のリスナーと順不同でも冪等なズーム適用で appliedZoom を最新化し、
    // 表示中ワークツリーのウィンドウサイズを新ズームで再計算する
    await applyUiZoom(settings.value);
    const wt = currentWorktree.value;
    if (initialized.value && wt) {
      await nextTick();
      await applyWindowSize(wt);
    }
  });

  const appWindow = getCurrentWindow();

  unlistenInit = await appWindow.listen<{ worktrees: TrayWorktreeData[] }>(
    "tray-init",
    async (event) => {
      allWorktrees.value = event.payload.worktrees;
      currentIndex.value = 0;
      initialized.value = true;

      if (allWorktrees.value.length > 0) {
        await showWorktree(allWorktrees.value[0]);
      }
    }
  );

  // 準備完了をメインに通知
  await emitTo("main", "tray-ready", {});
});

onUnmounted(() => {
  window.removeEventListener("keydown", onWindowKeydown, true);
  unlistenInit?.();
  unlistenSettings?.();
  unlistenArtifact?.();
  unlistenUnread?.();
});
</script>

<template>
  <div class="h-screen flex flex-col text-[#cdd6f4] select-none" style="background-color: var(--bg-base)">
    <!-- ヘッダー (drag-region) -->
    <div
      ref="headerRef"
      class="flex items-center justify-between border-b border-[#313244] shrink-0 px-4 py-2"
      style="background-color: var(--bg-mantle-translucent)"
      @mousedown.left="onHeaderDrag"
    >
      <div class="flex items-center gap-2">
        <!-- Mac: トラフィックライト (閉じるのみ) -->
        <MacTrafficLights
          v-if="isMac"
          :is-window-focused="true"
          :show-minimize="false"
          :show-maximize="false"
          @close="onClose"
        />
        <span class="pi pi-bell text-[#cba6f7] pointer-events-none" />
        <span class="text-sm font-semibold text-[#cba6f7] pointer-events-none">
          {{ currentWorktree?.worktreeName ?? t('notification') }}
        </span>
        <span
          v-if="allWorktrees.length > 1"
          class="text-xs text-[#6c7086] pointer-events-none"
        >
          {{ currentIndex + 1 }} / {{ allWorktrees.length }}
        </span>
        <span
          v-if="currentWorktree?.branchName"
          v-tooltip.bottom="getWorktreeTaskTooltip(currentWorktree.repositoryName, currentWorktree.branchName) ? { value: getWorktreeTaskTooltip(currentWorktree.repositoryName, currentWorktree.branchName), escape: false, showDelay: 300, class: 'task-tooltip-sm' } : undefined"
          class="flex items-center gap-1 text-xs font-mono text-[#9399b2]"
          :class="{ 'cursor-help': getWorktreeTaskTooltip(currentWorktree.repositoryName, currentWorktree.branchName), 'pointer-events-none': !getWorktreeTaskTooltip(currentWorktree.repositoryName, currentWorktree.branchName) }"
        >
          <span class="pi pi-code-branch" style="font-size: 10px" />
          {{ currentWorktree.branchName }}
        </span>
        <span
          v-if="currentWorktree?.hotkeyChar"
          class="text-[10px] px-1.5 py-0.5 rounded font-mono font-medium pointer-events-none"
          style="background: rgba(203,166,247,0.15); color: #cba6f7; border: 1px solid rgba(203,166,247,0.3)"
        >Alt+{{ currentWorktree.hotkeyChar.toUpperCase() }}</span>
        <button
          v-if="currentWorktree?.autoApproval"
          class="text-[10px] px-1.5 py-0.5 rounded font-medium cursor-pointer border-none"
          style="background: rgba(166, 227, 161, 0.15); color: #a6e3a1; border: 1px solid rgba(166, 227, 161, 0.3)"
          :title="t('editAutoApprovalPrompt')"
          @click="onClickAutoApproval"
        >{{ t('autoApprovalBadge') }}</button>
        <button
          v-if="currentWorktree?.aiJudging"
          class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-semibold cursor-pointer border-none"
          style="background: #f9e2af; color: #1e1e2e"
          @click="onCancelAiJudging"
        >
          <span class="pi pi-spin pi-spinner" style="font-size: 9px" />
          {{ t('aiJudgingBadge') }}
        </button>
      </div>
      <div class="flex items-center gap-4">
        <button
          v-if="currentWorktree"
          class="pointer-events-auto w-6 h-6 flex items-center justify-center rounded hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4] transition-colors"
          :title="t('openInIde')"
          @click="onOpenInIde"
        >
          <span class="pi pi-code text-xs" />
        </button>
        <ArtifactUrlHoverMenu v-if="currentWorktree" :urls="artifactUrls">
          <button
            class="pointer-events-auto w-6 h-6 flex items-center justify-center rounded hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4] transition-colors"
            :title="t('openArtifacts')"
            @click="onOpenArtifacts"
          >
            <ArtifactIcon :has-url="artifactUrls.length > 0" class="text-xs" />
          </button>
        </ArtifactUrlHoverMenu>
        <button
          v-if="!isMac"
          class="pointer-events-auto w-6 h-6 flex items-center justify-center rounded hover:bg-[#313244] text-[#6c7086] hover:text-[#f38ba8] transition-colors"
          :title="t('close')"
          @click="onClose"
        >
          <span class="pi pi-times text-xs" />
        </button>
      </div>
    </div>

    <!-- description 情報バー: ある worktree のみ常時表示。高さは applyWindowSize に加算され、
         ターミナル領域は削られない（トレイは no-resize でターミナルを縮める権限が無いため） -->
    <div
      v-if="currentWorktree?.description?.trim()"
      ref="descBarRef"
      class="shrink-0 px-4 py-1.5 border-b border-[#313244] text-[11px] leading-relaxed text-[#a6adc8] tray-desc-bar"
      style="background-color: var(--bg-mantle-translucent)"
      :title="currentWorktree.description"
    >
      {{ currentWorktree.description }}
    </div>

    <!-- コンテンツ -->
    <div class="flex-1 min-h-0 overflow-hidden">
      <div v-if="!initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
        {{ t('loading') }}
      </div>

      <FrameContainer
        v-else-if="terminalEntries.size > 0"
        :node="root"
        :terminal-entries="terminalEntries"
        :terminal-unread="terminalUnreadByTab"
        @switch-terminal="switchTerminal"
        @close-terminal="closeTerminal"
        @title-change="() => {}"
        @split-request="onSplitRequest"
        @tab-drop="onTabDrop"
        @tab-edge-drop="onTabEdgeDrop"
        @tab-reorder="onTabReorder"
        @request-add-terminal="() => {}"
        @resize-end="updateContainerSizes"
      />

      <div v-else-if="initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
        {{ t('noTerminals') }}
      </div>
    </div>

    <!-- フッター -->
    <div ref="footerRef" class="flex items-center justify-end gap-2 border-t border-[#313244] shrink-0 px-4 py-2" style="background-color: var(--bg-mantle-translucent)">
      <!-- ウィンドウで開く: 押したらトレイは閉じる（= このワークツリーを開いて確認を終える） -->
      <button
        class="shrink-0 px-3 py-1.5 text-sm rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition-colors disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-[#313244]"
        :disabled="!currentWorktree || closing"
        @click="onShowInWindow"
      >
        {{ t('openInWindow') }}
      </button>

      <!-- 前へ -->
      <button
        class="shrink-0 px-3 py-1.5 text-sm rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition-colors disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-[#313244]"
        :disabled="isFirst || navBusy"
        @click="onPrev"
      >
        {{ t('prev') }}
      </button>

      <!-- 次へ / 完了 + ▾ (split button)。isLast では primary が「完了」に変わるので
           「次へ」と「完了」が同時に並ぶことはなく、フッターの要素数は常に一定。
           primary も ▾ も緑で、状態でラベルと動作だけが変わる -->
      <div class="relative flex shrink-0">
        <button
          class="px-4 py-1.5 text-sm rounded-l bg-[#a6e3a1] hover:bg-[#89c98a] text-[#1e1e2e] font-semibold transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          :disabled="isLast ? closing : navBusy"
          @click="isLast ? onDone() : onNext()"
        >
          {{ isLast ? t('done') : t('next') }}
        </button>
        <!-- ▾ は isLast でも有効（「アーカイブ化して完了」に到達させるため） -->
        <button
          class="px-2 py-1.5 text-sm rounded-r text-[#1e1e2e] border-l border-[#1e1e2e] transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          :class="menuOpen ? 'bg-[#89c98a]' : 'bg-[#a6e3a1] hover:bg-[#89c98a]'"
          :disabled="closing"
          :title="t('moreActions')"
          :aria-expanded="menuOpen"
          @click="menuOpen = !menuOpen"
        >
          <span class="pi pi-chevron-up" style="font-size: 10px" />
        </button>

        <!-- クリックアウト用の透明バックドロップ（メニューより下、他要素より上） -->
        <div v-if="menuOpen" class="fixed inset-0 z-[190]" @click="menuOpen = false" />

        <!-- トレイウィンドウは高さ固定なのでメニューは上向きに開く。
             split button はフッター右端にあるので right-0 で右揃えにする
             （left-0 だと min-w-max がウィンドウ右外へはみ出して項目が切れる） -->
        <div
          v-if="menuOpen"
          class="absolute bottom-full right-0 mb-1 z-[191] min-w-max rounded border border-[#45475a] py-1 shadow-lg"
          style="background-color: #1e1e2e"
        >
          <button
            v-if="currentWorktree?.canArchive"
            class="flex w-full items-center gap-2 whitespace-nowrap px-3 py-1.5 text-left text-xs text-[#f38ba8] hover:bg-[#313244]"
            @click="onClickArchive"
          >
            <span class="pi pi-inbox" style="font-size: 11px" />
            {{ isLast ? t('archiveAndDone') : t('archiveAndNext') }}
          </button>
          <span v-else class="block whitespace-nowrap px-3 py-1.5 text-xs text-[#6c7086]">
            {{ t('noMenuActions') }}
          </span>
        </div>
      </div>
    </div>

    <!-- IDE 選択ダイアログ -->
    <IdeSelectDialog
      v-if="showIdeDialog"
      :ides="detectedIdes"
      @select="onIdeSelected"
      @cancel="showIdeDialog = false"
    />

    <!-- 自動承認 追加プロンプト編集ダイアログ -->
    <AutoApprovalPromptDialog
      v-if="showAutoApprovalPromptDialog"
      :worktree-id="currentWorktree?.worktreeId ?? ''"
      :worktree-name="currentWorktree?.worktreeName ?? ''"
      :current-prompt="currentWorktree?.autoApprovalPrompt ?? ''"
      :last-command="currentWorktree?.lastJudgedCommand ?? ''"
      @save="onSaveAutoApprovalPrompt"
      @cancel="showAutoApprovalPromptDialog = false"
    />

    <!-- アーカイブ簡易確認ダイアログ -->
    <TrayArchiveConfirmDialog
      v-if="showArchiveConfirm && currentWorktree"
      :worktree-name="currentWorktree.worktreeName"
      :branch-name="currentWorktree.branchName"
      :dirty-count="archiveDirtyCount"
      :is-last="isLast"
      @confirm="onArchiveConfirmed"
      @cancel="showArchiveConfirm = false"
    />

    <!-- TerminalView のマウント先 -->
    <div
      data-offscreen
      style="position:fixed; left:-10000px; top:-10000px; width:1000px; height:1000px; overflow:hidden; pointer-events:none"
    >
      <template v-for="[tid, entry] in terminalEntries" :key="tid">
        <TerminalView
          :ref="(el) => setTerminalRef(tid, el)"
          :no-resize="true"
          :auto-start="false"
          :initial-session-id="entry.sessionId"
          :initial-snapshot="entry.snapshot"
          :initial-cols="entry.cols"
          :initial-rows="entry.rows"
          @exit="handleTerminalExit(tid)"
          @title-change="() => {}"
        />
      </template>
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "notification": "Notification",
    "close": "Close",
    "openInIde": "Open in IDE",
    "openArtifacts": "Artifacts",
    "loading": "Loading...",
    "noTerminals": "No terminals",
    "prev": "← Prev",
    "next": "Next →",
    "done": "Done ✓",
    "openInWindow": "Open in window",
    "moreActions": "More actions",
    "archiveAndNext": "Archive & next",
    "archiveAndDone": "Archive & done",
    "noMenuActions": "No actions available",
    "autoApprovalBadge": "Auto approval",
    "aiJudgingBadge": "AI judging",
    "editAutoApprovalPrompt": "Edit additional prompt"
  },
  "ja": {
    "notification": "通知",
    "close": "閉じる",
    "openInIde": "IDE で開く",
    "openArtifacts": "アーティファクト",
    "loading": "読み込み中...",
    "noTerminals": "ターミナルがありません",
    "prev": "← 前へ",
    "next": "次へ →",
    "done": "完了 ✓",
    "openInWindow": "ウィンドウで開く",
    "moreActions": "その他の操作",
    "archiveAndNext": "アーカイブ化して次へ",
    "archiveAndDone": "アーカイブ化して完了",
    "noMenuActions": "利用できる操作はありません",
    "autoApprovalBadge": "自動承認",
    "aiJudgingBadge": "AI判定中",
    "editAutoApprovalPrompt": "追加プロンプトを編集"
  }
}
</i18n>

<style scoped>
/* description 情報バー: 最大2行クランプ（WorktreeCard の .card-desc-inner に準拠） */
.tray-desc-bar {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  word-break: break-word;
}
</style>
