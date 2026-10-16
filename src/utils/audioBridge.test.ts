import { describe, expect, it } from "vitest";
import { audioCommandFor } from "./audioBridge";

describe("audioCommandFor", () => {
  it("ignores methods outside the audio namespace", () => {
    expect(audioCommandFor("aiTerminal.open", {})).toBeNull();
    expect(audioCommandFor("net.fetch", {})).toBeNull();
    expect(audioCommandFor("audioX.start", {})).toBeNull();
  });

  it("maps start with and without a sample rate (Rust validates the rate)", () => {
    expect(audioCommandFor("audio.start", {})).toEqual({ command: "app_audio_start", args: { sampleRate: null } });
    expect(audioCommandFor("audio.start", { sampleRate: 48000 })).toEqual({
      command: "app_audio_start",
      args: { sampleRate: 48000 },
    });
    // 数値以外も握りつぶさず素通しする（Rust の引数の型エラーになる）
    expect(audioCommandFor("audio.start", { sampleRate: "16000" })).toEqual({
      command: "app_audio_start",
      args: { sampleRate: "16000" },
    });
  });

  it("maps stop without any argument", () => {
    expect(audioCommandFor("audio.stop", {})).toEqual({ command: "app_audio_stop", args: {} });
    // アプリが他のインスタンスの停止を指定する余地を作らない
    expect(audioCommandFor("audio.stop", { worktreeId: "x", artifactId: "y" })).toEqual({
      command: "app_audio_stop",
      args: {},
    });
  });

  it("never maps to the viewer-only stop command", () => {
    expect(() => audioCommandFor("audio.stopForInstance", {})).toThrow();
    expect(() => audioCommandFor("audio.unknown", {})).toThrow();
  });
});
