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

  // 承認ダイアログの行テンプレートは、末尾の `<template v-else>` がメモリーとして描画する。
  // 種別を足したのに専用の分岐を足し忘れると、新しい権限が「永続メモリー」として承認されてしまうので、ここで落とす。
  // `scopeKinds` のように、同じ分岐を共有する種別（subWindowFlag など）も `r.kind === '…'` で書かれている必要がある
  it("AppGrantDialog renders an explicit branch for every row kind (the trailing v-else is the memory row)", () => {
    const src = readFileSync(new URL("../components/AppGrantDialog.vue", import.meta.url), "utf-8").replace(/\r\n/g, "\n");
    const memoryRow = "<template v-else>\n              <span class=\"kind\">{{ t(\"kindMemory\") }}</span>";
    expect(src).toContain(memoryRow);
    for (const k of PERMISSION_ROW_KINDS) {
      if (k === "memory") continue; // メモリー自身は末尾の v-else
      expect(src, k).toMatch(new RegExp(`r\\.kind === '${k}'`));
    }
  });

  // アプリ一覧の展開行は `t("kind_" + kind)` で種別名を引く。i18n キーの足し忘れは、生のキー文字列が出る
  it("AppsTable has a kind label for every row kind that uses the default label", () => {
    const src = readFileSync(new URL("../components/AppsTable.vue", import.meta.url), "utf-8");
    const special = new Set(["fs", "db"]); // 読み書きで別のキー（kindFsRead / kindFsWrite …）を使う
    for (const k of PERMISSION_ROW_KINDS) {
      if (special.has(k)) continue;
      expect(src, k).toContain(`"kind_${k}"`);
    }
  });
});
