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
import type { NotificationSoundSettings } from "../types/settings";

export type NotificationKind = "approval" | "completed" | "general";

export interface NotifyWorktreeEvent {
  worktree_name: string;
  kind: NotificationKind | "hook";
  body?: string;
  agent?: string;
  /** false のとき通知系（トレイバッジ / ポップアップ / 通知音 / OS通知）を一括で抑制する */
  tray?: boolean;
}

interface NotificationEntry {
  count: number;
  firstNotifiedAt: number; // Date.now()
  kind: NotificationKind;
}

// worktreeId → 未確認の通知エントリ
const notifications = reactive(new Map<string, NotificationEntry>());
let initialized = false;
let osNotificationEnabled: (() => boolean) | undefined;
let getSoundSettings: (() => NotificationSoundSettings | undefined) | undefined;
let storedNotificationTitles: Record<NotificationKind, string> = {
  general: "Notification",
  approval: "Notification",
  completed: "Notification",
};

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
    const entries: Record<string, { count: number; kind: NotificationKind; firstNotifiedAt: number }> = {};
    for (const [id, entry] of notifications) {
      entries[id] = { count: entry.count, kind: entry.kind, firstNotifiedAt: entry.firstNotifiedAt };
    }
    invoke("sync_notification_state", { entries }).catch(() => {});
  }, 100);
}

/**
 * 通知音を再生する。OS通知とは独立して動作する。
 */
export function playSoundForKind(kind: NotificationKind) {
  const ss = getSoundSettings?.();
  if (!ss) return;
  const sound = ss[kind];
  if (sound) {
    playNotificationSound(sound, ss.volume ?? 80).catch(() => {});
  }
}

/**
 * OS通知を送信する。App.vue の自動承認不承認ハンドラからも呼ばれる。
 */
export async function sendOsNotification(worktreeName: string, title?: string, kind?: NotificationKind) {
  if (!osNotificationEnabled?.()) return;
  let permitted = await isPermissionGranted();
  if (!permitted) {
    const permission = await requestPermission();
    permitted = permission === "granted";
  }
  if (permitted) {
    const resolvedTitle = title ?? (kind ? storedNotificationTitles[kind] : storedNotificationTitles.general);
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
    shouldHold?: (worktreeId: string, kind: NotificationKind) => boolean,
    isOsNotificationEnabledFn?: () => boolean,
    focusWorktree?: (worktreeId: string) => void,
    notificationTitles?: Record<NotificationKind, string>,
    getSoundSettingsFn?: () => NotificationSoundSettings | undefined,
  ) {
    if (initialized) return;
    initialized = true;
    osNotificationEnabled = isOsNotificationEnabledFn;
    getSoundSettings = getSoundSettingsFn;
    if (notificationTitles) storedNotificationTitles = notificationTitles;

    await listen<NotifyWorktreeEvent>("notify-worktree", async (event) => {
      const { worktree_name: worktreeName, kind } = event.payload;
      // hook はモニタリング目的の MCP ブロードキャスト専用。UI 通知はスキップ
      if (kind === "hook") return;
      // trayNotification オフのワークツリー由来。自動承認は notify-worktree を別途購読しており、
      // そちらは `tray` をイベント単位で持ち回って判定する（#168）ので、ここだけ止める
      if (event.payload.tray === false) return;
      const id = resolveWorktreeId(worktreeName);
      if (id) {
        if (shouldHold?.(id, kind)) return;
        addNotification(id, kind);
        playSoundForKind(kind);
        await sendOsNotification(worktreeName, undefined, kind);
      }
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

  function addNotification(worktreeId: string, kind: NotificationKind = "general") {
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
