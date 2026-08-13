import { onScopeDispose } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useToast } from "primevue/usetoast";
import { useI18n } from "vue-i18n";
import type { SpawnWarningPayload } from "../types/event";

/** ワークツリー間イベントのトースト（issue #120 §7 / #125 / #130 / #137）。
 *
 *  ## 配送トーストは出さない（#137）
 *
 *  かつては `event-delivered` を listen して配送のたびにトーストを出していたが、画面が
 *  動くわりに「今どのワークツリーが何を購読しているか」という**状態**が分からなかった。
 *  購読状態はワークツリーカードの購読バッジが常時見せる方式へ移したので、この経路は
 *  丸ごと廃止した（Rust 側の emit も削除済み）。
 *
 *  ここに残るのは `event-spawn-warning` だけ。これは「イベントが届いた」通知ではなく
 *  **「自動化が危険域で動いた」通知**で、spawn 自体は通っている。端末数と webview ハングに
 *  相関があるため、黙って増やすと人間が気づけないまま画面が固まる。
 *
 *  ## 表示責任
 *
 *  Rust 側は `app.emit`（全 webview へのブロードキャスト）なので、素直に listen すると
 *  1回の警告で複数ウィンドウがトーストする。`shouldShow` で「そのワークツリーを表示して
 *  いるウィンドウ」だけに絞り、1件=1トーストを保つ。 */

export interface EventToastOptions {
  /** この通知をこのウィンドウが表示すべきか。
   *  `worktreeId` はワークツリーを解決できなかったとき null になりうる。 */
  shouldShow: (worktreeId: string | null) => boolean;
}

/** `event-spawn-warning` を listen してトーストを出す。
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

  // 端末数が危険域にある状態で自動 spawn した。**止めてはいない**ので、これを出さないと
  // 人間は端末が増え続けていることに気づけない。
  function onSpawnWarning(payload: SpawnWarningPayload): void {
    if (!opts.shouldShow(payload.worktreeId ?? null)) return;
    toast.add({
      severity: "warn",
      summary: t("eventDelivery.spawnWarningSummary"),
      detail: t("eventDelivery.spawnWarningDetail", {
        name: payload.worktreeName ?? "",
        live: payload.liveSessions,
        threshold: payload.threshold,
        pending: payload.pending,
      }),
      life: 10000,
    });
  }

  void (async () => {
    const fn = await listen<SpawnWarningPayload>("event-spawn-warning", (e) =>
      onSpawnWarning(e.payload),
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
