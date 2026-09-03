import { describe, it, expect } from "vitest";
import { buildResumeCommand, isSafeSessionId } from "./resumeCommand";

describe("isSafeSessionId", () => {
  it("英数字とハイフンの UUID を通す", () => {
    expect(isSafeSessionId("5e0560f8-8a52-4070-828a-a3ce48cf216c")).toBe(true);
  });

  it("空文字と 65 文字以上を弾く", () => {
    expect(isSafeSessionId("")).toBe(false);
    expect(isSafeSessionId("a".repeat(64))).toBe(true);
    expect(isSafeSessionId("a".repeat(65))).toBe(false);
  });

  it("シェルのメタ文字を弾く", () => {
    expect(isSafeSessionId("abc; rm -rf /")).toBe(false);
    expect(isSafeSessionId("abc$(id)")).toBe(false);
    expect(isSafeSessionId('abc"def')).toBe(false);
    expect(isSafeSessionId("abc def")).toBe(false);
    expect(isSafeSessionId("abc\ndef")).toBe(false);
    expect(isSafeSessionId("abc_def")).toBe(false);
  });
});

describe("buildResumeCommand", () => {
  const sid = "5e0560f8-8a52-4070-828a-a3ce48cf216c";

  it("claude は --resume", () => {
    expect(buildResumeCommand("claude", sid)).toBe(`claude --resume ${sid}`);
  });

  it("codex は resume サブコマンド", () => {
    expect(buildResumeCommand("codex", sid)).toBe(`codex resume ${sid}`);
  });

  it("未対応エージェントは null", () => {
    expect(buildResumeCommand("gemini", sid)).toBeNull();
    expect(buildResumeCommand("cline", sid)).toBeNull();
    expect(buildResumeCommand("", sid)).toBeNull();
  });

  it("不正な sessionId は null", () => {
    expect(buildResumeCommand("claude", "")).toBeNull();
    expect(buildResumeCommand("claude", "abc; rm -rf /")).toBeNull();
    expect(buildResumeCommand("codex", "a".repeat(65))).toBeNull();
  });
});
