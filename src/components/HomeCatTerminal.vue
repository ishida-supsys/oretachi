<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { invoke } from "@tauri-apps/api/core";
import { useCat } from "../composables/useCat";

interface ReportSummary {
  worktree_added: number;
  worktree_removed: number;
  artifact_added: number;
  artifact_removed: number;
  ai_result_count: number;
}

const { t } = useI18n();
const containerRef = ref<HTMLDivElement | null>(null);

const { start, stop, redraw, topic } = useCat();

let terminal: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let resizeObserver: ResizeObserver | null = null;
let meowTimer: ReturnType<typeof setTimeout> | null = null;
let reportTimer: ReturnType<typeof setInterval> | null = null;

function scheduleMeow() {
  const delay = 15000 + Math.random() * 10000; // 15〜25秒
  meowTimer = setTimeout(() => {
    const choices = [t("meow1"), t("meow2")];
    topic(choices[Math.floor(Math.random() * choices.length)], 0);
    scheduleMeow();
  }, delay);
}

async function fetchReport() {
  try {
    const now = new Date();
    const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
    const tzOffsetMin = now.getTimezoneOffset();
    const summary = await invoke<ReportSummary>("get_report_summary", { date: today, tzOffsetMin });
    if (summary.worktree_added > 0 || summary.worktree_removed > 0) {
      topic(
        t("reportWt", {
          added: summary.worktree_added,
          removed: summary.worktree_removed,
        }),
        0,
      );
    }
  } catch {
    // レポートDB未初期化などは無視
  }
}

onMounted(() => {
  if (!containerRef.value) return;

  terminal = new Terminal({
    allowProposedApi: true,
    allowTransparency: true,
    disableStdin: true,
    cursorBlink: false,
    cursorStyle: "bar",
    cursorInactiveStyle: "none",
    scrollback: 0,
    fontFamily: '"Cascadia Code", Consolas, Menlo, "SF Mono", Monaco, monospace',
    fontSize: 14,
    theme: {
      background: "#00000000",
      foreground: "#7d86a4",
      cursor: "#f5e0dc",
      cursorAccent: "#1e1e2e",
      black: "#45475a",
      red: "#f38ba8",
      green: "#a6e3a1",
      yellow: "#f9e2af",
      blue: "#89b4fa",
      magenta: "#f5c2e7",
      cyan: "#94e2d5",
      white: "#bac2de",
      brightBlack: "#585b70",
      brightRed: "#f38ba8",
      brightGreen: "#a6e3a1",
      brightYellow: "#f9e2af",
      brightBlue: "#89b4fa",
      brightMagenta: "#f5c2e7",
      brightCyan: "#94e2d5",
      brightWhite: "#a6adc8",
    },
  });

  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);

  terminal.open(containerRef.value);
  fitAddon.fit();
  start(terminal);
  scheduleMeow();
  reportTimer = setInterval(fetchReport, 60000);

  resizeObserver = new ResizeObserver(() => {
    if (containerRef.value && containerRef.value.offsetWidth > 0) {
      fitAddon?.fit();
      terminal?.write('\x1b[2J'); // バッファリフローによるアーティファクトをクリア
      redraw();
    }
  });
  resizeObserver.observe(containerRef.value);
});

onUnmounted(() => {
  if (meowTimer) { clearTimeout(meowTimer); meowTimer = null; }
  if (reportTimer) { clearInterval(reportTimer); reportTimer = null; }
  stop();
  resizeObserver?.disconnect();
  terminal?.dispose();
  terminal = null;
});

defineExpose({ topic });
</script>

<template>
  <div ref="containerRef" class="home-cat-terminal" />
</template>

<style scoped>
.home-cat-terminal {
  width: 100%;
  height: 100%;
}

.home-cat-terminal :deep(.xterm-viewport) {
  overflow: hidden !important;
  background-color: transparent !important;
}

.home-cat-terminal :deep(.xterm-helper-textarea) {
  display: none;
}
</style>

<i18n lang="json">
{
  "en": {
    "meow1": "Meow",
    "meow2": "Mew",
    "reportWt": "Today: WT+{added}/-{removed}, Meow"
  },
  "ja": {
    "meow1": "ニャー",
    "meow2": "ニャン",
    "reportWt": "今日はWT{added}件追加{removed}件削除された、ニャー"
  }
}
</i18n>
