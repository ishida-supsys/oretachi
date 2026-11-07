import { describe, expect, it } from "vitest";
import { processCommandFor } from "./processBridge";

describe("processCommandFor", () => {
  it("ignores methods outside the process namespace", () => {
    expect(processCommandFor("net.fetch", {})).toBeNull();
    expect(processCommandFor("aiTerminal.open", {})).toBeNull();
    expect(processCommandFor("processes.start", {})).toBeNull();
    expect(processCommandFor("process", {})).toBeNull();
  });

  it("maps start with the command name and the parameter values only", () => {
    expect(processCommandFor("process.start", { name: "run-workflow", params: { request: "worktree://runs/a/r.json" } })).toEqual({
      command: "app_process_start",
      args: { name: "run-workflow", params: { request: "worktree://runs/a/r.json" } },
    });
    // params 省略は null（Rust が空として扱う）
    expect(processCommandFor("process.start", { name: "x" })?.args).toEqual({ name: "x", params: null });
  });

  it("maps kill / list / read, never naming another instance or an executable", () => {
    expect(processCommandFor("process.kill", { id: "p1", worktreeId: "other" })).toEqual({
      command: "app_process_kill",
      args: { id: "p1" },
    });
    expect(processCommandFor("process.list", { worktreeId: "other" })).toEqual({ command: "app_process_list", args: {} });
    expect(processCommandFor("process.read", { id: "p1" })).toEqual({
      command: "app_process_read",
      args: { id: "p1", sinceSeq: null },
    });
    expect(processCommandFor("process.read", { id: "p1", sinceSeq: 12 })?.args).toEqual({ id: "p1", sinceSeq: 12 });
    // 実行ファイル・引数・cwd・env は渡せない（start の args に載らない）
    const start = processCommandFor("process.start", { name: "x", exec: "cmd.exe", args: ["/c", "calc"], cwd: "C:\\", env: { A: "1" } });
    expect(Object.keys(start!.args).sort()).toEqual(["name", "params"]);
  });

  it("never maps unknown names", () => {
    expect(() => processCommandFor("process.killAll", {})).toThrow();
    expect(() => processCommandFor("process.unknown", {})).toThrow();
  });
});
