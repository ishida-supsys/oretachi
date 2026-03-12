import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
  onAction,
} from "@tauri-apps/plugin-notification";

export type NotificationKind = "approval" | "completed" | "general";

export interface NotifyWorktreeEvent {
  worktree_name: string;
  kind: NotificationKind;
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
let storedNotificationTitles: Record<NotificationKind, string> = {
  general: "Notification",
  approval: "Notification",
  completed: "Notification",
};

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
    shouldHold?: (worktreeId: string) => boolean,
    isOsNotificationEnabledFn?: () => boolean,
    focusWorktree?: (worktreeId: string) => void,
    notificationTitles?: Record<NotificationKind, string>
  ) {
    if (initialized) return;
    initialized = true;
    osNotificationEnabled = isOsNotificationEnabledFn;
    if (notificationTitles) storedNotificationTitles = notificationTitles;

    await listen<NotifyWorktreeEvent>("notify-worktree", async (event) => {
      const { worktree_name: worktreeName, kind } = event.payload;
      const id = resolveWorktreeId(worktreeName);
      if (id) {
        if (shouldHold?.(id)) return;
        addNotification(id, kind);
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
  }

  /** 特定ワークツリーの通知をクリアする */
  function clearNotification(worktreeId: string) {
    notifications.delete(worktreeId);
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
    getNotifiedWorktreeIds,
    getTotalNotificationCount,
  };
}
