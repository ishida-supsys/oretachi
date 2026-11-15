import { describe, it, expect } from "vitest";
import { appFrameAllow, REACT_ARTIFACT_FRAME_ALLOW } from "./artifactFrameAllow";

describe("appFrameAllow", () => {
  it("承認済みのときだけ clipboard-write を委譲する", () => {
    expect(appFrameAllow(true)).toBe("clipboard-write");
  });

  it("未承認のときは allow 属性を付けない", () => {
    expect(appFrameAllow(false)).toBeUndefined();
  });

  it("clipboard-read は委譲しない", () => {
    expect(appFrameAllow(true)).not.toContain("clipboard-read");
  });
});

describe("REACT_ARTIFACT_FRAME_ALLOW", () => {
  it("fullscreen を保ったまま clipboard-write を足す（読み取りは含めない）", () => {
    expect(REACT_ARTIFACT_FRAME_ALLOW.split(/;\s*/)).toEqual(["fullscreen", "clipboard-write"]);
  });
});
