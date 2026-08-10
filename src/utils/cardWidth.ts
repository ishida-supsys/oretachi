/** TerminalThumbnail の固定幅 */
const THUMBNAIL_W = 107;
/** .terminals-row の gap */
const THUMBNAIL_GAP = 8;
/** カードの padding 12px × 2 */
const CARD_PADDING = 24;
/** ヘッダーボタンが収まる最小幅 */
const MIN_WIDTH = 260;
/** 1行に表示するターミナルの最大数 */
const MAX_TERMINALS_PER_ROW = 2;

/**
 * カード一覧（masonry）の列幅を、各カードが持つターミナル数から算出する。
 * サムネイルが折り返さずに収まる自然幅の最大値を採る。
 */
export function computeNaturalCardWidth(terminalCounts: number[]): number {
  let max = MIN_WIDTH;
  for (const count of terminalCounts) {
    const n = Math.min(count, MAX_TERMINALS_PER_ROW);
    if (n <= 0) continue;
    const w = CARD_PADDING + n * THUMBNAIL_W + (n - 1) * THUMBNAIL_GAP;
    if (w > max) max = w;
  }
  return max;
}
