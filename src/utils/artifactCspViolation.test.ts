import { describe, it, expect } from "vitest";
import {
  ARTIFACT_CSP_TEXT_LIMIT,
  ARTIFACT_CSP_VIOLATION_LIMIT,
  ARTIFACT_CSP_VIOLATION_MARKER,
  buildCspViolationReportJs,
  createCspNonce,
  HTML_ARTIFACT_CSP,
  mergeCspViolations,
  readArtifactCspViolationMessage,
  type ArtifactCspViolation,
} from "./artifactCspViolation";

const NONCE = "nonce-1";
const frame = { contentWindow: {} } as unknown as HTMLIFrameElement;
const other = { contentWindow: {} } as unknown as HTMLIFrameElement;

function message(data: unknown, source: unknown = frame.contentWindow): MessageEvent {
  return { data, source } as unknown as MessageEvent;
}

function violationMessage(violations: unknown, extra: Record<string, unknown> = {}): MessageEvent {
  return message({
    [ARTIFACT_CSP_VIOLATION_MARKER]: true,
    nonce: NONCE,
    violations,
    ...extra,
  });
}

describe("HTML_ARTIFACT_CSP", () => {
  it("外部スクリプト・外部 CSS・fetch を塞ぐ", () => {
    expect(HTML_ARTIFACT_CSP).toContain("default-src 'none'");
    expect(HTML_ARTIFACT_CSP).toContain("script-src 'unsafe-inline' 'unsafe-eval';");
    expect(HTML_ARTIFACT_CSP).toContain("style-src 'unsafe-inline';");
    expect(HTML_ARTIFACT_CSP).toContain("connect-src 'none'");
  });

  it("画像だけは https: を許す", () => {
    expect(HTML_ARTIFACT_CSP).toContain("img-src data: blob: https:");
  });

  it("外部フォントは許さない（font-src は default-src 'none' に落ちる）", () => {
    expect(HTML_ARTIFACT_CSP).not.toContain("font-src");
  });

  it("meta では効かないディレクティブを含めない", () => {
    for (const d of ["frame-ancestors", "sandbox", "report-uri"]) {
      expect(HTML_ARTIFACT_CSP).not.toContain(d);
    }
  });
});

describe("createCspNonce", () => {
  it("呼ぶたびに異なる値を返す", () => {
    expect(createCspNonce()).not.toBe(createCspNonce());
  });
});

describe("buildCspViolationReportJs", () => {
  it("securitypolicyviolation を購読して親へマーカーと nonce を送る", () => {
    const js = buildCspViolationReportJs(NONCE);
    expect(js).toContain("securitypolicyviolation");
    expect(js).toContain(ARTIFACT_CSP_VIOLATION_MARKER);
    expect(js).toContain(JSON.stringify(NONCE));
    expect(js).toContain("postMessage");
  });

  it("コンテンツに差し替えられる前に setTimeout と parent を束縛する", () => {
    const js = buildCspViolationReportJs(NONCE);
    expect(js).toContain("var st=setTimeout;var target=parent;");
  });

  it("nonce をそのまま埋めずエスケープする", () => {
    expect(buildCspViolationReportJs('a"b')).toContain('"a\\"b"');
  });
});

describe("readArtifactCspViolationMessage", () => {
  const read = (e: MessageEvent, f: HTMLIFrameElement | null = frame) =>
    readArtifactCspViolationMessage(e, f, NONCE);

  it("対象 iframe の現在の文書から来た違反を返す", () => {
    const event = violationMessage([{ directive: "script-src", blockedUri: "https://cdn.example/x.js" }]);
    expect(read(event)).toEqual({
      violations: [{ directive: "script-src", blockedUri: "https://cdn.example/x.js" }],
      truncated: false,
    });
  });

  it("別 iframe から来たものは無視する", () => {
    expect(read(violationMessage([{ directive: "script-src", blockedUri: "" }]), other)).toBeNull();
  });

  it("frame が null なら無視する", () => {
    expect(read(violationMessage([{ directive: "script-src", blockedUri: "" }]), null)).toBeNull();
  });

  it("nonce が違えば無視する（差し替え前の文書からの遅延メッセージ）", () => {
    const stale = message({
      [ARTIFACT_CSP_VIOLATION_MARKER]: true,
      nonce: "nonce-0",
      violations: [{ directive: "img-src", blockedUri: "http://example/a.png" }],
    });
    expect(read(stale)).toBeNull();
  });

  it("nonce が無いメッセージは無視する", () => {
    const event = message({
      [ARTIFACT_CSP_VIOLATION_MARKER]: true,
      violations: [{ directive: "img-src", blockedUri: "" }],
    });
    expect(read(event)).toBeNull();
  });

  it("マーカーが無いメッセージは無視する", () => {
    expect(read(message({ nonce: NONCE, violations: [] }))).toBeNull();
  });

  it("violations が配列でなければ無視する", () => {
    expect(read(violationMessage("script-src"))).toBeNull();
  });

  it("data が null でも落ちない", () => {
    expect(read(message(null))).toBeNull();
  });

  it("形が違う要素は捨て、残りが無ければ null を返す", () => {
    const event = violationMessage([
      null,
      "script-src",
      { directive: 1, blockedUri: "https://cdn.example/x.js" },
      { directive: "img-src", blockedUri: "http://example/a.png" },
    ]);
    expect(read(event)).toEqual({
      violations: [{ directive: "img-src", blockedUri: "http://example/a.png" }],
      truncated: false,
    });
    expect(read(violationMessage([null, 3]))).toBeNull();
  });

  it("truncated が立っていれば違反が空でも報告する", () => {
    expect(read(violationMessage([], { truncated: true }))).toEqual({
      violations: [],
      truncated: true,
    });
  });

  // 収集スクリプトを介さず直接 postMessage された巨大なペイロードで親を止めないこと
  it("上限を超える配列は走査ごと打ち切り、truncated を立てる", () => {
    const many = Array.from({ length: 10_000 }, (_, i) => ({
      directive: "img-src",
      blockedUri: `https://example/${i}.png`,
    }));
    const report = read(violationMessage(many));
    expect(report?.violations).toHaveLength(ARTIFACT_CSP_VIOLATION_LIMIT);
    expect(report?.truncated).toBe(true);
  });

  it("超長文字列を切り詰める", () => {
    const long = "x".repeat(ARTIFACT_CSP_TEXT_LIMIT + 500);
    const report = read(violationMessage([{ directive: long, blockedUri: long }]));
    expect(report?.violations[0].directive).toHaveLength(ARTIFACT_CSP_TEXT_LIMIT);
    expect(report?.violations[0].blockedUri).toHaveLength(ARTIFACT_CSP_TEXT_LIMIT);
  });
});

describe("mergeCspViolations", () => {
  const a: ArtifactCspViolation = { directive: "script-src", blockedUri: "https://cdn.example/x.js" };
  const b: ArtifactCspViolation = { directive: "img-src", blockedUri: "http://example/a.png" };

  it("発生順を保って追加する", () => {
    expect(mergeCspViolations([a], [b])).toEqual([a, b]);
  });

  it("ディレクティブと URL が同じものは1件にまとめる", () => {
    expect(mergeCspViolations([a], [{ ...a }, b])).toEqual([a, b]);
  });

  it("同じ URL でもディレクティブが違えば別件として扱う", () => {
    const sameUri: ArtifactCspViolation = { directive: "img-src", blockedUri: a.blockedUri };
    expect(mergeCspViolations([a], [sameUri])).toEqual([a, sameUri]);
  });

  it("上限を超えて溜め込まない", () => {
    const many = Array.from({ length: ARTIFACT_CSP_VIOLATION_LIMIT + 10 }, (_, i) => ({
      directive: "img-src",
      blockedUri: `https://example/${i}.png`,
    }));
    expect(mergeCspViolations([], many)).toHaveLength(ARTIFACT_CSP_VIOLATION_LIMIT);
  });

  it("元の配列を書き換えない", () => {
    const current = [a];
    mergeCspViolations(current, [b]);
    expect(current).toEqual([a]);
  });
});
