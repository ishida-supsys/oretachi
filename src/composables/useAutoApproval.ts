import type { Terminal } from "@xterm/xterm";
import { invoke } from "@tauri-apps/api/core";
import { debug } from "@tauri-apps/plugin-log";

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
  cwd: string = ""
): Promise<boolean> {
  const promptFound = hasApprovalPrompt(content);

  await debug(
    `[AutoApproval] analyze start worktreeId=${worktreeId} totalLines=${content.split("\n").length} hasApprovalPrompt=${promptFound}`
  );

  if (!promptFound) {
    await debug("[AutoApproval] → skip: no approval prompt detected");
    return false;
  }

  // AI 判定: claude --model haiku で安全性を判定
  try {
    const safe = await invoke<boolean>("judge_approval", {
      worktreeId,
      content,
      cwd,
    });
    await debug(`[AutoApproval] AI judgment: ${safe ? "safe" : "unsafe"}`);
    return safe;
  } catch (e) {
    await debug(`[AutoApproval] AI judgment failed: ${e}`);
    return false; // エラー時は安全側 (承認しない)
  }
}

/** 進行中のAI判定をキャンセル */
export async function cancelApproval(worktreeId: string): Promise<void> {
  try {
    await invoke("cancel_approval", { worktreeId });
  } catch (e) {
    await debug(`[AutoApproval] cancelApproval failed: ${e}`);
  }
}
