import { describe, it, expect } from "vitest";
import {
  resolveTrayNotification,
  buildTrayNotificationMap,
  initialTrayNotification,
} from "./trayNotification";
import type { AppSettings, Workgroup, WorktreeEntry } from "../types/settings";

const groups: Workgroup[] = [
  { id: "g-first", trayNotification: false },
  { id: "g-on", trayNotification: true },
  { id: "g-unset" },
];

/** useWorkgroups.groupOf と同じフォールバック規則（未設定/不明なら先頭グループ） */
function makeGroupOf(list: Workgroup[]) {
  return (worktree: Pick<WorktreeEntry, "workgroupId">) => {
    const id = worktree.workgroupId;
    if (id && list.some((g) => g.id === id)) return list.find((g) => g.id === id);
    return list[0];
  };
}

const groupOf = makeGroupOf(groups);

const baseWorktree: WorktreeEntry = {
  id: "wt-1",
  name: "wt",
  repositoryId: "r",
  repositoryName: "repo",
  path: "/path",
  branchName: "main",
};

describe("resolveTrayNotification", () => {
  it("ワークツリー個別の値がそのまま実効値になる", () => {
    expect(resolveTrayNotification({ trayNotification: true })).toBe(true);
    expect(resolveTrayNotification({ trayNotification: false })).toBe(false);
  });

  it("個別未設定なら true（既存 settings.json との後方互換）", () => {
    expect(resolveTrayNotification({})).toBe(true);
  });

  // #171: ワークグループの trayNotification は「作成時の初期値」であり、
  // 解決時のフォールバック先ではない。ここでグループを見てしまうと、
  // グループ設定の変更が既存ワークツリーへ遡って効く。
  it("所属ワークグループの設定は実効値に影響しない", () => {
    for (const workgroupId of [undefined, "", "g-first", "g-on", "no-such-group"]) {
      // 引数の型からも workgroupId は落ちているが、呼び出し側は WorktreeEntry を
      // そのまま渡すため、余計なプロパティがあっても無視されることを確かめる
      const wt: WorktreeEntry = { ...baseWorktree, workgroupId };
      expect(resolveTrayNotification(wt)).toBe(true);
    }
  });

  // settings.rs の Option フィールドには skip_serializing_if が無いため、get_settings は
  // 未設定を undefined ではなく null で返す（既存 settings.json の "autoApproval": null と同じ形）。
  // 型上は boolean | undefined なので type-check では拾えない。
  it("Rust 由来の null は未設定として扱う", () => {
    const nulled = { trayNotification: null } as unknown as Partial<WorktreeEntry>;
    expect(resolveTrayNotification(nulled)).toBe(true);
  });
});

describe("buildTrayNotificationMap", () => {
  it("ワークツリー ID ごとの実効値を返す", () => {
    const settings = {
      workgroups: groups,
      worktrees: [
        // 先頭グループが false でも、個別未設定なら true のまま（#171）
        { id: "a", workgroupId: "g-first" },
        { id: "b", workgroupId: "g-first", trayNotification: true },
        { id: "c", workgroupId: "g-first", trayNotification: false },
        { id: "d" },
      ],
    } as unknown as AppSettings;

    const map = buildTrayNotificationMap(settings);
    expect(map.get("a")).toBe(true);
    expect(map.get("b")).toBe(true);
    expect(map.get("c")).toBe(false);
    expect(map.get("d")).toBe(true);
  });
});

describe("initialTrayNotification", () => {
  it("グループが明示設定していればその値を焼き込む", () => {
    expect(initialTrayNotification({ workgroupId: "g-first" }, groupOf)).toBe(false);
    expect(initialTrayNotification({ workgroupId: "g-on" }, groupOf)).toBe(true);
  });

  it("グループ未設定なら undefined（キーを書かない = 実効値 true）", () => {
    expect(initialTrayNotification({ workgroupId: "g-unset" }, groupOf)).toBeUndefined();
    // グループが 1 つも無い（先頭グループも取れない）ケース
    expect(initialTrayNotification({}, makeGroupOf([]))).toBeUndefined();
  });

  // useWorkgroups.groupOf の「未設定/不明なら先頭グループ」規則に乗る。
  // UI 上は先頭グループのカードに並ぶため、ここも先頭グループの初期値を焼き込む。
  it("workgroupId が未設定・空文字・不明なら先頭グループの値を焼き込む", () => {
    for (const workgroupId of [undefined, "", "no-such-group"]) {
      expect(initialTrayNotification({ workgroupId }, groupOf)).toBe(false);
    }
  });

  // Rust は未設定を null で返す。`?? undefined` で正規化しないと
  // entry.trayNotification に null が書かれ、settings.json に無駄なキーが残る。
  it("Rust 由来の null は undefined へ正規化する", () => {
    const nulledGroup = [{ id: "g", trayNotification: null }] as unknown as Workgroup[];
    expect(initialTrayNotification({ workgroupId: "g" }, makeGroupOf(nulledGroup))).toBeUndefined();
  });
});
