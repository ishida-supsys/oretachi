import type { Terminal } from "@xterm/xterm";
import { invoke } from "@tauri-apps/api/core";
import { debug } from "@tauri-apps/plugin-log";

export interface TerminalForApproval {
  id: number;
  getTerminal(): Terminal | null;
  write(data: string): Promise<void>;
}

export interface ApprovalLoopResult {
  approved: boolean;
  lastCommand: string | undefined;
}

/** AI判定の結果 */
export interface JudgeResult {
  safe: boolean;
  command?: string;
}

/** xterm バッファの末尾 N 行をテキストとして取得（ANSI 除去済み） */
export function getRecentLines(terminal: Terminal, n: number): string {
  const buf = terminal.buffer.active;
  const end = buf.length;
  const start = Math.max(0, end - n);
  const lines: string[] = [];
  for (let i = start; i < end; i++) {
    const line = buf.getLine(i);
    if (line) {
      lines.push(line.translateToString(true));
    }
  }
  return lines.join("\n");
}

/** テキスト内に承認プロンプトが含まれるか判定 */
export function hasApprovalPrompt(content: string): boolean {
  return content
    .split("\n")
    .some((line) =>
      /❯\s*Yes|►\s*Yes|\(Y\/n\)|\[Y\/n\]|Allow\s+\w|Do you want to/i.test(line)
    );
}

/** ターミナル内容を解析し自動承認すべきか判定 */
export async function analyzeForApproval(
  worktreeId: string,
  content: string,
  cwd: string = "",
  additionalPrompt?: string,
): Promise<JudgeResult> {
  const promptFound = hasApprovalPrompt(content);

  await debug(
    `[AutoApproval] analyze start worktreeId=${worktreeId} totalLines=${content.split("\n").length} hasApprovalPrompt=${promptFound}`
  );

  if (!promptFound) {
    await debug("[AutoApproval] → skip: no approval prompt detected");
    return { safe: false };
  }

  // AI 判定: claude --model haiku で安全性を判定
  try {
    const result = await invoke<JudgeResult>("judge_approval", {
      worktreeId,
      content,
      cwd,
      additionalPrompt: additionalPrompt || null,
    });
    await debug(`[AutoApproval] AI judgment: ${result.safe ? "safe" : "unsafe"} command=${result.command ?? "none"}`);
    return result;
  } catch (e) {
    await debug(`[AutoApproval] AI judgment failed: ${e}`);
    return { safe: false }; // エラー時は安全側 (承認しない)
  }
}

/** 全ターミナルを走査し最初に承認できたものでEnterを送信する */
export async function runApprovalLoop(
  terminals: TerminalForApproval[],
  worktreeId: string,
  cwd: string,
  additionalPrompt?: string,
): Promise<ApprovalLoopResult> {
  let approved = false;
  let lastCommand: string | undefined;

  for (const termRef of terminals) {
    const terminal = termRef.getTerminal();
    if (!terminal) {
      await debug(`[AutoApproval] tid=${termRef.id} terminal=null, skip`);
      continue;
    }
    // 事前に末尾30行でプロンプト判定し、無ければAI判定と200行取得をスキップ。
    // (大半の tick は「プロンプト無し」なのでここで早期returnすれば debug log ノイズも減る)
    const quickContent = getRecentLines(terminal, 30);
    if (!hasApprovalPrompt(quickContent)) {
      continue;
    }
    const content = getRecentLines(terminal, 200);
    await debug(`[AutoApproval] tid=${termRef.id} content(last200)=${content.slice(-200)}`);
    const judgeResult = await analyzeForApproval(worktreeId, content, cwd, additionalPrompt);
    if (judgeResult.command) {
      lastCommand = judgeResult.command;
    }
    if (judgeResult.safe) {
      // バッファ再チェック: AI判定完了後、承認プロンプトがまだあるか確認
      const freshContent = getRecentLines(terminal, 10);
      if (!hasApprovalPrompt(freshContent)) {
        await debug(`[AutoApproval] tid=${termRef.id} → prompt disappeared, skip Enter`);
        break;
      }
      await debug(`[AutoApproval] tid=${termRef.id} → approved, sending Enter`);
      await termRef.write("\r");
      approved = true;
      break;
    } else {
      await debug(`[AutoApproval] tid=${termRef.id} → not approved`);
    }
  }

  return { approved, lastCommand };
}

/** 進行中のAI判定をキャンセル */
export async function cancelApproval(worktreeId: string): Promise<void> {
  try {
    await invoke("cancel_approval", { worktreeId });
  } catch (e) {
    await debug(`[AutoApproval] cancelApproval failed: ${e}`);
  }
}
