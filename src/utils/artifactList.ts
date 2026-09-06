import type { ArtifactMeta } from "../types/artifact";

/**
 * ピン止め desc → updated_at desc で並べ替える。
 * pinned は状態サイドカー由来でフロントしか持たないため、Rust 側ではなくここでソートする。
 */
export function sortArtifacts(
  artifacts: ArtifactMeta[],
  isPinned: (id: string) => boolean,
): ArtifactMeta[] {
  return [...artifacts].sort((a, b) => {
    const pinDiff = Number(isPinned(b.id)) - Number(isPinned(a.id));
    if (pinDiff !== 0) return pinDiff;
    return b.updated_at - a.updated_at;
  });
}

/**
 * サイドバーの絞り込み。タイトルと content_type のみが対象で、本文は見ない。
 * query が空なら元の配列をそのまま返す。
 */
export function filterArtifacts(artifacts: ArtifactMeta[], query: string): ArtifactMeta[] {
  const q = query.trim().toLowerCase();
  if (!q) return artifacts;
  return artifacts.filter(
    (a) => a.title.toLowerCase().includes(q) || a.content_type.toLowerCase().includes(q),
  );
}

/** 選択履歴のスタックと現在位置。位置 -1 は「まだどこにも居ない」を表す */
export interface SelectionHistory {
  entries: string[];
  index: number;
}

/** 空の履歴を新しく作る（共有インスタンスを配ると取り違えの元なので毎回生成する） */
export function createHistory(): SelectionHistory {
  return { entries: [], index: -1 };
}

/**
 * 新しい選択を積む。現在位置より後ろ（進む側）は捨てる。
 * 現在位置と同じ ID なら何も動かさない（同じ場所への再選択は遷移ではない）。
 */
export function pushHistory(history: SelectionHistory, id: string): SelectionHistory {
  if (history.entries[history.index] === id) return history;
  const entries = [...history.entries.slice(0, history.index + 1), id];
  return { entries, index: entries.length - 1 };
}

/**
 * 存在しなくなった ID を履歴から取り除く。
 * 現在位置は「元の位置以前で生き残った最後の要素」へ寄せる（無ければ -1）。
 *
 * 間に挟まっていた ID が消えると隣接重複ができる（["a","b","a"] から "b" が消えると
 * ["a","a"]）。そのまま残すと「戻る」で同じ ID を選び直すことになり、表示が変わらず
 * ボタンが無反応に見えるので、ここで畳み込む。
 */
export function pruneHistory(history: SelectionHistory, aliveIds: Iterable<string>): SelectionHistory {
  const alive = new Set(aliveIds);
  const entries: string[] = [];
  let index = -1;
  history.entries.forEach((id, i) => {
    if (!alive.has(id)) return;
    if (entries[entries.length - 1] !== id) entries.push(id);
    if (i <= history.index) index = entries.length - 1;
  });
  return { entries, index };
}

export function canGoBack(history: SelectionHistory): boolean {
  return history.index > 0;
}

export function canGoForward(history: SelectionHistory): boolean {
  return history.index < history.entries.length - 1;
}
