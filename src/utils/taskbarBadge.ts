/** タスクバーのオーバーレイに出すバッジ文字列。0 件なら null（オーバーレイを消す）。 */
export function badgeLabel(count: number): string | null {
  if (!(count > 0)) return null;
  return count >= 10 ? "9+" : String(Math.floor(count));
}

/** 赤丸＋白字のバッジを描き、RGBA のバイト列を返す。 */
export function renderBadgeRgba(label: string, size = 32): Uint8Array {
  const canvas = new OffscreenCanvas(size, size);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("2d context unavailable");
  ctx.fillStyle = "#ef4444"; // PrimeVue danger 相当
  ctx.beginPath();
  ctx.arc(size / 2, size / 2, size / 2, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "#ffffff";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  const fontPx = Math.round(label.length > 1 ? size * 0.55 : size * 0.7);
  ctx.font = `bold ${fontPx}px sans-serif`;
  ctx.fillText(label, size / 2, size / 2 + size * 0.04);
  return new Uint8Array(ctx.getImageData(0, 0, size, size).data.buffer);
}
