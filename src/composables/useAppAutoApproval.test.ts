import { describe, it, expect, vi, beforeEach } from "vitest";
import { ref } from "vue";

// listen した Tauri イベントのハンドラを捕まえて、テストから直接叩けるようにする
const handlers = new Map<string, (event: { payload: unknown }) => unknown>();
const emitToMock = vi.fn(async () => {});
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => unknown) => {
    handlers.set(name, handler);
    return () => {};
  }),
  emitTo: (...args: unknown[]) => emitToMock(...(args as [])),
}));
vi.mock("../utils/log", () => ({ logDebug: vi.fn() }));

const runApprovalLoopMock = vi.fn();
vi.mock("../utils/autoApproval", () => ({
  runApprovalLoop: (...args: unknown[]) => runApprovalLoopMock(...args),
  cancelApproval: vi.fn(async () => {}),
}));

import { useAppAutoApproval } from "./useAppAutoApproval";
import type { Worktree } from "../types/worktree";
import type { AppSettings } from "../types/settings";

const WT_ID = "wt-1";
const WT_NAME = "worktree-alpha";

function makeWorktree(): Worktree {
  return {
    id: WT_ID,
    name: WT_NAME,
    path: "X:/wt/alpha",
    branchName: "worktree/alpha",
    terminals: [],
  } as unknown as Worktree;
}

interface Harness {
  notify: (payload: { kind?: string; tray?: boolean; worktree_name?: string }) => Promise<void>;
  subResult: (payload: {
    approved: boolean;
    tray?: boolean;
    command?: string;
    pendingTray?: boolean;
  }) => Promise<void>;
  addNotification: ReturnType<typeof vi.fn>;
  playSoundForKind: ReturnType<typeof vi.fn>;
  sendOsNotification: ReturnType<typeof vi.fn>;
  detached: { value: boolean };
  focused: { value: boolean };
}

async function setup(options?: { autoApproval?: boolean }): Promise<Harness> {
  handlers.clear();
  emitToMock.mockClear();
  runApprovalLoopMock.mockReset();
  runApprovalLoopMock.mockResolvedValue({ approved: false, lastCommand: undefined });

  const detached = { value: false };
  const focused = { value: false };
  const addNotification = vi.fn();
  const playSoundForKind = vi.fn();
  const sendOsNotification = vi.fn(async () => {});

  const settings = ref({
    worktrees: [{ id: WT_ID, autoApproval: options?.autoApproval ?? true }],
  } as unknown as AppSettings);

  const auto = useAppAutoApproval({
    worktrees: ref([makeWorktree()]),
    settings,
    scheduleSave: vi.fn(),
    isDetached: () => detached.value,
    getTerminalRef: () => undefined,
    autoApprovalPromptMap: new Map(),
    lastJudgedCommandMap: new Map(),
    addNotification,
    isWorktreeFocused: () => focused.value,
    onClickAutoApproval: vi.fn(),
    playSoundForKind,
    sendOsNotification,
    t: (key: string) => key,
  });
  await auto.init();

  return {
    notify: async (payload) => {
      await handlers.get("notify-worktree")!({
        payload: { worktree_name: WT_NAME, kind: "general", ...payload },
      });
    },
    subResult: async (payload) => {
      await handlers.get("sub-auto-approve-result")!({ payload: { worktreeId: WT_ID, ...payload } });
    },
    addNotification,
    playSoundForKind,
    sendOsNotification,
    detached,
    focused,
  };
}

/** runApprovalLoop を外部から解決できるようにして、AI 判定中の状態を作る */
function deferApprovalLoop() {
  let resolve!: (v: { approved: boolean; lastCommand: string | undefined }) => void;
  const promise = new Promise<{ approved: boolean; lastCommand: string | undefined }>((r) => {
    resolve = r;
  });
  runApprovalLoopMock.mockReturnValueOnce(promise);
  return { resolve: (approved: boolean) => resolve({ approved, lastCommand: undefined }) };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useAppAutoApproval: 明示通知の提示", () => {
  it("自動承認 ON・未フォーカスなら AI 非承認時にトレイ通知を出す", async () => {
    const h = await setup();
    await h.notify({ tray: true });
    expect(h.addNotification).toHaveBeenCalledWith(WT_ID, "approval");
    expect(h.playSoundForKind).toHaveBeenCalledWith("approval");
    expect(h.sendOsNotification).toHaveBeenCalledTimes(1);
  });

  it("フォーカス中なら出さない", async () => {
    const h = await setup();
    h.focused.value = true;
    await h.notify({ tray: true });
    expect(h.addNotification).not.toHaveBeenCalled();
  });

  it("AI が承認したなら出さない", async () => {
    const h = await setup();
    runApprovalLoopMock.mockResolvedValue({ approved: true, lastCommand: "ls" });
    await h.notify({ tray: true });
    expect(h.addNotification).not.toHaveBeenCalled();
  });

  it("tray: false のフック由来通知は従来どおり抑制される（#154 / #155 回帰なし）", async () => {
    const h = await setup();
    await h.notify({ tray: false });
    expect(runApprovalLoopMock).toHaveBeenCalledTimes(1); // 自動承認自体は走る
    expect(h.addNotification).not.toHaveBeenCalled();
  });

  it("自動承認 OFF のワークツリーでは何もしない（通知は useNotifications 側の担当）", async () => {
    const h = await setup({ autoApproval: false });
    await h.notify({ tray: true });
    expect(runApprovalLoopMock).not.toHaveBeenCalled();
    expect(h.addNotification).not.toHaveBeenCalled();
  });

  it("completed / hook は無視する", async () => {
    const h = await setup();
    await h.notify({ kind: "completed", tray: true });
    await h.notify({ kind: "hook", tray: true });
    expect(runApprovalLoopMock).not.toHaveBeenCalled();
  });
});

describe("useAppAutoApproval: AI 判定中に届いた通知（#168）", () => {
  it("判定中の明示通知は失われず、判定完了後に提示される", async () => {
    const h = await setup();
    const first = deferApprovalLoop();
    const judging = h.notify({ tray: true });

    await h.notify({ tray: true }); // 判定中に届いた明示通知
    expect(h.addNotification).not.toHaveBeenCalled();

    first.resolve(true); // 1件目は AI が承認 → 1件目自体の通知は出ない
    await judging;

    expect(h.addNotification).toHaveBeenCalledTimes(1);
    expect(h.addNotification).toHaveBeenCalledWith(WT_ID, "approval");
  });

  it("判定中に届いた後続の tray:false は、預かった明示通知を抑制しない", async () => {
    const h = await setup();
    const first = deferApprovalLoop();
    const judging = h.notify({ tray: true });

    await h.notify({ tray: true });
    await h.notify({ tray: false });

    first.resolve(true);
    await judging;

    expect(h.addNotification).toHaveBeenCalledTimes(1);
  });

  it("判定中に届いたのが tray:false だけなら提示しない", async () => {
    const h = await setup();
    const first = deferApprovalLoop();
    const judging = h.notify({ tray: true });

    await h.notify({ tray: false });

    first.resolve(true);
    await judging;

    expect(h.addNotification).not.toHaveBeenCalled();
  });

  it("判定中は自動承認を二重起動しない", async () => {
    const h = await setup();
    const first = deferApprovalLoop();
    const judging = h.notify({ tray: true });

    await h.notify({ tray: true });
    expect(runApprovalLoopMock).toHaveBeenCalledTimes(1);

    first.resolve(false);
    await judging;
    // 預かり分は「判定を回さず提示」なので、判定は 1 回のまま
    expect(runApprovalLoopMock).toHaveBeenCalledTimes(1);
    // 判定結果ぶん + 預かりぶんで 2 件
    expect(h.addNotification).toHaveBeenCalledTimes(2);
  });

  it("判定結果ぶんと預かりぶんが同時でも通知音と OS 通知は 1 回に畳む", async () => {
    const h = await setup();
    const first = deferApprovalLoop();
    const judging = h.notify({ tray: true });

    await h.notify({ tray: true });

    first.resolve(false);
    await judging;

    expect(h.addNotification).toHaveBeenCalledTimes(2); // バッジ count はイベント件数ぶん
    expect(h.playSoundForKind).toHaveBeenCalledTimes(1);
    expect(h.sendOsNotification).toHaveBeenCalledTimes(1);
  });

  it("提示不要なら通知音も OS 通知も鳴らさない", async () => {
    const h = await setup();
    h.focused.value = true;
    await h.notify({ tray: true });
    expect(h.playSoundForKind).not.toHaveBeenCalled();
    expect(h.sendOsNotification).not.toHaveBeenCalled();
  });

  it("預かり分は通知の前に取り出すので、通知処理が落ちても次回に持ち越さない", async () => {
    const h = await setup();
    h.sendOsNotification.mockRejectedValueOnce(new Error("permission denied"));
    const first = deferApprovalLoop();
    const judging = h.notify({ tray: true });
    await h.notify({ tray: true });
    first.resolve(false);
    await expect(judging).rejects.toThrow("permission denied");

    // 次の判定で預かり分が蒸し返されない
    h.addNotification.mockClear();
    await h.notify({ tray: true });
    expect(h.addNotification).toHaveBeenCalledTimes(1);
  });
});

describe("useAppAutoApproval: サブウィンドウ経由", () => {
  it("sub-try-auto-approve に tray を載せて渡す", async () => {
    const h = await setup();
    h.detached.value = true;
    await h.notify({ tray: true });
    expect(emitToMock).toHaveBeenCalledWith(
      `sub-${WT_ID}`,
      "sub-try-auto-approve",
      expect.objectContaining({ tray: true }),
    );
    await h.notify({ tray: false });
    expect(emitToMock).toHaveBeenLastCalledWith(
      `sub-${WT_ID}`,
      "sub-try-auto-approve",
      expect.objectContaining({ tray: false }),
    );
  });

  it("sub-auto-approve-result はイベント単位の tray で判定する", async () => {
    const h = await setup();
    await h.subResult({ approved: false, tray: true });
    expect(h.addNotification).toHaveBeenCalledTimes(1);

    await h.subResult({ approved: false, tray: false });
    expect(h.addNotification).toHaveBeenCalledTimes(1);

    // 直前に tray:false が来ていても、次の明示通知は抑制されない（ラッチ廃止・#168）
    await h.subResult({ approved: false, tray: true });
    expect(h.addNotification).toHaveBeenCalledTimes(2);
  });

  it("sub-auto-approve-result の tray 未指定は表示扱い（後方互換）", async () => {
    const h = await setup();
    await h.subResult({ approved: false });
    expect(h.addNotification).toHaveBeenCalledTimes(1);
  });

  it("サブウィンドウが預かった分は pendingTray で運ばれ、判定結果とは別に提示される", async () => {
    const h = await setup();
    // AI が承認した = 判定結果ぶんの通知は無いが、預かり分は提示する
    await h.subResult({ approved: true, tray: true, pendingTray: true });
    expect(h.addNotification).toHaveBeenCalledTimes(1);
    expect(h.playSoundForKind).toHaveBeenCalledTimes(1);
  });

  it("pendingTray が tray:false なら提示しない", async () => {
    const h = await setup();
    await h.subResult({ approved: true, tray: true, pendingTray: false });
    expect(h.addNotification).not.toHaveBeenCalled();
  });

  it("判定結果ぶんと pendingTray ぶんが同時でも通知音は 1 回", async () => {
    const h = await setup();
    await h.subResult({ approved: false, tray: true, pendingTray: true });
    expect(h.addNotification).toHaveBeenCalledTimes(2);
    expect(h.playSoundForKind).toHaveBeenCalledTimes(1);
    expect(h.sendOsNotification).toHaveBeenCalledTimes(1);
  });
});
