import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, message, open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { writeImage as clipboardWriteImage } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useArtifactWindow, ARTIFACT_NAVIGATE_EVENT, type ArtifactNavigateEvent } from "../composables/useArtifactWindow";
import type {
  ArtifactMeta,
  ArtifactData,
  ArtifactState,
  ArtifactChangedEvent,
  ArtifactStateChangedEvent,
  RepoArtifactChangedEvent,
  CopyArtifactResult,
} from "../types/artifact";
import type {
  ArtifactCapabilities,
  ArtifactDataSource,
  ArtifactScopeKind,
  ArtifactScopeRef,
  ArtifactViewerHost,
} from "./artifactDataSource";

// JSONの "type" フィールドを content_type にマッピングする
// (Rust側は serde(rename="type") でJSONに保存するため)
function mapMeta(raw: any): ArtifactMeta {
  return { ...raw, content_type: raw.type ?? raw.content_type };
}
function mapArtifact(raw: any): ArtifactData {
  return { ...raw, content_type: raw.type ?? raw.content_type };
}

const TAURI_CAPABILITIES: ArtifactCapabilities = {
  pin: true,
  lock: true,
  delete: true,
  export: true,
  import: true,
  copyPng: true,
  transfer: true,
};

/**
 * ArtifactViewerApp.vue が使う Tauri 実装を組み立てる。
 * URL の解釈（scope 未指定は worktree スコープとして扱う互換動作を含む）は
 * 従来の ArtifactViewerApp.vue にあったものをそのまま移した。
 */
export function createTauriArtifactViewerContext(search: string): {
  dataSource: ArtifactDataSource;
  host: ArtifactViewerHost;
} {
  const params = new URLSearchParams(search);
  const kind: ArtifactScopeKind = params.get("scope") === "repository" ? "repository" : "worktree";
  const worktreeId = params.get("worktreeId") ?? "";
  const repositoryId = params.get("repositoryId") ?? "";
  const isRepositoryScope = kind === "repository";
  const scopeId = isRepositoryScope ? repositoryId : worktreeId;

  const scope: ArtifactScopeRef = { kind, worktreeId, repositoryId };

  // スコープごとの Tauri コマンド差分を吸収する薄いラッパ
  function invokeList(): Promise<any[]> {
    return isRepositoryScope
      ? invoke<any[]>("list_repo_artifacts", { repositoryId })
      : invoke<any[]>("list_artifacts", { worktreeId });
  }

  function invokeRead(artifactId: string): Promise<string> {
    return isRepositoryScope
      ? invoke<string>("read_repo_artifact", { repositoryId, artifactId })
      : invoke<string>("read_artifact", { worktreeId, artifactId });
  }

  const dataSource: ArtifactDataSource = {
    scope,
    capabilities: TAURI_CAPABILITIES,

    async list() {
      const list = await invokeList();
      return list.map(mapMeta);
    },

    async read(artifactId) {
      const raw = await invokeRead(artifactId);
      return mapArtifact(JSON.parse(raw));
    },

    listStates() {
      return invoke<Record<string, ArtifactState>>("list_artifact_states", { scope: kind, scopeId });
    },

    resolveScopeName() {
      return invoke<{ displayName: string; repositoryName: string | null }>("resolve_artifact_scope", {
        scope: kind,
        id: scopeId,
      });
    },

    setMemory(artifactId, memory) {
      return invoke("set_artifact_memory", { scope: kind, scopeId, artifactId, memory });
    },

    callMcpTool(artifactId, tool, params) {
      return invoke<string>("artifact_call_mcp_tool", { scope: kind, scopeId, artifactId, tool, params });
    },

    setPinned(artifactId, pinned) {
      return invoke("set_artifact_pinned", { scope: kind, scopeId, artifactId, pinned });
    },

    lockHeartbeatIntervalMs() {
      return invoke<number>("artifact_lock_heartbeat_interval");
    },

    touchLock(artifactId) {
      return invoke("artifact_lock_touch", { scope: kind, scopeId, artifactId });
    },

    releaseLock() {
      return invoke("artifact_lock_release");
    },

    deleteArtifact(artifactId) {
      return isRepositoryScope
        ? invoke("delete_repo_artifact", { repositoryId, artifactId })
        : invoke("delete_artifact", { worktreeId, artifactId });
    },

    exportArtifact(artifactId, destPath, viewHtml) {
      return invoke<{ path: string; bytes: number }>("export_artifact", {
        scope: kind,
        scopeId,
        artifactId,
        destPath,
        viewHtml,
      });
    },

    importArtifact(zipPath) {
      return invoke<{ artifactId: string; title: string; renamedFrom: string | null }>("import_artifact", {
        scope: kind,
        scopeId,
        zipPath,
      });
    },

    copyToRepository(artifactId, overwrite) {
      return invoke<CopyArtifactResult>("copy_artifact_to_repository", { worktreeId, artifactId, overwrite });
    },

    async onArtifactChanged(handler) {
      if (isRepositoryScope) {
        const unlisten = await listen<RepoArtifactChangedEvent>("repo-artifact-changed", (event) => {
          if (event.payload.repositoryId !== repositoryId) return;
          handler({ artifactId: event.payload.artifactId, command: event.payload.command, autoOpen: true });
        });
        return unlisten;
      }
      const unlisten = await listen<ArtifactChangedEvent>("artifact-changed", (event) => {
        if (event.payload.worktreeId !== worktreeId) return;
        handler({
          artifactId: event.payload.artifactId,
          command: event.payload.command,
          autoOpen: event.payload.autoOpen !== false,
        });
      });
      return unlisten;
    },

    async onStateChanged(handler) {
      const unlisten = await listen<ArtifactStateChangedEvent>("artifact-state-changed", (event) => {
        if (event.payload.scope !== kind || event.payload.scopeId !== scopeId) return;
        handler({ artifactId: event.payload.artifactId });
      });
      return unlisten;
    },
  };

  const { openArtifactViewer, openRepositoryArtifactViewer } = useArtifactWindow();

  const host: ArtifactViewerHost = {
    confirm(msg, opts) {
      return ask(msg, opts);
    },

    async showError(msg, title) {
      await message(msg, { title, kind: "error" });
    },

    async pickSavePath(defaultName) {
      const destPath = await saveFileDialog({
        defaultPath: defaultName,
        filters: [{ name: "zip", extensions: ["zip"] }],
      });
      return destPath ?? null;
    },

    async pickZipToOpen() {
      const zipPath = await openFileDialog({
        multiple: false,
        filters: [{ name: "zip", extensions: ["zip"] }],
      });
      return typeof zipPath === "string" ? zipPath : null;
    },

    writeImage(bytes) {
      return clipboardWriteImage(bytes);
    },

    async setWindowTitle(title) {
      await getCurrentWindow().setTitle(title);
    },

    openExternalUrl(url) {
      return openUrl(url);
    },

    openScopeViewer(targetScope, id, artifactId) {
      return targetScope === "worktree"
        ? openArtifactViewer(id, artifactId)
        : openRepositoryArtifactViewer(id, artifactId);
    },

    async onNavigate(handler) {
      const unlisten = await listen<ArtifactNavigateEvent>(ARTIFACT_NAVIGATE_EVENT, (event) => {
        handler(event.payload.artifactId);
      });
      return unlisten;
    },
  };

  return { dataSource, host };
}
