/** リーフ: タブグループ（1ペイン内で複数ターミナルをタブ切替） */
export interface FrameLeaf {
  type: "leaf";
  id: string;
  terminalIds: number[];
  activeTerminalId: number | null;
}

/** コンテナ: Splitter で子を並べる */
export interface FrameContainer {
  type: "container";
  id: string;
  layout: "horizontal" | "vertical";
  children: FrameNode[];
  sizes: number[]; // 各 child の % サイズ (合計100)
}

export type FrameNode = FrameLeaf | FrameContainer;
