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

import { migrateHomeWorktree, migrateRepositoryWorktrees, migrateTrayNotification } from "./useSettings";
import { isHomeWorktree } from "../utils/homeWorktree";
import { isRepositoryWorktree } from "../utils/repositoryWorktree";
import type { AppSettings, Workgroup, WorktreeEntry } from "../types/settings";

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

// #171 の仕様変更でワークグループへのフォールバックが無くなるため、移行しないと
// グループを OFF にしていたユーザーのワークツリーが一斉に通知オンへ戻る。
describe("migrateTrayNotification（一度きりの移行焼き込み）", () => {
  function loadedWith(worktrees: Partial<WorktreeEntry>[], groups: Workgroup[]): AppSettings {
    return { worktrees, workgroups: groups } as unknown as AppSettings;
  }

  it("未設定のワークツリーへ移行前の実効値（所属グループの値）を焼き込む", () => {
    const loaded = loadedWith(
      [
        { id: "a", workgroupId: "g-off" },
        { id: "b", workgroupId: "g-on" },
      ],
      [{ id: "g-off", trayNotification: false }, { id: "g-on", trayNotification: true }],
    );

    expect(migrateTrayNotification(loaded)).toBe(true);
    expect(loaded.worktrees[0].trayNotification).toBe(false);
    expect(loaded.worktrees[1].trayNotification).toBe(true);
  });

  it("個別に設定済みの値は上書きしない", () => {
    const loaded = loadedWith(
      [
        { id: "a", workgroupId: "g-off", trayNotification: true },
        { id: "b", workgroupId: "g-off", trayNotification: false },
      ],
      [{ id: "g-off", trayNotification: false }],
    );

    migrateTrayNotification(loaded);
    expect(loaded.worktrees[0].trayNotification).toBe(true);
    expect(loaded.worktrees[1].trayNotification).toBe(false);
  });

  // 旧仕様の groupOf と同じく「workgroupId 未設定 / 不明なら先頭グループ」
  it("workgroupId 未設定・不明なら先頭グループの値を焼き込む", () => {
    const loaded = loadedWith(
      [{ id: "a" }, { id: "b", workgroupId: "" }, { id: "c", workgroupId: "deleted" }],
      [{ id: "g-first", trayNotification: false }, { id: "g-2", trayNotification: true }],
    );

    migrateTrayNotification(loaded);
    expect(loaded.worktrees.every((w) => w.trayNotification === false)).toBe(true);
  });

  it("グループが未設定なら何も書かない（実効値 true のまま）", () => {
    const loaded = loadedWith([{ id: "a", workgroupId: "g" }], [{ id: "g" }]);

    expect(migrateTrayNotification(loaded)).toBe(true); // フラグ永続化のため true
    expect(loaded.worktrees[0]).not.toHaveProperty("trayNotification");
  });

  // Rust の Option は skip_serializing_if が無いので get_settings は null を返す。
  // null を「設定済み」と誤認すると移行が丸ごと素通りする。
  it("Rust 由来の null は未設定として扱う", () => {
    const loaded = loadedWith(
      [{ id: "a", workgroupId: "g", trayNotification: null } as unknown as WorktreeEntry],
      [{ id: "g", trayNotification: false }],
    );

    migrateTrayNotification(loaded);
    expect(loaded.worktrees[0].trayNotification).toBe(false);
  });

  // **一度きり**であることが要。毎回走ると oretachi_set_tray_notification の
  // enabled 省略呼び出し（キー削除 = 未設定に戻す）が次回起動で無言に巻き戻る。
  it("2回目以降は走らない — 未設定へ戻した値をグループ既定値で再適用しない", () => {
    const loaded = loadedWith(
      [{ id: "a", workgroupId: "g-off" }],
      [{ id: "g-off", trayNotification: false }],
    );
    migrateTrayNotification(loaded);
    expect(loaded.trayNotificationMigrated).toBe(true);

    // MCP の enabled 省略呼び出し相当（キーを消して未設定へ戻す）
    delete loaded.worktrees[0].trayNotification;

    expect(migrateTrayNotification(loaded)).toBe(false);
    expect(loaded.worktrees[0]).not.toHaveProperty("trayNotification");
  });
});
