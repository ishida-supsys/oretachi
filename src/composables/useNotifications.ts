import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
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
// worktreeId → 直近の notify-worktree が tray === false だったか。
// 自動承認 ON のワークツリーは approval/general が shouldHold で保留され、AI 非承認時に
// useAppAutoApproval が直接通知を出すため、そちらから参照して抑制するのに使う。
const traySuppression = new Map<string, boolean>();
let initialized = false;
let osNotificationEnabled: (() => boolean) | undefined;
let getSoundSettings: (() => NotificationSoundSettings | undefined) | undefined;
let storedNotificationTitles: Record<NotificationKind, string> = {
  general: "Notification",
  approval: "Notification",
  completed: "Notification",
};

/**
 * 直近の notify-worktree が tray === false だったワークツリーか。
 * 自動承認 ON のワークツリーは approval/general が shouldHold で保留され、通知は
 * useAppAutoApproval が AI 非承認判定後に直接発火する。その経路の抑制判定に使う。
 */
export function isTraySuppressed(worktreeId: string): boolean {
  return traySuppression.get(worktreeId) === true;
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
      const id = resolveWorktreeId(worktreeName);
      // 自動承認経路が後から参照するので、抑制するイベントについても先に記録しておく
      if (id) traySuppression.set(id, event.payload.tray === false);
      // trayNotification オフのワークツリー由来。自動承認は notify-worktree を別途購読しているのでここだけ止める
      if (event.payload.tray === false) return;
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
  }

  /** 特定ワークツリーの通知をクリアする */
  function clearNotification(worktreeId: string) {
    notifications.delete(worktreeId);
  }

  /** 存在しないワークツリーの stale な通知エントリを削除する */
  function purgeStaleNotifications(activeWorktreeIds: Set<string>) {
    for (const id of notifications.keys()) {
      if (!activeWorktreeIds.has(id)) {
        notifications.delete(id);
      }
    }
    for (const id of traySuppression.keys()) {
      if (!activeWorktreeIds.has(id)) {
        traySuppression.delete(id);
      }
    }
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
