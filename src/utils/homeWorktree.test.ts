import { describe, it, expect } from "vitest";
import { HOME_WORKTREE_ID, isHomeWorktree, sortHomeFirst, makeHomeWorktreeEntry } from "./homeWorktree";

describe("isHomeWorktree", () => {
  it("isHome が true のときだけ true", () => {
    expect(isHomeWorktree({ isHome: true })).toBe(true);
    expect(isHomeWorktree({ isHome: false })).toBe(false);
    expect(isHomeWorktree({})).toBe(false);
    expect(isHomeWorktree(undefined)).toBe(false);
    expect(isHomeWorktree(null)).toBe(false);
  });
});

describe("sortHomeFirst", () => {
  it("ホームを先頭に移動し、残りの相対順序は維持する", () => {
    const a = { id: "a", isHome: false };
    const b = { id: "b", isHome: true };
    const c = { id: "c", isHome: false };
    expect(sortHomeFirst([a, b, c]).map((w) => w.id)).toEqual(["b", "a", "c"]);
  });

  it("ホームが既に先頭ならそのまま", () => {
    const home = { id: "home", isHome: true };
    const a = { id: "a", isHome: false };
    expect(sortHomeFirst([home, a]).map((w) => w.id)).toEqual(["home", "a"]);
  });

  it("ホームが無ければ元の配列をそのまま返す（新規配列を作らない）", () => {
    const list = [{ id: "a", isHome: false }, { id: "b", isHome: false }];
    expect(sortHomeFirst(list)).toBe(list);
  });

  it("空配列でも壊れない", () => {
    expect(sortHomeFirst<{ isHome?: boolean }>([])).toEqual([]);
  });
});

describe("makeHomeWorktreeEntry", () => {
  it("path に worktreeBaseDir を、isHome に true を持つエントリを作る", () => {
    const entry = makeHomeWorktreeEntry("X:/devel/worktree");
    expect(entry.id).toBe(HOME_WORKTREE_ID);
    expect(entry.path).toBe("X:/devel/worktree");
    expect(entry.isHome).toBe(true);
    // git ワークツリーではないのでリポジトリ・ブランチは持たない
    expect(entry.repositoryId).toBe("");
    expect(entry.branchName).toBe("");
  });
});
