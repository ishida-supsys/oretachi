import { describe, it, expect } from "vitest";
import { buildPermissionView } from "./appPermissionView";
import type { AppResolvedPermissions } from "../types/appInstance";

const DANGEROUS = [
  "oretachi_write_terminal",
  "oretachi_send_keys",
  "oretachi_answer_prompt",
  "oretachi_add_task",
];

const named = (stored: string, label = stored) => ({ raw: label, stored, label, wildcard: false });
const wild = (stored: string) => ({ raw: stored, stored, label: stored, wildcard: true });

describe("buildPermissionView", () => {
  it("marks the user-equivalent tools as danger and others as normal", () => {
    const perm: AppResolvedPermissions = {
      mcp: { tools: ["notify_worktree", "oretachi_write_terminal", "oretachi_add_task"], targets: [] },
    };
    const v = buildPermissionView(perm, null, DANGEROUS);
    expect(v.rows.map((r) => [r.value, r.severity])).toEqual([
      ["notify_worktree", "normal"],
      ["oretachi_write_terminal", "danger"],
      ["oretachi_add_task", "danger"],
    ]);
    expect(v.hasDanger).toBe(true);
    expect(v.hasCritical).toBe(false);
    expect(v.ownWorktreeOnly).toBe(true);
  });

  it("treats wildcard targets as the strongest warning and named targets as normal", () => {
    const perm: AppResolvedPermissions = {
      mcp: { tools: ["oretachi_show_worktree"], targets: [named("wt-1", "feature-x"), wild("*")] },
    };
    const v = buildPermissionView(perm, null, DANGEROUS);
    const targets = v.rows.filter((r) => r.kind === "target");
    expect(targets.map((r) => r.severity)).toEqual(["normal", "critical"]);
    expect(targets[0].label).toBe("feature-x");
    expect(v.hasCritical).toBe(true);
    expect(v.ownWorktreeOnly).toBe(false);
  });

  it("shows the resolved memory limit", () => {
    const v = buildPermissionView({ memory: { maxBytes: 4096 } }, null, DANGEROUS);
    expect(v.rows).toHaveLength(1);
    expect(v.rows[0]).toMatchObject({ kind: "memory", value: "4096", severity: "normal" });
  });

  it("reports added and removed rows when re-approving", () => {
    const previous: AppResolvedPermissions = {
      mcp: { tools: ["notify_worktree", "oretachi_poll_inbox"], targets: [] },
    };
    const next: AppResolvedPermissions = {
      mcp: { tools: ["notify_worktree", "oretachi_write_terminal"], targets: [wild("*")] },
      memory: { maxBytes: 10 },
    };
    const v = buildPermissionView(next, previous, DANGEROUS);
    expect(v.rows.filter((r) => r.added).map((r) => r.value).sort()).toEqual(
      ["*", "10", "oretachi_write_terminal"].sort(),
    );
    expect(v.removed.map((r) => r.value)).toEqual(["oretachi_poll_inbox"]);
  });

  it("has no added rows and no removed rows on a first approval", () => {
    const v = buildPermissionView({ mcp: { tools: ["notify_worktree"], targets: [] } }, null, DANGEROUS);
    expect(v.rows.every((r) => !r.added)).toBe(true);
    expect(v.removed).toEqual([]);
  });
});
