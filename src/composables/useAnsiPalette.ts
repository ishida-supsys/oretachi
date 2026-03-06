// Catppuccin Mocha テーマ色 (TerminalView.vue のテーマ定義と一致)
const ANSI_0_15: [number, number, number][] = [
  [0x45, 0x47, 0x5a], // 0  black
  [0xf3, 0x8b, 0xa8], // 1  red
  [0xa6, 0xe3, 0xa1], // 2  green
  [0xf9, 0xe2, 0xaf], // 3  yellow
  [0x89, 0xb4, 0xfa], // 4  blue
  [0xf5, 0xc2, 0xe7], // 5  magenta
  [0x94, 0xe2, 0xd5], // 6  cyan
  [0xba, 0xc2, 0xde], // 7  white
  [0x58, 0x5b, 0x70], // 8  brightBlack
  [0xf3, 0x8b, 0xa8], // 9  brightRed
  [0xa6, 0xe3, 0xa1], // 10 brightGreen
  [0xf9, 0xe2, 0xaf], // 11 brightYellow
  [0x89, 0xb4, 0xfa], // 12 brightBlue
  [0xf5, 0xc2, 0xe7], // 13 brightMagenta
  [0x94, 0xe2, 0xd5], // 14 brightCyan
  [0xa6, 0xad, 0xc8], // 15 brightWhite
];

// 256色パレット構築
const PALETTE: [number, number, number][] = new Array(256);

// 0-15: ターミナルテーマ色
for (let i = 0; i < 16; i++) {
  PALETTE[i] = ANSI_0_15[i];
}

// 16-231: 6x6x6 カラーキューブ
for (let i = 0; i < 216; i++) {
  const r = Math.floor(i / 36);
  const g = Math.floor((i % 36) / 6);
  const b = i % 6;
  PALETTE[16 + i] = [
    r === 0 ? 0 : 55 + r * 40,
    g === 0 ? 0 : 55 + g * 40,
    b === 0 ? 0 : 55 + b * 40,
  ];
}

// 232-255: グレースケール 24段階
for (let i = 0; i < 24; i++) {
  const v = 8 + i * 10;
  PALETTE[232 + i] = [v, v, v];
}

export function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

export function xtermRgbToRgb(rgb: number): [number, number, number] {
  return [(rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff];
}

export function paletteToRgb(index: number): [number, number, number] {
  return PALETTE[index & 0xff] ?? [0, 0, 0];
}
