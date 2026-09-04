import { describe, it, expect, vi } from "vitest";

// `useSettings` はモジュール読み込みの時点で Tauri のプラグイン群に触るため、
// useWorktrees.test.ts と同じく最小のスタブを当てる。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => "windows" }));
vi.mock("@tauri-apps/api/event", () => ({ emit: vi.fn(), listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "main" }) }));
vi.mock("@tauri-apps/plugin-log", () => ({
  debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn(),
}));
vi.mock("../i18n", () => ({ setLocale: vi.fn() }));

import { migrateHomeWorktree, migrateRepositoryWorktrees } from "./useSettings";
import { isHomeWorktree } from "../utils/homeWorktree";
import { isRepositoryWorktree } from "../utils/repositoryWorktree";
import type { AppSettings, Workgroup } from "../types/settings";

function settingsWith(groups: Workgroup[]): AppSettings {
  return {
    worktreeBaseDir: "X:/devel/worktree",
    worktrees: [],
    repositories: [{ id: "X:/devel/oretachi", name: "oretachi", path: "X:/devel/oretachi" }],
    workgroups: groups,
  } as unknown as AppSettings;
}

// 擬似ワークツリー（ホーム / リポジトリ）も「新規作成」なので、通常ワークツリーと同じく
// 所属グループの trayNotification を作成時に焼き込む（#171）。ここが漏れると、
// migrateWorkgroups が先頭グループへ割り当てた後もグループ既定値を継承できず、
// リポジトリを追加するたびに通知オンの擬似ワークツリーが増える。
describe("擬似ワークツリー生成時の trayNotification 焼き込み", () => {
  it("先頭グループが false ならホーム / リポジトリ擬似ワークツリーへ焼き込む", () => {
    const loaded = settingsWith([{ id: "g-first", trayNotification: false }, { id: "g-2" }]);

    expect(migrateHomeWorktree(loaded)).toBe(true);
    expect(migrateRepositoryWorktrees(loaded).changed).toBe(true);

    expect(loaded.worktrees.find(isHomeWorktree)?.trayNotification).toBe(false);
    expect(loaded.worktrees.find(isRepositoryWorktree)?.trayNotification).toBe(false);
  });

  it("先頭グループが未設定ならキーを書かない（実効値 true のまま）", () => {
    const loaded = settingsWith([{ id: "g-first" }]);

    migrateHomeWorktree(loaded);
    migrateRepositoryWorktrees(loaded);

    expect(loaded.worktrees.find(isHomeWorktree)).not.toHaveProperty("trayNotification");
    expect(loaded.worktrees.find(isRepositoryWorktree)).not.toHaveProperty("trayNotification");
  });

  it("グループが1件も無い初回起動でも落ちず、キーを書かない", () => {
    const loaded = settingsWith([]);

    migrateHomeWorktree(loaded);
    migrateRepositoryWorktrees(loaded);

    expect(loaded.worktrees.find(isHomeWorktree)).not.toHaveProperty("trayNotification");
    expect(loaded.worktrees.find(isRepositoryWorktree)).not.toHaveProperty("trayNotification");
  });

  // 冪等性: 2 回目以降の migrate は生成が走らないので、ユーザーが後から個別に変えた値を
  // グループ既定値で踏み直さない。
  it("既存の擬似ワークツリーの値はグループ既定値で上書きしない", () => {
    const loaded = settingsWith([{ id: "g-first", trayNotification: false }]);
    migrateHomeWorktree(loaded);
    migrateRepositoryWorktrees(loaded);

    // ユーザーが個別にオンへ戻す
    for (const wt of loaded.worktrees) wt.trayNotification = true;

    expect(migrateHomeWorktree(loaded)).toBe(false);
    expect(migrateRepositoryWorktrees(loaded).changed).toBe(false);
    expect(loaded.worktrees.every((w) => w.trayNotification === true)).toBe(true);
  });
});
