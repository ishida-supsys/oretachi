import { describe, it, expect } from "vitest";
import { artifactSrcdocSourceKey } from "./reactArtifactSrcdoc";

describe("artifactSrcdocSourceKey", () => {
  it("content が同じでもモジュールが変われば別のキーになる", () => {
    // teamwork-parent の計画フローは content を一度も変えず、進捗のたびに
    // data/flow モジュールだけを更新する。ここを見落とすと iframe は作り直されるのに
    // 初期メモリーが古いまま固定され、保存した pan/zoom が巻き戻る
    const content = "const App = () => null;";
    const before = artifactSrcdocSourceKey(content, { "data/flow": "export default [1];" });
    const after = artifactSrcdocSourceKey(content, { "data/flow": "export default [1,2];" });
    expect(before).not.toBe(after);
  });

  it("モジュールが増えれば別のキーになる", () => {
    const content = "const App = () => null;";
    expect(artifactSrcdocSourceKey(content, { a: "1" })).not.toBe(
      artifactSrcdocSourceKey(content, { a: "1", b: "2" }),
    );
  });

  it("中身が同じなら別オブジェクトでも同じキーになる", () => {
    // 親がアーティファクト一覧を作り直しただけで初期メモリーを取り込み直すと、
    // 入力中に srcdoc の _memory が変わって iframe が落ちる
    const content = "const App = () => null;";
    expect(artifactSrcdocSourceKey(content, { "data/flow": "x" })).toBe(
      artifactSrcdocSourceKey(content, { "data/flow": "x" }),
    );
  });

  it("modules 未指定と空オブジェクトは同じキーになる", () => {
    expect(artifactSrcdocSourceKey("x")).toBe(artifactSrcdocSourceKey("x", {}));
  });

  it("content が変われば別のキーになる", () => {
    expect(artifactSrcdocSourceKey("a", {})).not.toBe(artifactSrcdocSourceKey("b", {}));
  });
});
