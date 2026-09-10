import { describe, it, expect } from "vitest";
import {
  exportFileName,
  hasStandaloneView,
  wrapSvgDocument,
  injectCspMeta,
  buildExportViewHtml,
} from "./artifactExport";
import { HTML_ARTIFACT_CSP } from "./artifactCspViolation";
import type { ArtifactData } from "../types/artifact";

function artifact(overrides: Partial<ArtifactData>): ArtifactData {
  return {
    id: "a1",
    title: "レポート",
    content_type: "text/markdown",
    content: "# hello",
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

describe("exportFileName", () => {
  it("タイトルと ID を並べた .zip 名を作る", () => {
    expect(exportFileName("report", "a1")).toBe("report-a1.zip");
  });

  it("ファイル名に使えない文字を潰す", () => {
    expect(exportFileName('a/b:c*d?e"f<g>h|i', "a1")).toBe("a_b_c_d_e_f_g_h_i-a1.zip");
  });

  it("タイトルが空なら ID だけで組み立てる", () => {
    expect(exportFileName("", "a1")).toBe("a1-a1.zip");
  });

  it("長すぎるタイトルは切り詰める", () => {
    const name = exportFileName("あ".repeat(200), "a1");
    expect(name).toBe(`${"あ".repeat(60)}-a1.zip`);
  });

  it("先頭のドットは落とす（隠しファイル・拡張子誤認を避ける）", () => {
    expect(exportFileName("...report", "a1")).toBe("report-a1.zip");
  });
});

describe("hasStandaloneView", () => {
  it("単体 HTML を作れる種類だけ true", () => {
    expect(hasStandaloneView("application/vnd.ant.react")).toBe(true);
    expect(hasStandaloneView("text/html")).toBe(true);
    expect(hasStandaloneView("image/svg+xml")).toBe(true);
    expect(hasStandaloneView("application/vnd.ant.mermaid")).toBe(true);
    expect(hasStandaloneView("text/markdown")).toBe(false);
    expect(hasStandaloneView("text/csv")).toBe(false);
    expect(hasStandaloneView("application/vnd.ant.code")).toBe(false);
  });
});

describe("wrapSvgDocument", () => {
  it("タイトルはエスケープし、SVG はそのまま入れる", () => {
    const html = wrapSvgDocument('<script>"x"', '<svg width="10" height="10"></svg>');
    expect(html).toContain("<title>&lt;script&gt;&quot;x&quot;</title>");
    expect(html).toContain('<svg width="10" height="10"></svg>');
  });

  it("持ち出した先でも外部通信しないよう CSP を入れる", () => {
    expect(wrapSvgDocument("t", "<svg></svg>")).toContain("default-src 'none'");
  });
});

describe("buildExportViewHtml", () => {
  const noMermaid = async () => {
    throw new Error("should not be called");
  };

  it("text/html は中身を保ったまま CSP を足す", async () => {
    const html = await buildExportViewHtml(
      artifact({ content_type: "text/html", content: "<html><head></head><body>hi</body></html>" }),
      undefined,
      noMermaid,
    );
    expect(html).toContain("<body>hi</body>");
    expect(html).toContain(HTML_ARTIFACT_CSP);
  });

  it("svg は HTML に包む", async () => {
    const html = await buildExportViewHtml(
      artifact({ content_type: "image/svg+xml", content: "<svg />" }),
      undefined,
      noMermaid,
    );
    expect(html).toContain("<svg />");
    expect(html).toContain("<!DOCTYPE html>");
  });

  it("mermaid は描画結果の SVG を包む", async () => {
    const html = await buildExportViewHtml(
      artifact({ content_type: "application/vnd.ant.mermaid", content: "graph TD; A-->B" }),
      undefined,
      async (source) => `<svg data-source="${source}"></svg>`,
    );
    expect(html).toContain('<svg data-source="graph TD; A-->B"></svg>');
  });

  it("単体 HTML を作れない種類は null（zip には素のソースだけが入る）", async () => {
    expect(await buildExportViewHtml(artifact({}), undefined, noMermaid)).toBeNull();
    expect(
      await buildExportViewHtml(artifact({ content_type: "text/csv" }), undefined, noMermaid),
    ).toBeNull();
  });
});

describe("injectCspMeta", () => {
  it("head があればその直後へ入れる（他のリソースより前に効かせる）", () => {
    const html = injectCspMeta("<html><head><title>t</title></head><body>x</body></html>", "CSP");
    expect(html).toBe(
      '<html><head>\n<meta http-equiv="Content-Security-Policy" content="CSP" />' +
        "<title>t</title></head><body>x</body></html>",
    );
  });

  it("head が無ければ html の直後に作る", () => {
    const html = injectCspMeta("<html><body>x</body></html>", "CSP");
    expect(html).toContain('<head><meta http-equiv="Content-Security-Policy" content="CSP" /></head>');
    expect(html).toContain("<body>x</body>");
  });

  it("コメント内の head は無視する（そこへ入れると CSP が黙って効かなくなる）", () => {
    const html = injectCspMeta("<!-- <head> --><html><head></head><body>x</body></html>", "CSP");
    expect(html).toBe(
      '<!-- <head> --><html><head>\n<meta http-equiv="Content-Security-Policy" content="CSP" />' +
        "</head><body>x</body></html>",
    );
  });

  it("断片に doctype が付いていれば剥がしてから包む", () => {
    const html = injectCspMeta("<!DOCTYPE html>\n<p>x</p>", "CSP");
    expect(html.match(/<!DOCTYPE/gi)).toHaveLength(1);
    expect(html).toContain("<body>\n<p>x</p>\n</body>");
  });

  it("断片なら最小の枠で包む", () => {
    const html = injectCspMeta("<p>x</p>", "CSP");
    expect(html.startsWith("<!DOCTYPE html>")).toBe(true);
    expect(html).toContain('content="CSP"');
    expect(html).toContain("<p>x</p>");
  });

  it("既定は アプリ内表示と同じ CSP", () => {
    expect(injectCspMeta("<html><head></head></html>")).toContain(HTML_ARTIFACT_CSP);
  });
});
