import type { Terminal } from "@xterm/xterm";
import { hexToRgb, xtermRgbToRgb, paletteToRgb } from "./useAnsiPalette";

// デフォルト色 (Catppuccin Mocha)
const DEFAULT_FG = hexToRgb("#cdd6f4");
const DEFAULT_BG = hexToRgb("#1e1e2e");

// セルサイズ: Monaco ミニマップ風
const CELL_W = 1;
const CELL_H = 2;

// FNV-1a 32bit ハッシュ (差分検出用)
function fnv1a(data: Uint8ClampedArray): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < data.length; i++) {
    hash ^= data[i];
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

const sharedCanvas = document.createElement("canvas");

export function renderToDataUrl(terminal: Terminal): string | null {
  const cols = terminal.cols;
  const rows = terminal.rows;
  if (cols <= 1 || rows <= 1) return null;

  const buffer = terminal.buffer.active;
  const canvasW = cols * CELL_W;
  const canvasH = rows * CELL_H;

  if (sharedCanvas.width !== canvasW || sharedCanvas.height !== canvasH) {
    sharedCanvas.width = canvasW;
    sharedCanvas.height = canvasH;
  }

  const ctx = sharedCanvas.getContext("2d");
  if (!ctx) return null;

  const nullCell = buffer.getNullCell();
  const startRow = buffer.viewportY;

  for (let row = 0; row < rows; row++) {
    const line = buffer.getLine(startRow + row);
    if (!line) continue;

    const rowData = new Uint8ClampedArray(cols * CELL_W * CELL_H * 4);

    for (let col = 0; col < cols; col++) {
      const cell = line.getCell(col, nullCell);
      if (!cell) continue;

      const chars = cell.getChars();
      const hasChar = chars !== "" && chars !== " ";

      let r: number, g: number, b: number;

      if (hasChar) {
        if (cell.isFgDefault()) {
          [r, g, b] = DEFAULT_FG;
        } else if (cell.isFgRGB()) {
          [r, g, b] = xtermRgbToRgb(cell.getFgColor());
        } else {
          [r, g, b] = paletteToRgb(cell.getFgColor());
        }
      } else {
        if (cell.isBgDefault()) {
          [r, g, b] = DEFAULT_BG;
        } else if (cell.isBgRGB()) {
          [r, g, b] = xtermRgbToRgb(cell.getBgColor());
        } else {
          [r, g, b] = paletteToRgb(cell.getBgColor());
        }
      }

      for (let py = 0; py < CELL_H; py++) {
        const offset = (py * cols + col) * 4;
        rowData[offset] = r;
        rowData[offset + 1] = g;
        rowData[offset + 2] = b;
        rowData[offset + 3] = 255;
      }
    }

    const imageData = new ImageData(rowData, cols * CELL_W, CELL_H);
    ctx.putImageData(imageData, 0, row * CELL_H);
  }

  return sharedCanvas.toDataURL();
}

export function useTerminalThumbnail() {
  let prevHashes: number[] = [];

  function renderToCanvas(canvas: HTMLCanvasElement, terminal: Terminal): boolean {
    const cols = terminal.cols;
    const rows = terminal.rows;

    if (cols <= 1 || rows <= 1) return false;

    const buffer = terminal.buffer.active;

    const canvasW = cols * CELL_W;
    const canvasH = rows * CELL_H;

    // キャンバスサイズが変わった場合はリセット
    if (canvas.width !== canvasW || canvas.height !== canvasH) {
      canvas.width = canvasW;
      canvas.height = canvasH;
      prevHashes = [];
    }

    const ctx = canvas.getContext("2d");
    if (!ctx) return false;

    // セルオブジェクト再利用でGC圧を抑制
    const nullCell = buffer.getNullCell();
    const startRow = buffer.viewportY;

    for (let row = 0; row < rows; row++) {
      const line = buffer.getLine(startRow + row);
      if (!line) continue;

      // 1行分のピクセルデータ (cols × CELL_H × 4 bytes)
      const rowData = new Uint8ClampedArray(cols * CELL_W * CELL_H * 4);

      for (let col = 0; col < cols; col++) {
        const cell = line.getCell(col, nullCell);
        if (!cell) continue;

        const chars = cell.getChars();
        const hasChar = chars !== "" && chars !== " ";

        let r: number, g: number, b: number;

        if (hasChar) {
          // 文字ありセル → 前景色
          if (cell.isFgDefault()) {
            [r, g, b] = DEFAULT_FG;
          } else if (cell.isFgRGB()) {
            [r, g, b] = xtermRgbToRgb(cell.getFgColor());
          } else {
            [r, g, b] = paletteToRgb(cell.getFgColor());
          }
        } else {
          // 文字なしセル → 背景色
          if (cell.isBgDefault()) {
            [r, g, b] = DEFAULT_BG;
          } else if (cell.isBgRGB()) {
            [r, g, b] = xtermRgbToRgb(cell.getBgColor());
          } else {
            [r, g, b] = paletteToRgb(cell.getBgColor());
          }
        }

        // CELL_H行分 (2px) を同色で埋める
        for (let py = 0; py < CELL_H; py++) {
          const offset = (py * cols + col) * 4;
          rowData[offset] = r;
          rowData[offset + 1] = g;
          rowData[offset + 2] = b;
          rowData[offset + 3] = 255;
        }
      }

      // 差分検出: 変更なし行はスキップ
      const hash = fnv1a(rowData);
      if (prevHashes[row] === hash) continue;
      prevHashes[row] = hash;

      const imageData = new ImageData(rowData, cols * CELL_W, CELL_H);
      ctx.putImageData(imageData, 0, row * CELL_H);
    }

    return true;
  }

  function dispose() {
    prevHashes = [];
  }

  return { renderToCanvas, dispose };
}
