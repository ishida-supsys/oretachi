import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  OrphanedGroupView,
  SubscriptionView,
  TerminalUnread,
} from "../types/event";
import { buildSubscriptionCounts } from "../utils/subscriptionCounts";

/** ワークツリー間イベントの購読状態（issue #120 §7 / #125）。
 *
 *  購読・配送を画面に出さないと「勝手にエージェントが動き出した」ように見える。
 *  特に #125 で再バインド（タブが死んでもアプリを再起動しても購読が生き残り、新しいタブへ
 *  引き継がれる）が入ったので、どのタブが何を購読していて何件未読なのかが見えないと
 *  追跡不能になる。
 *
 *  モジュールシングルトン（`useArchivePersistence.ts` / `useHomePanel.ts` と同じ流儀）で、
 *  ホームの購読パネルとタブバーの未読バッジが同じ状態を共有する。 */

export const subscriptions = ref<SubscriptionView[]>([]);
export const orphanedGroups = ref<OrphanedGroupView[]>([]);
/** pty session_id → 未 ack 件数。タブバーのバッジが引く */
export const terminalUnread = ref<Map<number, number>>(new Map());
/** 生存中の全端末（引き継ぎ先候補の解決に使う） */
export const liveTerminals = ref<TerminalUnread[]>([]);
export const isLoading = ref(false);

/** 手動引き継ぎの候補: 生存している AI エージェント端末。 */
export const agentTerminals = computed(() =>
  liveTerminals.value
    .filter((tm) => tm.isAiAgent)
    .map((tm) => ({
      sessionId: tm.sessionId,
      worktreeId: tm.worktreeId,
      label: `#${tm.sessionId} ${tm.agentName ?? ""}`.trim(),
    })),
);

/** worktreeId → 購読件数（ワークツリーカードの購読バッジ用、#137）。
 *
 *  `subscriptions` から畳むだけなので、`loadSubscriptions` の直列化ガードと
 *  `event-inbox-changed` の listen をそのまま共有する（新しい invoke は増やさない）。 */
export const subscriptionCounts = computed(() => buildSubscriptionCounts(subscriptions.value));

/** 未 ack の総数（トレイ / ホームの見出し用）。 */
export const totalUnacked = computed(() =>
  subscriptions.value.reduce((sum, s) => sum + s.unacked, 0),
);

/** イベント DB が未初期化のときは全コマンドが Err を返す。購読機能だけが無効な状態なので
 *  UI 全体を壊さず空表示にする。 */
async function safeInvoke<T>(cmd: string, fallback: T, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    console.warn(`[event] ${cmd} failed:`, e);
    return fallback;
  }
}

/** 読み込み中に来た更新要求。取りこぼすと未読バッジが古いまま固定される
 *  （`event-inbox-changed` は変化があったときしか飛ばないので自然回復しない）。
 *  `useArchivePersistence.ts` / `useTaskPersistence.ts` と同じ流儀で1回だけ再実行する。 */
let pendingReload = false;

export async function loadSubscriptions(): Promise<void> {
  if (isLoading.value) {
    pendingReload = true;
    return;
  }
  isLoading.value = true;
  try {
    subscriptions.value = await safeInvoke<SubscriptionView[]>("event_list_subscriptions", []);
    orphanedGroups.value = await safeInvoke<OrphanedGroupView[]>(
      "event_list_orphaned_groups",
      [],
    );
    // 引き継ぎ先の候補は端末側の情報から引くので一緒に更新する
    await loadTerminalUnread();
  } finally {
    isLoading.value = false;
  }

  if (pendingReload) {
    pendingReload = false;
    await loadSubscriptions();
  }
}

/** `loadSubscriptions` と同じ理由の直列化。`event-inbox-changed` は Reconcile /
 *  EventQueued / 引き継ぎ / ack など複数箇所から連続で飛ぶので、素朴に叩くと
 *  `event_terminal_unread` の invoke が並走し、**先発（古い）が後着したときに
 *  新しいスナップショットを上書き**する。このイベントは変化があったときしか
 *  飛ばないので、一度ズレると次の変化まで誤った件数で固定される。 */
let unreadLoading = false;
let pendingUnreadReload = false;

export async function loadTerminalUnread(): Promise<void> {
  if (unreadLoading) {
    pendingUnreadReload = true;
    return;
  }
  unreadLoading = true;
  try {
    const rows = await safeInvoke<TerminalUnread[]>("event_terminal_unread", []);
    liveTerminals.value = rows;
    const next = new Map<number, number>();
    for (const row of rows) {
      if (row.unacked > 0) next.set(row.sessionId, row.unacked);
    }
    terminalUnread.value = next;
  } finally {
    unreadLoading = false;
  }

  if (pendingUnreadReload) {
    pendingUnreadReload = false;
    await loadTerminalUnread();
  }
}

/** 直近の操作エラー。UI が拾ってトーストなどで見せる（握り潰すと黙って失敗する）。 */
export const lastActionError = ref<string | null>(null);

async function runAction(cmd: string, args: Record<string, unknown>): Promise<boolean> {
  try {
    await invoke(cmd, args);
    lastActionError.value = null;
    return true;
  } catch (e) {
    // `event_rebind_group` は候補一覧が古い（タブが消えた / 別ワークツリーへ移った）と
    // 正当に失敗する。黙って捨てると「押したのに何も起きない」になる。
    console.warn(`[event] ${cmd} failed:`, e);
    lastActionError.value = String(e);
    return false;
  } finally {
    await loadSubscriptions();
  }
}

export async function unsubscribe(subscriptionId: string): Promise<boolean> {
  return runAction("event_unsubscribe", { subscriptionId });
}

/** 引き継ぎ待ちグループを、人間が選んだ生存タブへ手動で引き継ぐ。
 *  自動の引き継ぎは「新しい AI タブ1つにつき1グループ」なので、前回より少ないタブしか
 *  開かなかった場合にグループが取り残される。その解消手段。 */
export async function rebindGroup(
  worktreeId: string,
  deadTerminalId: string,
  sessionId: number,
): Promise<boolean> {
  return runAction("event_rebind_group", { worktreeId, deadTerminalId, sessionId });
}

/** そのタブの未 ack を全件既読にする。エージェントが ack しないまま忘れた分を人間が畳む。 */
export async function ackAll(terminalId: string): Promise<boolean> {
  return runAction("event_ack_all", { terminalId });
}

/** 自動 spawn 要求への応答。**成否にかかわらず必ず呼ぶ**。呼ばないと Rust 側の
 *  単一フライトが 60 秒間解放されない（逆に、呼ばなくても撃ち直しループにはならない）。 */
export async function reportSpawnResult(
  requestId: string,
  sessionId: number | null,
): Promise<void> {
  try {
    await invoke("event_spawn_result", { requestId, sessionId });
  } catch (e) {
    console.warn("[event] event_spawn_result failed:", e);
  }
}

let initialized = false;

/** inbox / 購読の変化を購読して一覧と未読件数を同期する。App.vue から一度だけ呼ぶ。
 *  メインウィンドウはアプリと寿命を共にするので unlisten は保持しない
 *  （App.vue の他のグローバルリスナーと同じ扱い）。 */
export async function initEventSubscriptions(): Promise<void> {
  if (initialized) return;
  initialized = true;
  await listen("event-inbox-changed", () => {
    void loadSubscriptions();
  });
  await loadSubscriptions();
}

/** 現在の購読者数。リスナーは1本だけ張り、最後の解除で本当に外す。 */
let unreadRefCount = 0;
let unreadUnlisten: UnlistenFn | null = null;
let unreadPending: Promise<void> | null = null;

/** 未読件数だけを同期する軽量版（#130）。購読パネルを持たないサブウィンドウ /
 *  トレイポップアップ用。
 *
 *  `initEventSubscriptions` との違いは2つ:
 *  - 購読一覧 / 引き継ぎ待ち一覧は読まない（表示しないので無駄な invoke になる）
 *  - **unlisten を返す**。サブ / トレイは閉じるウィンドウなので、
 *    `useEventListeners` の `collect()` や `onUnmounted` で必ず解除する。
 *
 *  `initEventSubscriptions` とはフラグを分ける（共有すると片方の抑止でもう片方が
 *  黙って無効化され、「バッジが出ない」という発見しづらい状態になる）。
 *  多重呼び出しは参照カウントで扱う。単純な bool ガードだと2人目が no-op の
 *  unlisten を受け取り、1人目が解除した時点でリスナー不在のまま誰も気づけない。 */
export async function initTerminalUnread(): Promise<UnlistenFn> {
  unreadRefCount += 1;
  if (unreadRefCount === 1) {
    unreadPending = (async () => {
      unreadUnlisten = await listen("event-inbox-changed", () => {
        void loadTerminalUnread();
      });
      await loadTerminalUnread();
    })();
  }
  await unreadPending;

  let released = false;
  return () => {
    // 同じ購読者が2回解除しても他人のぶんを巻き込まないようにする
    if (released) return;
    released = true;
    unreadRefCount -= 1;
    if (unreadRefCount > 0) return;
    unreadUnlisten?.();
    unreadUnlisten = null;
    unreadPending = null;
  };
}
