/**
 * 再起動復元タブへ投入する AI エージェントの resume コマンドを組み立てる (#157)。
 *
 * 戻り値は PTY へそのまま流し込まれる。`event_delivery.rs` の `is_safe_session_id` と
 * 同じ検証をフロント側でも行い、外れたら投入しない（引用符やセミコロンが混ざると
 * 別のコマンドとして解釈されうる）。
 */

/** `--resume` へ渡してよい session ID か（英数字とハイフンのみ / 1〜64 文字）。 */
export function isSafeSessionId(sessionId: string): boolean {
  return /^[A-Za-z0-9-]{1,64}$/.test(sessionId);
}

/**
 * agentType に対応する resume コマンドを返す。
 * 未対応のエージェント（gemini / cline 等）と不正な sessionId は `null`。
 */
export function buildResumeCommand(agentType: string, sessionId: string): string | null {
  if (!isSafeSessionId(sessionId)) return null;
  switch (agentType) {
    case "claude":
      return `claude --resume ${sessionId}`;
    case "codex":
      return `codex resume ${sessionId}`;
    default:
      return null;
  }
}
