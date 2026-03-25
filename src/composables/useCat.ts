import { onUnmounted } from "vue";
import type { Terminal } from "@xterm/xterm";

// 猫AA最大表示幅: " じしˍ,)ノ" = 1+2(じ)+2(し)+1(ˍ)+1(,)+1())+2(ノ) = 10列
const CAT_DISPLAY_WIDTH = 10;
const BLINK_DURATION_MS = 200;
const REDRAW_INTERVAL_MS = 1000;
const BLINK_MIN_MS = 7000;
const BLINK_MAX_MS = 13000;

// セリフ関連
const TYPING_INTERVAL_MS = 150;   // 文字送り速度
const HOLDING_DURATION_MS = 3000; // 全文表示後の保持時間
const IDLE_RECHECK_MS = 2000;     // トピックなし時の再チェック間隔
const MAX_QUEUE_SIZE = 20;
const MAX_SPEECH_WIDTH = 24;      // セリフ枠込みの最大表示列数

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
  let currentSpeechText = "";
  let displayedCharCount = 0; // 文字送りで何文字まで表示したか（Unicode文字単位）
  let speechTimer: ReturnType<typeof setTimeout> | null = null;

  // ----- 座標 -----

  function getPos() {
    if (!terminal) return null;
    const { rows, cols } = terminal;
    return {
      startRow: rows - 2, // 1-based ANSI; 猫3行 = rows-2, rows-1, rows
      startCol: Math.max(1, cols - CAT_DISPLAY_WIDTH + 1),
      speechRow: rows - 3, // 猫の1行上
    };
  }

  // ----- 猫本体の描画 -----

  function drawCatFrame(lines: string[]) {
    if (!terminal) return;
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

  // ----- セリフの描画/消去 -----

  function drawSpeechLine(text: string) {
    if (!terminal) return;
    const pos = getPos();
    if (!pos) return;
    if (pos.speechRow < 1) return;

    const framed = `< ${text} >`;
    const framedWidth = measureWidth(framed);
    // 右端を猫の右端に揃える
    const speechCol = Math.max(
      1,
      pos.startCol + CAT_DISPLAY_WIDTH - framedWidth,
    );

    let seq = "\x1b7";
    // 先に最大幅分をスペースで消去してから描画（長さ変化による残骸を防ぐ）
    const clearWidth = MAX_SPEECH_WIDTH + 2;
    const clearCol = Math.max(1, pos.startCol + CAT_DISPLAY_WIDTH - clearWidth);
    seq += `\x1b[${pos.speechRow};${clearCol}H${" ".repeat(clearWidth)}`;
    seq += `\x1b[${pos.speechRow};${speechCol}H${framed}`;
    seq += "\x1b8";
    terminal.write(seq);
  }

  function clearSpeechLine() {
    if (!terminal) return;
    const pos = getPos();
    if (!pos) return;
    if (pos.speechRow < 1) return;

    const clearWidth = MAX_SPEECH_WIDTH + 2;
    const clearCol = Math.max(1, pos.startCol + CAT_DISPLAY_WIDTH - clearWidth);
    let seq = "\x1b7";
    seq += `\x1b[${pos.speechRow};${clearCol}H${" ".repeat(clearWidth)}`;
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

  // テキストをUnicode文字単位の配列に分解
  function splitChars(text: string): string[] {
    return [...text]; // スプレッド構文でサロゲートペア対応
  }

  function advanceSpeech() {
    clearSpeechTimer();

    switch (speechState) {
      case "idle": {
        const next = pickNextTopic();
        if (next) {
          // テキストが長すぎる場合は MAX_SPEECH_WIDTH に収まる範囲で截断
          let truncated = "";
          for (const ch of splitChars(next.text)) {
            const candidate = truncated + ch;
            if (measureWidth(`< ${candidate} >`) > MAX_SPEECH_WIDTH) break;
            truncated = candidate;
          }
          currentSpeechText = truncated;
          displayedCharCount = 0;
          speechState = "typing";
          speechTimer = setTimeout(advanceSpeech, TYPING_INTERVAL_MS);
        } else {
          clearSpeechLine();
          speechTimer = setTimeout(advanceSpeech, IDLE_RECHECK_MS);
        }
        break;
      }

      case "typing": {
        const chars = splitChars(currentSpeechText);
        displayedCharCount = Math.min(displayedCharCount + 1, chars.length);
        const partial = chars.slice(0, displayedCharCount).join("");
        drawSpeechLine(partial);

        if (displayedCharCount >= chars.length) {
          speechState = "holding";
          speechTimer = setTimeout(advanceSpeech, HOLDING_DURATION_MS);
        } else {
          speechTimer = setTimeout(advanceSpeech, TYPING_INTERVAL_MS);
        }
        break;
      }

      case "holding": {
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
        const partial = splitChars(currentSpeechText)
          .slice(0, displayedCharCount)
          .join("");
        drawSpeechLine(partial);
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
      const partial = splitChars(currentSpeechText)
        .slice(0, displayedCharCount)
        .join("");
      drawSpeechLine(partial);
    }
  }

  onUnmounted(stop);

  return { start, stop, redraw, topic };
}
