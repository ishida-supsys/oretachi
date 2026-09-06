import { ref, computed, type Ref, type ComputedRef } from "vue";

/**
 * アーティファクトビューアの「戻る / 進む」履歴スタック。
 *
 * 同一ウィンドウ内の遷移だけを積む。ウィンドウを跨ぐ遷移（別ワークツリーのビューアを
 * 開く / 既存ビューアへ artifact-navigate を投げる）は受け側で `replace` を使い、
 * スタックを伸ばさない。
 *
 * 本文の読み込みは持たない（呼び出し側の責務）。ここは ID の並びと現在位置だけを管理する。
 */
export interface ArtifactHistory {
  entries: Ref<string[]>;
  index: Ref<number>;
  current: ComputedRef<string | null>;
  canGoBack: ComputedRef<boolean>;
  canGoForward: ComputedRef<boolean>;
  push: (id: string) => void;
  replace: (id: string) => void;
  prune: (removedId: string) => void;
  moveTo: (nextIndex: number) => void;
}

export function useArtifactHistory(): ArtifactHistory {
  const entries = ref<string[]>([]);
  const index = ref(-1);

  const current = computed(() => (index.value < 0 ? null : entries.value[index.value] ?? null));
  const canGoBack = computed(() => index.value > 0);
  const canGoForward = computed(() => index.value < entries.value.length - 1);

  /** 現在位置の先を捨てて新しい遷移を積む（ブラウザの履歴と同じ挙動） */
  function push(id: string) {
    if (entries.value[index.value] === id) return;
    entries.value = [...entries.value.slice(0, index.value + 1), id];
    index.value = entries.value.length - 1;
  }

  /** 現在位置を差し替える。スタックの深さは変わらない */
  function replace(id: string) {
    if (index.value < 0) {
      entries.value = [id];
      index.value = 0;
      return;
    }
    const next = entries.value.slice(0, index.value + 1);
    next[index.value] = id;
    entries.value = next;
  }

  /** 削除されたアーティファクトを履歴から抜き、現在位置を追従させる */
  function prune(removedId: string) {
    if (!entries.value.includes(removedId)) return;
    let removedBefore = 0;
    for (let i = 0; i < index.value; i++) {
      if (entries.value[i] === removedId) removedBefore += 1;
    }
    const wasCurrent = entries.value[index.value] === removedId;
    entries.value = entries.value.filter((id) => id !== removedId);
    // 現在位置が消えた場合は1つ前へ下がる（タブを閉じたときと同じ挙動）
    const next = index.value - removedBefore - (wasCurrent ? 1 : 0);
    index.value = Math.max(-1, Math.min(next, entries.value.length - 1));
  }

  function moveTo(nextIndex: number) {
    if (nextIndex < 0 || nextIndex >= entries.value.length) return;
    index.value = nextIndex;
  }

  return { entries, index, current, canGoBack, canGoForward, push, replace, prune, moveTo };
}
