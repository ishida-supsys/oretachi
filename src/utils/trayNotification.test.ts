import { describe, it, expect } from "vitest";
import {
  resolveTrayNotificationMode,
  buildTrayNotificationModeMap,
  initialTrayNotification,
} from "./trayNotification";
import type { AppSettings, Workgroup, WorktreeEntry } from "../types/settings";

const groups: Workgroup[] = [
  { id: "g-first", trayNotification: "off" },
  { id: "g-on", trayNotification: "all" },
  { id: "g-need-input", trayNotification: "need_input" },
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

describe("resolveTrayNotificationMode", () => {
  it("ワークツリー個別の値がそのまま実効値になる", () => {
    expect(resolveTrayNotificationMode({ trayNotification: "all" })).toBe("all");
    expect(resolveTrayNotificationMode({ trayNotification: "need_input" })).toBe("need_input");
    expect(resolveTrayNotificationMode({ trayNotification: "off" })).toBe("off");
  });

  it("個別未設定なら all（既存 settings.json との後方互換）", () => {
    expect(resolveTrayNotificationMode({})).toBe("all");
  });

  // #171: ワークグループの trayNotification は「作成時の初期値」であり、
  // 解決時のフォールバック先ではない。ここでグループを見てしまうと、
  // グループ設定の変更が既存ワークツリーへ遡って効く。
  it("所属ワークグループの設定は実効値に影響しない", () => {
    for (const workgroupId of [undefined, "", "g-first", "g-on", "no-such-group"]) {
      // 引数の型からも workgroupId は落ちているが、呼び出し側は WorktreeEntry を
      // そのまま渡すため、余計なプロパティがあっても無視されることを確かめる
      const wt: WorktreeEntry = { ...baseWorktree, workgroupId };
      expect(resolveTrayNotificationMode(wt)).toBe("all");
    }
  });

  // settings.rs の Option フィールドには skip_serializing_if が無いため、get_settings は
  // 未設定を undefined ではなく null で返す（既存 settings.json の "autoApproval": null と同じ形）。
  it("Rust 由来の null は未設定として扱う", () => {
    const nulled = { trayNotification: null } as unknown as Partial<WorktreeEntry>;
    expect(resolveTrayNotificationMode(nulled)).toBe("all");
  });

  // Rust は読み込み時に旧 bool を all/off へ正規化して返すため通常は文字列しか来ないが、
  // 念のため bool もここで吸収する（型上は TrayNotificationMode | boolean | null | undefined）。
  it("旧形式の bool も all/off へ正規化する", () => {
    expect(resolveTrayNotificationMode({ trayNotification: true as unknown as never })).toBe("all");
    expect(resolveTrayNotificationMode({ trayNotification: false as unknown as never })).toBe("off");
  });

  it("未知の文字列は未設定(all)として扱う", () => {
    expect(resolveTrayNotificationMode({ trayNotification: "bogus" as unknown as never })).toBe("all");
  });
});

describe("buildTrayNotificationModeMap", () => {
  it("ワークツリー ID ごとの実効モードを返す", () => {
    const settings = {
      workgroups: groups,
      worktrees: [
        // 先頭グループが off でも、個別未設定なら all のまま（#171）
        { id: "a", workgroupId: "g-first" },
        { id: "b", workgroupId: "g-first", trayNotification: "all" },
        { id: "c", workgroupId: "g-first", trayNotification: "off" },
        { id: "d" },
        { id: "e", workgroupId: "g-first", trayNotification: "need_input" },
      ],
    } as unknown as AppSettings;

    const map = buildTrayNotificationModeMap(settings);
    expect(map.get("a")).toBe("all");
    expect(map.get("b")).toBe("all");
    expect(map.get("c")).toBe("off");
    expect(map.get("d")).toBe("all");
    expect(map.get("e")).toBe("need_input");
  });
});

describe("initialTrayNotification", () => {
  it("グループが明示設定していればその値を焼き込む", () => {
    expect(initialTrayNotification({ workgroupId: "g-first" }, groupOf)).toBe("off");
    expect(initialTrayNotification({ workgroupId: "g-on" }, groupOf)).toBe("all");
    expect(initialTrayNotification({ workgroupId: "g-need-input" }, groupOf)).toBe("need_input");
  });

  it("グループ未設定なら undefined（キーを書かない = 実効値 all）", () => {
    expect(initialTrayNotification({ workgroupId: "g-unset" }, groupOf)).toBeUndefined();
    // グループが 1 つも無い（先頭グループも取れない）ケース
    expect(initialTrayNotification({}, makeGroupOf([]))).toBeUndefined();
  });

  // useWorkgroups.groupOf の「未設定/不明なら先頭グループ」規則に乗る。
  // UI 上は先頭グループのカードに並ぶため、ここも先頭グループの初期値を焼き込む。
  it("workgroupId が未設定・空文字・不明なら先頭グループの値を焼き込む", () => {
    for (const workgroupId of [undefined, "", "no-such-group"]) {
      expect(initialTrayNotification({ workgroupId }, groupOf)).toBe("off");
    }
  });

  // Rust は未設定を null で返す。`?? undefined` で正規化しないと
  // entry.trayNotification に null が書かれ、settings.json に無駄なキーが残る。
  it("Rust 由来の null は undefined へ正規化する", () => {
    const nulledGroup = [{ id: "g", trayNotification: null }] as unknown as Workgroup[];
    expect(initialTrayNotification({ workgroupId: "g" }, makeGroupOf(nulledGroup))).toBeUndefined();
  });
});
