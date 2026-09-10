/**
 * SVG を PNG のバイト列に焼く。issue へ直接貼れるよう、クリップボードへ画像として
 * 載せるために使う（SVG / mermaid アーティファクトのエクスポート導線）。
 */

/** PNG に焼けなかったときのサイズ既定値（width/height も viewBox も無い SVG 用） */
const FALLBACK_SIZE = { width: 1200, height: 800 };

/** 長辺の上限。巨大な図をそのまま等倍×scale で焼くとメモリを食い潰すため */
const MAX_EDGE = 8000;

/**
 * SVG のルート要素から描画サイズを取る。`width`/`height` が px で入っていればそれを、
 * 無ければ `viewBox` の寸法を使う。どちらも読めなければ既定値。
 */
export function parseSvgSize(svg: string): { width: number; height: number } {
  // DOM を使わないのは、この関数だけ node のテストから呼びたいため。
  // 見るのはルート `<svg>` の属性 3 つだけなので、開始タグを切り出せば足りる。
  const openTag = svg.match(/<svg\b[^>]*>/i)?.[0];
  if (!openTag) return { ...FALLBACK_SIZE };

  const attr = (name: string): string | null =>
    openTag.match(new RegExp(`\\b${name}\\s*=\\s*["']([^"']*)["']`, "i"))?.[1] ?? null;

  const asPx = (value: string | null): number | null => {
    if (!value) return null;
    const num = Number.parseFloat(value.replace(/px$/, ""));
    // 百分率 (`100%`) は親のサイズ依存なので px として使えない
    return Number.isFinite(num) && num > 0 && !value.includes("%") ? num : null;
  };

  const width = asPx(attr("width"));
  const height = asPx(attr("height"));
  if (width && height) return { width, height };

  const viewBox = attr("viewBox");
  if (viewBox) {
    const parts = viewBox.split(/[\s,]+/).map(Number);
    if (parts.length === 4 && parts[2] > 0 && parts[3] > 0) {
      return { width: parts[2], height: parts[3] };
    }
  }
  // 片方だけ読めても縦横比が分からない。既定値と混ぜると歪んだ PNG になるので、
  // 「サイズ不明」として既定値で揃える
  return { ...FALLBACK_SIZE };
}

/**
 * 出力の長辺が `MAX_EDGE` を超えないよう倍率を下げる。
 * 元寸だけで上限を超える巨大な図では 1 倍未満まで下げる（canvas の面積上限に当たると
 * `toBlob` が null を返して PNG 化そのものが落ちるため）。
 */
export function clampScale(width: number, height: number, scale: number): number {
  const edge = Math.max(width, height);
  if (edge <= 0) return scale;
  return Math.min(scale, MAX_EDGE / edge);
}

/**
 * SVG テキストを PNG のバイト列へ。背景は透過ではなくビューアと同じ濃色で塗る
 * （透過のまま issue へ貼ると、ライトテーマの本文で文字が読めなくなるため）。
 */
export async function svgToPngBytes(
  svg: string,
  options?: { scale?: number; background?: string },
): Promise<Uint8Array> {
  const { width, height } = parseSvgSize(svg);
  const scale = clampScale(width, height, options?.scale ?? 2);
  const background = options?.background ?? "#1e1e2e";

  const url = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  const image = await new Promise<HTMLImageElement>((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("SVG を画像として読み込めませんでした"));
    img.src = url;
  });

  const canvas = document.createElement("canvas");
  canvas.width = Math.round(width * scale);
  canvas.height = Math.round(height * scale);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("canvas を初期化できませんでした");
  ctx.fillStyle = background;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(image, 0, 0, canvas.width, canvas.height);

  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("PNG へ変換できませんでした");
  return new Uint8Array(await blob.arrayBuffer());
}
