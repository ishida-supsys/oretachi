import type { AiAgentKind } from "../types/settings";

/** detect_ai_agents コマンドが返す検出済みエージェント情報 */
export interface AiAgentInfo {
  kind: AiAgentKind;
  name: string;
  command: string;
}

export const AI_AGENT_LABELS: Record<AiAgentKind, string> = {
  claudeCode: "Claude Code",
  geminiCli: "Gemini CLI",
  codexCli: "Codex CLI",
  clineCli: "Cline CLI",
};

export const ALL_AGENT_KINDS: AiAgentKind[] = ["claudeCode", "geminiCli", "codexCli", "clineCli"];
