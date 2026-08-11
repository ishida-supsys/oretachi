import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  OrphanedGroupView,
  SubscriptionView,
  TerminalUnread,
} from "../types/event";

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

export async function loadSubscriptions(): Promise<void> {
  if (isLoading.value) return;
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
}

export async function loadTerminalUnread(): Promise<void> {
  const rows = await safeInvoke<TerminalUnread[]>("event_terminal_unread", []);
  liveTerminals.value = rows;
  const next = new Map<number, number>();
  for (const row of rows) {
    if (row.unacked > 0) next.set(row.sessionId, row.unacked);
  }
  terminalUnread.value = next;
}

export async function unsubscribe(subscriptionId: string): Promise<void> {
  await invoke("event_unsubscribe", { subscriptionId });
  await loadSubscriptions();
}

/** 引き継ぎ待ちグループを、人間が選んだ生存タブへ手動で引き継ぐ。
 *  自動の引き継ぎは「新しい AI タブ1つにつき1グループ」なので、前回より少ないタブしか
 *  開かなかった場合にグループが取り残される。その解消手段。 */
export async function rebindGroup(
  worktreeId: string,
  deadTerminalId: string,
  sessionId: number,
): Promise<void> {
  await invoke("event_rebind_group", { worktreeId, deadTerminalId, sessionId });
  await Promise.all([loadSubscriptions(), loadTerminalUnread()]);
}

export async function ackMessages(terminalId: string, ids: string[]): Promise<void> {
  await invoke("event_ack", { terminalId, ids });
  await Promise.all([loadSubscriptions(), loadTerminalUnread()]);
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
