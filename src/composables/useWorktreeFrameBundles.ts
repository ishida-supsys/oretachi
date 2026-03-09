import { ref, reactive, shallowReactive, nextTick } from "vue";
import type { Ref } from "vue";
import { useWorktreeFrame } from "./useWorktreeFrame";
import type { SubTerminalEntry } from "../types/terminal";
import type { Worktree } from "../types/worktree";
import type { FrameNode } from "../types/frame";
import type TerminalView from "../components/TerminalView.vue";

export interface WorktreeFrameBundle {
  frame: ReturnType<typeof useWorktreeFrame>;
  terminalEntries: Map<number, SubTerminalEntry>;
  terminalRefs: Map<number, InstanceType<typeof TerminalView>>;
}

/**
 * メインウィンドウのワークツリーごとのフレームバンドル管理composable。
 * バンドルのライフサイクル・切替・フレーム操作ヘルパーを提供する。
 */
export function useWorktreeFrameBundles(options: {
  worktrees: Ref<Worktree[]>;
  viewMode: Ref<string>;
  terminalWorktreeMap: Map<number, string>;
  terminalExitCodes: Map<number, number>;
  terminalAgentStatus: Map<number, boolean>;
  removeTerminal: (worktreeId: string, terminalId: number) => void;
  clearNotification: (worktreeId: string) => void;
}) {
  const {
    worktrees,
    viewMode,
    terminalWorktreeMap,
    terminalExitCodes,
    terminalAgentStatus,
    removeTerminal,
    clearNotification,
  } = options;

  const bundles = shallowReactive(new Map<string, WorktreeFrameBundle>());
  const activeWorktreeId = ref<string | null>(null);

  /** worktreeId のバンドルを取得（なければ作成）*/
  function ensureWorktreeFrame(worktreeId: string, restoreLayout?: FrameNode) {
    if (bundles.has(worktreeId)) return;
    const entries = reactive(new Map<number, SubTerminalEntry>());
    const refs = reactive(new Map<number, InstanceType<typeof TerminalView>>());
    const frame = useWorktreeFrame({
      terminalEntries: entries,
      terminalRefs: refs,
      onTerminalClosed: async (terminalId) => {
        terminalWorktreeMap.delete(terminalId);
        terminalExitCodes.delete(terminalId);
        terminalAgentStatus.delete(terminalId);
        removeTerminal(worktreeId, terminalId);
      },
    });

    const wt = worktrees.value.find((w) => w.id === worktreeId);
    if (wt && wt.terminals.length > 0) {
      for (const t of wt.terminals) {
        entries.set(t.id, { id: t.id, title: t.title, sessionId: 0, snapshot: "" });
      }
      if (restoreLayout) {
        frame.root.value = restoreLayout;
      } else {
        frame.initLayout(wt.terminals.map((t) => t.id));
      }
      frame.lastFocusedLeafId.value = frame.getAllLeafs()[0]?.id ?? "";
    }

    bundles.set(worktreeId, { frame, terminalEntries: entries, terminalRefs: refs });
  }

  function getTerminalRef(terminalId: number): InstanceType<typeof TerminalView> | undefined {
    const wid = terminalWorktreeMap.get(terminalId);
    if (!wid) return undefined;
    return bundles.get(wid)?.terminalRefs.get(terminalId);
  }

  async function switchToWorktree(worktreeId: string) {
    clearNotification(worktreeId);
    viewMode.value = "terminal";
    activeWorktreeId.value = worktreeId;
    await nextTick();
    const bundle = bundles.get(worktreeId);
    if (bundle) {
      bundle.frame.mountTerminalsToHosts();
      const leafs = bundle.frame.getLeafsWithTerminals();
      if (leafs.length > 0) {
        const leaf = leafs[0];
        if (leaf.activeTerminalId !== null) {
          const term = bundle.terminalRefs.get(leaf.activeTerminalId);
          if (term) {
            await term.handleTabActivated();
            term.focus();
          }
        }
      }
    }
  }

  // ────────────────────────────────────────────────
  // フレーム操作ヘルパー（テンプレートから呼び出し）
  // ────────────────────────────────────────────────

  function onFrameSwitch(wid: string, leafId: string, tid: number) {
    bundles.get(wid)?.frame.switchTerminal(leafId, tid);
  }

  function onFrameClose(wid: string, leafId: string, tid: number) {
    bundles.get(wid)?.frame.closeTerminal(leafId, tid);
  }

  function onFrameTabDrop(
    wid: string,
    srcLeafId: string,
    tid: number,
    tgtLeafId: string,
    insertIndex?: number
  ) {
    bundles.get(wid)?.frame.onTabDrop(srcLeafId, tid, tgtLeafId, insertIndex);
  }

  function onFrameTabEdgeDrop(
    wid: string,
    srcLeafId: string,
    tid: number,
    tgtLeafId: string,
    dir: "left" | "right" | "top" | "bottom"
  ) {
    bundles.get(wid)?.frame.onTabEdgeDrop(srcLeafId, tid, tgtLeafId, dir);
  }

  function onFrameTabReorder(wid: string, leafId: string, tid: number, insertIndex: number) {
    bundles.get(wid)?.frame.onTabReorder(leafId, tid, insertIndex);
  }

  return {
    bundles,
    activeWorktreeId,
    ensureWorktreeFrame,
    getTerminalRef,
    switchToWorktree,
    onFrameSwitch,
    onFrameClose,
    onFrameTabDrop,
    onFrameTabEdgeDrop,
    onFrameTabReorder,
  };
}
