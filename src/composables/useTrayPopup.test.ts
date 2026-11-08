import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// フェイク WebviewWindow: once で登録されたハンドラを保持し、テストから fire で発火する
class FakeWindow {
  static instances: FakeWindow[] = [];
  static getByLabel = vi.fn(async (_label: string): Promise<FakeWindow | null> => null);
  handlers = new Map<string, (e: unknown) => void>();
  setFocus = vi.fn(async () => {});
  destroy = vi.fn(async () => {});
  constructor(public label: string) {
    FakeWindow.instances.push(this);
  }
  async once(event: string, cb: (e: unknown) => void) {
    this.handlers.set(event, cb);
    return () => {};
  }
  fire(event: string, payload: unknown = {}) {
    this.handlers.get(event)?.(payload);
  }
}

vi.mock("@tauri-apps/api/webviewWindow", () => ({ WebviewWindow: FakeWindow }));

type Mod = typeof import("./useTrayPopup");
let useTrayPopup: Mod["useTrayPopup"];

const wts = [] as never[];
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("useTrayPopup / 二重生成防止 (#432)", () => {
  beforeEach(async () => {
    FakeWindow.instances = [];
    FakeWindow.getByLabel.mockReset();
    FakeWindow.getByLabel.mockResolvedValue(null);
    vi.stubGlobal("window", { location: { origin: "http://localhost", pathname: "/" } });
    vi.resetModules();
    ({ useTrayPopup } = await import("./useTrayPopup"));
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("created 前の2回目呼び出しではコンストラクタが1回だけ、created 後に setFocus へ進む", async () => {
    const { openTrayPopup } = useTrayPopup();
    await openTrayPopup(wts);
    const second = openTrayPopup(wts);
    await flush();
    expect(FakeWindow.instances).toHaveLength(1);
    expect(FakeWindow.instances[0].setFocus).not.toHaveBeenCalled();

    FakeWindow.instances[0].fire("tauri://created");
    await second;
    expect(FakeWindow.instances).toHaveLength(1);
    expect(FakeWindow.instances[0].setFocus).toHaveBeenCalledTimes(1);
  });

  it("生成完了後の2回目呼び出しは setFocus のみ", async () => {
    const { openTrayPopup } = useTrayPopup();
    await openTrayPopup(wts);
    FakeWindow.instances[0].fire("tauri://created");
    await flush();
    await openTrayPopup(wts);
    expect(FakeWindow.instances).toHaveLength(1);
    expect(FakeWindow.instances[0].setFocus).toHaveBeenCalledTimes(1);
  });

  it("tauri://error の後は再度生成できる", async () => {
    const { openTrayPopup, isTrayPopupOpen } = useTrayPopup();
    await openTrayPopup(wts);
    FakeWindow.instances[0].fire("tauri://error", "boom");
    await flush();
    expect(isTrayPopupOpen()).toBe(false);
    await openTrayPopup(wts);
    expect(FakeWindow.instances).toHaveLength(2);
  });

  it("setFocus が失敗しても getByLabel で実在すれば作り直さない", async () => {
    const { openTrayPopup } = useTrayPopup();
    await openTrayPopup(wts);
    const win = FakeWindow.instances[0];
    win.fire("tauri://created");
    await flush();
    win.setFocus.mockRejectedValueOnce(new Error("fail"));
    FakeWindow.getByLabel.mockResolvedValueOnce(win);
    await openTrayPopup(wts);
    expect(FakeWindow.instances).toHaveLength(1);
  });

  it("setFocus が失敗し実在もしないときだけ作り直す", async () => {
    const { openTrayPopup } = useTrayPopup();
    await openTrayPopup(wts);
    const win = FakeWindow.instances[0];
    win.fire("tauri://created");
    await flush();
    win.setFocus.mockRejectedValueOnce(new Error("fail"));
    await openTrayPopup(wts);
    expect(FakeWindow.instances).toHaveLength(2);
  });

  it("created が来なくてもタイムアウトで待機が解け、既存の1枚にフォーカスする", async () => {
    vi.useFakeTimers();
    const { openTrayPopup } = useTrayPopup();
    await openTrayPopup(wts);
    const second = openTrayPopup(wts);
    await vi.advanceTimersByTimeAsync(5000);
    await second;
    expect(FakeWindow.instances).toHaveLength(1);
    expect(FakeWindow.instances[0].setFocus).toHaveBeenCalledTimes(1);
  });

  it("タイムアウト後も created 前なら、setFocus 失敗・実在未確認でも作り直さない", async () => {
    vi.useFakeTimers();
    const { openTrayPopup } = useTrayPopup();
    await openTrayPopup(wts);
    const win = FakeWindow.instances[0];
    win.setFocus.mockRejectedValue(new Error("not registered yet"));
    const second = openTrayPopup(wts);
    await vi.advanceTimersByTimeAsync(5000);
    await second;
    expect(FakeWindow.instances).toHaveLength(1);

    // 遅れて created が届いたら通常どおりフォーカスへ進む
    win.fire("tauri://created");
    win.setFocus.mockResolvedValue(undefined);
    await openTrayPopup(wts);
    expect(FakeWindow.instances).toHaveLength(1);
  });

  it("closeTrayPopup 後は再度生成できる（生成中に閉じた場合も待機が解ける）", async () => {
    const { openTrayPopup, closeTrayPopup } = useTrayPopup();
    await openTrayPopup(wts);
    await closeTrayPopup();
    expect(FakeWindow.instances[0].destroy).toHaveBeenCalledTimes(1);
    await openTrayPopup(wts);
    expect(FakeWindow.instances).toHaveLength(2);
  });
});
