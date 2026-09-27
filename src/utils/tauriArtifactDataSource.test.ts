import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined);
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...(args as [string, unknown?])),
}));

type ListenHandler = (event: { payload: unknown }) => void;
const listenMock = vi.fn();
const listenHandlers = new Map<string, ListenHandler[]>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: ListenHandler) => {
    listenMock(event);
    const list = listenHandlers.get(event) ?? [];
    list.push(handler);
    listenHandlers.set(event, list);
    return Promise.resolve(() => {
      const remaining = (listenHandlers.get(event) ?? []).filter((h) => h !== handler);
      listenHandlers.set(event, remaining);
    });
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTitle: vi.fn() }),
}));

const askMock = vi.fn();
const messageMock = vi.fn();
const openFileDialogMock = vi.fn();
const saveFileDialogMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (...args: unknown[]) => askMock(...args),
  message: (...args: unknown[]) => messageMock(...args),
  open: (...args: unknown[]) => openFileDialogMock(...args),
  save: (...args: unknown[]) => saveFileDialogMock(...args),
}));

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeImage: vi.fn(),
}));

const openUrlMock = vi.fn();
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...args: unknown[]) => openUrlMock(...args),
}));

const openArtifactViewerMock = vi.fn();
const openRepositoryArtifactViewerMock = vi.fn();
vi.mock("../composables/useArtifactWindow", () => ({
  ARTIFACT_NAVIGATE_EVENT: "artifact-navigate",
  useArtifactWindow: () => ({
    openArtifactViewer: openArtifactViewerMock,
    openRepositoryArtifactViewer: openRepositoryArtifactViewerMock,
  }),
}));

function emit(event: string, payload: unknown) {
  for (const handler of listenHandlers.get(event) ?? []) {
    handler({ payload });
  }
}

import { createTauriArtifactViewerContext } from "./tauriArtifactDataSource";

describe("createTauriArtifactViewerContext", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    invokeMock.mockImplementation(async () => undefined);
    listenMock.mockClear();
    listenHandlers.clear();
  });

  it("scope 未指定は worktree スコープとして扱う（旧 URL 互換）", () => {
    const { dataSource } = createTauriArtifactViewerContext("?worktreeId=wt1");
    expect(dataSource.scope).toEqual({ kind: "worktree", worktreeId: "wt1", repositoryId: "" });
  });

  it("worktree スコープの list は list_artifacts を worktreeId で呼び、type→content_type を変換する", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_artifacts") return [{ id: "a1", title: "A", type: "text/markdown", created_at: 1, updated_at: 2 }];
      return undefined;
    });
    const { dataSource } = createTauriArtifactViewerContext("?scope=worktree&worktreeId=wt1");

    const list = await dataSource.list();

    expect(invokeMock).toHaveBeenCalledWith("list_artifacts", { worktreeId: "wt1" });
    expect(list).toEqual([
      { id: "a1", title: "A", type: "text/markdown", content_type: "text/markdown", created_at: 1, updated_at: 2 },
    ]);
  });

  it("repository スコープの list は list_repo_artifacts を repositoryId で呼ぶ", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_repo_artifacts") return [];
      return undefined;
    });
    const { dataSource } = createTauriArtifactViewerContext("?scope=repository&repositoryId=repo1");

    await dataSource.list();

    expect(invokeMock).toHaveBeenCalledWith("list_repo_artifacts", { repositoryId: "repo1" });
  });

  it("read は JSON をパースして type→content_type を変換する", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "read_artifact") {
        return JSON.stringify({ id: "a1", title: "A", type: "text/markdown", content: "hi", created_at: 1, updated_at: 2 });
      }
      return undefined;
    });
    const { dataSource } = createTauriArtifactViewerContext("?worktreeId=wt1");

    const data = await dataSource.read("a1");

    expect(invokeMock).toHaveBeenCalledWith("read_artifact", { worktreeId: "wt1", artifactId: "a1" });
    expect(data.content_type).toBe("text/markdown");
    expect(data.content).toBe("hi");
  });

  it("deleteArtifact は worktree/repository でコマンドと引数を切り替える", async () => {
    const worktree = createTauriArtifactViewerContext("?scope=worktree&worktreeId=wt1");
    await worktree.dataSource.deleteArtifact?.("a1");
    expect(invokeMock).toHaveBeenCalledWith("delete_artifact", { worktreeId: "wt1", artifactId: "a1" });

    const repo = createTauriArtifactViewerContext("?scope=repository&repositoryId=repo1");
    await repo.dataSource.deleteArtifact?.("a2");
    expect(invokeMock).toHaveBeenCalledWith("delete_repo_artifact", { repositoryId: "repo1", artifactId: "a2" });
  });

  it("capabilities は全て true", () => {
    const { dataSource } = createTauriArtifactViewerContext("?worktreeId=wt1");
    expect(dataSource.capabilities).toEqual({
      pin: true,
      lock: true,
      delete: true,
      export: true,
      import: true,
      copyPng: true,
      transfer: true,
    });
  });

  it("onArtifactChanged(worktree) は自スコープのイベントだけ通し、autoOpen の既定は true", async () => {
    const { dataSource } = createTauriArtifactViewerContext("?scope=worktree&worktreeId=wt1");
    const handler = vi.fn();
    await dataSource.onArtifactChanged(handler);

    emit("artifact-changed", { worktreeId: "other", artifactId: "x", command: "create" });
    expect(handler).not.toHaveBeenCalled();

    emit("artifact-changed", { worktreeId: "wt1", artifactId: "a1", command: "create" });
    expect(handler).toHaveBeenCalledWith({ artifactId: "a1", command: "create", autoOpen: true });

    emit("artifact-changed", { worktreeId: "wt1", artifactId: "a2", command: "create", autoOpen: false });
    expect(handler).toHaveBeenCalledWith({ artifactId: "a2", command: "create", autoOpen: false });
  });

  it("onArtifactChanged(repository) は repo-artifact-changed を購読し、自スコープのイベントだけ通す", async () => {
    const { dataSource } = createTauriArtifactViewerContext("?scope=repository&repositoryId=repo1");
    const handler = vi.fn();
    await dataSource.onArtifactChanged(handler);

    emit("repo-artifact-changed", { repositoryId: "other", artifactId: "x", command: "create" });
    expect(handler).not.toHaveBeenCalled();

    emit("repo-artifact-changed", { repositoryId: "repo1", artifactId: "a1", command: "delete" });
    expect(handler).toHaveBeenCalledWith({ artifactId: "a1", command: "delete", autoOpen: true });
  });

  it("onStateChanged は scope/scopeId が一致するイベントだけ通す", async () => {
    const { dataSource } = createTauriArtifactViewerContext("?scope=worktree&worktreeId=wt1");
    const handler = vi.fn();
    await dataSource.onStateChanged(handler);

    emit("artifact-state-changed", { scope: "worktree", scopeId: "other", artifactId: "a1" });
    expect(handler).not.toHaveBeenCalled();

    emit("artifact-state-changed", { scope: "worktree", scopeId: "wt1", artifactId: "a1" });
    expect(handler).toHaveBeenCalledWith({ artifactId: "a1" });
  });

  it("host.openScopeViewer は scope に応じて openArtifactViewer / openRepositoryArtifactViewer を呼ぶ", async () => {
    const { host } = createTauriArtifactViewerContext("?worktreeId=wt1");

    await host.openScopeViewer("worktree", "wt2", "a1");
    expect(openArtifactViewerMock).toHaveBeenCalledWith("wt2", "a1");

    await host.openScopeViewer("repository", "repo2", "a2");
    expect(openRepositoryArtifactViewerMock).toHaveBeenCalledWith("repo2", "a2");
  });
});
