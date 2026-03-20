import { ref } from "vue";
import type { FrameLeaf, FrameContainer, FrameNode } from "../types/frame";

let idCounter = 0;
function newId(prefix: string): string {
  return `${prefix}-${++idCounter}`;
}

function makeLeaf(terminalIds: number[]): FrameLeaf {
  return {
    type: "leaf",
    id: newId("leaf"),
    terminalIds: [...terminalIds],
    activeTerminalId: terminalIds[0] ?? null,
  };
}

function makeContainer(
  layout: "horizontal" | "vertical",
  children: FrameNode[],
  sizes: number[]
): FrameContainer {
  return {
    type: "container",
    id: newId("container"),
    layout,
    children,
    sizes,
  };
}

export function useFrameLayout() {
  const root = ref<FrameNode>(makeLeaf([]));

  // ────────────────────────────────────────────────
  // 基本検索
  // ────────────────────────────────────────────────

  function findLeafById(nodeId: string, node: FrameNode = root.value): FrameLeaf | null {
    if (node.type === "leaf") return node.id === nodeId ? node : null;
    for (const child of node.children) {
      const result = findLeafById(nodeId, child);
      if (result) return result;
    }
    return null;
  }

  function findLeafByTerminalId(terminalId: number, node: FrameNode = root.value): FrameLeaf | null {
    if (node.type === "leaf") {
      return node.terminalIds.includes(terminalId) ? node : null;
    }
    for (const child of node.children) {
      const result = findLeafByTerminalId(terminalId, child);
      if (result) return result;
    }
    return null;
  }

  function findParent(
    nodeId: string,
    node: FrameNode = root.value,
    parent: FrameContainer | null = null,
    parentIndex: number = -1
  ): { parent: FrameContainer; index: number } | null {
    if (node.id === nodeId) {
      if (parent) return { parent, index: parentIndex };
      return null; // root は親なし
    }
    if (node.type === "container") {
      for (let i = 0; i < node.children.length; i++) {
        const result = findParent(nodeId, node.children[i], node, i);
        if (result) return result;
      }
    }
    return null;
  }

  // ────────────────────────────────────────────────
  // 初期化
  // ────────────────────────────────────────────────

  function initLayout(terminalIds: number[]) {
    root.value = makeLeaf(terminalIds);
  }

  // ────────────────────────────────────────────────
  // 分割
  // ────────────────────────────────────────────────

  /**
   * leafId のリーフを指定方向に分割して新しいリーフを返す。
   * direction: "left"|"right" → horizontal, "top"|"bottom" → vertical
   * 新リーフには terminalIds を格納する（省略時は空）。
   */
  function splitLeaf(
    leafId: string,
    direction: "left" | "right" | "top" | "bottom",
    terminalIds: number[] = []
  ): FrameLeaf {
    const layout = direction === "left" || direction === "right" ? "horizontal" : "vertical";
    const insertBefore = direction === "left" || direction === "top";
    const newLeaf = makeLeaf(terminalIds);

    const parentInfo = findParent(leafId);
    const existingLeaf = findLeafById(leafId);
    if (!existingLeaf) return newLeaf;

    if (parentInfo === null) {
      // root を分割
      const sizes = [50, 50];
      const children: FrameNode[] = insertBefore
        ? [newLeaf, existingLeaf]
        : [existingLeaf, newLeaf];
      root.value = makeContainer(layout, children, sizes);
    } else {
      const { parent, index } = parentInfo;

      if (parent.layout === layout) {
        // 同方向: 親コンテナの children に挿入
        const insertIndex = insertBefore ? index : index + 1;
        parent.children.splice(insertIndex, 0, newLeaf);
        // sizes を均等再分配
        const count = parent.children.length;
        parent.sizes = Array(count).fill(Math.floor(100 / count));
        // 端数を最後に加算
        const sum = parent.sizes.reduce((a, b) => a + b, 0);
        parent.sizes[parent.sizes.length - 1] += 100 - sum;
      } else {
        // 異方向: 既存リーフを新コンテナで包む
        const sizes = [50, 50];
        const children: FrameNode[] = insertBefore
          ? [newLeaf, existingLeaf]
          : [existingLeaf, newLeaf];
        const newContainer = makeContainer(layout, children, sizes);
        parent.children[index] = newContainer;
      }
    }

    return newLeaf;
  }

  // ────────────────────────────────────────────────
  // ターミナル操作
  // ────────────────────────────────────────────────

  function addTerminalToLeaf(leafId: string, terminalId: number) {
    const leaf = findLeafById(leafId);
    if (!leaf) return;
    if (!leaf.terminalIds.includes(terminalId)) {
      leaf.terminalIds.push(terminalId);
    }
    leaf.activeTerminalId = terminalId;
  }

  function removeTerminalFromLeaf(leafId: string, terminalId: number) {
    const leaf = findLeafById(leafId);
    if (!leaf) return;
    const idx = leaf.terminalIds.indexOf(terminalId);
    if (idx === -1) return;
    leaf.terminalIds.splice(idx, 1);
    if (leaf.activeTerminalId === terminalId) {
      leaf.activeTerminalId = leaf.terminalIds[Math.max(0, idx - 1)] ?? null;
    }
  }

  function moveTerminal(
    terminalId: number,
    sourceLeafId: string,
    targetLeafId: string,
    insertIndex?: number
  ) {
    const source = findLeafById(sourceLeafId);
    const target = findLeafById(targetLeafId);
    if (!source || !target) return;

    if (sourceLeafId === targetLeafId) {
      // 同一リーフ内の並び替え
      if (insertIndex === undefined) return;
      const arr = source.terminalIds;
      const oldIdx = arr.indexOf(terminalId);
      if (oldIdx === -1) return;
      if (oldIdx === insertIndex || oldIdx === insertIndex - 1) return;
      arr.splice(oldIdx, 1);
      const adjusted = insertIndex > oldIdx ? insertIndex - 1 : insertIndex;
      arr.splice(adjusted, 0, terminalId);
      source.activeTerminalId = terminalId;
      return;
    }

    removeTerminalFromLeaf(sourceLeafId, terminalId);
    if (insertIndex !== undefined) {
      target.terminalIds.splice(insertIndex, 0, terminalId);
    } else {
      target.terminalIds.push(terminalId);
    }
    target.activeTerminalId = terminalId;
  }

  function setActiveTerminal(leafId: string, terminalId: number) {
    const leaf = findLeafById(leafId);
    if (leaf) leaf.activeTerminalId = terminalId;
  }

  // ────────────────────────────────────────────────
  // ツリー整理
  // ────────────────────────────────────────────────

  /**
   * 空リーフを削除し、子が1つのコンテナを子で置換する（再帰）。
   * root が空リーフになった場合はそのまま残す。
   */
  function pruneNode(node: FrameNode): FrameNode | null {
    if (node.type === "leaf") {
      return node.terminalIds.length === 0 ? null : node;
    }

    // 子を再帰的にプルーン
    const prunedChildren: FrameNode[] = [];
    for (const child of node.children) {
      const pruned = pruneNode(child);
      if (pruned) prunedChildren.push(pruned);
    }

    if (prunedChildren.length === 0) return null;
    if (prunedChildren.length === 1) return prunedChildren[0];

    // sizes を再計算（均等）
    const count = prunedChildren.length;
    const sizes = Array(count).fill(Math.floor(100 / count));
    sizes[sizes.length - 1] += 100 - sizes.reduce((a, b) => a + b, 0);

    return {
      ...node,
      children: prunedChildren,
      sizes,
    };
  }

  function pruneTree() {
    const result = pruneNode(root.value);
    if (result === null) {
      // 全部空になったら空リーフをrootに
      root.value = makeLeaf([]);
    } else {
      root.value = result;
    }
  }

  // ────────────────────────────────────────────────
  // ユーティリティ
  // ────────────────────────────────────────────────

  function getAllLeafs(): FrameLeaf[] {
    const leafs: FrameLeaf[] = [];
    function collect(node: FrameNode) {
      if (node.type === "leaf") {
        leafs.push(node);
      } else {
        node.children.forEach(collect);
      }
    }
    collect(root.value);
    return leafs;
  }

  return {
    root,
    initLayout,
    findLeafByTerminalId,
    splitLeaf,
    addTerminalToLeaf,
    removeTerminalFromLeaf,
    moveTerminal,
    setActiveTerminal,
    pruneTree,
    getAllLeafs,
  };
}
