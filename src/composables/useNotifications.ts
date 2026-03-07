import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

interface NotificationEntry {
  count: number;
  firstNotifiedAt: number; // Date.now()
}

// worktreeId → 未確認の通知エントリ
const notifications = reactive(new Map<string, NotificationEntry>());
let initialized = false;

export function useNotifications() {
  /**
   * 通知リスナーを初期化する。App.vue の onMounted で一度だけ呼ぶ。
   * @param resolveWorktreeId ワークツリー名 → ID の解決関数
   */
  async function initNotificationListener(
    resolveWorktreeId: (name: string) => string | undefined,
    shouldHold?: (worktreeId: string) => boolean,
    isOsNotificationEnabled?: () => boolean
  ) {
    if (initialized) return;
    initialized = true;

    await listen<string>("notify-worktree", async (event) => {
      const worktreeName = event.payload;
      const id = resolveWorktreeId(worktreeName);
      if (id) {
        if (shouldHold?.(id)) return;
        addNotification(id);
        if (isOsNotificationEnabled?.()) {
          let permitted = await isPermissionGranted();
          if (!permitted) {
            const permission = await requestPermission();
            permitted = permission === "granted";
          }
          if (permitted) {
            sendNotification({ title: "Worktree通知", body: worktreeName });
          }
        }
      }
    });
  }

  function addNotification(worktreeId: string) {
    const existing = notifications.get(worktreeId);
    if (existing) {
      existing.count += 1;
    } else {
      notifications.set(worktreeId, { count: 1, firstNotifiedAt: Date.now() });
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
