import { describe, it, expect } from "vitest";
import {
  summarizeGrant,
  appRowBadges,
  appRowName,
  canStart,
  canStop,
  canOpenWindow,
} from "./appGrantSummary";
import type { AppInstanceListItem, AppResolvedPermissions } from "../types/appInstance";

const item = (over: Partial<AppInstanceListItem> = {}): AppInstanceListItem => ({
  worktreeId: "wt-1",
  worktreeName: "feature-x",
  artifactId: "art-1",
  artifactTitle: "Deploy Console",
  appName: "deploy",
  granted: true,
  grantedAt: 1700000000,
  resolved: {},
  state: null,
  running: false,
  reapprovalPending: false,
  ...over,
});

describe("summarizeGrant", () => {
  it("returns no parts for an empty permission set", () => {
    expect(summarizeGrant({})).toEqual([]);
  });

  it("counts tools and flags only critical factors", () => {
    const perm: AppResolvedPermissions = {
      mcp: {
        tools: ["notify_worktree", "oretachi_add_task"],
        targets: [{ raw: "*", stored: "*", label: "*", wildcard: true }],
      },
      memory: { maxBytes: 65536 },
    };
    const parts = summarizeGrant(perm);
    expect(parts.map((p) => p.key)).toEqual(["summaryTools", "summaryWildcard", "summaryMemory"]);
    expect(parts[0].params).toEqual({ count: 2 });
    expect(parts.map((p) => p.critical)).toEqual([false, true, false]);
    expect(parts[2].params).toEqual({ bytes: "65536" });
  });

  it("says 'other worktrees' for named targets", () => {
    const parts = summarizeGrant({
      mcp: { tools: [], targets: [{ raw: "a", stored: "wt-a", label: "a", wildcard: false }] },
    });
    expect(parts.map((p) => p.key)).toEqual(["summaryTargets"]);
  });

  it("splits fs by access and marks the AI terminal critical", () => {
    const parts = summarizeGrant({
      fs: {
        scopes: [
          { scheme: "worktree", prefix: "", path: "C:\\w\\", write: false },
          { scheme: "artifact", prefix: "out/", path: "C:\\a\\out\\", write: true },
        ],
        allowSensitivePaths: false,
      },
      aiTerminal: { agents: ["claude"], maxConcurrent: 2 },
    });
    expect(parts.find((p) => p.key === "summaryFs")?.params).toEqual({ read: 1, write: 1 });
    expect(parts.find((p) => p.key === "summaryAiTerminal")?.critical).toBe(true);
  });
});

describe("row state", () => {
  it("shows stopped + ungranted for an app that was never approved", () => {
    expect(appRowBadges(item({ granted: false, grantedAt: null, resolved: null }))).toEqual(["stopped", "ungranted"]);
  });

  it("shows starting while the approval dialog is pending", () => {
    expect(appRowBadges(item({ state: "starting" }))).toEqual(["starting"]);
  });

  it("adds reapproval to a running app with pending permission changes", () => {
    expect(appRowBadges(item({ state: "running", running: true, reapprovalPending: true }))).toEqual([
      "running",
      "reapproval",
    ]);
  });

  it("falls back from title to app name to id", () => {
    expect(appRowName(item())).toBe("Deploy Console");
    expect(appRowName(item({ artifactTitle: "" }))).toBe("deploy");
    expect(appRowName(item({ artifactTitle: "", appName: "" }))).toBe("art-1");
  });
});

describe("row actions", () => {
  it("allows start only when stopped and the artifact still exists", () => {
    expect(canStart(item())).toBe(true);
    expect(canStart(item({ state: "running", running: true }))).toBe(false);
    expect(canStart(item({ appName: "" }))).toBe(false);
  });

  it("allows stop while starting or running", () => {
    expect(canStop(item())).toBe(false);
    expect(canStop(item({ state: "starting" }))).toBe(true);
    expect(canStop(item({ state: "running", running: true }))).toBe(true);
  });

  it("allows opening the window only for existing artifacts", () => {
    expect(canOpenWindow(item())).toBe(true);
    expect(canOpenWindow(item({ appName: "" }))).toBe(false);
  });
});
