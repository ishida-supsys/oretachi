import { describe, it, expect } from "vitest";
import { aiTerminalCommandFor } from "./aiTerminalBridge";

describe("aiTerminalCommandFor", () => {
  it("ignores methods outside the aiTerminal namespace", () => {
    expect(aiTerminalCommandFor("mcp.call", {})).toBeNull();
    expect(aiTerminalCommandFor("memory.set", {})).toBeNull();
  });

  it("maps each method to its command", () => {
    expect(aiTerminalCommandFor("aiTerminal.open", { title: "t" })).toEqual({
      command: "app_ai_terminal_open",
      args: { params: { title: "t" } },
    });
    expect(aiTerminalCommandFor("aiTerminal.write", { terminalId: "x", text: "hi" })).toEqual({
      command: "app_ai_terminal_write",
      args: { terminalId: "x", text: "hi", submit: null },
    });
    expect(aiTerminalCommandFor("aiTerminal.write", { terminalId: "x", text: "hi", submit: false })?.args.submit).toBe(false);
    expect(aiTerminalCommandFor("aiTerminal.list", {})).toEqual({ command: "app_ai_terminal_list", args: {} });
    expect(aiTerminalCommandFor("aiTerminal.close", { terminalId: "x" })?.command).toBe("app_ai_terminal_close");
    expect(aiTerminalCommandFor("aiTerminal.inspectPrompt", { terminalId: "x" })?.command).toBe(
      "app_ai_terminal_inspect_prompt",
    );
  });

  it("never forwards allow_confirm or other permission-like arguments for sendKeys", () => {
    const c = aiTerminalCommandFor("aiTerminal.sendKeys", {
      terminalId: "x",
      keys: ["Up", "Down"],
      allowConfirm: true,
      allow_confirm: true,
    });
    expect(c?.args).toEqual({ terminalId: "x", keys: ["Up", "Down"] });
  });

  it("passes unknown open options through so that Rust can reject them", () => {
    const c = aiTerminalCommandFor("aiTerminal.open", { permissionMode: "bypass" });
    expect(c?.args).toEqual({ params: { permissionMode: "bypass" } });
  });

  it("rejects malformed arguments and unknown methods", () => {
    expect(() => aiTerminalCommandFor("aiTerminal.write", { text: "hi" })).toThrow("terminalId");
    expect(() => aiTerminalCommandFor("aiTerminal.write", { terminalId: "x" })).toThrow("text");
    expect(() => aiTerminalCommandFor("aiTerminal.sendKeys", { terminalId: "x", keys: "Up" })).toThrow("keys");
    expect(() => aiTerminalCommandFor("aiTerminal.sendKeys", { terminalId: "x", keys: [1] })).toThrow("keys");
    expect(() => aiTerminalCommandFor("aiTerminal.answerPrompt", { terminalId: "x" })).toThrow("answer");
    expect(() => aiTerminalCommandFor("aiTerminal.nope", {})).toThrow("無い API");
  });
});
