import { ref, nextTick } from "vue";
import { useFrameLayout } from "./useFrameLayout";
import { useTerminalReparenting } from "./useTerminalReparenting";
import type { SubTerminalEntry } from "../types/terminal";
import type TerminalView from "../components/TerminalView.vue";

/**
 * ワークツリー単位のフレームレイアウト操作を共通化するcomposable。
 * SubWindowApp / App.vue (メインウィンドウのワークツリーペイン) で共用する。
 */
export function useWorktreeFrame(options: {
  terminalEntries: Map<number, SubTerminalEntry>;
  terminalRefs: Map<number, InstanceType<typeof TerminalView>>;
  /** ターミナルが閉じられた後に呼ばれるコールバック（App.vue固有のクリーンアップ等） */
  onTerminalClosed?: (terminalId: number) => void | Promise<void>;
}) {
  const { terminalEntries, terminalRefs, onTerminalClosed } = options;

  const {
    root,
    initLayout,
    addTerminalToLeaf,
    removeTerminalFromLeaf,
    moveTerminal,
    setActiveTerminal,
    splitLeaf,
    pruneTree,
    findLeafByTerminalId,
    getAllLeafs,
  } = useFrameLayout();

  const lastFocusedLeafId = ref<string>("");

  const { setTerminalRef, returnAllToOffscreen, mountTerminalsToHosts } =
    useTerminalReparenting(terminalEntries, terminalRefs);

  function getLeafsWithTerminals() {
    return getAllLeafs().filter((l) => l.terminalIds.length > 0);
  }

  async function switchTerminal(leafId: string, terminalId: number) {
    setActiveTerminal(leafId, terminalId);
    lastFocusedLeafId.value = leafId;
    await nextTick();
    const term = terminalRefs.get(terminalId);
    if (term) {
      await term.handleTabActivated();
      term.focus();
    }
  }

  async function closeTerminal(leafId: string, terminalId: number) {
    const entry = terminalEntries.get(terminalId);
    if (!entry) return;

    const term = terminalRefs.get(terminalId);
    if (term?.isRunning) {
      await term.kill();
    }

    // kill 中に pty-exit → 再入で既に削除済みの可能性
    if (!terminalEntries.has(terminalId)) return;

    returnAllToOffscreen();

    terminalEntries.delete(terminalId);
    removeTerminalFromLeaf(leafId, terminalId);
    pruneTree();

    if (onTerminalClosed) {
      await onTerminalClosed(terminalId);
    }

    await nextTick();
    mountTerminalsToHosts();

    for (const [tid] of terminalEntries) {
      const t = terminalRefs.get(tid);
      if (t) await t.handleTabActivated();
    }

    // 削除後にアクティブなターミナルにフォーカス
    const leafs = getLeafsWithTerminals();
    if (leafs.length > 0) {
      const firstLeaf = leafs[0];
      if (firstLeaf.activeTerminalId !== null) {
        const activeTerm = terminalRefs.get(firstLeaf.activeTerminalId);
        if (activeTerm) activeTerm.focus();
      }
    }
  }

  function handleTerminalExit(tid: number) {
    const leaf = findLeafByTerminalId(tid);
    if (leaf) closeTerminal(leaf.id, tid);
  }

  async function onSplitRequest(leafId: string, direction: "left" | "right" | "top" | "bottom") {
    returnAllToOffscreen();
    splitLeaf(leafId, direction);
    lastFocusedLeafId.value = leafId;
    await nextTick();
    mountTerminalsToHosts();
    for (const [tid] of terminalEntries) {
      const term = terminalRefs.get(tid);
      if (term) await term.handleTabActivated();
    }
  }

  function onTabReorder(leafId: string, terminalId: number, insertIndex: number) {
    moveTerminal(terminalId, leafId, leafId, insertIndex);
  }

  async function onTabDrop(
    sourceLeafId: string,
    terminalId: number,
    targetLeafId: string,
    insertIndex?: number
  ) {
    if (sourceLeafId === targetLeafId) return;

    returnAllToOffscreen();
    moveTerminal(terminalId, sourceLeafId, targetLeafId, insertIndex);
    pruneTree();
    lastFocusedLeafId.value = targetLeafId;

    await nextTick();
    mountTerminalsToHosts();

    for (const [tid] of terminalEntries) {
      const term = terminalRefs.get(tid);
      if (term) await term.handleTabActivated();
    }
    const movedTerm = terminalRefs.get(terminalId);
    if (movedTerm) movedTerm.focus();
  }

  async function onTabEdgeDrop(
    sourceLeafId: string,
    terminalId: number,
    targetLeafId: string,
    direction: "left" | "right" | "top" | "bottom"
  ) {
    returnAllToOffscreen();
    const newLeaf = splitLeaf(targetLeafId, direction);
    moveTerminal(terminalId, sourceLeafId, newLeaf.id);
    pruneTree();
    lastFocusedLeafId.value = newLeaf.id;

    await nextTick();
    mountTerminalsToHosts();

    for (const [tid] of terminalEntries) {
      const term = terminalRefs.get(tid);
      if (term) await term.handleTabActivated();
    }
    const movedTerm = terminalRefs.get(terminalId);
    if (movedTerm) movedTerm.focus();
  }

  return {
    root,
    initLayout,
    addTerminalToLeaf,
    setActiveTerminal,
    lastFocusedLeafId,
    setTerminalRef,
    returnAllToOffscreen,
    mountTerminalsToHosts,
    getAllLeafs,
    getLeafsWithTerminals,
    switchTerminal,
    closeTerminal,
    handleTerminalExit,
    onSplitRequest,
    onTabDrop,
    onTabEdgeDrop,
    onTabReorder,
  };
}
