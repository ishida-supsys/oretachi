import { describe, it, expect } from "vitest";
import {
  createPendingNotifyStore,
  queuePendingNotify,
  shouldNotifyAfterJudge,
  takePendingNotify,
  trayOf,
} from "./autoApprovalNotify";

describe("trayOf", () => {
  it("tray 未指定は表示扱い", () => {
    expect(trayOf({})).toBe(true);
  });

  it("tray: false のときだけ非表示", () => {
    expect(trayOf({ tray: false })).toBe(false);
    expect(trayOf({ tray: true })).toBe(true);
  });
});

describe("shouldNotifyAfterJudge", () => {
  it("未承認・未フォーカス・tray:true なら提示する", () => {
    expect(shouldNotifyAfterJudge({ approved: false, focused: false, tray: true })).toBe(true);
  });

  it("AI が承認したなら提示しない", () => {
    expect(shouldNotifyAfterJudge({ approved: true, focused: false, tray: true })).toBe(false);
  });

  it("フォーカス中なら提示しない", () => {
    expect(shouldNotifyAfterJudge({ approved: false, focused: true, tray: true })).toBe(false);
  });

  it("tray:false（trayNotification オフのフック由来）は提示しない", () => {
    expect(shouldNotifyAfterJudge({ approved: false, focused: false, tray: false })).toBe(false);
  });
});

describe("pending notify store", () => {
  it("預かった通知は 1 度だけ取り出せる", () => {
    const store = createPendingNotifyStore();
    queuePendingNotify(store, "wt-1", true);
    expect(takePendingNotify(store, "wt-1")).toEqual({ tray: true });
    expect(takePendingNotify(store, "wt-1")).toBeUndefined();
  });

  it("預かりが無ければ undefined", () => {
    expect(takePendingNotify(createPendingNotifyStore(), "wt-1")).toBeUndefined();
  });

  it("後続の tray:false は先に預かった明示通知を抑制しない", () => {
    const store = createPendingNotifyStore();
    queuePendingNotify(store, "wt-1", true);
    queuePendingNotify(store, "wt-1", false);
    expect(takePendingNotify(store, "wt-1")).toEqual({ tray: true });
  });

  it("tray:false だけを預かった場合は tray:false のまま", () => {
    const store = createPendingNotifyStore();
    queuePendingNotify(store, "wt-1", false);
    queuePendingNotify(store, "wt-1", false);
    expect(takePendingNotify(store, "wt-1")).toEqual({ tray: false });
  });

  it("ワークツリーごとに独立している", () => {
    const store = createPendingNotifyStore();
    queuePendingNotify(store, "wt-1", true);
    queuePendingNotify(store, "wt-2", false);
    expect(takePendingNotify(store, "wt-1")).toEqual({ tray: true });
    expect(takePendingNotify(store, "wt-2")).toEqual({ tray: false });
  });
});
