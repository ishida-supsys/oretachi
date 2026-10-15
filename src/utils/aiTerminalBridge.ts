/**
 * アプリ（iframe）の `aiTerminal.*` ブリッジ呼び出しを、Rust コマンドへの呼び出しへ写す純関数。
 *
 * **ここでは権限の判断をしない。** 許可・所有の検査・上限は Rust（`ai_terminal.rs`）が行う唯一の関門で、
 * ここは「method 名 → コマンド名」と引数の形の写像だけ。`allow_confirm` のような権限に関わる引数は
 * そもそも写像に存在しない（アプリは Enter / Esc を含むキー列を送れない。確定は `answerPrompt`）。
 */

import { ARTIFACT_BRIDGE_METHOD_AI_TERMINAL_PREFIX } from "./artifactMemory";

export interface AiTerminalCommand {
  command: string;
  args: Record<string, unknown>;
}

const isObject = (v: unknown): v is Record<string, unknown> =>
  v !== null && typeof v === "object" && !Array.isArray(v);

function requireTerminalId(params: Record<string, unknown>): string {
  const id = params.terminalId;
  if (typeof id !== "string" || id === "") throw new Error("terminalId が必要です");
  return id;
}

/** `aiTerminal.*` 以外なら null。未知の `aiTerminal.*` は例外 */
export function aiTerminalCommandFor(
  method: string,
  params: Record<string, unknown>,
): AiTerminalCommand | null {
  if (!method.startsWith(ARTIFACT_BRIDGE_METHOD_AI_TERMINAL_PREFIX)) return null;
  const name = method.slice(ARTIFACT_BRIDGE_METHOD_AI_TERMINAL_PREFIX.length);
  switch (name) {
    case "open":
      // 未知のキー（permissionMode など）は Rust が拒否する（deny_unknown_fields）。握りつぶさず素通しする
      return { command: "app_ai_terminal_open", args: { params: isObject(params) ? params : {} } };
    case "write": {
      const text = params.text;
      if (typeof text !== "string") throw new Error("text が必要です");
      const submit = typeof params.submit === "boolean" ? params.submit : null;
      return { command: "app_ai_terminal_write", args: { terminalId: requireTerminalId(params), text, submit } };
    }
    case "sendKeys": {
      const keys = params.keys;
      if (!Array.isArray(keys) || keys.some((k) => typeof k !== "string")) {
        throw new Error("keys は文字列の配列で指定してください");
      }
      return { command: "app_ai_terminal_send_keys", args: { terminalId: requireTerminalId(params), keys } };
    }
    case "read":
      return {
        command: "app_ai_terminal_read",
        args: {
          terminalId: requireTerminalId(params),
          maxBytes: typeof params.maxBytes === "number" ? params.maxBytes : null,
          fromCursor: typeof params.fromCursor === "number" ? params.fromCursor : null,
        },
      };
    case "inspectPrompt":
      return { command: "app_ai_terminal_inspect_prompt", args: { terminalId: requireTerminalId(params) } };
    case "answerPrompt": {
      if (!isObject(params.answer)) throw new Error("answer が必要です");
      return {
        command: "app_ai_terminal_answer_prompt",
        args: { terminalId: requireTerminalId(params), answer: params.answer },
      };
    }
    case "close":
      return { command: "app_ai_terminal_close", args: { terminalId: requireTerminalId(params) } };
    case "list":
      return { command: "app_ai_terminal_list", args: {} };
    default:
      throw new Error(`専用 AI ターミナルに無い API です: ${method}`);
  }
}
