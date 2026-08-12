import { describe, it, expect } from "vitest";
import { mainWindowShowsDelivery, subWindowShowsDelivery } from "./eventToastScope";

const DETACHED = new Set(["wt-detached"]);
const isDetached = (id: string) => DETACHED.has(id);

describe("配送トーストの表示責任 (1配送=1トースト)", () => {
  // [worktreeId, メインが出すか, サブ(wt-detached 担当)が出すか]
  const cases: Array<[string | null, boolean, boolean]> = [
    // 分離済み: サブウィンドウの担当。メインは抑制する
    ["wt-detached", false, true],
    // 非分離: メインの担当。別ワークツリーのサブウィンドウは出さない
    ["wt-attached", true, false],
    // 解決不能: どのサブウィンドウにも紐づかないのでメインが拾う
    [null, true, false],
  ];

  for (const [worktreeId, expectMain, expectSub] of cases) {
    it(`worktreeId=${JSON.stringify(worktreeId)} は ちょうど1つのウィンドウが出す`, () => {
      const main = mainWindowShowsDelivery(worktreeId, isDetached);
      const sub = subWindowShowsDelivery(worktreeId, "wt-detached");
      expect(main).toBe(expectMain);
      expect(sub).toBe(expectSub);
      // 不変条件: 重複も取りこぼしも無い
      expect([main, sub].filter(Boolean).length).toBe(1);
    });
  }

  it("無関係なサブウィンドウは他ワークツリー宛を出さない", () => {
    expect(subWindowShowsDelivery("wt-detached", "wt-other")).toBe(false);
    expect(subWindowShowsDelivery(null, "wt-other")).toBe(false);
  });

  it("worktreeId が空文字のサブウィンドウ (クエリ欠落) は何も出さない", () => {
    expect(subWindowShowsDelivery("wt-detached", "")).toBe(false);
    expect(subWindowShowsDelivery(null, "")).toBe(false);
  });
});
