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

function fakeFrame(left: number, top: number) {
  return { getBoundingClientRect: () => ({ left, top }) } as unknown as HTMLIFrameElement;
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

  it("iframe / ポップアップが未マウントなら何もしない", () => {
    const { popup, calls } = fakePopup();
    applyFrameLinkHover({ href: "https://example.com", rect }, null, popup);
    expect(calls).toEqual([]);
    expect(() => applyFrameLinkHover({ href: null, rect: null }, fakeFrame(0, 0), null)).not.toThrow();
  });
});
