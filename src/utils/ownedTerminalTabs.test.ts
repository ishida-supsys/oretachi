import { describe, it, expect } from "vitest";
import { collectOwnedByTab } from "./ownedTerminalTabs";

describe("collectOwnedByTab", () => {
  const owned = new Map([[7, { appName: "Dash" }]]);

  it("maps session ids to the frontend terminal ids", () => {
    const entries: [number, { sessionId?: number | null } | undefined][] = [
      [1, { sessionId: 3 }],
      [2, { sessionId: 7 }],
      [3, { sessionId: null }],
      [4, undefined],
    ];
    expect([...collectOwnedByTab(owned, entries)]).toEqual([[2, { appName: "Dash" }]]);
  });

  it("returns early when nothing is owned", () => {
    expect(collectOwnedByTab(new Map(), [[1, { sessionId: 7 }]]).size).toBe(0);
  });

  it("accumulates into an existing map", () => {
    const into = new Map([[9, { appName: "Other" }]]);
    collectOwnedByTab(owned, [[2, { sessionId: 7 }]], into);
    expect([...into.keys()].sort()).toEqual([2, 9]);
  });
});
