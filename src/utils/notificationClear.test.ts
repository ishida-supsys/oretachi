import { describe, expect, it } from "vitest";
import { shouldClearNotification } from "./notificationClear";

describe("shouldClearNotification", () => {
  const entry = { count: 1, kind: "completed" };

  it("条件なしは常に true（後方互換）", () => {
    expect(shouldClearNotification(undefined, {})).toBe(true);
    expect(shouldClearNotification(entry, {})).toBe(true);
  });

  it("条件ありでエントリが無ければ false", () => {
    expect(shouldClearNotification(undefined, { expectedCount: 1 })).toBe(false);
    expect(shouldClearNotification(undefined, { expectedKind: "completed" })).toBe(false);
  });

  it("指定した項目だけ比較する", () => {
    expect(shouldClearNotification(entry, { expectedCount: 1, expectedKind: "completed" })).toBe(true);
    expect(shouldClearNotification(entry, { expectedCount: 1 })).toBe(true);
    expect(shouldClearNotification(entry, { expectedKind: "completed" })).toBe(true);
  });

  it("件数か種別が違えば false（approval が積まれた場合など）", () => {
    expect(shouldClearNotification({ count: 2, kind: "completed" }, { expectedCount: 1, expectedKind: "completed" })).toBe(false);
    expect(shouldClearNotification({ count: 1, kind: "approval" }, { expectedCount: 1, expectedKind: "completed" })).toBe(false);
  });
});
