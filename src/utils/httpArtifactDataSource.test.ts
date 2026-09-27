import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createHttpArtifactViewerContext, fetchWorktreesIndex, UnauthorizedError } from "./httpArtifactDataSource";
import type { ViewerSseEvent } from "./webEvents";

/**
 * `subscribeViewerEvents` を差し替えて、実際の `EventSource` 無しに
 * `onArtifactChanged` / `onStateChanged` の配送ロジックだけを検証する(#341)。
 */
const sseHandlers = new Set<(event: ViewerSseEvent) => void>();
vi.mock("./webEvents", () => ({
  subscribeViewerEvents: vi.fn((handler: (event: ViewerSseEvent) => void) => {
    sseHandlers.add(handler);
    return () => sseHandlers.delete(handler);
  }),
}));

function emitSseEvent(event: ViewerSseEvent) {
  for (const h of [...sseHandlers]) h(event);
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const WORKTREES_INDEX = {
  worktrees: [
    {
      id: "issue-326",
      name: "issue-326",
      repositoryId: "/repo/oretachi",
      repositoryName: "oretachi",
      branchName: "worktree/issue-326",
      description: "desc",
      isHome: false,
      isRepository: false,
      artifactCount: 2,
      lastUpdatedAt: 100,
    },
  ],
  repositories: [
    {
      key: "abcd1234",
      id: "/repo/oretachi",
      name: "oretachi",
      artifactCount: 1,
      lastUpdatedAt: 50,
    },
  ],
};

describe("httpArtifactDataSource", () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    sseHandlers.clear();
  });

  describe("fetchWorktreesIndex", () => {
    it("returns the parsed index", async () => {
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      const onUnauthorized = vi.fn();
      const result = await fetchWorktreesIndex(onUnauthorized);
      expect(result).toEqual(WORKTREES_INDEX);
      expect(onUnauthorized).not.toHaveBeenCalled();
    });

    it("calls onUnauthorized and throws UnauthorizedError on 401", async () => {
      fetchMock.mockResolvedValueOnce(new Response(null, { status: 401 }));
      const onUnauthorized = vi.fn();
      await expect(fetchWorktreesIndex(onUnauthorized)).rejects.toBeInstanceOf(UnauthorizedError);
      expect(onUnauthorized).toHaveBeenCalledOnce();
    });

    it("surfaces the {error} message on other failures", async () => {
      fetchMock.mockResolvedValueOnce(jsonResponse({ error: "boom" }, 500));
      await expect(fetchWorktreesIndex(vi.fn())).rejects.toThrow("boom");
    });
  });

  describe("createHttpArtifactViewerContext (worktree scope)", () => {
    it("does not need /api/worktrees to resolve the scope", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      expect(ctx).not.toBeNull();
      expect(fetchMock).not.toHaveBeenCalled();
      expect(ctx!.dataSource.scope).toEqual({ kind: "worktree", worktreeId: "issue-326", repositoryId: "" });
      expect(ctx!.dataSource.capabilities).toEqual({
        pin: false,
        lock: false,
        delete: false,
        export: false,
        import: false,
        copyPng: false,
        transfer: false,
      });
    });

    it("maps `type` to `content_type` for list() and read()", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(
        jsonResponse([{ id: "a", title: "A", type: "text/markdown", created_at: 1, updated_at: 2 }]),
      );
      const list = await ctx!.dataSource.list();
      expect(fetchMock).toHaveBeenCalledWith("/api/worktrees/issue-326/artifacts", { credentials: "same-origin" });
      expect(list).toEqual([
        { id: "a", title: "A", type: "text/markdown", content_type: "text/markdown", created_at: 1, updated_at: 2 },
      ]);

      fetchMock.mockResolvedValueOnce(
        jsonResponse({
          artifact: { id: "a", title: "A", type: "text/markdown", content: "# hi", created_at: 1, updated_at: 2 },
          memory: { foo: "bar" },
          memoryUpdatedAt: 42,
        }),
      );
      const data = await ctx!.dataSource.read("a");
      expect(fetchMock).toHaveBeenLastCalledWith("/api/worktrees/issue-326/artifacts/a", {
        credentials: "same-origin",
      });
      expect(data.content_type).toBe("text/markdown");
      expect(data.content).toBe("# hi");

      const states = await ctx!.dataSource.listStates();
      expect(states).toEqual({ a: { memory: { foo: "bar" }, memoryUpdatedAt: 42 } });
    });

    it("percent-encodes ids in request paths", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "a/b", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(jsonResponse([]));
      await ctx!.dataSource.list();
      expect(fetchMock).toHaveBeenCalledWith("/api/worktrees/a%2Fb/artifacts", { credentials: "same-origin" });
    });

    it("setMemory POSTs to .../memory and updates the cached state", async () => {
      const onUnauthorized = vi.fn();
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized,
        navigateTo: vi.fn(),
      });

      // read() で lastMemory を仕込む
      fetchMock.mockResolvedValueOnce(
        jsonResponse({
          artifact: { id: "a", title: "A", type: "text/markdown", content: "# hi", created_at: 1, updated_at: 2 },
          memory: { foo: "old" },
          memoryUpdatedAt: 1,
        }),
      );
      await ctx!.dataSource.read("a");

      fetchMock.mockResolvedValueOnce(jsonResponse({ memoryUpdatedAt: 99 }));
      await ctx!.dataSource.setMemory("a", { foo: "bar" });
      expect(fetchMock).toHaveBeenLastCalledWith("/api/worktrees/issue-326/artifacts/a/memory", {
        credentials: "same-origin",
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ memory: { foo: "bar" } }),
      });

      const states = await ctx!.dataSource.listStates();
      expect(states).toEqual({ a: { memory: { foo: "bar" }, memoryUpdatedAt: 99 } });
    });

    it("setMemory(null) sends null and clears the cached memory", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(
        jsonResponse({
          artifact: { id: "a", title: "A", type: "text/markdown", content: "# hi", created_at: 1, updated_at: 2 },
          memory: { foo: "old" },
          memoryUpdatedAt: 1,
        }),
      );
      await ctx!.dataSource.read("a");

      fetchMock.mockResolvedValueOnce(jsonResponse({ memoryUpdatedAt: 0 }));
      await ctx!.dataSource.setMemory("a", null);
      expect(fetchMock).toHaveBeenLastCalledWith(
        "/api/worktrees/issue-326/artifacts/a/memory",
        expect.objectContaining({ body: JSON.stringify({ memory: null }) }),
      );
      const states = await ctx!.dataSource.listStates();
      expect(states).toEqual({ a: { memory: undefined, memoryUpdatedAt: 0 } });
    });

    it("callMcpTool POSTs to .../call-tool and returns the result", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(jsonResponse({ result: "tool-result" }));
      const result = await ctx!.dataSource.callMcpTool("a", "oretachi_get_worktree_status", { query: "x" });
      expect(result).toBe("tool-result");
      expect(fetchMock).toHaveBeenCalledWith("/api/worktrees/issue-326/artifacts/a/call-tool", {
        credentials: "same-origin",
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ tool: "oretachi_get_worktree_status", params: { query: "x" } }),
      });
    });

    it("callMcpTool surfaces the {error} message and calls onUnauthorized on 401", async () => {
      const onUnauthorized = vi.fn();
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized,
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(jsonResponse({ error: "ホワイトリストにないツールです" }, 400));
      await expect(ctx!.dataSource.callMcpTool("a", "not_whitelisted", {})).rejects.toThrow(
        "ホワイトリストにないツールです",
      );

      fetchMock.mockResolvedValueOnce(new Response(null, { status: 401 }));
      await expect(ctx!.dataSource.callMcpTool("a", "tool", {})).rejects.toBeInstanceOf(UnauthorizedError);
      expect(onUnauthorized).toHaveBeenCalledOnce();
    });

    it("navigates to the worktree page directly for openScopeViewer(worktree)", async () => {
      const navigateTo = vi.fn();
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo,
      });
      await ctx!.host.openScopeViewer("worktree", "other-wt", "art-1");
      expect(navigateTo).toHaveBeenCalledWith("/worktrees/other-wt/artifacts/art-1");
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it("resolves the repository key via /api/worktrees for openScopeViewer(repository)", async () => {
      const navigateTo = vi.fn();
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo,
      });
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      await ctx!.host.openScopeViewer("repository", "/repo/oretachi", "art-1");
      expect(navigateTo).toHaveBeenCalledWith("/repositories/abcd1234/artifacts/art-1");
    });

    it("throws if the repository id from a link can't be resolved", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      await expect(ctx!.host.openScopeViewer("repository", "/unknown", "art-1")).rejects.toThrow();
    });

    it("relays notifyExternalNavigate to the handler registered via host.onNavigate", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      const handler = vi.fn();
      const unlisten = await ctx!.host.onNavigate(handler);
      ctx!.notifyExternalNavigate("art-2");
      expect(handler).toHaveBeenCalledWith("art-2");
      unlisten();
      ctx!.notifyExternalNavigate("art-3");
      expect(handler).toHaveBeenCalledOnce();
    });
  });

  describe("createHttpArtifactViewerContext (repository scope)", () => {
    it("resolves the real repository id via /api/worktrees", async () => {
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      const ctx = await createHttpArtifactViewerContext("repository", "abcd1234", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      expect(ctx!.dataSource.scope).toEqual({
        kind: "repository",
        worktreeId: "",
        repositoryId: "/repo/oretachi",
      });
    });

    it("returns null for an unknown repository key", async () => {
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      const ctx = await createHttpArtifactViewerContext("repository", "does-not-exist", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      expect(ctx).toBeNull();
    });

    it("resolveScopeName reads name from the index", async () => {
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      const ctx = await createHttpArtifactViewerContext("repository", "abcd1234", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      const name = await ctx!.dataSource.resolveScopeName();
      expect(name).toEqual({ displayName: "oretachi", repositoryName: "oretachi" });
    });
  });

  describe("SSE auto-update (#341)", () => {
    it("onArtifactChanged forwards only events for this worktree scope", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      const handler = vi.fn();
      await ctx!.dataSource.onArtifactChanged(handler);

      emitSseEvent({ type: "artifact-changed", scope: "worktree", scopeId: "other-wt", artifactId: "a" });
      expect(handler).not.toHaveBeenCalled();

      emitSseEvent({ type: "artifact-changed", scope: "worktree", scopeId: "issue-326", artifactId: "a", command: "create" });
      expect(handler).toHaveBeenCalledWith({ artifactId: "a", command: "create", autoOpen: true });
    });

    it("onArtifactChanged passes autoOpen: false through (hook-driven URL auto-registration)", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      const handler = vi.fn();
      await ctx!.dataSource.onArtifactChanged(handler);

      emitSseEvent({
        type: "artifact-changed",
        scope: "worktree",
        scopeId: "issue-326",
        artifactId: "a",
        command: "create",
        autoOpen: false,
      });
      expect(handler).toHaveBeenCalledWith({ artifactId: "a", command: "create", autoOpen: false });
    });

    it("onArtifactChanged resync reloads the last read() artifact, or is a no-op if nothing was read", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      const handler = vi.fn();
      await ctx!.dataSource.onArtifactChanged(handler);

      emitSseEvent({ type: "resync" });
      expect(handler).not.toHaveBeenCalled();

      fetchMock.mockResolvedValueOnce(
        jsonResponse({
          artifact: { id: "a", title: "A", type: "text/markdown", content: "hi", created_at: 1, updated_at: 2 },
          memory: null,
          memoryUpdatedAt: 0,
        }),
      );
      await ctx!.dataSource.read("a");

      emitSseEvent({ type: "resync" });
      expect(handler).toHaveBeenCalledWith({ artifactId: "a", command: "update", autoOpen: true });
    });

    it("onArtifactChanged forwards repository-scope events matched by repoKey", async () => {
      fetchMock.mockResolvedValueOnce(jsonResponse(WORKTREES_INDEX));
      const ctx = await createHttpArtifactViewerContext("repository", "abcd1234", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      const handler = vi.fn();
      await ctx!.dataSource.onArtifactChanged(handler);

      emitSseEvent({ type: "artifact-changed", scope: "repository", scopeId: "/repo/oretachi", repoKey: "other-key", artifactId: "a" });
      expect(handler).not.toHaveBeenCalled();

      emitSseEvent({ type: "artifact-changed", scope: "repository", scopeId: "/repo/oretachi", repoKey: "abcd1234", artifactId: "a", command: "delete" });
      expect(handler).toHaveBeenCalledWith({ artifactId: "a", command: "delete", autoOpen: true });
    });

    it("unsubscribing onArtifactChanged stops delivery", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      const handler = vi.fn();
      const unsubscribe = await ctx!.dataSource.onArtifactChanged(handler);
      unsubscribe();

      emitSseEvent({ type: "artifact-changed", scope: "worktree", scopeId: "issue-326", artifactId: "a" });
      expect(handler).not.toHaveBeenCalled();
    });

    it("onStateChanged re-fetches memory and only reacts to the currently read artifact", async () => {
      const ctx = await createHttpArtifactViewerContext("worktree", "issue-326", {
        onUnauthorized: vi.fn(),
        navigateTo: vi.fn(),
      });
      fetchMock.mockResolvedValueOnce(
        jsonResponse({
          artifact: { id: "a", title: "A", type: "text/markdown", content: "hi", created_at: 1, updated_at: 2 },
          memory: { foo: "old" },
          memoryUpdatedAt: 1,
        }),
      );
      await ctx!.dataSource.read("a");

      const handler = vi.fn();
      await ctx!.dataSource.onStateChanged(handler);

      // 別のアーティファクト宛の通知は listStates() に反映されないので無視する
      emitSseEvent({ type: "state-changed", scope: "worktree", scopeId: "issue-326", artifactId: "b" });
      expect(handler).not.toHaveBeenCalled();
      expect(fetchMock).toHaveBeenCalledTimes(1);

      fetchMock.mockResolvedValueOnce(
        jsonResponse({
          artifact: { id: "a" },
          memory: { foo: "new" },
          memoryUpdatedAt: 2,
        }),
      );
      emitSseEvent({ type: "state-changed", scope: "worktree", scopeId: "issue-326", artifactId: "a" });
      await vi.waitFor(() => expect(handler).toHaveBeenCalledWith({ artifactId: "a" }));

      const states = await ctx!.dataSource.listStates();
      expect(states).toEqual({ a: { memory: { foo: "new" }, memoryUpdatedAt: 2 } });
    });
  });
});
