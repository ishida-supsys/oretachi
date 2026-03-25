<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { useCat } from "../composables/useCat";

const { t } = useI18n();
const containerRef = ref<HTMLDivElement | null>(null);

const { start, stop, redraw, topic } = useCat();

let terminal: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let resizeObserver: ResizeObserver | null = null;
let meowTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleMeow() {
  const delay = 15000 + Math.random() * 10000; // 15〜25秒
  meowTimer = setTimeout(() => {
    const choices = [t("meow1"), t("meow2")];
    topic(choices[Math.floor(Math.random() * choices.length)], 0);
    scheduleMeow();
  }, delay);
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

  resizeObserver = new ResizeObserver(() => {
    if (containerRef.value && containerRef.value.offsetWidth > 0) {
      fitAddon?.fit();
      redraw();
    }
  });
  resizeObserver.observe(containerRef.value);
});

onUnmounted(() => {
  if (meowTimer) { clearTimeout(meowTimer); meowTimer = null; }
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

.terminal-container :deep(.xterm) {
  height: 100%;
  padding: 4px;
  background-color: var(--bg-terminal) !important;
}

.home-cat-terminal :deep(.xterm-viewport) {
  overflow: hidden !important;
  background-color: var(--bg-terminal) !important;
}

.home-cat-terminal :deep(.xterm-helper-textarea) {
  display: none;
}
</style>

<i18n lang="json">
{
  "en": {
    "meow1": "Meow",
    "meow2": "Mew"
  },
  "ja": {
    "meow1": "ニャー",
    "meow2": "ニャン"
  }
}
</i18n>
