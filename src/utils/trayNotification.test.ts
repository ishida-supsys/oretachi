import { describe, it, expect } from "vitest";
import { resolveTrayNotification, buildTrayNotificationMap } from "./trayNotification";
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

describe("resolveTrayNotification", () => {
  it("ワークツリー個別がワークグループ既定値より優先される", () => {
    expect(resolveTrayNotification({ trayNotification: true, workgroupId: "g-first" }, groupOf)).toBe(true);
    expect(resolveTrayNotification({ trayNotification: false, workgroupId: "g-on" }, groupOf)).toBe(false);
  });

  it("個別未設定ならワークグループ既定値に従う", () => {
    expect(resolveTrayNotification({ workgroupId: "g-first" }, groupOf)).toBe(false);
    expect(resolveTrayNotification({ workgroupId: "g-on" }, groupOf)).toBe(true);
  });

  // Rust の settings::resolve_workgroup と同じ「未設定/不明なら先頭グループ」規則。
  // ここで先頭グループへ落ちないと、UI 上は先頭グループのカードに並んでいるのに
  // そのグループの trayNotification が効かないワークツリーが生まれる。
  it("workgroupId 未設定なら先頭グループの既定値に従う", () => {
    expect(resolveTrayNotification({}, groupOf)).toBe(false);
    expect(resolveTrayNotification({ workgroupId: "" }, groupOf)).toBe(false);
  });

  it("workgroupId が不明なら先頭グループの既定値に従う", () => {
    expect(resolveTrayNotification({ workgroupId: "no-such-group" }, groupOf)).toBe(false);
  });

  // settings.rs の Option フィールドには skip_serializing_if が無いため、get_settings は
  // 未設定を undefined ではなく null で返す（既存 settings.json の "autoApproval": null と同じ形）。
  // 型上は boolean | undefined なので type-check では拾えない。
  it("Rust 由来の null は未設定として扱う", () => {
    const nulled = { trayNotification: null } as unknown as Partial<WorktreeEntry>;
    expect(resolveTrayNotification(nulled, groupOf)).toBe(false); // 先頭グループの false へ落ちる
    expect(resolveTrayNotification({ ...nulled, workgroupId: "g-on" }, groupOf)).toBe(true);
    expect(resolveTrayNotification({ ...nulled, workgroupId: "g-unset" }, groupOf)).toBe(true);
  });

  it("引き当てたグループが未設定なら true", () => {
    expect(resolveTrayNotification({ workgroupId: "g-unset" }, groupOf)).toBe(true);
    // グループが 1 つも無い（先頭グループも取れない）ケース
    expect(resolveTrayNotification({}, makeGroupOf([]))).toBe(true);
  });
});

describe("buildTrayNotificationMap", () => {
  it("ワークツリー ID ごとの実効値を返す", () => {
    const settings = {
      workgroups: groups,
      worktrees: [
        { id: "a", workgroupId: "g-first" },
        { id: "b", workgroupId: "g-first", trayNotification: true },
        { id: "c", workgroupId: "g-unset" },
        // 未設定は先頭グループ（trayNotification: false）に落ちる
        { id: "d" },
      ],
    } as unknown as AppSettings;

    const map = buildTrayNotificationMap(settings, groupOf);
    expect(map.get("a")).toBe(false);
    expect(map.get("b")).toBe(true);
    expect(map.get("c")).toBe(true);
    expect(map.get("d")).toBe(false);
  });
});
