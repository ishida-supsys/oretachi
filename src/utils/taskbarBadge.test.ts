import { describe, expect, it } from "vitest";
import { badgeLabel } from "./taskbarBadge";

describe("badgeLabel", () => {
  it("0 件は null", () => {
    expect(badgeLabel(0)).toBeNull();
  });
  it("1〜9 は数字", () => {
    expect(badgeLabel(1)).toBe("1");
    expect(badgeLabel(9)).toBe("9");
  });
  it("10 以上は 9+", () => {
    expect(badgeLabel(10)).toBe("9+");
    expect(badgeLabel(12345)).toBe("9+");
  });
});
