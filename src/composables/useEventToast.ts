import { onScopeDispose } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useToast } from "primevue/usetoast";
import { useI18n } from "vue-i18n";
import type { SpawnRejectedPayload } from "../types/event";

/** ワークツリー間イベントのトースト（issue #120 §7 / #125 / #130 / #137）。
 *
 *  ## 配送トーストは出さない（#137）
 *
 *  かつては `event-delivered` を listen して配送のたびにトーストを出していたが、画面が
 *  動くわりに「今どのワークツリーが何を購読しているか」という**状態**が分からなかった。
 *  購読状態はワークツリーカードの購読バッジが常時見せる方式へ移したので、この経路は
 *  丸ごと廃止した（Rust 側の emit も削除済み）。
 *
 *  ここに残るのは `event-spawn-rejected` だけ。これは「イベントが届いた」通知ではなく
 *  **「自動化が意図的に止まった」通知**で、黙って消すと「spawn すると言ったのに何も
 *  起きない」になり人間が気づけない。
 *
 *  ## 表示責任
 *
 *  Rust 側は `app.emit`（全 webview へのブロードキャスト）なので、素直に listen すると
 *  1回の拒否で複数ウィンドウがトーストする。`shouldShow` で「そのワークツリーを表示して
 *  いるウィンドウ」だけに絞り、1件=1トーストを保つ。 */

export interface EventToastOptions {
  /** この通知をこのウィンドウが表示すべきか。
   *  `worktreeId` はワークツリーを解決できなかったとき null になりうる。 */
  shouldShow: (worktreeId: string | null) => boolean;
}

/** `event-spawn-rejected` を listen してトーストを出す。
 *
 *  **`<script setup>` の同期部分から呼ぶこと。** `useToast` / `useI18n` はコンポーネント
 *  インスタンスが有効な間しか呼べず、`onMounted` の `await` を跨いだ後では失敗する。
 *  listen の登録自体は非同期に進み、スコープ破棄時に自動で解除される。 */
export function useEventToast(opts: EventToastOptions): { stop: () => void } {
  const toast = useToast();
  // 呼び出し側の SFC がローカル <i18n> ブロックを持つとスコープがそちらになるため、
  // 明示的にグローバルカタログ（src/i18n/{en,ja}.ts）を見る。
  const { t } = useI18n({ useScope: "global" });

  let unlisten: UnlistenFn | null = null;
  let stopped = false;

  // 自動 spawn が端末数上限で拒否された。黙って落とすと「spawn すると言ったのに
  // 何も起きない」になるので必ず出す。
  function onSpawnRejected(payload: SpawnRejectedPayload): void {
    if (!opts.shouldShow(payload.worktreeId ?? null)) return;
    toast.add({
      severity: "warn",
      summary: t("eventDelivery.spawnRejectedSummary"),
      detail: t("eventDelivery.spawnRejectedDetail", {
        name: payload.worktreeName ?? "",
        live: payload.liveSessions,
        limit: payload.limit,
        pending: payload.pending,
      }),
      life: 10000,
    });
  }

  void (async () => {
    const fn = await listen<SpawnRejectedPayload>("event-spawn-rejected", (e) =>
      onSpawnRejected(e.payload),
    );
    // 登録が終わる前に stop されていたら、その場で解除する（取り残すとリークする）
    if (stopped) {
      fn();
      return;
    }
    unlisten = fn;
  })();

  function stop(): void {
    if (stopped) return;
    stopped = true;
    unlisten?.();
    unlisten = null;
  }

  onScopeDispose(stop);

  return { stop };
}
