import { describe, it, expect } from "vitest";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
    // PowerShell 5.1 は引数の `"` をエスケープしないので、`\"` へ直してから渡す
    expect(buildClaudeLaunchCommand({ tempPath: "C:\\t\\p.txt", isPowerShell: true, endOfOptions: true })).toBe(
      '$p = Get-Content "C:\\t\\p.txt" -Raw -Encoding UTF8; $p = [regex]::Replace($p,\'(\\\\*)"\',\'$1$1\\"\'); if ($p -match \'\\s\') { $p = [regex]::Replace($p,\'(\\\\+)\\z\',\'$1$1\') }; claude -- "$p"; Remove-Item "C:\\t\\p.txt"\r',
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

// Windows PowerShell 5.1 と引数解析（CommandLineToArgvW 系）を実際に通して、欠けずに 1 引数で届くことを確かめる。
describe.skipIf(process.platform !== "win32")("PowerShell 5.1 launch (real shell)", () => {
  const cases: string[] = [
    'say "hi" and \\"x\\" end',
    "trailing backslash \\",
    "trailing backslashes \\\\",
    'C:\\dir\\ \\\\"q\\\\" end',
    "it's $env:PATH $(1+1) `tick` end",
    'line1\nline2 "q"\nline3 \\',
    '日本語 "引用" \\',
    "--dangerously-skip-permissions --x",
    "nospace\\",
    'nospace"quote',
    // 空のプロンプトは呼び出し側が一時ファイルを作らない（tempPath 無し）ので対象外
  ];
  it("delivers every untrusted prompt as exactly one intact argument", () => {
    const dir = mkdtempSync(join(tmpdir(), "alc-"));
    try {
      const script = join(dir, "argv.js");
      writeFileSync(script, "process.stdout.write(JSON.stringify(process.argv.slice(2)))");
      for (const prompt of cases) {
        const tmp = join(dir, "p.txt");
        writeFileSync(tmp, prompt, "utf8");
        const cmd = buildClaudeLaunchCommand({ tempPath: tmp, isPowerShell: true, endOfOptions: true })
          .replace("; claude -- ", `; node "${script}" -- `)
          .replace(/; Remove-Item.*$/s, "");
        const r = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", "-"], {
          input: cmd,
          encoding: "utf8",
        });
        // `--` の後ろがちょうど 1 引数で、中身が欠けていないこと（node は `--` 自体も argv に残す）
        expect(JSON.parse(r.stdout), JSON.stringify(prompt)).toEqual(["--", prompt]);
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }, 60_000);
});
