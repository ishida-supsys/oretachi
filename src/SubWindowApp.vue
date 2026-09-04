<script setup lang="ts">
import { ref, reactive, onMounted, onUnmounted, nextTick, watch, computed } from "vue";
import { emitTo, listen } from "@tauri-apps/api/event";
import { useEventListeners } from "./composables/useEventListeners";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import TerminalView from "./components/TerminalView.vue";
import FrameContainer from "./components/FrameContainer.vue";
import WorktreeHeader from "./components/WorktreeHeader.vue";
import { useWorktreeFrame } from "./composables/useWorktreeFrame";
import { useSettings } from "./composables/useSettings";
import { useHotkeyListener, useAltCharKeyListener } from "./composables/useHotkeys";
import { renderToDataUrl } from "./composables/useTerminalThumbnail";
import { isDirty, clearDirty } from "./composables/usePtyDispatcher";
import { useWindowFocus } from "./composables/useWindowFocus";
import { runApprovalLoop } from "./utils/autoApproval";
import type { TerminalForApproval } from "./utils/autoApproval";
import {
  createPendingNotifyStore,
  queuePendingNotify,
  takePendingNotify,
  trayOf,
} from "./utils/autoApprovalNotify";
import { useIdeSelect } from "./composables/useIdeSelect";
import { useArtifactWindow } from "./composables/useArtifactWindow";
import { extractUrlArtifacts } from "./utils/artifactUrl";
import type { UrlArtifactEntry } from "./types/artifact";
import { invoke } from "@tauri-apps/api/core";
import { logDebug } from "./utils/log";
import { buildResumeCommand } from "./utils/resumeCommand";
import IdeSelectDialog from "./components/IdeSelectDialog.vue";
import AutoApprovalPromptDialog from "./components/AutoApprovalPromptDialog.vue";
import Toast from "primevue/toast";
import { useEventToast } from "./composables/useEventToast";
import {
  initSubscriptionsScoped,
  initTerminalUnread,
  terminalUnread,
} from "./composables/useEventSubscriptions";
import SubscriptionFocusDialog from "./components/SubscriptionFocusDialog.vue";
import { collectUnreadByTab } from "./utils/terminalUnread";
import { subWindowShowsDelivery } from "./utils/eventToastScope";
import type { SubTerminalEntry, WebSessionInfo, AiSessionInfo } from "./types/terminal";
import type { SavedTerminal } from "./types/worktree";
import type { FrameNode } from "./types/frame";
import { useI18n } from "vue-i18n";
import { useWorktreeTaskMap } from "./composables/useWorktreeTaskMap";

const { t } = useI18n();
const { getTooltipText: getWorktreeTaskTooltip } = useWorktreeTaskMap();

// クエリパラメータ
const params = new URLSearchParams(window.location.search);
const worktreeId = params.get("worktreeId") ?? "";
const worktreeName = params.get("worktreeName") ?? "";
const worktreePath = params.get("worktreePath") ?? "";
const branchName = params.get("branchName") ?? "";
const repositoryName = params.get("repositoryName") ?? "";

// ターミナルエントリ（Map）
const terminalEntries = reactive(new Map<number, SubTerminalEntry>());
const initialized = ref(false);

// terminalId → 直近コマンドの終了コード
const terminalExitCodes = reactive(new Map<number, number>());

// terminalId → AIエージェント稼働中フラグ
const terminalAgentStatus = reactive(new Map<number, boolean>());

// terminalId → Webセッション情報
const terminalWebSessions = reactive(new Map<number, WebSessionInfo>());

// terminalId → AIエージェントセッション情報（タブツールチップ用）
const terminalAiSessions = reactive(new Map<number, AiSessionInfo>());

// タブ（フロントの terminalId）→ 未 ack のワークツリーイベント件数（#120 §7 / #130）。
// バックエンドは pty の session_id で数えるので、上の pty-ai-agent-changed と同じ
// session_id → terminalId の逆引きで詰め替える。
const terminalUnreadByTab = computed(() =>
  collectUnreadByTab(terminalUnread.value, terminalEntries),
);

// 自動 spawn 拒否のトースト（#130 / #137）。**メインは isDetached のワークツリーを出さない**
// ので、分離済みワークツリーぶんを出すのはこのウィンドウの責任。
// `<script setup>` の同期部分で呼ぶ必要がある（onMounted の await を跨ぐと useToast が失敗する）。
useEventToast({ shouldShow: (wid) => subWindowShowsDelivery(wid, worktreeId) });

// 自動承認フラグ
const autoApproval = ref(false);

// 自動承認 追加プロンプト
const additionalPrompt = ref("");

// 自動承認ダイアログ状態
const showAutoApprovalPromptDialog = ref(false);
const lastJudgedCommand = ref("");

// IDE 選択
const { showIdeDialog, detectedIdes, openInIde, onIdeSelected } = useIdeSelect();

// アーティファクト
const { openArtifactViewer } = useArtifactWindow();
const artifactCount = ref(0);
const artifactUrls = ref<UrlArtifactEntry[]>([]);

async function refreshArtifactCount() {
  try {
    const list = await invoke<unknown[]>("list_artifacts", { worktreeId });
    artifactCount.value = list.length;
    artifactUrls.value = extractUrlArtifacts(list);
  } catch {
    artifactCount.value = 0;
    artifactUrls.value = [];
  }
}

async function requestOpenArtifacts() {
  await openArtifactViewer(worktreeId, worktreeName);
}

async function onSaveAutoApprovalPrompt(wid: string, prompt: string) {
  additionalPrompt.value = prompt.trim();
  showAutoApprovalPromptDialog.value = false;
  await emitTo("main", "sub-save-auto-approval-prompt", { worktreeId: wid, prompt: prompt.trim() });
}

/** ヘッダの購読バッジから開く購読関係ダイアログ（#137）。null なら閉じている。
 *  メインへ投げず**このウィンドウ内で出す**。 */
const subscriptionDialogWorktreeId = ref<string | null>(null);

/** ダイアログで選ばれたワークツリーへフォーカスする。
 *
 *  サブウィンドウが担当するのは自分のワークツリーだけなので、他のワークツリーが
 *  選ばれたらメインへ中継する（分離中ならメイン側がそのサブウィンドウを前面に出す）。
 *  自分自身が選ばれた場合は既にここが見えているので、ダイアログを閉じるだけでよい。 */
async function onFocusFromSubscriptions(wid: string) {
  subscriptionDialogWorktreeId.value = null;
  if (wid === worktreeId) return;
  await emitTo("main", "sub-focus-worktree", { worktreeId: wid });
}

// AI判定進行中フラグ
const aiJudging = ref(false);
// AI 判定中に届いた sub-try-auto-approve の預かり分（#168）
const pendingNotify = createPendingNotifyStore();

// ウィンドウのフォーカス状態
const { isWindowFocused } = useWindowFocus();

// フォーカス状態変化をメインウィンドウに同期
watch(isWindowFocused, (focused) => {
  emitTo("main", "sub-window-focus-changed", { worktreeId, focused });
});

// TerminalView ref 管理
const terminalRefs = reactive(new Map<number, InstanceType<typeof TerminalView>>());

// フレームレイアウト（useWorktreeFrameで共通化）
const {
  root,
  initLayout,
  addTerminalToLeaf,
  lastFocusedLeafId,
  setTerminalRef,
  mountTerminalsToHosts,
  getAllLeafs,
  getLeafsWithTerminals,
  switchTerminal,
  switchNextTerminal,
  switchPrevTerminal,
  closeActiveTerminal,
  closeTerminal,
  handleTerminalExit,
  resolveLeafId,
  onSplitRequest,
  onTabDrop,
  onTabEdgeDrop,
  onTabReorder,
  updateContainerSizes,
} = useWorktreeFrame({
  terminalEntries,
  terminalRefs,
  onTerminalActivated: (terminalId) => maybeInjectAiResume(terminalId),
  onTerminalClosed: async (terminalId) => {
    terminalExitCodes.delete(terminalId);
    terminalAgentStatus.delete(terminalId);
    terminalWebSessions.delete(terminalId);
    terminalAiSessions.delete(terminalId);
    pendingAiRestore.delete(terminalId);
    await emitTo("main", "sub-remove-terminal", { worktreeId, terminalId });
  },
});

// terminalId → 未投入の AI resume 情報 (#157)。sub-init の resumeAiSessions から積み、
// そのタブが初めてアクティブ表示されたときに 1 回だけ投入する。
const pendingAiRestore = new Map<number, AiSessionInfo>();

/**
 * サブウィンドウが初めてフォーカスされるまで resume 投入を遅らせる (#157)。
 *
 * 起動時に復元される分離済みワークツリーは全部まとめてサブウィンドウが作られるので、
 * `sub-init` の時点で投入すると分離ワークツリーの数だけエージェントが同時起動する。
 * メインウィンドウ側の「タブを初めて開いたとき」に相当する契機がこれ。
 */
function injectAiResumeWhenFocused(terminalId: number) {
  if (!pendingAiRestore.has(terminalId)) return;
  if (isWindowFocused.value) {
    void maybeInjectAiResume(terminalId);
    return;
  }
  const stop = watch(isWindowFocused, (focused) => {
    if (!focused) return;
    stop();
    void maybeInjectAiResume(terminalId);
  });
}

/** 復元タブが初めてアクティブ表示されたときに resume コマンドを投入する (#157) */
async function maybeInjectAiResume(terminalId: number) {
  const info = pendingAiRestore.get(terminalId);
  if (!info) return;
  // 再投入を防ぐため、成否によらず先にマーカーを落とす
  pendingAiRestore.delete(terminalId);
  const command = buildResumeCommand(info.agentType, info.sessionId);
  if (!command) return;
  const term = terminalRefs.get(terminalId);
  if (!term) return;
  try {
    // waitForReady は attachPty 失敗時に永久にハングしうるため 5 秒のタイムアウトを設ける。
    const timeout = new Promise<"timeout">((resolve) => setTimeout(() => resolve("timeout"), 5000));
    const result = await Promise.race([term.waitForReady().then(() => "ready" as const), timeout]);
    // 未 ready のまま諦めた場合はマーカーを戻し、次のアクティブ化で再試行させる
    if (result !== "ready") {
      pendingAiRestore.set(terminalId, info);
      logDebug(`[Terminal] AI resume deferred (pty not ready) terminalId=${terminalId}`);
      return;
    }
    await term.write(command.endsWith("\r") ? command : `${command}\r`);
    logDebug(`[Terminal] AI resume injected terminalId=${terminalId} agent=${info.agentType}`);
  } catch (e) {
    logDebug(`[Terminal] AI resume injection failed terminalId=${terminalId}: ${e}`);
  }
}

// ────────────────────────────────────────────────
// イベントハンドラ
// ────────────────────────────────────────────────

async function requestAddTerminal(leafId?: string) {
  if (leafId) lastFocusedLeafId.value = leafId;
  await emitTo("main", "sub-add-terminal-request", { worktreeId });
}

async function requestOpenInIde() {
  await openInIde(worktreePath, { worktreeId, worktreeName, origin: "sub" });
}

// ホーム / リポジトリの擬似ワークツリーか（ヘッダーでブランチ名の代わりにパスを出すため）
const isHome = computed(
  () => settings.value.worktrees.find((w) => w.id === worktreeId)?.isHome === true,
);
const isRepository = computed(
  () => settings.value.worktrees.find((w) => w.id === worktreeId)?.isRepository === true,
);

function onTerminalTitleChange(terminalId: number, title: string) {
  const entry = terminalEntries.get(terminalId);
  if (entry) entry.title = title;
  emitTo("main", "sub-title-update", { worktreeId, terminalId, title });
}

function onTerminalFocus(terminalId: number) {
  emitTo("main", "sub-clear-notification", { worktreeId });
  const leaf = getAllLeafs().find((l) => l.terminalIds.includes(terminalId));
  if (leaf) {
    lastFocusedLeafId.value = leaf.id;
  }
}

function getFirstLeafId(): string {
  function find(node: import("./types/frame").FrameNode): string {
    if (node.type === "leaf") return node.id;
    return find(node.children[0]);
  }
  return find(root.value);
}

// ────────────────────────────────────────────────
// イベントリスナー
// ────────────────────────────────────────────────

const { settings, loadSettings } = useSettings();

// このサブウィンドウのホットキー文字
const hotkeyChar = computed(() =>
  settings.value.worktrees.find((w) => w.id === worktreeId)?.hotkeyChar
);

// サブウィンドウのホットキーリスナー
useHotkeyListener(() => {
  const hk = settings.value.hotkeys;
  if (!hk || !initialized.value) return [];

  return [
    {
      binding: hk.homeTab,
      handler: () => {
        WebviewWindow.getByLabel("main").then(async (w) => {
          if (w) {
            await w.setFocus();
            await emitTo("main", "go-home");
          }
        });
      },
    },
    { binding: hk.terminalNext, handler: switchNextTerminal },
    { binding: hk.terminalPrev, handler: switchPrevTerminal },
    { binding: hk.terminalAdd, handler: () => requestAddTerminal(lastFocusedLeafId.value || undefined) },
    { binding: hk.terminalClose, handler: closeActiveTerminal },
  ];
});

// Alt+[char] を受けてメインに委譲（自分自身の hotkeyChar は無視）
// 未割り当ての Alt+英数字は preventDefault せずターミナル(PTY)へ透過させる。
// これを奪うと Claude Code の Alt+V(画像添付)などがサブウィンドウで効かなくなる (#150)。
// メインウィンドウ側 (useAppHotkeys.ts) と同じガード条件に揃えている。
useAltCharKeyListener((char, event) => {
  if (char === hotkeyChar.value?.toLowerCase()) return;
  // homeTab は useHotkeyListener 側で処理されるのでここでは触らない
  const homeTab = settings.value.hotkeys?.homeTab;
  if (homeTab?.alt && homeTab.key.length === 1 && homeTab.key.toLowerCase() === char) return;
  const hasTarget = settings.value.worktrees.some((w) => w.hotkeyChar === char);
  if (!hasTarget) return;
  event.preventDefault();
  event.stopPropagation();
  emitTo("main", "sub-alt-char-focus", { char });
});

const { collect } = useEventListeners();
let thumbnailInterval: ReturnType<typeof setInterval> | null = null;
let closingByMain = false;

onMounted(async () => {
  await loadSettings();
  // アーティファクト数の初期取得
  refreshArtifactCount();

  collect(await listen("settings-changed", async () => {
    await loadSettings();
  }));

  // タブ未読バッジの同期（#130）。event-inbox-changed は全 webview に届くので中継は不要。
  collect(await initTerminalUnread());

  // ヘッダの購読バッジ用に購読一覧も同期する（#137）。`initTerminalUnread` は未読件数しか
  // 読まないので、これが無いとバッジが常に 0 件（＝非表示）になる。
  collect(await initSubscriptionsScoped());

  // アーティファクト変更時にカウントを更新
  collect(await listen<{ worktreeId: string; artifactId: string; command: string }>("artifact-changed", (event) => {
    if (event.payload.worktreeId === worktreeId) {
      refreshArtifactCount();
    }
  }));

  // AIエージェントインジケーター: sessionId → terminalId に変換して terminalAgentStatus を更新
  collect(await listen<{ sessions: Record<number, { isAgent: boolean; agentName?: string; sessionId?: string }> }>("pty-ai-agent-changed", (event) => {
    const sessionToTerminal = new Map<number, number>();
    for (const [tid, entry] of terminalEntries) {
      if (entry.sessionId) sessionToTerminal.set(entry.sessionId, tid);
    }
    for (const [sessionIdStr, info] of Object.entries(event.payload.sessions)) {
      const sid = Number(sessionIdStr);
      const tid = sessionToTerminal.get(sid);
      if (tid != null) {
        if (info.isAgent) {
          terminalAgentStatus.set(tid, true);
          if (info.sessionId && info.agentName) {
            terminalAiSessions.set(tid, { agentType: info.agentName, sessionId: info.sessionId });
          }
        } else {
          terminalAgentStatus.delete(tid);
          // 復元直後の未投入タブは素のシェルなのでエージェント非検出になる。resume を
          // 投入するまでインジケータとツールチップの元データを保たせる (#157)。
          if (pendingAiRestore.has(tid)) continue;
          terminalAiSessions.delete(tid);
        }
      }
    }
  }));

  // メインウィンドウからのWebセッション情報を受信
  collect(await listen<{ terminalId: number; info: WebSessionInfo }>("sub-web-session", (event) => {
    terminalWebSessions.set(event.payload.terminalId, event.payload.info);
  }));

  // メインウィンドウからのAIセッション情報を受信
  collect(await listen<{ terminalId: number; info: AiSessionInfo }>("sub-ai-session", (event) => {
    terminalAiSessions.set(event.payload.terminalId, event.payload.info);
  }));

  const appWindow = getCurrentWindow();

  // 初期ターミナルデータを受信
  collect(await appWindow.listen<{
    worktreeId: string;
    terminals: SubTerminalEntry[];
    autoApproval?: boolean;
    autoApprovalPrompt?: string;
    layout?: FrameNode;
    webSessions?: Record<number, WebSessionInfo>;
    aiSessions?: Record<number, AiSessionInfo>;
    resumeAiSessions?: Record<number, AiSessionInfo>;
  }>(
    "sub-init",
    async (event) => {
      if (event.payload.worktreeId !== worktreeId) return;

      autoApproval.value = event.payload.autoApproval ?? false;
      additionalPrompt.value = event.payload.autoApprovalPrompt ?? "";

      for (const t of event.payload.terminals) {
        terminalEntries.set(t.id, { ...t });
        if (t.isAiAgent) {
          terminalAgentStatus.set(t.id, true);
        }
      }

      if (event.payload.webSessions) {
        for (const [idStr, info] of Object.entries(event.payload.webSessions)) {
          terminalWebSessions.set(Number(idStr), info);
        }
      }

      if (event.payload.aiSessions) {
        for (const [idStr, info] of Object.entries(event.payload.aiSessions)) {
          terminalAiSessions.set(Number(idStr), info);
        }
      }

      if (event.payload.resumeAiSessions) {
        for (const [idStr, info] of Object.entries(event.payload.resumeAiSessions)) {
          const tid = Number(idStr);
          pendingAiRestore.set(tid, info);
          // terminalAgentStatus は「エージェントが実際に動いている」フラグなので立てない
          // （メイン側の同名マップと同じ理由 / sub-get-layout の isAiAgent 経由で伝播する）。
          terminalAiSessions.set(tid, info);
        }
      }

      const ids = event.payload.terminals.map((t) => t.id);

      // レイアウト復元: layout が渡された場合はそのまま設定
      if (event.payload.layout) {
        root.value = event.payload.layout;
      } else {
        initLayout(ids);
      }

      // 最初のリーフを lastFocusedLeafId に設定
      const leafs = getAllLeafs();
      if (leafs.length > 0) {
        lastFocusedLeafId.value = leafs[0].id;
      }

      initialized.value = true;

      // terminal-host が DOM に出るまで待ってから移動
      await nextTick();
      mountTerminalsToHosts();

      const firstLeaf = leafs[0];
      if (firstLeaf?.activeTerminalId !== null && firstLeaf?.activeTerminalId !== undefined) {
        const term = terminalRefs.get(firstLeaf.activeTerminalId);
        if (term) {
          await term.handleTabActivated();
          term.focus();
          injectAiResumeWhenFocused(firstLeaf.activeTerminalId);
        }
      }
    }
  ));

  // ターミナル追加レスポンス
  collect(await appWindow.listen<{ terminalId: number; sessionId: number; title: string; pendingCommand?: string }>(
    "sub-add-terminal-response",
    async (event) => {
      const { terminalId, sessionId, title, pendingCommand } = event.payload;
      terminalEntries.set(terminalId, { id: terminalId, title, sessionId, snapshot: "" });

      // lastFocusedLeafId のリーフに追加（なければ root リーフに）
      const targetLeafId = resolveLeafId(lastFocusedLeafId.value) || getFirstLeafId();
      addTerminalToLeaf(targetLeafId, terminalId);
      lastFocusedLeafId.value = targetLeafId;

      // terminal-host が DOM に出るまで待ってから移動
      await nextTick();
      mountTerminalsToHosts();

      const term = terminalRefs.get(terminalId);
      if (term) {
        await term.handleTabActivated();
        term.focus();
        // MCP からの pendingCommand を流し込む (App.vue 側で末尾 \r 正規化済み)
        // waitForReady は attachPty 失敗時に永久にハングしうるため 5 秒のタイムアウトを設ける。
        if (pendingCommand) {
          try {
            const ready = term.waitForReady();
            const timeout = new Promise<"timeout">((resolve) => setTimeout(() => resolve("timeout"), 5000));
            const result = await Promise.race([ready.then(() => "ready" as const), timeout]);
            if (result === "ready") {
              await term.write(pendingCommand);
            }
          } catch (e) {
            // 書き込み失敗は無視 (ターミナルがすでに閉じられている等)
            void e;
          }
        }
      }
    }
  ));

  // メインウィンドウからのフォーカスリクエスト
  collect(await appWindow.listen<{ terminalId: number }>(
    "sub-focus-terminal",
    async (event) => {
      const { terminalId } = event.payload;
      const leaf = getLeafsWithTerminals().find((l) => l.terminalIds.includes(terminalId));
      if (leaf) {
        await switchTerminal(leaf.id, terminalId);
      }
    }
  ));

  // メインウィンドウが「メインに戻す」を選択
  collect(await appWindow.listen<{ worktreeId: string }>(
    "sub-closing-by-main",
    async (event) => {
      if (event.payload.worktreeId === worktreeId) {
        closingByMain = true;
        // 全ターミナルの PTY を detach (メイン側で引き継ぐため kill しない)
        for (const [, entry] of terminalEntries) {
          const termRef = terminalRefs.get(entry.id);
          if (termRef?.isRunning) {
            termRef.detach();
          }
        }
        // kill 完了をメインに通知（destroy 前に送信）
        await emitTo("main", "sub-window-closed-ack", { worktreeId });
        await appWindow.destroy();
      }
    }
  ));

  // X ボタンでクローズ
  appWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    if (!closingByMain) {
      for (const [, entry] of terminalEntries) {
        const termRef = terminalRefs.get(entry.id);
        if (termRef?.isRunning) {
          await termRef.kill();
        }
      }
      await emitTo("main", "sub-window-closing", { worktreeId });
    }
    await appWindow.destroy();
  });

  // サムネイル送信ループ（pty出力があった場合のみ送信）
  const lastThumbnailUrls = new Map<number, string>();
  thumbnailInterval = setInterval(() => {
    for (const [id] of terminalEntries) {
      const ref = terminalRefs.get(id);
      const sid = ref?.sessionId;
      if (sid == null || !isDirty(sid)) continue;
      const terminal = ref?.getTerminal();
      if (terminal) {
        const url = renderToDataUrl(terminal);
        if (url && url !== lastThumbnailUrls.get(id)) {
          lastThumbnailUrls.set(id, url);
          emitTo("main", "sub-thumbnail-update", { terminalId: id, imageUrl: url });
        }
        clearDirty(sid);
      }
    }
  }, 1000);

  // メインウィンドウからのレイアウト情報取得要求
  collect(await appWindow.listen("sub-get-layout", async () => {
    const layout = JSON.parse(JSON.stringify(root.value));
    const terminals = Array.from(terminalEntries.values()).map((entry) => {
      const termRef = terminalRefs.get(entry.id);
      const snapshot = termRef?.serializeBuffer(300) ?? entry.snapshot;
      const termObj = termRef?.getTerminal();
      return {
        id: entry.id,
        title: entry.title,
        sessionId: entry.sessionId,
        snapshot,
        isAiAgent: entry.isAiAgent ?? false,
        // 未投入の復元タブをメインへ戻すときにセッション UUID とマーカーを引き継ぐ (#157)
        aiSession: terminalAiSessions.get(entry.id),
        resumePending: pendingAiRestore.has(entry.id),
        rows: termObj?.rows ?? 24,
        cols: termObj?.cols ?? 80,
      };
    });
    const physicalSize = await appWindow.innerSize();
    const scaleFactor = await appWindow.scaleFactor();
    const windowSize = {
      width: Math.round(physicalSize.width / scaleFactor),
      height: Math.round(physicalSize.height / scaleFactor),
    };
    await emitTo("main", "sub-layout-response", { worktreeId, layout, terminals, windowSize });
  }));

  // 自動承認フラグ更新
  collect(await appWindow.listen<{ autoApproval: boolean }>(
    "sub-set-auto-approval",
    (event) => {
      autoApproval.value = event.payload.autoApproval;
    }
  ));

  // 自動承認 追加プロンプト更新
  collect(await appWindow.listen<{ prompt: string }>(
    "sub-set-auto-approval-prompt",
    (event) => {
      additionalPrompt.value = event.payload.prompt;
    }
  ));

  // 自動承認チェック（notify-worktree トリガー）
  collect(await appWindow.listen<{ additionalPrompt?: string; tray?: boolean }>("sub-try-auto-approve", async (event) => {
    if (event.payload.additionalPrompt !== undefined) {
      additionalPrompt.value = event.payload.additionalPrompt;
    }
    // tray はイベント単位の属性。メイン側の通知判定へそのまま返す（#168）
    const tray = trayOf(event.payload);
    logDebug(`[AutoApproval] sub-try-auto-approve received autoApproval=${autoApproval.value} tray=${tray}`);
    if (!autoApproval.value) {
      await emitTo("main", "sub-auto-approve-result", { worktreeId, approved: false, tray });
      return;
    }

    // 重複防止: 既にAI判定が進行中ならスキップ。ただしイベントは預かり、
    // 判定完了後に必ずメインへ返す（ここで捨てると明示通知が消える・#168）
    if (aiJudging.value) {
      queuePendingNotify(pendingNotify, worktreeId, tray);
      logDebug(`[AutoApproval] already in progress for sub-window ${worktreeId}, queued for later (tray=${tray})`);
      return;
    }

    logDebug(`[AutoApproval] terminalEntries.size=${terminalEntries.size}`);
    aiJudging.value = true;
    let loopResult: { approved: boolean; lastCommand: string | undefined };
    try {
      const terminalForApproval: TerminalForApproval[] = Array.from(terminalEntries.keys()).flatMap((tid) => {
        const ref = terminalRefs.get(tid);
        if (!ref) return [];
        return [{ id: tid, getTerminal: () => ref.getTerminal(), write: (d: string) => ref.write(d) }];
      });
      loopResult = await runApprovalLoop(terminalForApproval, worktreeId, worktreePath, additionalPrompt.value);
    } finally {
      aiJudging.value = false;
    }
    logDebug(`[AutoApproval] sub result: approved=${loopResult.approved} command=${loopResult.lastCommand ?? "none"}`);
    if (loopResult.lastCommand) lastJudgedCommand.value = loopResult.lastCommand;
    // 判定中に預かった分は同じ結果イベントに載せて返す。2 回 emit すると通知音と
    // OS 通知が重なるため、提示の畳み込みはメイン側 1 箇所に任せる（#168）
    const pending = takePendingNotify(pendingNotify, worktreeId);
    if (pending) logDebug(`[AutoApproval] flush queued notification for sub-window ${worktreeId}`);
    await emitTo("main", "sub-auto-approve-result", {
      worktreeId,
      approved: loopResult.approved,
      command: loopResult.lastCommand,
      tray,
      pendingTray: pending?.tray,
    });
  }));

  // AI判定キャンセル
  collect(await appWindow.listen("sub-cancel-auto-approve", async () => {
    logDebug(`[AutoApproval] sub-cancel-auto-approve received`);
    await invoke("cancel_approval", { worktreeId });
    aiJudging.value = false;
  }));

  // セッション保存リクエスト
  collect(await appWindow.listen("sub-session-save-request", async () => {
    const terminals: SavedTerminal[] = Array.from(terminalEntries.values()).map((entry) => {
      const termRef = terminalRefs.get(entry.id);
      return {
        title: entry.title,
        buffer: termRef?.serializeBuffer() ?? "",
        aiSession: terminalAiSessions.get(entry.id),
      };
    }).filter((t) => t.buffer !== "");
    await emitTo("main", "sub-session-save-response", { worktreeId, terminals });
  }));

  // メインに準備完了を通知
  await emitTo("main", "sub-ready", { worktreeId });
});

onUnmounted(() => {
  if (thumbnailInterval) clearInterval(thumbnailInterval);
});

async function onCancelAiJudging() {
  await invoke("cancel_approval", { worktreeId });
  aiJudging.value = false;
}
</script>

<template>
  <div
    class="h-screen flex flex-col text-[#cdd6f4] select-none"
    style="background-color: var(--bg-base)"
    :class="[{ 'gaming-border': settings.appearance?.enableGamingBorder }, settings.appearance?.enableGamingBorder ? `gaming-theme-${settings.appearance?.gamingBorderTheme ?? 'gaming'}` : '']"
  >
    <!-- 初期化中 -->
    <div v-if="!initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
      {{ t('connecting') }}
    </div>

    <template v-else>
      <!-- ヘッダー -->
      <WorktreeHeader
        :worktree-id="worktreeId"
        :worktree-name="worktreeName"
        :branch-name="branchName"
        :hotkey-char="hotkeyChar"
        :artifact-count="artifactCount"
        :artifact-urls="artifactUrls"
        :auto-approval="autoApproval"
        :ai-judging="aiJudging"
        :is-window-focused="isWindowFocused"
        :show-window-controls="true"
        :task-tooltip="getWorktreeTaskTooltip(repositoryName, branchName)"
        :is-home="isHome"
        :is-repository="isRepository"
        :home-path="worktreePath"
        @open-in-ide="requestOpenInIde"
        @open-artifacts="requestOpenArtifacts"
        @open-subscriptions="subscriptionDialogWorktreeId = $event"
        @cancel-ai-judging="onCancelAiJudging"
        @click-auto-approval="showAutoApprovalPromptDialog = true"
      />

      <!-- フレームレイアウト -->
      <div class="flex-1 min-h-0 overflow-hidden">
        <FrameContainer
          :node="root"
          :terminal-entries="terminalEntries"
          :terminal-exit-codes="terminalExitCodes"
          :terminal-agent-status="terminalAgentStatus"
          :terminal-web-sessions="terminalWebSessions"
          :terminal-ai-sessions="terminalAiSessions"
          :terminal-unread="terminalUnreadByTab"
          @switch-terminal="switchTerminal"
          @close-terminal="closeTerminal"
          @title-change="onTerminalTitleChange"
          @split-request="onSplitRequest"
          @tab-drop="onTabDrop"
          @tab-edge-drop="onTabEdgeDrop"
          @tab-reorder="onTabReorder"
          @request-add-terminal="requestAddTerminal"
          @resize-end="updateContainerSizes"
        />
      </div>
    </template>

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
      :worktree-id="worktreeId"
      :worktree-name="worktreeName"
      :current-prompt="additionalPrompt"
      :last-command="lastJudgedCommand"
      @save="onSaveAutoApprovalPrompt"
      @cancel="showAutoApprovalPromptDialog = false"
    />

    <!-- 購読関係ダイアログ（ヘッダの購読バッジから開く、#137）。
         メインへ投げずこのウィンドウ内で出す -->
    <SubscriptionFocusDialog
      v-if="subscriptionDialogWorktreeId"
      :worktree-id="subscriptionDialogWorktreeId"
      :worktree-name="worktreeName"
      @focus="onFocusFromSubscriptions"
      @cancel="subscriptionDialogWorktreeId = null"
    />

    <!-- 配送トースト (#130)。ToastService は main.ts が全ウィンドウモードに登録済みなので
         outlet を置くだけでよい。見た目を揃えるため App.vue と同じ #message スロットを使う
         (スタイルは styles.css の .toast-message-content でグローバル定義済み)。
         サブウィンドウは進行中トースト (severity=info) を出さないのでスピナーは持たない。 -->
    <Toast position="bottom-right">
      <template #message="slotProps">
        <div class="toast-message-content">
          <div>
            <div class="font-semibold">{{ slotProps.message.summary }}</div>
            <div v-if="slotProps.message.detail" class="text-sm">{{ slotProps.message.detail }}</div>
          </div>
        </div>
      </template>
    </Toast>

    <!-- TerminalView のマウント先。手動 DOM reparenting で terminal-host に移動する -->
    <div data-offscreen style="position:fixed; left:-10000px; top:-10000px; width:1000px; height:1000px; overflow:hidden; pointer-events:none">
      <template v-for="[tid, entry] in terminalEntries" :key="tid">
        <TerminalView
          :ref="(el) => setTerminalRef(tid, el)"
          :auto-start="false"
          :initial-session-id="entry.sessionId"
          :initial-snapshot="entry.snapshot"
          @exit="handleTerminalExit(tid)"
          @title-change="onTerminalTitleChange(tid, $event)"
          @focus="() => onTerminalFocus(tid)"
          @exit-code-change="(code: number) => terminalExitCodes.set(tid, code)"
        />
      </template>
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "connecting": "Connecting...",
    "ideNotInstalled": "None of Cursor, VS Code, Antigravity are installed.",
    "ideNotInstalledTitle": "IDE not found",
    "ideLaunchFailed": "Failed to launch IDE: {error}"
  },
  "ja": {
    "connecting": "接続中...",
    "ideNotInstalled": "Cursor、VS Code、Antigravity のいずれもインストールされていません。",
    "ideNotInstalledTitle": "IDE が見つかりません",
    "ideLaunchFailed": "IDE の起動に失敗しました: {error}"
  }
}
</i18n>
