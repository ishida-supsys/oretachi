import { describe, it, expect } from "vitest";
import { buildClaudeLaunchCommand, isSafeModelName, ownedAiTerminalFlags } from "./agentLaunchCommand";

describe("buildClaudeLaunchCommand", () => {
  it("keeps the exact task-execution commands (PowerShell / POSIX)", () => {
    expect(buildClaudeLaunchCommand({ flags: "--permission-mode plan", tempPath: "C:\\t\\p.txt", isPowerShell: true })).toBe(
      '$p = Get-Content "C:\\t\\p.txt" -Raw -Encoding UTF8; claude --permission-mode plan "$p"; Remove-Item "C:\\t\\p.txt"\r',
    );
    expect(buildClaudeLaunchCommand({ flags: "--permission-mode plan", tempPath: "/tmp/p.txt", isPowerShell: false })).toBe(
      'claude --permission-mode plan "$(cat "/tmp/p.txt")"; rm "/tmp/p.txt"\r',
    );
  });

  it("launches without a prompt when there is no temp file", () => {
    expect(buildClaudeLaunchCommand({ isPowerShell: true })).toBe("claude\r");
    expect(buildClaudeLaunchCommand({ flags: "--model opus", isPowerShell: false })).toBe("claude --model opus\r");
  });

  it("puts -- before an untrusted prompt so that it can never be read as a flag", () => {
    expect(
      buildClaudeLaunchCommand({ flags: '--model "opus"', tempPath: "/tmp/p.txt", isPowerShell: false, endOfOptions: true }),
    ).toBe('claude --model "opus" -- "$(cat "/tmp/p.txt")"; rm "/tmp/p.txt"\r');
    expect(buildClaudeLaunchCommand({ tempPath: "C:\\t\\p.txt", isPowerShell: true, endOfOptions: true })).toBe(
      '$p = Get-Content "C:\\t\\p.txt" -Raw -Encoding UTF8; claude -- "$p"; Remove-Item "C:\\t\\p.txt"\r',
    );
    // プロンプトが無いときは `--` も付けない
    expect(buildClaudeLaunchCommand({ isPowerShell: false, endOfOptions: true })).toBe("claude\r");
  });
});

describe("owned AI terminal flags", () => {
  it("never adds permission flags", () => {
    expect(ownedAiTerminalFlags()).toBeUndefined();
    expect(ownedAiTerminalFlags("")).toBeUndefined();
    expect(ownedAiTerminalFlags("claude-opus-4-1")).toBe('--model "claude-opus-4-1"');
    // [ ] はグロブにならないよう引用符で包む
    expect(ownedAiTerminalFlags("sonnet[1m]")).toBe('--model "sonnet[1m]"');
  });

  it("rejects model names that could inject shell syntax or another flag", () => {
    for (const bad of ["a b", "a;b", "$(x)", 'a"b', "a`b", "a|b", "a&b", "a\nb", "-x", "--dangerously-skip-permissions", "x".repeat(101)]) {
      expect(isSafeModelName(bad)).toBe(false);
      expect(() => ownedAiTerminalFlags(bad)).toThrow();
    }
  });
});
