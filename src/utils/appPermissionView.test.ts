import { describe, it, expect } from "vitest";
import { buildPermissionView, displayFsPath } from "./appPermissionView";
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

  describe("fs", () => {
    const scope = (scheme: "worktree" | "artifact", prefix: string, path: string, write: boolean) => ({
      scheme,
      prefix,
      path,
      write,
    });

    it("shows resolved absolute paths with read as normal and write as danger", () => {
      const perm: AppResolvedPermissions = {
        fs: {
          worktreeRoot: String.raw`\\?\C:\wt`,
          scopes: [
            scope("worktree", "src/", String.raw`\\?\C:\wt\src`, false),
            scope("worktree", "out/", String.raw`\\?\C:\wt\out`, true),
          ],
          allowSensitivePaths: false,
        },
      };
      const v = buildPermissionView(perm, null, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.access, r.label, r.severity])).toEqual([
        ["fs", "read", String.raw`C:\wt\src`, "normal"],
        ["fs", "write", String.raw`C:\wt\out`, "danger"],
      ]);
      expect(v.hasDanger).toBe(true);
      expect(v.hasCritical).toBe(false);
    });

    it("treats lifting the sensitive-path denylist as the strongest warning", () => {
      const perm: AppResolvedPermissions = {
        fs: {
          scopes: [scope("worktree", "", "/home/u/wt", true)],
          allowSensitivePaths: true,
        },
      };
      const v = buildPermissionView(perm, null, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.severity])).toEqual([
        ["fs", "danger"],
        ["fsSensitive", "critical"],
      ]);
      expect(v.hasCritical).toBe(true);
      // ワイルドカードの越境先ではないので、「他ワークツリーに届く」警告の根拠にはしない
      expect(v.rows.some((r) => r.wildcard)).toBe(false);
    });

    it("reports a widened fs scope as added and the old one as removed", () => {
      const before: AppResolvedPermissions = {
        fs: { scopes: [scope("artifact", "", "/data/a", false)], allowSensitivePaths: false },
      };
      const after: AppResolvedPermissions = {
        fs: {
          scopes: [scope("artifact", "", "/data/a", false), scope("artifact", "data/", "/data/a/data", true)],
          allowSensitivePaths: true,
        },
      };
      const v = buildPermissionView(after, before, DANGEROUS);
      expect(v.rows.filter((r) => r.added).map((r) => r.kind).sort()).toEqual(["fs", "fsSensitive"]);
      expect(v.removed).toEqual([]);
    });

    it("lists a narrowed fs scope as removed", () => {
      const before: AppResolvedPermissions = {
        fs: {
          scopes: [scope("artifact", "", "/data/a", false), scope("artifact", "data/", "/data/a/data", true)],
          allowSensitivePaths: true,
        },
      };
      const after: AppResolvedPermissions = {
        fs: { scopes: [scope("artifact", "", "/data/a", false)], allowSensitivePaths: false },
      };
      expect(buildPermissionView(after, before, DANGEROUS).removed.map((r) => r.kind).sort()).toEqual([
        "fs",
        "fsSensitive",
      ]);
    });
  });

  describe("db", () => {
    const scope = (scheme: "worktree" | "artifact", prefix: string, path: string, write: boolean) => ({
      scheme,
      prefix,
      path,
      write,
    });

    it("shows resolved absolute paths; a read scope is normal and a write scope is danger", () => {
      const perm: AppResolvedPermissions = {
        db: {
          worktreeRoot: String.raw`\\?\C:\wt`,
          scopes: [
            scope("worktree", "ro/", String.raw`\\?\C:\wt\ro`, false),
            scope("worktree", "data/", String.raw`\\?\C:\wt\data`, true),
          ],
          allowSensitivePaths: false,
        },
      };
      const v = buildPermissionView(perm, null, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.access, r.label, r.severity])).toEqual([
        ["db", "read", String.raw`C:\wt\ro`, "normal"],
        ["db", "write", String.raw`C:\wt\data`, "danger"],
      ]);
      expect(v.hasDanger).toBe(true);
      expect(v.hasCritical).toBe(false);
    });

    it("keeps db and fs rows apart even for the same path", () => {
      const same = { scopes: [scope("artifact", "", "/data/a", true)], allowSensitivePaths: false };
      const v = buildPermissionView({ fs: same, db: same }, null, DANGEROUS);
      expect(v.rows.map((r) => r.kind)).toEqual(["fs", "db"]);
      // db を新しく申請したときは、fs が同じでも db の行だけが「追加」になる
      const w = buildPermissionView({ fs: same, db: same }, { fs: same }, DANGEROUS);
      expect(w.rows.filter((r) => r.added).map((r) => r.kind)).toEqual(["db"]);
    });

    it("treats lifting the sensitive-path denylist for db as the strongest warning", () => {
      const perm: AppResolvedPermissions = {
        db: { scopes: [scope("worktree", "", "/home/u/wt", false)], allowSensitivePaths: true },
      };
      const v = buildPermissionView(perm, null, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.severity])).toEqual([
        ["db", "normal"],
        ["dbSensitive", "critical"],
      ]);
      expect(v.hasCritical).toBe(true);
    });

    it("lists a narrowed db scope as removed", () => {
      const before: AppResolvedPermissions = {
        db: { scopes: [scope("artifact", "", "/data/a", true)], allowSensitivePaths: true },
      };
      const after: AppResolvedPermissions = {
        db: { scopes: [scope("artifact", "", "/data/a", false)], allowSensitivePaths: false },
      };
      const v = buildPermissionView(after, before, DANGEROUS);
      expect(v.removed.map((r) => r.kind).sort()).toEqual(["db", "dbSensitive"]);
      expect(v.rows.filter((r) => r.added).map((r) => r.access)).toEqual(["read"]);
    });
  });

  describe("displayFsPath", () => {
    it("strips the Windows extended-length prefixes for display only", () => {
      expect(displayFsPath(String.raw`\\?\C:\x\y`)).toBe(String.raw`C:\x\y`);
      expect(displayFsPath(String.raw`\\?\UNC\srv\share\x`)).toBe(String.raw`\\srv\share\x`);
      expect(displayFsPath(String.raw`C:\plain`)).toBe(String.raw`C:\plain`);
      expect(displayFsPath("/home/u/wt")).toBe("/home/u/wt");
    });
  });
});
