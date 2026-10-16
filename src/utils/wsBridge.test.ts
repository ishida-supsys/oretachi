import { describe, expect, it } from "vitest";
import { wsCommandFor } from "./wsBridge";

describe("wsCommandFor", () => {
  it("ignores methods outside the net.ws namespace", () => {
    expect(wsCommandFor("net.fetch", {})).toBeNull();
    expect(wsCommandFor("audio.start", {})).toBeNull();
    expect(wsCommandFor("net.wsX.open", {})).toBeNull();
  });

  it("maps open with protocols (Rust validates the URL and the protocols)", () => {
    expect(wsCommandFor("net.ws.open", { id: "w1", url: "wss://a.example.com/s" })).toEqual({
      command: "app_ws_open",
      args: { id: "w1", url: "wss://a.example.com/s", opts: { protocols: [] } },
    });
    expect(wsCommandFor("net.ws.open", { id: "w1", url: "wss://a.example.com/s", protocols: ["v1"] })).toEqual({
      command: "app_ws_open",
      args: { id: "w1", url: "wss://a.example.com/s", opts: { protocols: ["v1"] } },
    });
  });

  it("maps send and close, never naming another instance", () => {
    expect(wsCommandFor("net.ws.send", { id: "w1", data: "hi", binary: false })).toEqual({
      command: "app_ws_send",
      args: { id: "w1", data: "hi", binary: false },
    });
    // binary は真偽値 true のときだけ（文字列の "true" などは text 扱いにして Rust 側の検査へ任せる）
    expect(wsCommandFor("net.ws.send", { id: "w1", data: "AQID", binary: true })?.args.binary).toBe(true);
    expect(wsCommandFor("net.ws.send", { id: "w1", data: "x", binary: "true" })?.args.binary).toBe(false);
    expect(wsCommandFor("net.ws.close", { id: "w1" })).toEqual({
      command: "app_ws_close",
      args: { id: "w1", code: null, reason: null },
    });
    expect(wsCommandFor("net.ws.close", { id: "w1", code: 1000, reason: "bye", worktreeId: "x" })).toEqual({
      command: "app_ws_close",
      args: { id: "w1", code: 1000, reason: "bye" },
    });
  });

  it("never maps to close_all or unknown names", () => {
    expect(() => wsCommandFor("net.ws.closeAll", {})).toThrow();
    expect(() => wsCommandFor("net.ws.unknown", {})).toThrow();
  });
});
