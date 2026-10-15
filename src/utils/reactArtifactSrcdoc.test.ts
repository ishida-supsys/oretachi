import { describe, it, expect } from "vitest";
import { artifactSrcdocSourceKey, buildReactSrcdoc, buildVendorHead } from "./reactArtifactSrcdoc";
import { ARTIFACT_APP_FLAG, ARTIFACT_STANDALONE_FLAG } from "./artifactMemory";

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

describe("buildReactSrcdoc", () => {
  it("既定ではスタンドアロンフラグを立てない（ビューアの iframe は親と話す）", () => {
    const html = buildReactSrcdoc("<html><head></head>", "const App = () => null;");
    // ブリッジ本体はフラグを「読む」ので、見るのは立てているかどうか
    expect(html).not.toContain(`window["${ARTIFACT_STANDALONE_FLAG}"]=true;`);
  });

  it("app ではブリッジより先にアプリフラグを立て、未指定なら立てない", () => {
    const plain = buildReactSrcdoc("<html><head></head>", "const App = () => null;");
    expect(plain).not.toContain(`window["${ARTIFACT_APP_FLAG}"]=true;`);
    const html = buildReactSrcdoc("<html><head></head>", "const App = () => null;", undefined, undefined, {
      app: true,
    });
    const flagAt = html.indexOf(`window["${ARTIFACT_APP_FLAG}"]=true;`);
    expect(flagAt).toBeGreaterThan(-1);
    expect(flagAt).toBeLessThan(html.indexOf("__oretachi="));
  });

  it("standalone ではブリッジより先にフラグを立てる", () => {
    const html = buildReactSrcdoc("<html><head></head>", "const App = () => null;", undefined, undefined, {
      standalone: true,
    });
    const flagAt = html.indexOf(`window["${ARTIFACT_STANDALONE_FLAG}"]=true;`);
    // ブリッジは読み込み時に一度だけフラグを見るので、順序が逆だと効かない
    expect(flagAt).toBeGreaterThan(-1);
    expect(flagAt).toBeLessThan(html.indexOf("__oretachi="));
  });
});

describe("buildVendorHead の CSP", () => {
  const csp = /http-equiv="Content-Security-Policy" content="([^"]*)"/.exec(buildVendorHead("", "", "", ""))?.[1] ?? "";

  it("WebAssembly の instantiate に必要な 'unsafe-eval' を許可している（アプリの wasm 実行 #377）", () => {
    // 'unsafe-eval' は Chromium では WebAssembly のコンパイルも許可する。外すと fs.readBytes → instantiate が動かない
    expect(csp).toContain("script-src 'unsafe-inline' 'unsafe-eval'");
  });

  it("外部通信は許可しない（既定が 'none' で connect-src を足していない）", () => {
    expect(csp).toContain("default-src 'none'");
    expect(csp).not.toContain("connect-src");
  });
});
