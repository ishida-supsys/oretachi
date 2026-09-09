import { describe, it, expect } from "vitest";
import { applyFrameLinkHover, type ArtifactLinkHoverPopupApi } from "./artifactLinkHover";
import type { ArtifactLinkRect } from "./artifactFrameLink";

function fakePopup() {
  const calls: Array<{ kind: "show"; href: string; rect: ArtifactLinkRect } | { kind: "scheduleHide" | "hideNow" }> = [];
  const popup: ArtifactLinkHoverPopupApi = {
    showFor: (href, rect) => calls.push({ kind: "show", href, rect }),
    scheduleHide: () => calls.push({ kind: "scheduleHide" }),
    hideNow: () => calls.push({ kind: "hideNow" }),
  };
  return { popup, calls };
}

function fakeFrame(left: number, top: number, width = 800, height = 600) {
  return {
    getBoundingClientRect: () => ({ left, top, width, height }),
  } as unknown as HTMLIFrameElement;
}

describe("applyFrameLinkHover", () => {
  const rect: ArtifactLinkRect = { left: 10, top: 20, width: 100, height: 16 };

  it("iframe の位置を足して親のビューポート座標に直す", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover({ href: "https://example.com", rect }, fakeFrame(40, 200), popup);
    expect(calls).toEqual([
      {
        kind: "show",
        href: "https://example.com",
        rect: { left: 50, top: 220, width: 100, height: 16 },
      },
    ]);
  });

  it("href が null なら猶予付きで閉じる（即閉じない）", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover({ href: null, rect: null }, fakeFrame(0, 0), popup);
    expect(calls).toEqual([{ kind: "scheduleHide" }]);
  });

  it("座標が無い表示要求も閉じる扱いにする", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover({ href: "https://example.com", rect: null }, fakeFrame(0, 0), popup);
    expect(calls).toEqual([{ kind: "scheduleHide" }]);
  });

  it("iframe の外を指す座標は矩形内へ丸める（本文が親アプリの UI 上に出せないようにする）", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover(
      { href: "https://evil.example", rect: { left: -400, top: -300, width: 1, height: 1 } },
      fakeFrame(40, 200, 800, 600),
      popup,
    );
    // 左上へ丸められ、iframe の左上（40, 200）に貼り付く
    expect(calls).toEqual([
      { kind: "show", href: "https://evil.example", rect: { left: 40, top: 200, width: 1, height: 1 } },
    ]);
  });

  it("iframe より大きい座標・サイズも矩形内で打ち切る", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover(
      { href: "https://evil.example", rect: { left: 9000, top: 9000, width: 9000, height: 9000 } },
      fakeFrame(40, 200, 800, 600),
      popup,
    );
    expect(calls).toEqual([
      { kind: "show", href: "https://evil.example", rect: { left: 840, top: 800, width: 0, height: 0 } },
    ]);
  });

  it("iframe が非表示（幅・高さ 0）なら閉じる。teleport 先には display:none が効かない", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover({ href: "https://example.com", rect }, fakeFrame(0, 0, 0, 0), popup);
    expect(calls).toEqual([{ kind: "hideNow" }]);
  });

  it("iframe / ポップアップが未マウントなら何もしない", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover({ href: "https://example.com", rect }, null, popup);
    expect(calls).toEqual([]);
    expect(() => applyFrameLinkHover({ href: null, rect: null }, fakeFrame(0, 0), null)).not.toThrow();
  });
});
