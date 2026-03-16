import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { platform } from "@tauri-apps/plugin-os";
import type { Ref } from "vue";
import type TerminalView from "../components/TerminalView.vue";
import type { WorktreeEntry, Repository, AppSettings } from "../types/settings";
import type { Worktree } from "../types/worktree";
import type { AddWorktreeTaskCode, AgentWorktreeTaskCode } from "../types/task";

export function useTaskExecution(deps: {
  t: (key: string) => string;
  settings: Ref<AppSettings>;
  worktrees: Ref<Worktree[]>;
  addWorktreePlaceholder: (entry: WorktreeEntry) => void;
  invokeWorktreeAdd: (entry: WorktreeEntry) => Promise<boolean>;
  commitWorktree: (entry: WorktreeEntry) => void;
  rollbackWorktree: (worktreeId: string) => void;
  isDetached: (worktreeId: string) => boolean;
  getDetachedSessionId: (terminalId: number) => number | null;
  tryAutoAssignHotkey: (worktreeId: string) => void;
  terminalAgentStatus: Map<number, boolean>;
  getTerminalRef: (terminalId: number) => InstanceType<typeof TerminalView> | undefined;
  onAddTerminal: (worktreeId: string) => Promise<void>;
  onMoveToSubWindow: (worktreeId: string) => Promise<void>;
  loadingWorktrees: Map<string, string>;
  pendingScripts: Map<string, string>;
  autoApprovalMap: Map<string, boolean>;
}) {
  const {
    t,
    settings,
    worktrees,
    addWorktreePlaceholder,
    invokeWorktreeAdd,
    commitWorktree,
    rollbackWorktree,
    isDetached,
    getDetachedSessionId,
    tryAutoAssignHotkey,
    terminalAgentStatus,
    getTerminalRef,
    onAddTerminal,
    onMoveToSubWindow,
    loadingWorktrees,
    pendingScripts,
    autoApprovalMap,
  } = deps;

  function randomSuffix(): string {
    return Math.random().toString(36).slice(2, 6);
  }

  function resolveShell(_worktreeId: string): string | undefined {
    return settings.value.terminal.shell || undefined;
  }

  function buildScriptCommand(repo: Repository, entry: WorktreeEntry): string {
    const scriptPath = repo.execScript!;
    const shell = resolveShell(entry.id);
    const repoName = repo.name;
    const wtName = entry.name;
    const shellLower = (shell ?? "").toLowerCase();

    const isWindows = platform() === "windows";
    const isPowerShell = shellLower.includes("powershell") || shellLower.includes("pwsh") || (shell === undefined && isWindows);
    const isCmd = !isPowerShell && shellLower.includes("cmd");

    if (isCmd) {
      return `set ORETACHI_REPO_NAME=${repoName}&& set ORETACHI_WORKTREE_NAME=${wtName}&& call "${scriptPath}"\r`;
    } else if (isPowerShell) {
      return `$env:ORETACHI_REPO_NAME="${repoName}"; $env:ORETACHI_WORKTREE_NAME="${wtName}"; Set-ExecutionPolicy -Scope Process Bypass; & "${scriptPath}"\r`;
    } else {
      return `ORETACHI_REPO_NAME="${repoName}" ORETACHI_WORKTREE_NAME="${wtName}" sh "${scriptPath}"\r`;
    }
  }

  async function waitForSessionReady(worktreeId: string): Promise<number | null> {
    for (let i = 0; i < 100; i++) {
      const wt = worktrees.value.find((w) => w.id === worktreeId);
      const terminal = wt?.terminals[0];
      if (terminal) {
        const ref = getTerminalRef(terminal.id);
        const s = ref?.sessionId;
        if (s !== null && s !== undefined) return s;
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  }

  async function waitForScriptCompletion(worktreeId: string, timeoutMs = 300_000): Promise<void> {
    const sid = await waitForSessionReady(worktreeId);
    if (sid === null) return;

    const targetSid = sid;
    await new Promise<void>((resolve) => {
      let timer: ReturnType<typeof setTimeout>;
      let unlistenFn: (() => void) | null = null;

      timer = setTimeout(() => {
        unlistenFn?.();
        resolve();
      }, timeoutMs);

      listen<{ sessionId: number; data: number[] }>("pty-output", (event) => {
        if (event.payload.sessionId !== targetSid) return;
        const text = new TextDecoder().decode(new Uint8Array(event.payload.data));
        if (/\x1b\]777;exit_code;\d+/.test(text)) {
          clearTimeout(timer);
          unlistenFn?.();
          resolve();
        }
      }).then((fn) => {
        unlistenFn = fn;
      });
    });
  }

  async function sendPromptToRunningAgent(
    sessionId: number,
    worktreeId: string,
    terminalId: number,
    prompt: string,
  ): Promise<void> {
    // ブラケット付きペーストモードで囲み、改行を含むテキストを一括入力として送信
    const data = `\x1b[200~${prompt}\x1b[201~\r`;
    if (isDetached(worktreeId)) {
      const bytes = Array.from(new TextEncoder().encode(data));
      await invoke("pty_write", { sessionId, data: bytes });
    } else {
      const termRef = getTerminalRef(terminalId);
      if (termRef) {
        await termRef.write(data);
      } else {
        const bytes = Array.from(new TextEncoder().encode(data));
        await invoke("pty_write", { sessionId, data: bytes });
      }
    }
  }

  async function executeAddWorktree(code: AddWorktreeTaskCode): Promise<string> {
    const repo = settings.value.repositories.find((r) => r.name === code.repository);
    if (!repo) throw new Error(`リポジトリが見つかりません: ${code.repository}`);

    const suffix = randomSuffix();
    const worktreeName = `${repo.name}-${suffix}`;
    const entry: WorktreeEntry = {
      id: `${Date.now()}-${suffix}`,
      name: worktreeName,
      repositoryId: repo.id,
      repositoryName: repo.name,
      path: `${settings.value.worktreeBaseDir}/${worktreeName}`,
      branchName: code.branch,
    };

    addWorktreePlaceholder(entry);
    loadingWorktrees.set(entry.id, t("creatingText"));

    try {
      await invokeWorktreeAdd(entry);
      commitWorktree(entry);
      tryAutoAssignHotkey(entry.id);

      // デフォルト: 自動承認
      if (settings.value.worktreeDefaults?.autoApproval) {
        autoApprovalMap.set(entry.id, true);
        const wtEntry = settings.value.worktrees.find((w) => w.id === entry.id);
        if (wtEntry) wtEntry.autoApproval = true;
      }

      if (repo.execScript) {
        pendingScripts.set(entry.id, buildScriptCommand(repo, entry));
      }

      await onAddTerminal(entry.id);
      await waitForSessionReady(entry.id);

      if (repo.execScript) {
        await waitForScriptCompletion(entry.id);
      }

      // デフォルト: サブウィンドウで開く
      if (settings.value.worktreeDefaults?.openInSubWindow) {
        await onMoveToSubWindow(entry.id);
      }
    } catch (e) {
      rollbackWorktree(entry.id);
      loadingWorktrees.delete(entry.id);
      throw e;
    }

    loadingWorktrees.delete(entry.id);
    return entry.id;
  }

  async function executeAgentWorktree(code: AgentWorktreeTaskCode): Promise<void> {
    // 既存のワークツリーを repository名 + branch名 で検索
    const wt = worktrees.value.find(
      (w) => w.repositoryName === code.repository && w.branchName === code.branch
    );
    if (!wt) {
      throw new Error(`ワークツリーが見つかりません: ${code.repository}/${code.branch}`);
    }

    // 初期ターミナル（execScript完了後のプロンプトが出ている状態）を再利用
    const terminal = wt.terminals[0];
    if (!terminal) {
      throw new Error(`ターミナルが見つかりません: ${wt.name}`);
    }

    // sessionId を取得してエージェント実行中か判定
    let sessionId: number | null;
    let agentRunning: boolean;
    if (isDetached(wt.id)) {
      sessionId = getDetachedSessionId(terminal.id);
      agentRunning = sessionId != null
        ? await invoke<boolean>("pty_is_ai_agent", { sessionId })
        : false;
    } else {
      const termRef = getTerminalRef(terminal.id);
      sessionId = termRef?.sessionId ?? null;
      agentRunning = terminalAgentStatus.get(terminal.id) === true;
    }

    // 既存エージェントが実行中の場合はプロンプトを直接送信して続行
    if (agentRunning && sessionId != null) {
      await sendPromptToRunningAgent(sessionId, wt.id, terminal.id, code.prompt);
      return;
    }

    // 一時ファイルにプロンプトを書き出し
    const tempPath = await invoke<string>("write_temp_prompt", { content: code.prompt });

    const agentKind = settings.value.aiAgent?.taskAddAgent ?? settings.value.aiAgent?.approvalAgent ?? "claudeCode";
    const isWindows = platform() === "windows";
    const shell = resolveShell(wt.id);
    const shellLower = (shell ?? "").toLowerCase();
    const isPowerShell =
      shellLower.includes("powershell") ||
      shellLower.includes("pwsh") ||
      (shell === undefined && isWindows);

    let agentCmd: string;
    switch (agentKind) {
      case "claudeCode": agentCmd = code.remoteExec ? "claude --remote --permission-mode plan" : "claude --permission-mode plan"; break;
      case "geminiCli":  agentCmd = "gemini"; break;
      case "codexCli":   agentCmd = "codex"; break;
      case "clineCli":   agentCmd = "cline"; break;
      default:           agentCmd = agentKind;
    }

    let command: string;
    if (isPowerShell) {
      command = `$p = Get-Content "${tempPath}" -Raw -Encoding UTF8; ${agentCmd} $p; Remove-Item "${tempPath}"\r`;
    } else {
      command = `${agentCmd} "$(cat "${tempPath}")"; rm "${tempPath}"\r`;
    }

    if (isDetached(wt.id)) {
      // サブウィンドウに移動済みの場合: pty_write で直接PTYに書き込む
      const sid = getDetachedSessionId(terminal.id);
      if (sid === null) {
        throw new Error(`セッションIDが見つかりません: ${wt.name}`);
      }
      const bytes = Array.from(new TextEncoder().encode(command));
      await invoke("pty_write", { sessionId: sid, data: bytes });
      await invoke("pty_set_ai_agent", { sessionId: sid, isAgent: true });
    } else {
      // メインウィンドウ: terminalRef 経由で書き込む
      const termRef = getTerminalRef(terminal.id);
      if (!termRef) {
        throw new Error(`ターミナルが見つかりません: ${wt.name}`);
      }
      await termRef.waitForReady();
      await termRef.write(command);
      const sid = termRef.sessionId;
      if (sid != null) await invoke("pty_set_ai_agent", { sessionId: sid, isAgent: true });
    }
  }

  return {
    executeAddWorktree,
    executeAgentWorktree,
    resolveShell,
    buildScriptCommand,
  };
}
