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
