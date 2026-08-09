import { describe, it, expect } from "vitest";
import type { Repository } from "../types/settings";
import {
  REPOSITORY_WORKTREE_ID_PREFIX,
  makeRepositoryWorktreeId,
  isRepositoryWorktree,
  isPseudoWorktree,
  makeRepositoryWorktreeEntry,
  sortPseudoFirst,
} from "./repositoryWorktree";

function repo(overrides: Partial<Repository> = {}): Repository {
  return {
    id: "X:\\devel\\oretachi",
    name: "oretachi",
    path: "X:\\devel\\oretachi",
    ...overrides,
  };
}

describe("makeRepositoryWorktreeId", () => {
  it("同じ入力からは常に同じ ID を返す", () => {
    expect(makeRepositoryWorktreeId("X:\\devel\\oretachi")).toBe(
      makeRepositoryWorktreeId("X:\\devel\\oretachi"),
    );
  });

  it("パスが違えば ID も違う（末尾セグメントが同じでも）", () => {
    const a = makeRepositoryWorktreeId("X:\\devel\\app");
    const b = makeRepositoryWorktreeId("Y:\\other\\app");
    expect(a).not.toBe(b);
  });

  it("ファイル名に使えない文字を含まない", () => {
    const id = makeRepositoryWorktreeId("X:\\devel\\my app (v2)/sub");
    expect(id.startsWith(REPOSITORY_WORKTREE_ID_PREFIX)).toBe(true);
    expect(id).toMatch(/^[A-Za-z0-9._-]+$/);
  });

  it("末尾セグメントが空でも ID を作れる", () => {
    const id = makeRepositoryWorktreeId("X:\\devel\\oretachi\\");
    expect(id).toMatch(/^repo-oretachi-[0-9a-f]{8}$/);
    expect(makeRepositoryWorktreeId("")).toMatch(/^repo-repo-[0-9a-f]{8}$/);
  });

  it("スラッシュ区切りでも同じ末尾セグメントを拾う", () => {
    expect(makeRepositoryWorktreeId("X:/devel/oretachi")).toMatch(/^repo-oretachi-[0-9a-f]{8}$/);
  });
});

describe("isRepositoryWorktree / isPseudoWorktree", () => {
  it("isRepository が true のときだけ isRepositoryWorktree は true", () => {
    expect(isRepositoryWorktree({ isRepository: true })).toBe(true);
    expect(isRepositoryWorktree({ isRepository: false })).toBe(false);
    expect(isRepositoryWorktree({})).toBe(false);
    expect(isRepositoryWorktree(undefined)).toBe(false);
    expect(isRepositoryWorktree(null)).toBe(false);
  });

  it("isPseudoWorktree はホームとリポジトリの両方を拾う", () => {
    expect(isPseudoWorktree({ isHome: true })).toBe(true);
    expect(isPseudoWorktree({ isRepository: true })).toBe(true);
    expect(isPseudoWorktree({})).toBe(false);
    expect(isPseudoWorktree(null)).toBe(false);
  });
});

describe("makeRepositoryWorktreeEntry", () => {
  it("リポジトリのルートを path とする擬似エントリを作る", () => {
    const entry = makeRepositoryWorktreeEntry(repo());
    expect(entry.id).toBe(makeRepositoryWorktreeId("X:\\devel\\oretachi"));
    expect(entry.path).toBe("X:\\devel\\oretachi");
    expect(entry.repositoryId).toBe("X:\\devel\\oretachi");
    expect(entry.repositoryName).toBe("oretachi");
    expect(entry.isRepository).toBe(true);
    // git ワークツリーではないのでブランチは持たない
    expect(entry.branchName).toBe("");
    expect(entry.isHome).toBeUndefined();
  });
});

describe("sortPseudoFirst", () => {
  it("ホーム → リポジトリ → 通常 の順に並べ、同種内の順序は維持する", () => {
    const list = [
      { id: "wt1" },
      { id: "repoB", isRepository: true },
      { id: "home", isHome: true },
      { id: "wt2" },
      { id: "repoA", isRepository: true },
    ];
    expect(sortPseudoFirst(list).map((w) => w.id)).toEqual([
      "home",
      "repoB",
      "repoA",
      "wt1",
      "wt2",
    ]);
  });

  it("擬似エントリが無ければ元の配列をそのまま返す（新規配列を作らない）", () => {
    const list = [{ id: "a", isHome: false }, { id: "b", isRepository: false }];
    expect(sortPseudoFirst(list)).toBe(list);
  });

  it("空配列でも壊れない", () => {
    expect(sortPseudoFirst<{ isHome?: boolean }>([])).toEqual([]);
  });
});
