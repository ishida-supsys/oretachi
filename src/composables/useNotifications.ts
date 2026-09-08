import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
  onAction,
} from "@tauri-apps/plugin-notification";
import { playNotificationSound } from "../utils/notificationSound";
import {
  isNotifyKind,
  passesTrayOff,
  resolveKindSetting,
  shouldPlaySound,
  shouldSendOsNotification,
  showsBadge,
} from "../utils/notificationKinds";
import type { NotifyKind, NotificationSoundSettings } from "../types/settings";

export interface NotifyWorktreeEvent {
  worktree_name: string;
  kind: NotifyKind;
  body?: string;
  agent?: string;
  /** false のとき通知系（トレイバッジ / ポップアップ / 通知音 / OS通知）を抑制する。
   *  例外は `approval` で、`false` でも提示する（#225。`passesTrayOff` を参照）。 */
  tray?: boolean;
}

/** `worktree.*` の発火を発火元ワークツリーへ伝えるイベント（#140）。
 *
 *  受信側にはトーストもバッジも出さない（#137）が、発火元では音 / OS 通知を鳴らせる
 *  ようにしたいので `notify-worktree` とは別の経路にしている。相乗りさせると
 *  (a) 全 MCP ピアへ broadcast され、(b) 自動承認リスナーが AI 判定を走らせてしまう。 */
interface WorktreeEventFiredPayload {
  worktreeName: string;
  kind: string;
}

interface NotificationEntry {
  count: number;
  firstNotifiedAt: number; // Date.now()
  kind: NotifyKind;
}

// worktreeId → 未確認の通知エントリ
const notifications = reactive(new Map<string, NotificationEntry>());
let initialized = false;
let osNotificationEnabled: (() => boolean) | undefined;
let getSoundSettings: (() => NotificationSoundSettings | undefined) | undefined;
let storedNotificationTitles: Partial<Record<NotifyKind, string>> = {};

/**
 * 未確認通知の現在値を Rust 側（NotificationRegistry）へ写す。
 *
 * バッジの実体はこのモジュールのメモリにしかなく Rust からは覗けないため、同期して
 * おかないと MCP の `oretachi_get_worktree_status` が notificationCount を返せず、
 * `oretachi_clear_worktree_notification` も「何件消したか」を答えられない。
 * 連続通知でIPCが詰まらないよう次のマイクロバッチまで畳んでから全置換で送る。
 */
let syncTimer: ReturnType<typeof setTimeout> | undefined;
function syncNotificationsToBackend() {
  if (syncTimer !== undefined) return;
  syncTimer = setTimeout(() => {
    syncTimer = undefined;
    const entries: Record<string, { count: number; kind: NotifyKind; firstNotifiedAt: number }> = {};
    for (const [id, entry] of notifications) {
      entries[id] = { count: entry.count, kind: entry.kind, firstNotifiedAt: entry.firstNotifiedAt };
    }
    invoke("sync_notification_state", { entries }).catch(() => {});
  }, 100);
}

/**
 * 通知音を再生する。OS通知とは独立して動作する。
 *
 * 種別ごとの ON/OFF（#140）はここで効く。`enabled` が false なら音の設定に
 * 関わらず鳴らさない。
 */
export function playSoundForKind(kind: NotifyKind) {
  const ss = getSoundSettings?.();
  if (!ss) return;
  const sound = shouldPlaySound(ss, kind);
  if (sound) {
    playNotificationSound(sound, ss.volume ?? 80).catch(() => {});
  }
}

/**
 * OS通知を送信する。App.vue の自動承認不承認ハンドラからも呼ばれる。
 */
export async function sendOsNotification(worktreeName: string, title?: string, kind?: NotifyKind) {
  if (!osNotificationEnabled?.()) return;
  // 種別ごとの ON/OFF（#140）。`title` 直指定の経路（自動承認の否決など）は
  // 呼び出し元が出すと決めているので、kind が無ければ従来どおり素通しする。
  if (kind && !shouldSendOsNotification(getSoundSettings?.(), kind)) return;
  let permitted = await isPermissionGranted();
  if (!permitted) {
    const permission = await requestPermission();
    permitted = permission === "granted";
  }
  if (permitted) {
    const resolvedTitle =
      title ??
      (kind ? storedNotificationTitles[kind] : undefined) ??
      storedNotificationTitles.general ??
      "Notification";
    sendNotification({ title: resolvedTitle, body: worktreeName, extra: { worktreeName } });
  }
}

export function useNotifications() {
  /**
   * 通知リスナーを初期化する。App.vue の onMounted で一度だけ呼ぶ。
   * @param resolveWorktreeId ワークツリー名 → ID の解決関数
   */
  async function initNotificationListener(
    resolveWorktreeId: (name: string) => string | undefined,
    shouldHold?: (worktreeId: string, kind: NotifyKind) => boolean,
    isOsNotificationEnabledFn?: () => boolean,
    focusWorktree?: (worktreeId: string) => void,
    notificationTitles?: Partial<Record<NotifyKind, string>>,
    getSoundSettingsFn?: () => NotificationSoundSettings | undefined,
  ) {
    if (initialized) return;
    initialized = true;
    // 写しの初期化。同期は「変化したとき」の一方向 push なので、これが無いと
    // プロセスは生きたまま webview だけリロードされたとき（WebView2 のレンダラ復帰、
    // dev の full reload）に JS 側は空なのに Rust 側の写しが古い件数を持ち続ける。
    // 空でも一度送って必ず突き合わせる。
    syncNotificationsToBackend();
    osNotificationEnabled = isOsNotificationEnabledFn;
    getSoundSettings = getSoundSettingsFn;
    if (notificationTitles) storedNotificationTitles = notificationTitles;

    await listen<NotifyWorktreeEvent>("notify-worktree", async (event) => {
      const { worktree_name: worktreeName, kind } = event.payload;
      // Rust 側で固定7値に検証済みだが、未知の値を設定キーとして引かせないよう念のため弾く。
      if (!isNotifyKind(kind)) return;
      // 種別ごとの ON/OFF（#140）。`hook` は既定 OFF だが、明示的に ON にすれば
      // 他の種別と同様に通知される（統合前は無条件でスキップしていた）。
      if (!resolveKindSetting(getSoundSettings?.(), kind).enabled) return;
      // trayNotification オフのワークツリー由来。自動承認は notify-worktree を別途購読しており、
      // そちらは `tray` をイベント単位で持ち回って判定する（#168）ので、ここだけ止める。
      // ただし `approval`（ツール許可 / プラン承認 / AskUserQuestion）は
      // 「人の入力を待って止まった」ことを伝える唯一のフック経路なので通す（#225）。
      if (event.payload.tray === false && !passesTrayOff(kind)) return;
      const id = resolveWorktreeId(worktreeName);
      if (id) {
        if (shouldHold?.(id, kind)) return;
        if (showsBadge(kind)) addNotification(id, kind);
        playSoundForKind(kind);
        await sendOsNotification(worktreeName, undefined, kind);
      }
    });

    // `worktree.*` の発火を発火元で鳴らす（#140）。受信側の画面は動かさないので
    // バッジは積まず、音と OS 通知だけを出す。`worktree.closed` は発火時点で
    // ワークツリーが消えており ID を引けないため、名前だけで完結させる。
    await listen<WorktreeEventFiredPayload>("worktree-event-fired", async (event) => {
      const { worktreeName, kind } = event.payload;
      if (!isNotifyKind(kind)) return;
      playSoundForKind(kind);
      await sendOsNotification(worktreeName, undefined, kind);
    });

    try {
      await onAction((notification) => {
        const name = notification.extra?.worktreeName as string | undefined;
        if (name && focusWorktree) {
          const id = resolveWorktreeId(name);
          if (id) focusWorktree(id);
        }
      });
    } catch {
      // notification:allow-register-listener が未許可の場合は無視
    }
  }

  function addNotification(worktreeId: string, kind: NotifyKind = "general") {
    const existing = notifications.get(worktreeId);
    if (existing) {
      existing.count += 1;
      existing.kind = kind;
    } else {
      notifications.set(worktreeId, { count: 1, firstNotifiedAt: Date.now(), kind });
    }
    syncNotificationsToBackend();
  }

  /** 特定ワークツリーの通知をクリアする */
  function clearNotification(worktreeId: string) {
    if (notifications.delete(worktreeId)) syncNotificationsToBackend();
  }

  /** 存在しないワークツリーの stale な通知エントリを削除する */
  function purgeStaleNotifications(activeWorktreeIds: Set<string>) {
    let purged = false;
    for (const id of notifications.keys()) {
      if (!activeWorktreeIds.has(id)) {
        notifications.delete(id);
        purged = true;
      }
    }
    if (purged) syncNotificationsToBackend();
  }

  /** firstNotifiedAt の昇順（古い順）でソートした worktreeId 配列を返す */
  function getNotifiedWorktreeIds(): string[] {
    return Array.from(notifications.entries())
      .sort((a, b) => a[1].firstNotifiedAt - b[1].firstNotifiedAt)
      .map(([id]) => id);
  }

  /** 全 count の合計を返す */
  function getTotalNotificationCount(): number {
    let total = 0;
    for (const entry of notifications.values()) {
      total += entry.count;
    }
    return total;
  }

  return {
    notifications,
    initNotificationListener,
    addNotification,
    clearNotification,
    purgeStaleNotifications,
    getNotifiedWorktreeIds,
    getTotalNotificationCount,
  };
}
