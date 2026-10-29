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

  it("treats the dedicated AI terminal as the strongest warning", () => {
    const v = buildPermissionView({ aiTerminal: { agents: ["claude"], maxConcurrent: 2 } }, null, DANGEROUS);
    expect(v.rows).toHaveLength(1);
    expect(v.rows[0]).toMatchObject({ kind: "aiTerminal", value: "claude×2", severity: "critical" });
    expect(v.hasCritical).toBe(true);
  });

  it("treats microphone audio as the strongest warning and shows the resolved values", () => {
    const v = buildPermissionView(
      { audio: { microphone: true, maxSeconds: 60, sampleRates: [16000, 48000] } },
      null,
      DANGEROUS,
    );
    expect(v.rows).toHaveLength(1);
    expect(v.rows[0]).toMatchObject({ kind: "audio", value: "60s · 16000/48000Hz", severity: "critical" });
    expect(v.hasCritical).toBe(true);
  });

  it("shows a change in the audio limits as a removed and an added row", () => {
    const before: AppResolvedPermissions = { audio: { microphone: true, maxSeconds: 60, sampleRates: [16000] } };
    const after: AppResolvedPermissions = { audio: { microphone: true, maxSeconds: 120, sampleRates: [16000] } };
    const v = buildPermissionView(after, before, DANGEROUS);
    expect(v.rows[0].added).toBe(true);
    expect(v.removed.map((r) => r.value)).toEqual(["60s · 16000Hz"]);
  });

  it("shows a change in the AI terminal limit as a removed and an added row", () => {
    const before: AppResolvedPermissions = { aiTerminal: { agents: ["claude"], maxConcurrent: 1 } };
    const after: AppResolvedPermissions = { aiTerminal: { agents: ["claude"], maxConcurrent: 3 } };
    const v = buildPermissionView(after, before, DANGEROUS);
    expect(v.rows[0].added).toBe(true);
    expect(v.removed.map((r) => r.value)).toEqual(["claude×1"]);
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

  describe("network", () => {
    it("lists WebSocket prefixes too; wss is normal and plain ws is danger", () => {
      const perm: AppResolvedPermissions = {
        network: { allow: ["wss://stream.example.com/v1/", "ws://plain.example.com/"] },
      };
      const v = buildPermissionView(perm, null, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.label, r.severity])).toEqual([
        ["network", "wss://stream.example.com/v1/", "normal"],
        ["network", "ws://plain.example.com/", "danger"],
      ]);
    });

    it("lists one row per URL prefix; https is normal and plain http is danger", () => {
      const perm: AppResolvedPermissions = {
        network: { allow: ["https://api.example.com/v1/", "http://plain.example.com/"] },
      };
      const v = buildPermissionView(perm, null, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.label, r.severity])).toEqual([
        ["network", "https://api.example.com/v1/", "normal"],
        ["network", "http://plain.example.com/", "danger"],
      ]);
      expect(v.hasDanger).toBe(true);
      expect(v.hasCritical).toBe(false);
    });

    it("shows only the added prefix as added and a dropped prefix as removed on re-approval", () => {
      const before: AppResolvedPermissions = { network: { allow: ["https://a.example.com/", "https://b.example.com/"] } };
      const after: AppResolvedPermissions = { network: { allow: ["https://a.example.com/", "https://c.example.com/"] } };
      const v = buildPermissionView(after, before, DANGEROUS);
      expect(v.rows.filter((r) => r.added).map((r) => r.value)).toEqual(["https://c.example.com/"]);
      expect(v.removed.map((r) => r.value)).toEqual(["https://b.example.com/"]);
    });
  });

  describe("media", () => {
    const scope = (prefix: string, scheme: "worktree" | "artifact" = "worktree") => ({
      scheme,
      prefix,
      path: `C:\\wt\\${prefix}`,
      write: false,
    });
    const mediaFs = (...scopes: ReturnType<typeof scope>[]) => ({ scopes, allowSensitivePaths: false });

    it("lists local scopes (always read) and URL prefixes as separate rows, apart from fs / network", () => {
      const perm: AppResolvedPermissions = {
        media: {
          fs: mediaFs(scope("videos/"), scope("clips/", "artifact")),
          allow: ["https://cdn.example.com/", "http://plain.example.com/"],
        },
      };
      const v = buildPermissionView(perm, null, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.label, r.severity, r.access ?? null, r.scheme ?? null])).toEqual([
        ["mediaRead", "C:\\wt\\videos/", "normal", "read", "worktree"],
        ["mediaRead", "C:\\wt\\clips/", "normal", "read", "artifact"],
        ["mediaNet", "https://cdn.example.com/", "normal", null, null],
        ["mediaNet", "http://plain.example.com/", "danger", null, null],
      ]);
      // fs / network の行にはならない（権限として独立している）
      expect(v.rows.some((r) => r.kind === "fs" || r.kind === "network")).toBe(false);
      expect(v.hasDanger).toBe(true);
      expect(v.hasCritical).toBe(false);
    });

    it("media without local scopes or URLs adds no rows", () => {
      expect(buildPermissionView({ media: { allow: [] } }, null, DANGEROUS).rows).toEqual([]);
    });

    it("shows added rows and removed rows on re-approval, URL and local separately", () => {
      const before: AppResolvedPermissions = {
        media: { fs: mediaFs(scope("videos/")), allow: ["https://a.example.com/"] },
      };
      const after: AppResolvedPermissions = {
        media: { fs: mediaFs(scope("videos/"), scope("music/")), allow: ["https://b.example.com/"] },
      };
      const v = buildPermissionView(after, before, DANGEROUS);
      expect(v.rows.filter((r) => r.added).map((r) => [r.kind, r.label])).toEqual([
        ["mediaRead", "C:\\wt\\music/"],
        ["mediaNet", "https://b.example.com/"],
      ]);
      expect(v.removed.map((r) => [r.kind, r.label])).toEqual([["mediaNet", "https://a.example.com/"]]);
    });

    it("a path that resolves elsewhere is a removed + added pair (re-approval shows the move)", () => {
      const before: AppResolvedPermissions = { media: { fs: mediaFs(scope("videos/")), allow: [] } };
      const moved = { ...scope("videos/"), path: "D:\\moved\\videos/" };
      const v = buildPermissionView({ media: { fs: mediaFs(moved), allow: [] } }, before, DANGEROUS);
      expect(v.rows.map((r) => [r.kind, r.label, r.added])).toEqual([["mediaRead", "D:\\moved\\videos/", true]]);
      expect(v.removed.map((r) => r.label)).toEqual(["C:\\wt\\videos/"]);
    });
  });

  describe("subWindow", () => {
    const sw = { maxCount: 2, maxWidth: 300, maxHeight: 400, transparent: false, frameless: false, alwaysOnTop: false, position: false };

    it("shows the limits as one row and nothing else when no appearance is requested", () => {
      const v = buildPermissionView({ subWindow: sw }, null, DANGEROUS);
      expect(v.rows).toHaveLength(1);
      expect(v.rows[0]).toMatchObject({ kind: "subWindow", value: "2×300x400", severity: "normal" });
      expect(v.hasDanger).toBe(false);
    });

    it("shows an omitted size limit as unlimited", () => {
      const v = buildPermissionView({ subWindow: { ...sw, maxWidth: undefined, maxHeight: undefined } }, null, DANGEROUS);
      expect(v.rows[0]).toMatchObject({ kind: "subWindow", value: "2×∞x∞" });
    });

    it("marks transparent / frameless / always-on-top as danger and position as normal", () => {
      const v = buildPermissionView(
        { subWindow: { ...sw, transparent: true, frameless: true, alwaysOnTop: true, position: true } },
        null,
        DANGEROUS,
      );
      const flags = v.rows.filter((r) => r.kind === "subWindowFlag");
      expect(flags.map((r) => [r.value, r.severity])).toEqual([
        ["transparent", "danger"],
        ["frameless", "danger"],
        ["alwaysOnTop", "danger"],
        ["position", "normal"],
      ]);
      expect(v.hasDanger).toBe(true);
      expect(v.hasCritical).toBe(false);
    });

    it("shows a newly requested appearance as added and a dropped one as removed on re-approval", () => {
      const before = { subWindow: { ...sw, transparent: true } };
      const after = { subWindow: { ...sw, alwaysOnTop: true } };
      const v = buildPermissionView(after, before, DANGEROUS);
      expect(v.rows.filter((r) => r.added).map((r) => r.value)).toEqual(["alwaysOnTop"]);
      expect(v.removed.map((r) => r.value)).toEqual(["transparent"]);
    });

    it("shows a changed limit as a removed and an added row", () => {
      const v = buildPermissionView({ subWindow: { ...sw, maxCount: 3 } }, { subWindow: sw }, DANGEROUS);
      expect(v.rows[0]).toMatchObject({ value: "3×300x400", added: true });
      expect(v.removed.map((r) => r.value)).toEqual(["2×300x400"]);
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
