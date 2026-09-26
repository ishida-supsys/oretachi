import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn(async () => undefined);
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...(args as [])),
}));

const checkMock = vi.fn();
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => checkMock(...(args as [])),
}));

vi.mock("../utils/log", () => ({ logError: vi.fn(), logInfo: vi.fn() }));

import { useUpdater, setBeforeInstallHook } from "./useUpdater";

// ダミーの update オブジェクト（downloadAndInstall は truthy であることしか見ない）
const DUMMY_UPDATE = { version: "1.2.3" } as unknown;

describe("useUpdater / setBeforeInstallHook", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    setBeforeInstallHook(null);
  });

  it("フック未登録でも invoke が呼ばれる", async () => {
    const { downloadAndInstall } = useUpdater();
    await downloadAndInstall(DUMMY_UPDATE as never);
    expect(invokeMock).toHaveBeenCalledWith("download_and_install_update");
  });

  it("登録したフックが invoke より先に呼ばれる", async () => {
    const order: string[] = [];
    invokeMock.mockImplementationOnce(async () => {
      order.push("invoke");
    });
    setBeforeInstallHook(async () => {
      order.push("hook");
    });

    const { downloadAndInstall } = useUpdater();
    await downloadAndInstall(DUMMY_UPDATE as never);

    expect(order).toEqual(["hook", "invoke"]);
  });

  it("フックが失敗しても invoke は呼ばれる（更新を止めない）", async () => {
    setBeforeInstallHook(async () => {
      throw new Error("save failed");
    });

    const { downloadAndInstall } = useUpdater();
    await expect(downloadAndInstall(DUMMY_UPDATE as never)).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("download_and_install_update");
  });

  it("update が null の場合はフックも invoke も呼ばれない", async () => {
    const hook = vi.fn(async () => {});
    setBeforeInstallHook(hook);

    const { downloadAndInstall } = useUpdater();
    await downloadAndInstall(null);

    expect(hook).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
