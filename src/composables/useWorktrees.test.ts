import { describe, it, expect, vi, beforeEach } from "vitest";

// `useWorktrees` は `useSettings` 経由で Tauri のプラグイン群に触るため、モジュール読み込みの
// 時点で落ちる。テストで意味を持つのは `invoke` だけなので、それ以外は最小のスタブにする。
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => "windows" }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(),
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  debug: vi.fn(),
  info: vi.fn(),
  warn: vi.fn(),
  error: vi.fn(),
}));
vi.mock("../i18n", () => ({ setLocale: vi.fn() }));

import { useWorktrees } from "./useWorktrees";
import { useSettings } from "./useSettings";
import type { WorktreeEntry } from "../types/settings";

const { worktrees, loadWorktreesFromSettings, removeWorktree } = useWorktrees();
const { settings } = useSettings();

function entry(id: string): WorktreeEntry {
  return {
    id,
    name: id,
    repositoryId: "repo-1",
    repositoryName: "repo",
    path: `X:/wt/${id}`,
    branchName: `worktree/${id}`,
  };
}

function seed(ids: string[]): void {
  settings.value.repositories = [{ id: "repo-1", name: "repo", path: "X:/git/repo" }];
  settings.value.worktrees = ids.map(entry);
  loadWorktreesFromSettings();
}

const ids = () => worktrees.value.map((w) => w.id);

describe("removeWorktree", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("削除待ちの間に配列の並びが変わっても、指定した id だけを消す", async () => {
    seed(["wt-a", "wt-b", "wt-c"]);

    // `git_worktree_remove` の完了を握って、その間に前方の要素を消して並びをずらす。
    // これが実機で起きていた状況（削除のリトライ中に別ワークツリーの削除が完了する / 並べ替え /
    // 他ウィンドウ発の再同期）に相当する。
    let releaseRemove: () => void = () => {};
    const removeStarted = new Promise<void>((resolveStarted) => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "git_worktree_remove") {
          resolveStarted();
          return new Promise<void>((r) => {
            releaseRemove = () => r();
          });
        }
        return Promise.resolve(undefined);
      });
    });

    const removing = removeWorktree("wt-c");
    await removeStarted;

    // wt-c は index 2 で始まったが、ここで先頭が消えて index 1 になる
    worktrees.value.splice(0, 1);
    releaseRemove();
    await removing;

    expect(ids()).toEqual(["wt-b"]);
    expect(settings.value.worktrees.map((w) => w.id)).toEqual(["wt-a", "wt-b"]);
  });

  it("並びが変わらない通常の削除はそのまま消える", async () => {
    seed(["wt-a", "wt-b"]);
    invokeMock.mockResolvedValue(undefined);

    await removeWorktree("wt-a");

    expect(ids()).toEqual(["wt-b"]);
    expect(settings.value.worktrees.map((w) => w.id)).toEqual(["wt-b"]);
  });

  it("待っている間に対象自身が消えていても、他のワークツリーを巻き込まない", async () => {
    seed(["wt-a", "wt-b"]);
    let releaseRemove: () => void = () => {};
    const removeStarted = new Promise<void>((resolveStarted) => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "git_worktree_remove") {
          resolveStarted();
          return new Promise<void>((r) => {
            releaseRemove = () => r();
          });
        }
        return Promise.resolve(undefined);
      });
    });

    const removing = removeWorktree("wt-a");
    await removeStarted;
    worktrees.value.splice(0, 1); // 対象自身が別経路で消えた
    releaseRemove();
    await removing;

    expect(ids()).toEqual(["wt-b"]);
  });
});
