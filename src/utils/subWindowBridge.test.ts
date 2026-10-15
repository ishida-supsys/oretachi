import { describe, expect, it } from "vitest";
import { subWindowCommandFor } from "./subWindowBridge";

describe("subWindowCommandFor", () => {
  it("ignores methods outside the subWindow namespace", () => {
    expect(subWindowCommandFor("aiTerminal.open", {})).toBeNull();
    expect(subWindowCommandFor("net.fetch", {})).toBeNull();
  });

  it("passes the open request through untouched (Rust validates it)", () => {
    const req = { width: 200, transparent: true, unknownKey: 1 };
    expect(subWindowCommandFor("subWindow.open", req)).toEqual({
      command: "app_subwindow_open",
      args: { request: req },
    });
  });

  it("maps the remaining calls to their commands", () => {
    expect(subWindowCommandFor("subWindow.close", { id: "w1" })).toEqual({
      command: "app_subwindow_close",
      args: { id: "w1" },
    });
    expect(subWindowCommandFor("subWindow.list", {})).toEqual({ command: "app_subwindow_list", args: {} });
    expect(subWindowCommandFor("subWindow.setPosition", { id: "w1", x: 1, y: -2 })).toEqual({
      command: "app_subwindow_set_position",
      args: { id: "w1", x: 1, y: -2 },
    });
    expect(subWindowCommandFor("subWindow.setSize", { id: "w1", width: 100, height: 200 })).toEqual({
      command: "app_subwindow_set_size",
      args: { id: "w1", width: 100, height: 200 },
    });
    // startDrag は呼んだウィンドウ自身だけを動かすので、id を取らない
    expect(subWindowCommandFor("subWindow.startDrag", { id: "w9" })).toEqual({
      command: "app_subwindow_start_drag",
      args: {},
    });
  });

  it("rejects malformed arguments and unknown methods", () => {
    expect(() => subWindowCommandFor("subWindow.close", {})).toThrow("id");
    expect(() => subWindowCommandFor("subWindow.setPosition", { id: "w1", x: "1", y: 2 })).toThrow("x");
    expect(() => subWindowCommandFor("subWindow.setSize", { id: "w1", width: NaN, height: 2 })).toThrow("width");
    expect(() => subWindowCommandFor("subWindow.destroyEverything", {})).toThrow("無い API");
  });
});
