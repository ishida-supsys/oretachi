import { describe, it, expect } from "vitest";
import { parseSvgSize, clampScale } from "./svgToPng";

describe("parseSvgSize", () => {
  it("width / height を px として読む", () => {
    expect(parseSvgSize('<svg width="640" height="480"></svg>')).toEqual({
      width: 640,
      height: 480,
    });
    expect(parseSvgSize('<svg width="640px" height="480px"></svg>')).toEqual({
      width: 640,
      height: 480,
    });
  });

  it("百分率は px として使えないので viewBox へ落ちる", () => {
    expect(parseSvgSize('<svg width="100%" height="100%" viewBox="0 0 300 150"></svg>')).toEqual({
      width: 300,
      height: 150,
    });
  });

  it("width だけある場合も viewBox を優先する（片方だけでは描画サイズが決まらない）", () => {
    expect(parseSvgSize('<svg width="640" viewBox="0 0 300 150"></svg>')).toEqual({
      width: 300,
      height: 150,
    });
  });

  it("片方だけで viewBox も無ければ既定値で揃える（比率が分からず歪むため）", () => {
    expect(parseSvgSize('<svg width="640"></svg>')).toEqual({ width: 1200, height: 800 });
  });

  it("どちらも無ければ既定値", () => {
    expect(parseSvgSize("<svg></svg>")).toEqual({ width: 1200, height: 800 });
    expect(parseSvgSize("not an svg")).toEqual({ width: 1200, height: 800 });
  });
});

describe("clampScale", () => {
  it("長辺が上限を超えないときは倍率をそのまま使う", () => {
    expect(clampScale(800, 600, 2)).toBe(2);
  });

  it("長辺が上限を超える倍率は丸める", () => {
    expect(clampScale(8000, 600, 2)).toBe(1);
    expect(clampScale(4000, 600, 4)).toBe(2);
  });

  it("元寸だけで上限を超える図は 1 倍未満まで下げる（canvas の面積上限で落ちるため）", () => {
    expect(clampScale(20000, 600, 2)).toBe(0.4);
  });

  it("サイズが 0 なら倍率をそのまま返す（0 除算しない）", () => {
    expect(clampScale(0, 0, 2)).toBe(2);
  });
});
