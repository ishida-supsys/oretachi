import { describe, it, expect } from "vitest";
import { resolveTrayNotification, buildTrayNotificationMap } from "./trayNotification";
import type { AppSettings, Workgroup } from "../types/settings";

const groups: Workgroup[] = [
  { id: "g-off", trayNotification: false },
  { id: "g-on", trayNotification: true },
  { id: "g-unset" },
];

describe("resolveTrayNotification", () => {
  it("未設定はどこにも設定がなければ true", () => {
    expect(resolveTrayNotification({}, groups)).toBe(true);
    expect(resolveTrayNotification({ workgroupId: "g-unset" }, groups)).toBe(true);
    expect(resolveTrayNotification({ workgroupId: "no-such-group" }, groups)).toBe(true);
    expect(resolveTrayNotification({}, undefined)).toBe(true);
  });

  it("ワークツリー個別がワークグループ既定値より優先される", () => {
    expect(resolveTrayNotification({ trayNotification: true, workgroupId: "g-off" }, groups)).toBe(true);
    expect(resolveTrayNotification({ trayNotification: false, workgroupId: "g-on" }, groups)).toBe(false);
  });

  it("個別未設定ならワークグループ既定値に従う", () => {
    expect(resolveTrayNotification({ workgroupId: "g-off" }, groups)).toBe(false);
    expect(resolveTrayNotification({ workgroupId: "g-on" }, groups)).toBe(true);
  });
});

describe("buildTrayNotificationMap", () => {
  it("ワークツリー ID ごとの実効値を返す", () => {
    const settings = {
      workgroups: groups,
      worktrees: [
        { id: "a", workgroupId: "g-off" },
        { id: "b", workgroupId: "g-off", trayNotification: true },
        { id: "c" },
      ],
    } as unknown as AppSettings;

    const map = buildTrayNotificationMap(settings);
    expect(map.get("a")).toBe(false);
    expect(map.get("b")).toBe(true);
    expect(map.get("c")).toBe(true);
  });
});
