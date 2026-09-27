import type { ArtifactState } from "../types/artifact";
import { mapArtifactData, mapArtifactMeta } from "./artifactPayload";
import type {
  ArtifactCapabilities,
  ArtifactDataSource,
  ArtifactScopeKind,
  ArtifactScopeRef,
  ArtifactViewerHost,
} from "./artifactDataSource";
import { buildWebPath } from "./webRoute";
import { subscribeViewerEvents, type ViewerSseEvent } from "./webEvents";

/** `/api/*` が Cookie 無し・失効で 401 を返したときに送出する */
export class UnauthorizedError extends Error {
  constructor() {
    super("unauthorized");
    this.name = "UnauthorizedError";
  }
}

export interface WorktreeIndexEntry {
  id: string;
  name: string;
  repositoryId: string;
  repositoryName: string;
  branchName: string;
  description: string | null;
  isHome: boolean;
  isRepository: boolean;
  artifactCount: number;
  lastUpdatedAt: number;
}

export interface RepositoryIndexEntry {
  key: string;
  id: string;
  name: string;
  artifactCount: number;
  lastUpdatedAt: number;
}

export interface WorktreesIndex {
  worktrees: WorktreeIndexEntry[];
  repositories: RepositoryIndexEntry[];
}

const WEB_CAPABILITIES: ArtifactCapabilities = {
  pin: false,
  lock: false,
  delete: false,
  export: false,
  import: false,
  copyPng: false,
  transfer: false,
};

async function apiFetch<T>(path: string, onUnauthorized: () => void): Promise<T> {
  const res = await fetch(path, { credentials: "same-origin" });
  if (res.status === 401) {
    onUnauthorized();
    throw new UnauthorizedError();
  }
  if (!res.ok) {
    let msg = `HTTP ${res.status}`;
    try {
      const body = await res.json();
      if (typeof body?.error === "string") msg = body.error;
    } catch {
      // 本文が JSON でなければ status だけを使う
    }
    throw new Error(msg);
  }
  return (await res.json()) as T;
}

/** `GET /api/worktrees`。ワークツリー一覧ページと、スコープ名・リポジトリ key 解決の両方で使う */
export function fetchWorktreesIndex(onUnauthorized: () => void): Promise<WorktreesIndex> {
  return apiFetch<WorktreesIndex>("/api/worktrees", onUnauthorized);
}

function artifactListPath(kind: ArtifactScopeKind, scopeKey: string): string {
  const root = kind === "worktree" ? "worktrees" : "repositories";
  return `/api/${root}/${encodeURIComponent(scopeKey)}/artifacts`;
}

/** アーティファクト一覧ページ向け: ビューア(データソース)を組み立てずに一覧だけ取る */
export async function fetchArtifactList(
  kind: ArtifactScopeKind,
  scopeKey: string,
  onUnauthorized: () => void,
) {
  const raw = await apiFetch<unknown[]>(artifactListPath(kind, scopeKey), onUnauthorized);
  return raw.map(mapArtifactMeta);
}

interface ScopeResolution {
  scope: ArtifactScopeRef;
  listPath: string;
  readPathFor: (artifactId: string) => string;
}

async function resolveScope(
  kind: ArtifactScopeKind,
  scopeKey: string,
  onUnauthorized: () => void,
): Promise<ScopeResolution | null> {
  const listPath = artifactListPath(kind, scopeKey);
  if (kind === "worktree") {
    return {
      scope: { kind, worktreeId: scopeKey, repositoryId: "" },
      listPath,
      readPathFor: (id) => `${listPath}/${encodeURIComponent(id)}`,
    };
  }
  // repository は URL に `repo_artifacts_key`(ハッシュ)しか載らないが、`artifact:` リンクの
  // 同一スコープ判定（ArtifactViewerApp.vue の scope.repositoryId 比較）は実 ID（絶対パス）で
  // 行われるため、一覧 API を叩いて実 ID を引いておく。
  const index = await fetchWorktreesIndex(onUnauthorized);
  const repo = index.repositories.find((r) => r.key === scopeKey);
  if (!repo) return null;
  return {
    scope: { kind, worktreeId: "", repositoryId: repo.id },
    listPath,
    readPathFor: (id) => `${listPath}/${encodeURIComponent(id)}`,
  };
}

export interface HttpArtifactViewerContext {
  dataSource: ArtifactDataSource;
  host: ArtifactViewerHost;
  /**
   * ブラウザの戻る/進むで同一スコープ内の artifactId が変わったときに呼ぶ。
   * `host.onNavigate` で登録されたハンドラ（ArtifactViewerApp.vue 内部）へ中継する。
   */
  notifyExternalNavigate: (artifactId: string) => void;
}

/**
 * ArtifactViewerApp.vue 向けの HTTP 実装を組み立てる。
 * 書き込み系（setMemory / callMcpTool）は未対応（#340 の範囲）。
 * 自動更新は `/api/events` (SSE) 経由で対応済み（#341）。
 * スコープ（repository の key）が解決できない場合は null を返す。
 */
export async function createHttpArtifactViewerContext(
  kind: ArtifactScopeKind,
  scopeKey: string,
  opts: { onUnauthorized: () => void; navigateTo: (path: string) => void },
): Promise<HttpArtifactViewerContext | null> {
  const resolution = await resolveScope(kind, scopeKey, opts.onUnauthorized);
  if (!resolution) return null;

  const { scope, listPath, readPathFor } = resolution;
  /** 直近に read() したアーティファクトの memory だけを保持する（Web 版に一括取得 API は無い） */
  let lastMemory: { artifactId: string; state: ArtifactState } | null = null;
  let navigateHandler: ((artifactId: string) => void) | null = null;

  /** イベントが自スコープ宛かどうか。repository は URL の key（ハッシュ）で照合する */
  function matchesScope(event: ViewerSseEvent): boolean {
    if (kind === "worktree") {
      return event.scope === "worktree" && event.scopeId === scopeKey;
    }
    return event.scope === "repository" && event.repoKey === scopeKey;
  }

  /** memory だけを読み直して `lastMemory` を更新する（本体は再取得しない） */
  async function refetchMemory(artifactId: string): Promise<ArtifactState> {
    const raw = await apiFetch<{
      memory: Record<string, unknown> | null;
      memoryUpdatedAt: number;
    }>(readPathFor(artifactId), opts.onUnauthorized);
    const state: ArtifactState = { memory: raw.memory ?? undefined, memoryUpdatedAt: raw.memoryUpdatedAt };
    lastMemory = { artifactId, state };
    return state;
  }

  const dataSource: ArtifactDataSource = {
    scope,
    capabilities: WEB_CAPABILITIES,

    async list() {
      const raw = await apiFetch<unknown[]>(listPath, opts.onUnauthorized);
      return raw.map(mapArtifactMeta);
    },

    async read(artifactId) {
      const raw = await apiFetch<{
        artifact: unknown;
        memory: Record<string, unknown> | null;
        memoryUpdatedAt: number;
      }>(readPathFor(artifactId), opts.onUnauthorized);
      lastMemory = {
        artifactId,
        state: { memory: raw.memory ?? undefined, memoryUpdatedAt: raw.memoryUpdatedAt },
      };
      return mapArtifactData(raw.artifact);
    },

    async listStates() {
      if (!lastMemory) return {};
      return { [lastMemory.artifactId]: lastMemory.state };
    },

    async resolveScopeName() {
      const index = await fetchWorktreesIndex(opts.onUnauthorized);
      if (kind === "worktree") {
        const w = index.worktrees.find((e) => e.id === scopeKey);
        return { displayName: w?.name ?? scopeKey, repositoryName: w?.repositoryName ?? null };
      }
      const r = index.repositories.find((e) => e.key === scopeKey);
      return { displayName: r?.name ?? scopeKey, repositoryName: r?.name ?? null };
    },

    setMemory() {
      return Promise.reject(new Error("Web 版ではメモリーの書き込みに未対応です"));
    },

    callMcpTool() {
      return Promise.reject(new Error("Web 版では MCP ツールの呼び出しに未対応です"));
    },

    async onArtifactChanged(handler) {
      return subscribeViewerEvents((event) => {
        if (event.type === "resync") {
          // 個々の差分は追えないので、選択中のもの(直近 read() した ID)を
          // 一覧・本体ごと読み直させる。何も読んでいなければ何もしない。
          if (!lastMemory) return;
          handler({ artifactId: lastMemory.artifactId, command: "update", autoOpen: true });
          return;
        }
        if (event.type !== "artifact-changed" || !event.artifactId) return;
        if (!matchesScope(event)) return;
        handler({ artifactId: event.artifactId, command: event.command ?? "update", autoOpen: true });
      });
    },

    async onStateChanged(handler) {
      return subscribeViewerEvents((event) => {
        // `listStates()` は直近 read() した1件しか返さないので、それ以外の
        // アーティファクト宛の通知は(表示に反映されないため)無視してよい。
        const targetId = event.type === "resync" ? lastMemory?.artifactId : event.artifactId;
        if (!targetId || targetId !== lastMemory?.artifactId) return;
        if (event.type !== "resync" && (event.type !== "state-changed" || !matchesScope(event))) return;
        void refetchMemory(targetId).then(() => handler({ artifactId: targetId }));
      });
    },
  };

  const host: ArtifactViewerHost = {
    async confirm(msg) {
      return window.confirm(msg);
    },

    async showError(msg) {
      window.alert(msg);
    },

    async pickSavePath() {
      return null;
    },

    async pickZipToOpen() {
      return null;
    },

    writeImage() {
      return Promise.reject(new Error("Web 版では画像コピーに未対応です"));
    },

    async setWindowTitle(title) {
      document.title = title;
    },

    async openExternalUrl(url) {
      window.open(url, "_blank", "noopener,noreferrer");
    },

    async openScopeViewer(targetScope, id, artifactId) {
      if (targetScope === "worktree") {
        opts.navigateTo(buildWebPath({ page: "view", scope: "worktree", scopeKey: id, artifactId }));
        return;
      }
      const index = await fetchWorktreesIndex(opts.onUnauthorized);
      const repo = index.repositories.find((e) => e.id === id);
      if (!repo) {
        throw new Error("リポジトリが見つかりません");
      }
      opts.navigateTo(buildWebPath({ page: "view", scope: "repository", scopeKey: repo.key, artifactId }));
    },

    async onNavigate(handler) {
      navigateHandler = handler;
      return () => {
        if (navigateHandler === handler) navigateHandler = null;
      };
    },
  };

  return {
    dataSource,
    host,
    notifyExternalNavigate: (artifactId) => navigateHandler?.(artifactId),
  };
}
