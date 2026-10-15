import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { PERMISSION_ROW_KINDS, isKnownRowKind } from "./appPermissionView";

describe("permission row kinds", () => {
  it("treats only the declared kinds as known", () => {
    for (const k of PERMISSION_ROW_KINDS) expect(isKnownRowKind(k)).toBe(true);
    expect(isKnownRowKind("wasm")).toBe(false);
    expect(isKnownRowKind("")).toBe(false);
  });

  // 起動画面の行テンプレートの末尾 v-else がメモリー表記で他の種別を拾った不具合（#388）の再発防止。
  // 種別を足したのに起動画面の分岐を足し忘れると、ここで落ちる
  it("AppLaunchPanel renders an explicit branch for every row kind and falls back safely", () => {
    const src = readFileSync(new URL("../components/artifact/AppLaunchPanel.vue", import.meta.url), "utf-8");
    for (const k of PERMISSION_ROW_KINDS) {
      expect(src, k).toMatch(new RegExp(`r\.kind === '${k}'`));
    }
    expect(src).toContain("!isKnownRowKind(r.kind)");
    expect(src).not.toMatch(/<template v-else>\{\{ t\("granted\.memory"/);
  });
});
