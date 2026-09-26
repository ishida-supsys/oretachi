import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn(async (cmd: string): Promise<boolean | undefined> => {
  if (cmd === "download_update") return true;
  return undefined;
});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...(args as [string])),
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
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "download_update") return true;
      return undefined;
    });
    setBeforeInstallHook(null);
  });

  it("フック未登録でも download_update → install_downloaded_update の順で呼ばれる", async () => {
    const { downloadAndInstall } = useUpdater();
    await downloadAndInstall(DUMMY_UPDATE as never);
    expect(invokeMock.mock.calls.map((c) => c[0])).toEqual([
      "download_update",
      "install_downloaded_update",
    ]);
  });

  it("登録したフックは download_update の後・install_downloaded_update の前に呼ばれる", async () => {
    const order: string[] = [];
    invokeMock.mockImplementation(async (cmd: string) => {
      order.push(cmd);
      if (cmd === "download_update") return true;
      return undefined;
    });
    setBeforeInstallHook(async () => {
      order.push("hook");
    });

    const { downloadAndInstall } = useUpdater();
    await downloadAndInstall(DUMMY_UPDATE as never);

    expect(order).toEqual(["download_update", "hook", "install_downloaded_update"]);
  });

  it("フックが失敗しても install_downloaded_update は呼ばれる（更新を止めない）", async () => {
    setBeforeInstallHook(async () => {
      throw new Error("save failed");
    });

    const { downloadAndInstall } = useUpdater();
    await expect(downloadAndInstall(DUMMY_UPDATE as never)).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("install_downloaded_update");
  });

  it("update が null の場合は invoke されない", async () => {
    const hook = vi.fn(async () => {});
    setBeforeInstallHook(hook);

    const { downloadAndInstall } = useUpdater();
    await downloadAndInstall(null);

    expect(hook).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("download_update が false の場合はフックも install_downloaded_update も呼ばれない", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "download_update") return false;
      return undefined;
    });
    const hook = vi.fn(async () => {});
    setBeforeInstallHook(hook);

    const { downloadAndInstall } = useUpdater();
    await downloadAndInstall(DUMMY_UPDATE as never);

    expect(hook).not.toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith("download_update");
    expect(invokeMock).not.toHaveBeenCalledWith("install_downloaded_update");
  });
});
