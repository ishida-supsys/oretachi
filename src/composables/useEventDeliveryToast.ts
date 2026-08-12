import { onScopeDispose } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useToast } from "primevue/usetoast";
import { useI18n } from "vue-i18n";
import type { DeliveredPayload, SpawnRejectedPayload } from "../types/event";

/** ワークツリー間イベントの配送トースト（issue #120 §7 / #125 / #130）。
 *
 *  配送を画面に出さないと「勝手にエージェントが動き出した」ように見える。
 *  #125 の可視化はメインウィンドウに閉じていたが、**サブウィンドウへ分離した
 *  ワークツリーこそ黙って動き出したように見える**ので、同じトーストを共有する。
 *
 *  ## 表示責任
 *
 *  Rust 側は `app.emit`（全 webview へのブロードキャスト）なので、素直に listen すると
 *  1回の配送で複数ウィンドウがトーストする。`shouldShow` で「そのワークツリーを表示して
 *  いるウィンドウ」だけに絞り、1配送=1トーストを保つ。
 *
 *  ## イベント種別で分岐しないこと
 *
 *  本文は Rust 側（`format_inbox_push_text`）が組み立てた `payload.text` を**そのまま**出す。
 *  `DeliveredPayload` に種別フィールドは無く、ここで `worktree.closed` を前提に文面を
 *  組み立てることもしない。#126 が `worktree.message`（自由文）を足しても、それ以外の
 *  未知の種別が来ても、この経路は壊れない。 */

/** トーストの detail に載せる本文の上限。トーストは幅 260px なので全文（最大 600 字）は入らない。
 *  全文はエージェント側の画面とホームの購読パネルで読める。 */
const DETAIL_MAX_CHARS = 120;

export interface EventDeliveryToastOptions {
  /** この配送 / 拒否をこのウィンドウが表示すべきか。
   *  `worktreeId` はワークツリーを解決できなかったとき null になりうる。 */
  shouldShow: (worktreeId: string | null) => boolean;
}

/** `event-delivered` / `event-spawn-rejected` を listen してトーストを出す。
 *
 *  **`<script setup>` の同期部分から呼ぶこと。** `useToast` / `useI18n` はコンポーネント
 *  インスタンスが有効な間しか呼べず、`onMounted` の `await` を跨いだ後では失敗する。
 *  listen の登録自体は非同期に進み、スコープ破棄時に自動で解除される。 */
export function useEventDeliveryToast(opts: EventDeliveryToastOptions): { stop: () => void } {
  const toast = useToast();
  // 呼び出し側の SFC がローカル <i18n> ブロックを持つとスコープがそちらになるため、
  // 明示的にグローバルカタログ（src/i18n/{en,ja}.ts）を見る。
  const { t } = useI18n({ useScope: "global" });

  let unlisteners: UnlistenFn[] = [];
  let stopped = false;

  function onDelivered(payload: DeliveredPayload): void {
    if (!opts.shouldShow(payload.worktreeId ?? null)) return;
    // 本文が欠けていても summary だけで成立させる（未知の種別・将来のペイロード変更に備える）
    const text = typeof payload.text === "string" ? payload.text : "";
    const detail = text.length > DETAIL_MAX_CHARS ? `${text.slice(0, DETAIL_MAX_CHARS)}…` : text;
    toast.add({
      severity: "success",
      summary: t("eventDelivery.deliveredSummary", {
        // 文言が {name} を描画するので、名前が引けないときも空にしない
        name: payload.worktreeName ?? payload.worktreeId ?? "?",
        count: payload.count ?? 0,
      }),
      detail: detail || undefined,
      life: 8000,
    });
  }

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
    const fns = await Promise.all([
      listen<DeliveredPayload>("event-delivered", (e) => onDelivered(e.payload)),
      listen<SpawnRejectedPayload>("event-spawn-rejected", (e) => onSpawnRejected(e.payload)),
    ]);
    // 登録が終わる前に stop されていたら、その場で解除する（取り残すとリークする）
    if (stopped) {
      for (const fn of fns) fn();
      return;
    }
    unlisteners = fns;
  })();

  function stop(): void {
    if (stopped) return;
    stopped = true;
    for (const fn of unlisteners) fn();
    unlisteners = [];
  }

  onScopeDispose(stop);

  return { stop };
}
