import { describe, it, expect, vi, beforeEach } from "vitest";

/** `notify-worktree` リスナーの提示判定（#225）。
 *
 *  自動承認 ON の経路は `useAppAutoApproval.test.ts` が見ている。こちらは
 *  **自動承認 OFF のワークツリー**、つまり #225 の主シナリオ（teamwork-parent が
 *  トレイ通知をオフにしたまま AskUserQuestion で止まる）を固定する。
 *
 *  `useNotifications` はモジュールレベルに `initialized` フラグと `notifications` Map を
 *  持つシングルトンなので、テストごとに `vi.resetModules()` して読み直す。 */

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => unknown) =>
    listenMock(name, handler),
}));

const invokeMock = vi.fn(async () => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...(args as [])),
}));

const sendNotificationMock = vi.fn();
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
  sendNotification: (...args: unknown[]) => sendNotificationMock(...(args as [])),
  onAction: vi.fn(async () => {}),
}));

const playNotificationSoundMock = vi.fn(async () => {});
vi.mock("../utils/notificationSound", () => ({
  playNotificationSound: (...args: unknown[]) => playNotificationSoundMock(...(args as [])),
}));

const WT_ID = "wt-1";
const WT_NAME = "worktree-alpha";

interface Harness {
  notify: (payload: { kind: string; tray?: boolean }) => Promise<void>;
  notifications: Map<string, { count: number }>;
}

async function setup(): Promise<Harness> {
  vi.resetModules();
  listenMock.mockReset();
  sendNotificationMock.mockReset();
  playNotificationSoundMock.mockClear();

  const handlers = new Map<string, (event: { payload: unknown }) => unknown>();
  listenMock.mockImplementation(async (name: string, handler) => {
    handlers.set(name, handler);
    return () => {};
  });

  const { useNotifications } = await import("./useNotifications");
  const n = useNotifications();
  await n.initNotificationListener(
    (name) => (name === WT_NAME ? WT_ID : undefined),
    // 自動承認 OFF なので保留しない
    () => false,
    () => true,
    undefined,
    undefined,
    // 音を鳴らす設定にして、approval が音・OS 通知まで到達したことを見る
    () => ({ volume: 80, kinds: { approval: { enabled: true, sound: "system:a.wav" } } }) as never,
  );

  return {
    notify: async (payload) => {
      await handlers.get("notify-worktree")!({
        payload: { worktree_name: WT_NAME, ...payload },
      });
    },
    notifications: n.notifications as unknown as Map<string, { count: number }>,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useNotifications: トレイ通知オフ（tray: false）の提示判定", () => {
  /** #225 の主シナリオ。ここが落ちると「ダイアログで止まったまま誰も気付けない」に戻る。 */
  it("tray: false でも approval はバッジ・音・OS 通知まで通る", async () => {
    const h = await setup();
    await h.notify({ kind: "approval", tray: false });
    expect(h.notifications.get(WT_ID)?.count).toBe(1);
    expect(playNotificationSoundMock).toHaveBeenCalledTimes(1);
    expect(sendNotificationMock).toHaveBeenCalledTimes(1);
  });

  /** teamwork-parent がトレイ通知をオフにする狙い（自分のノイズを止める）を壊していないこと。 */
  it("tray: false の completed / hook / general は従来どおり抑制される", async () => {
    const h = await setup();
    await h.notify({ kind: "completed", tray: false });
    await h.notify({ kind: "hook", tray: false });
    await h.notify({ kind: "general", tray: false });
    expect(h.notifications.size).toBe(0);
    expect(sendNotificationMock).not.toHaveBeenCalled();
  });

  it("tray 未指定・tray: true の approval は従来どおり通る", async () => {
    const h = await setup();
    await h.notify({ kind: "approval" });
    await h.notify({ kind: "approval", tray: true });
    expect(h.notifications.get(WT_ID)?.count).toBe(2);
  });
});
