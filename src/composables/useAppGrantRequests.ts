import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppGrantRequest } from "../types/appInstance";

/**
 * アプリの権限承認ダイアログ（Rust の `app-grant-request`）を main ウィンドウで受ける。
 *
 * 要求は Rust が解決まで済ませて送ってくる（承認時に再解決しない＝表示したものと保存するものが
 * 一致する）。差し替え・取り下げは `app-grant-cancelled` で届き、該当の要求をキューから外す。
 * 応答は `app_grant_respond`（main だけが呼べる）。複数インスタンスが同時に求めても 1 件ずつ出す。
 */
export function useAppGrantRequests() {
  const queue = ref<AppGrantRequest[]>([]);
  const current = computed(() => queue.value[0] ?? null);
  const responding = ref(false);
  const unlistens: UnlistenFn[] = [];
  let disposed = false;

  onMounted(async () => {
    const offRequest = await listen<AppGrantRequest>("app-grant-request", (event) => {
      const req = event.payload;
      // 同じ requestId の再送は 1 件に畳む
      queue.value = [...queue.value.filter((r) => r.requestId !== req.requestId), req];
    });
    const offCancel = await listen<{ requestId: string }>("app-grant-cancelled", (event) => {
      queue.value = queue.value.filter((r) => r.requestId !== event.payload.requestId);
    });
    if (disposed) {
      offRequest();
      offCancel();
      return;
    }
    unlistens.push(offRequest, offCancel);

    // main の webview が再読込されると、イベントで届いた要求はキューごと消える（起動は 10 分
    // ハングし、再承認は再送されない）。購読を張ったあとで、Rust が持つ待ちの一覧を取り戻す
    try {
      const pending = await invoke<AppGrantRequest[]>("list_pending_app_grants");
      const known = new Set(queue.value.map((r) => r.requestId));
      const missing = pending.filter((r) => !known.has(r.requestId));
      if (missing.length > 0) queue.value = [...missing, ...queue.value];
    } catch (e) {
      console.warn("list_pending_app_grants failed", e);
    }
  });

  onBeforeUnmount(() => {
    disposed = true;
    for (const off of unlistens) off();
  });

  async function respond(approved: boolean) {
    const req = current.value;
    if (!req || responding.value) return;
    responding.value = true;
    try {
      // 先にキューから外す: 応答が失敗しても同じダイアログを出し直さない
      // （Rust 側は差し替え・停止済みなら無害に無視する）
      queue.value = queue.value.filter((r) => r.requestId !== req.requestId);
      await invoke("app_grant_respond", { requestId: req.requestId, approved });
    } finally {
      responding.value = false;
    }
  }

  return { current, pendingCount: computed(() => queue.value.length), responding, respond };
}
