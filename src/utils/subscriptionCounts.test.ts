import { describe, it, expect } from "vitest";
import type { SubscriptionView } from "../types/event";
import {
  buildSubscriptionCounts,
  isExactWorktreeTarget,
} from "./subscriptionCounts";

/** `event_list_subscriptions` の1行。テストで効く欄だけ上書きする。 */
function sub(over: Partial<SubscriptionView>): SubscriptionView {
  return {
    id: "s1",
    subscriberTerminalId: "t1",
    subscriberWorktreeId: "wt-a",
    subscriberWorktreeName: "a",
    subscriberSessionId: 1,
    agentName: "claude",
    targetWorktreeId: "wt-b",
    targetWorktreeName: "b",
    targetKind: "worktree",
    targetLabel: "b",
    eventKinds: ["worktree.closed"],
    delivery: "turn_end",
    spawnIfClosed: false,
    state: "active",
    createdAt: 0,
    expiresAt: null,
    orphanedAt: null,
    unacked: 0,
    undelivered: 0,
    ...over,
  };
}

describe("buildSubscriptionCounts", () => {
  it("厳密一致は購読者の ↑ と対象の ↓ の両方に数える", () => {
    const counts = buildSubscriptionCounts([sub({})]);
    expect(counts.get("wt-a")).toEqual({ outgoing: 1, incoming: 0 });
    expect(counts.get("wt-b")).toEqual({ outgoing: 0, incoming: 1 });
  });

  // `*` が1つあるだけで全カードに ↓1 が付くとバッジの情報量が消える（#137 の決定事項）
  it.each([
    ["all", "*"],
    ["workgroup", "workgroup:g1"],
    ["repo", "repo:oretachi"],
  ])("ワイルドカード購読 (%s) は ↓ に数えない", (targetKind, targetWorktreeId) => {
    const counts = buildSubscriptionCounts([
      sub({ targetKind, targetWorktreeId, targetWorktreeName: null }),
    ]);
    // 購読している側の ↑ には出る
    expect(counts.get("wt-a")).toEqual({ outgoing: 1, incoming: 0 });
    // 対象側には何も付かない（そもそもキーが立たない）
    expect(counts.get(targetWorktreeId)).toBeUndefined();
    expect(counts.get("wt-b")).toBeUndefined();
  });

  // 除くと再起動直後に全部消えたように見える
  it("引き継ぎ待ち (orphaned) も数える", () => {
    const counts = buildSubscriptionCounts([
      sub({ state: "orphaned", subscriberSessionId: null }),
    ]);
    expect(counts.get("wt-a")).toEqual({ outgoing: 1, incoming: 0 });
    expect(counts.get("wt-b")).toEqual({ outgoing: 0, incoming: 1 });
  });

  it("同じワークツリーの ↑ と ↓ を混ぜても取り違えない", () => {
    const counts = buildSubscriptionCounts([
      sub({ id: "s1", subscriberWorktreeId: "wt-a", targetWorktreeId: "wt-b" }),
      sub({ id: "s2", subscriberWorktreeId: "wt-c", targetWorktreeId: "wt-a" }),
      sub({ id: "s3", subscriberWorktreeId: "wt-d", targetWorktreeId: "wt-a" }),
    ]);
    expect(counts.get("wt-a")).toEqual({ outgoing: 1, incoming: 2 });
  });

  it("購読者ワークツリーが解決できない行は ↑ に数えない", () => {
    const counts = buildSubscriptionCounts([sub({ subscriberWorktreeId: null })]);
    expect(counts.get("wt-a")).toBeUndefined();
    expect(counts.get("wt-b")).toEqual({ outgoing: 0, incoming: 1 });
  });

  it("空配列は空の Map", () => {
    expect(buildSubscriptionCounts([]).size).toBe(0);
  });
});

describe("isExactWorktreeTarget", () => {
  // #134 で踏んだ誤判定。名前が引けないのはクローズ済みだけではない
  it("targetWorktreeName が null でも targetKind が worktree なら厳密一致", () => {
    expect(isExactWorktreeTarget(sub({ targetWorktreeName: null }))).toBe(true);
  });

  it("ワイルドカードは厳密一致ではない", () => {
    expect(isExactWorktreeTarget(sub({ targetKind: "all" }))).toBe(false);
    expect(isExactWorktreeTarget(sub({ targetKind: "workgroup" }))).toBe(false);
    expect(isExactWorktreeTarget(sub({ targetKind: "repo" }))).toBe(false);
  });
});
