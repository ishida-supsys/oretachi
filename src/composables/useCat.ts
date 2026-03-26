import { onUnmounted } from "vue";
import type { Terminal } from "@xterm/xterm";

// 猫AA最大表示幅: " じしˍ,)ノ" = 1+2(じ)+2(し)+1(ˍ)+1(,)+1())+2(ノ) = 10列
const CAT_DISPLAY_WIDTH = 10;
const BLINK_DURATION_MS = 200;
const REDRAW_INTERVAL_MS = 1000;
const BLINK_MIN_MS = 7000;
const BLINK_MAX_MS = 13000;

// セリフ関連
const TYPING_INTERVAL_MS = 75;    // 文字送り速度
const HOLDING_DURATION_MS = 5000; // 全文表示後の保持時間
const IDLE_RECHECK_MS = 2000;     // トピックなし時の再チェック間隔
const MAX_QUEUE_SIZE = 20;
const MAX_SPEECH_WIDTH = 40;      // セリフ1行の最大表示列数
const MAX_SPEECH_LINES = 3;       // セリフ最大行数

const catFrames = {
  normal: [" ∧_∧", "( o.o )", " じしˍ,)ノ"],
  blink: [" ∧_∧", "( -.- )", " じしˍ,)ノ"],
};

// CJK・全角文字を2列として表示幅を計算
function measureWidth(text: string): number {
  let w = 0;
  for (const ch of text) {
    const cp = ch.codePointAt(0) ?? 0;
    if (
      (cp >= 0x1100 && cp <= 0x115f) ||                 // Hangul Jamo
      (cp >= 0x2e80 && cp <= 0xa4cf && cp !== 0x303f) || // CJK統合・部首・記号
      (cp >= 0xac00 && cp <= 0xd7a3) ||                 // Hangul Syllables
      (cp >= 0xf900 && cp <= 0xfaff) ||                 // CJK Compatibility
      (cp >= 0xfe10 && cp <= 0xfe6f) ||                 // CJK Forms
      (cp >= 0xff01 && cp <= 0xff60) ||                 // Fullwidth Forms
      (cp >= 0xffe0 && cp <= 0xffe6) ||                 // Fullwidth Signs
      (cp >= 0x20000 && cp <= 0x2fffd)                  // CJK Ext B+
    ) {
      w += 2;
    } else {
      w += 1;
    }
  }
  return w;
}

interface Topic {
  text: string;
  priority: number;
  createdAt: number;
}

type SpeechState = "idle" | "typing" | "holding";

export function useCat() {
  let terminal: Terminal | null = null;

  // 猫アニメーション状態
  let isBlinking = false;
  let redrawTimer: ReturnType<typeof setInterval> | null = null;
  let blinkScheduleTimer: ReturnType<typeof setTimeout> | null = null;
  let blinkRestoreTimer: ReturnType<typeof setTimeout> | null = null;

  // セリフ状態
  const topics: Topic[] = [];
  let speechState: SpeechState = "idle";
  let currentSpeechText = "";          // 全文字カウント用（折り返し前結合）
  let currentSpeechLines: string[] = []; // 折り返し済み行配列
  let displayedCharCount = 0;          // 文字送りで何文字まで表示したか
  let speechTimer: ReturnType<typeof setTimeout> | null = null;

  // ----- 座標 -----

  function getPos() {
    if (!terminal) return null;
    const { rows, cols } = terminal;
    return {
      startRow: rows - 2, // 1-based ANSI; 猫3行 = rows-2, rows-1, rows
      startCol: Math.max(1, cols - CAT_DISPLAY_WIDTH + 1),
      speechRow: rows - 3, // セリフ最下行（猫の1行上）
    };
  }

  // ----- 猫本体の描画 -----

  function drawCatFrame(lines: string[]) {
    if (!terminal || terminal.rows < 6 || terminal.cols < CAT_DISPLAY_WIDTH) return;
    const pos = getPos();
    if (!pos) return;
    const { startRow, startCol } = pos;
    let seq = "\x1b7";
    for (let i = 0; i < lines.length; i++) {
      seq += `\x1b[${startRow + i};${startCol}H${lines[i]}`;
    }
    seq += "\x1b8";
    terminal.write(seq);
  }

  // ----- テキスト折り返し -----

  // テキストをUnicode文字単位の配列に分解
  function splitChars(text: string): string[] {
    return [...text]; // スプレッド構文でサロゲートペア対応
  }

  // テキストを MAX_SPEECH_WIDTH 列で折り返し、最大 MAX_SPEECH_LINES 行に分割
  function wrapText(text: string): string[] {
    const lines: string[] = [];
    let line = "";
    let lineWidth = 0;
    for (const ch of splitChars(text)) {
      const chWidth = measureWidth(ch);
      if (lineWidth + chWidth > MAX_SPEECH_WIDTH && line.length > 0) {
        lines.push(line);
        if (lines.length >= MAX_SPEECH_LINES) return lines; // 上限に達したら打ち切り
        line = ch;
        lineWidth = chWidth;
      } else {
        line += ch;
        lineWidth += chWidth;
      }
    }
    if (line.length > 0) lines.push(line);
    return lines;
  }

  // ----- セリフの描画/消去 -----

  // lines の先頭から displayedCount 文字分を複数行に描画する
  function drawSpeechLines(lines: string[], displayedCount: number) {
    if (!terminal || terminal.rows < 6 || terminal.cols < MAX_SPEECH_WIDTH || lines.length === 0) return;
    const pos = getPos();
    if (!pos) return;

    const baseCol = Math.max(1, pos.startCol + CAT_DISPLAY_WIDTH - MAX_SPEECH_WIDTH - 4);
    const rightEdge = pos.startCol + CAT_DISPLAY_WIDTH - 4; // 右詰め基準列
    let seq = "\x1b7";
    let remaining = displayedCount;

    for (let i = 0; i < lines.length; i++) {
      const row = pos.speechRow - (lines.length - 1 - i);
      if (row < 1) continue;
      const lineChars = splitChars(lines[i]);
      const charsOnLine = Math.min(remaining, lineChars.length);
      remaining = Math.max(0, remaining - lineChars.length);
      const partial = lineChars.slice(0, charsOnLine).join("");
      // 1行のみの場合は右詰め、複数行は固定左端
      const col = lines.length === 1
        ? Math.max(1, rightEdge - measureWidth(partial))
        : baseCol;
      // クリア→描画 (折り返し防止のため端末幅にクランプ)
      const clearWidth = Math.min(MAX_SPEECH_WIDTH, terminal.cols - baseCol + 1);
      seq += `\x1b[${row};${baseCol}H${" ".repeat(clearWidth)}`;
      seq += `\x1b[${row};${col}H${partial}`;
    }
    seq += "\x1b8";
    terminal.write(seq);
  }

  // MAX_SPEECH_LINES 行分をクリア（行数に関わらず常に最大行数分消す）
  function clearSpeechLines() {
    if (!terminal || terminal.rows < 6 || terminal.cols < MAX_SPEECH_WIDTH) return;
    const pos = getPos();
    if (!pos) return;

    const baseCol = Math.max(1, pos.startCol + CAT_DISPLAY_WIDTH - MAX_SPEECH_WIDTH - 4);
    let seq = "\x1b7";
    for (let i = 0; i < MAX_SPEECH_LINES; i++) {
      const row = pos.speechRow - (MAX_SPEECH_LINES - 1 - i);
      if (row < 1) continue;
      const clearWidth = Math.min(MAX_SPEECH_WIDTH, terminal.cols - baseCol + 1);
      seq += `\x1b[${row};${baseCol}H${" ".repeat(clearWidth)}`;
    }
    seq += "\x1b8";
    terminal.write(seq);
  }

  // ----- Topic キュー -----

  function topic(text: string, priority: number = 0) {
    topics.push({ text, priority, createdAt: Date.now() });
    if (topics.length > MAX_QUEUE_SIZE) {
      // 優先度最低・最古を1件破棄
      topics.sort((a, b) => a.priority - b.priority || a.createdAt - b.createdAt);
      topics.shift();
    }
  }

  function pickNextTopic(): Topic | null {
    if (topics.length === 0) return null;
    topics.sort((a, b) => b.priority - a.priority || b.createdAt - a.createdAt);
    return topics.splice(0, 1)[0];
  }

  // ----- セリフ状態遷移 -----

  function clearSpeechTimer() {
    if (speechTimer) { clearTimeout(speechTimer); speechTimer = null; }
  }

  function advanceSpeech() {
    clearSpeechTimer();

    switch (speechState) {
      case "idle": {
        const next = pickNextTopic();
        if (next) {
          currentSpeechLines = wrapText(next.text);
          currentSpeechText = currentSpeechLines.join(""); // 文字カウント用
          displayedCharCount = 0;
          speechState = "typing";
          speechTimer = setTimeout(advanceSpeech, TYPING_INTERVAL_MS);
        } else {
          clearSpeechLines();
          speechTimer = setTimeout(advanceSpeech, IDLE_RECHECK_MS);
        }
        break;
      }

      case "typing": {
        const totalChars = splitChars(currentSpeechText).length;
        displayedCharCount = Math.min(displayedCharCount + 1, totalChars);
        drawSpeechLines(currentSpeechLines, displayedCharCount);

        if (displayedCharCount >= totalChars) {
          speechState = "holding";
          speechTimer = setTimeout(advanceSpeech, HOLDING_DURATION_MS);
        } else {
          speechTimer = setTimeout(advanceSpeech, TYPING_INTERVAL_MS);
        }
        break;
      }

      case "holding": {
        // セリフ表示後はキューをリセット（連続して喋らない）
        topics.length = 0;
        speechState = "idle";
        speechTimer = setTimeout(advanceSpeech, 0);
        break;
      }
    }
  }

  // ----- 瞬きスケジューラ -----

  function scheduleNextBlink() {
    const delay = BLINK_MIN_MS + Math.random() * (BLINK_MAX_MS - BLINK_MIN_MS);
    blinkScheduleTimer = setTimeout(() => {
      isBlinking = true;
      drawCatFrame(catFrames.blink);
      blinkRestoreTimer = setTimeout(() => {
        isBlinking = false;
        drawCatFrame(catFrames.normal);
        scheduleNextBlink();
      }, BLINK_DURATION_MS);
    }, delay);
  }

  // ----- ライフサイクル -----

  function start(term: Terminal) {
    terminal = term;
    drawCatFrame(catFrames.normal);
    scheduleNextBlink();
    advanceSpeech();

    redrawTimer = setInterval(() => {
      // 猫本体
      if (!isBlinking) drawCatFrame(catFrames.normal);
      // セリフ（表示中なら再描画してターミナル出力による上書きから復元）
      if (speechState === "typing" || speechState === "holding") {
        drawSpeechLines(currentSpeechLines, displayedCharCount);
      }
    }, REDRAW_INTERVAL_MS);
  }

  function stop() {
    if (redrawTimer) { clearInterval(redrawTimer); redrawTimer = null; }
    if (blinkScheduleTimer) { clearTimeout(blinkScheduleTimer); blinkScheduleTimer = null; }
    if (blinkRestoreTimer) { clearTimeout(blinkRestoreTimer); blinkRestoreTimer = null; }
    clearSpeechTimer();
    terminal = null;
  }

  function redraw() {
    if (!isBlinking) drawCatFrame(catFrames.normal);
    if (speechState === "typing" || speechState === "holding") {
      drawSpeechLines(currentSpeechLines, displayedCharCount);
    }
  }

  onUnmounted(stop);

  return { start, stop, redraw, topic };
}
