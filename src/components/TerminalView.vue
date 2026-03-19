<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, nextTick } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { SerializeAddon } from "@xterm/addon-serialize";
import { usePty } from "../composables/usePty";
import { usePtyWriteBatcher } from "../composables/usePtyWriteBatcher";
import { useSettings } from "../composables/useSettings";
import { matchesHotkey } from "../composables/useHotkeys";
import { useTerminalSearch } from "../composables/useTerminalSearch";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { useI18n } from "vue-i18n";
import { debug } from "@tauri-apps/plugin-log";

const { t } = useI18n();

const props = withDefaults(
  defineProps<{
    shell?: string;
    autoStart?: boolean;
    cwd?: string;
    initialSessionId?: number;
    initialSnapshot?: string;
    restoreSnapshot?: string;
    noResize?: boolean;
    initialCols?: number;
    initialRows?: number;
  }>(),
  {
    autoStart: true,
    noResize: false,
  }
);

const emit = defineEmits<{
  exit: [];
  ready: [];
  "title-change": [title: string];
  "exit-code-change": [exitCode: number];
  focus: [];
}>();

const containerRef = ref<HTMLDivElement | null>(null);
const xtermRef = ref<HTMLDivElement | null>(null);

const search = useTerminalSearch(() => terminal);

let terminal: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let serializeAddon: SerializeAddon | null = null;
let resizeObserver: ResizeObserver | null = null;
let resizeDebounce: ReturnType<typeof setTimeout> | null = null;

const { sessionId, spawn, attachToSession, write, resize, kill, isRunning, detach } = usePty();
const batcher = usePtyWriteBatcher(() => terminal);
const { settings } = useSettings();

async function handlePaste() {
  try {
    const text = await readText();
    if (text && terminal) {
      if (terminal.modes.bracketedPasteMode) {
        write(`\x1b[200~${text}\x1b[201~`);
      } else {
        write(text);
      }
    }
  } catch (err) {
    console.error("クリップボード読み取りに失敗:", err);
  }
}

function initTerminal() {
  if (!xtermRef.value) return;

  terminal = new Terminal({
    allowProposedApi: true,
    allowTransparency: true,
    cols: props.initialCols,
    rows: props.initialRows,
    fontFamily: '"Cascadia Code", Consolas, Menlo, "SF Mono", Monaco, monospace',
    fontSize: settings.value.terminal.fontSize,
    theme: {
      background: "#1e1e2e00",
      foreground: "#cdd6f4",
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
    scrollback: 10000,
  });

  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);

  serializeAddon = new SerializeAddon();
  terminal.loadAddon(serializeAddon);

  search.loadAddon(terminal);

  // WebGL addon (失敗時は Canvas フォールバック)
  try {
    const webglAddon = new WebglAddon();
    webglAddon.onContextLoss(() => {
      console.warn("[XTERM] WebGL onContextLoss fired!", { sessionId: sessionId.value });
      webglAddon.dispose();
      // dispose 後は xterm.js が自動で Canvas レンダラーにフォールバック。
      // 明示的に refresh を呼んで Canvas レンダラーで再描画させる。
      terminal?.refresh(0, terminal.rows - 1);
    });
    terminal.loadAddon(webglAddon);
  } catch {
    // Canvas renderer を使用
  }

  terminal.open(xtermRef.value);

  if (!props.noResize) {
    const isOffscreen = !!xtermRef.value?.closest('[data-offscreen]');
    fitAddon.fit();
    const initDims = fitAddon.proposeDimensions();
    debug(`[Terminal] initTerminal fit offscreen=${isOffscreen} dims=${JSON.stringify(initDims)} parentSize=${xtermRef.value?.parentElement?.clientWidth}x${xtermRef.value?.parentElement?.clientHeight}`);
  }

  terminal.onTitleChange((title) => {
    emit("title-change", title);
  });

  terminal.parser.registerOscHandler(777, (data: string) => {
    const match = data.match(/^exit_code;(\d+)$/);
    if (match) {
      emit("exit-code-change", parseInt(match[1], 10));
      return true;
    }
    return false;
  });

  terminal.textarea?.addEventListener("focus", () => {
    emit("focus");
  });

  terminal.attachCustomKeyEventHandler((event: KeyboardEvent) => {
    if (event.type !== "keydown") return true;
    if (event.isComposing || event.keyCode === 229) return true;
    if ((event.ctrlKey || event.metaKey) && event.key === "v") {
      handlePaste();
      return false;
    }
    if ((event.ctrlKey || event.metaKey) && event.key === "f") {
      event.preventDefault();
      search.toggleSearchBar();
      return false;
    }
    const hk = settings.value.hotkeys;
    if (hk) {
      if (matchesHotkey(event, hk.terminalNext)) return false;
      if (matchesHotkey(event, hk.terminalPrev)) return false;
      if (matchesHotkey(event, hk.terminalAdd)) return false;
      if (matchesHotkey(event, hk.terminalClose)) return false;
      if (matchesHotkey(event, hk.trayNext)) return false;
    }
    // Alt+英数字1文字 → ワークツリーフォーカス
    if (event.altKey && !event.ctrlKey && !event.shiftKey && event.key.length === 1) return false;
    return true;
  });

  terminal.onData((data) => {
    write(data);
  });

  terminal.onBinary((data) => {
    const bytes = Uint8Array.from(data, (c) => c.charCodeAt(0));
    write(bytes);
  });

  // ResizeObserver でリサイズ自動追従
  resizeObserver = new ResizeObserver((entries) => {
    const entry = entries[0];
    if (!entry || entry.contentRect.width === 0 || entry.contentRect.height === 0) return;
    // オフスクリーン div 内にいる場合はスキップ
    const isOffscreen = !!xtermRef.value?.closest('[data-offscreen]');
    if (isOffscreen) {
      debug(`[Terminal] ResizeObserver skipped (offscreen) sid=${sessionId.value} w=${entry.contentRect.width} h=${entry.contentRect.height}`);
      return;
    }
    if (props.noResize) return;
    debug(`[Terminal] ResizeObserver fired sid=${sessionId.value} w=${entry.contentRect.width} h=${entry.contentRect.height}`);
    if (resizeDebounce) clearTimeout(resizeDebounce);
    resizeDebounce = setTimeout(() => {
      if (fitAddon && terminal) {
        fitAddon.fit();
        if (!props.noResize) {
          const dims = fitAddon.proposeDimensions();
          if (dims) {
            debug(`[Terminal] ResizeObserver after fit sid=${sessionId.value} rows=${dims.rows} cols=${dims.cols}`);
            resize(dims.rows, dims.cols);
          }
        }
      }
    }, 100);
  });
  resizeObserver.observe(xtermRef.value);
}

async function startPty() {
  if (!terminal) return;

  const dims = fitAddon?.proposeDimensions() ?? { rows: 24, cols: 80 };

  await spawn(
    dims.rows ?? 24,
    dims.cols ?? 80,
    (data) => {
      batcher.enqueue(data);
    },
    () => {
      terminal?.write(`\r\n\x1b[33m[${t("processExited")}]\x1b[0m\r\n`);
      emit("exit");
    },
    props.shell,
    props.cwd
  );

  terminal.focus();
  emit("ready");
}

async function attachPty(id: number, snapshot?: string) {
  if (!terminal) return;

  if (snapshot) {
    terminal.write(snapshot);
  }

  await attachToSession(
    id,
    (data) => {
      batcher.enqueue(data);
    },
    () => {
      terminal?.write(`\r\n\x1b[33m[${t("processExited")}]\x1b[0m\r\n`);
      emit("exit");
    }
  );

  terminal.focus();
  emit("ready");
}

function serializeBuffer(scrollback?: number): string {
  return serializeAddon?.serialize(scrollback !== undefined ? { scrollback } : undefined) ?? "";
}

function focus() {
  terminal?.focus();
}

async function handleTabActivated() {
  await nextTick();
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (fitAddon && terminal) {
          if (!props.noResize) {
            const dimsBefore = fitAddon.proposeDimensions();
            const parentEl = xtermRef.value?.parentElement;
            debug(`[Terminal] handleTabActivated before fit sid=${sessionId.value} dims=${JSON.stringify(dimsBefore)} parentSize=${parentEl?.clientWidth}x${parentEl?.clientHeight}`);
            fitAddon.fit();
            const dimsAfter = fitAddon.proposeDimensions();
            debug(`[Terminal] handleTabActivated after fit sid=${sessionId.value} dims=${JSON.stringify(dimsAfter)}`);
            if (dimsAfter) {
              resize(dimsAfter.rows, dimsAfter.cols);
            }
          }
          terminal.refresh(0, terminal.rows - 1);
          terminal.scrollToBottom();
          // DOM reparenting後のIME textarea位置を再同期（blur→focusで強制再計算）
          terminal.blur();
          terminal.focus();
        }
        resolve();
      });
    });
  });
}

function getTerminal() {
  return terminal;
}

function waitForReady(): Promise<void> {
  if (sessionId.value !== null) return Promise.resolve();
  return new Promise((resolve) => {
    const stopWatch = watch(sessionId, (val) => {
      if (val !== null) {
        stopWatch();
        resolve();
      }
    });
  });
}

watch(
  () => settings.value.terminal.fontSize,
  (newSize) => {
    if (terminal) {
      terminal.options.fontSize = newSize;
      if (!props.noResize) {
        fitAddon?.fit();
        const dims = fitAddon?.proposeDimensions();
        if (dims) {
          resize(dims.rows, dims.cols);
        }
      }
    }
  }
);

onMounted(async () => {
  debug(`[TerminalView] onMounted initialSessionId=${props.initialSessionId} autoStart=${props.autoStart} cwd=${props.cwd}`);
  initTerminal();
  if (props.initialSessionId !== undefined) {
    debug(`[TerminalView] attaching to session ${props.initialSessionId}`);
    await attachPty(props.initialSessionId, props.initialSnapshot);
  } else if (props.autoStart) {
    debug(`[TerminalView] starting new PTY (no initialSessionId)`);
    if (props.restoreSnapshot) {
      terminal?.write(props.restoreSnapshot);
    }
    await startPty();
  }
});

onUnmounted(() => {
  if (resizeDebounce) clearTimeout(resizeDebounce);
  resizeObserver?.disconnect();
  batcher.dispose();
  search.dispose();
  serializeAddon?.dispose();
  terminal?.dispose();
});

defineExpose({
  startPty,
  attachPty,
  kill,
  detach,
  focus,
  write,
  getTerminal,
  isRunning,
  sessionId,
  serializeBuffer,
  handleTabActivated,
  waitForReady,
  containerRef,
  toggleSearchBar: search.toggleSearchBar,
  closeSearchBar: search.closeSearchBar,
});
</script>

<template>
  <div ref="containerRef" class="terminal-wrapper">
    <div v-if="search.showSearchBar.value" class="search-bar">
      <div class="search-input-group">
        <span class="pi pi-search search-icon" />
        <input
          :ref="(el) => (search.searchInputRef.value = el as HTMLInputElement | null)"
          v-model="search.searchQuery.value"
          class="search-input"
          type="text"
          :placeholder="t('searchPlaceholder')"
          @input="search.onSearchInput"
          @keydown="search.onSearchKeydown"
        />
        <span v-if="search.searchQuery.value" class="search-count">
          {{ search.searchCountText.value }}
        </span>
      </div>
      <button class="search-btn" :title="t('prevTitle')" @click="search.findPrevious">
        <span class="pi pi-chevron-up" />
      </button>
      <button class="search-btn" :title="t('nextTitle')" @click="search.findNext">
        <span class="pi pi-chevron-down" />
      </button>
      <button class="search-btn" :title="t('closeTitle')" @click="search.closeSearchBar">
        <span class="pi pi-times" />
      </button>
    </div>
    <div ref="xtermRef" class="terminal-container" />
  </div>
</template>

<style scoped>
.terminal-wrapper {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background-color: transparent;
}

.terminal-container {
  width: 100%;
  height: 100%;
  overflow: hidden;
}

.terminal-container :deep(.xterm) {
  height: 100%;
  padding: 4px;
  background-color: transparent !important;
}

.terminal-container :deep(.xterm-viewport) {
  overflow-y: auto;
  background-color: transparent !important;
}

.search-bar {
  position: absolute;
  top: 8px;
  right: 12px;
  z-index: 100;
  display: flex;
  align-items: center;
  gap: 4px;
  background-color: #313244;
  border: 1px solid #45475a;
  border-radius: 6px;
  padding: 4px 6px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
}

.search-input-group {
  display: flex;
  align-items: center;
  gap: 4px;
}

.search-icon {
  color: #6c7086;
  font-size: 12px;
}

.search-input {
  background: transparent;
  border: none;
  outline: none;
  color: #cdd6f4;
  font-size: 13px;
  font-family: "Cascadia Code", Consolas, Menlo, "SF Mono", Monaco, monospace;
  width: 180px;
  caret-color: #cdd6f4;
}

.search-input::placeholder {
  color: #6c7086;
}

.search-count {
  color: #6c7086;
  font-size: 11px;
  min-width: 60px;
  text-align: right;
  white-space: nowrap;
}

.search-btn {
  background: transparent;
  border: none;
  color: #6c7086;
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
  display: flex;
  align-items: center;
  font-size: 12px;
  transition: color 0.15s, background-color 0.15s;
}

.search-btn:hover {
  color: #cdd6f4;
  background-color: #45475a;
}
</style>

<i18n lang="json">
{
  "en": {
    "searchPlaceholder": "Search...",
    "prevTitle": "Previous (Shift+Enter)",
    "nextTitle": "Next (Enter)",
    "closeTitle": "Close (Esc)",
    "processExited": "Process exited"
  },
  "ja": {
    "searchPlaceholder": "検索...",
    "prevTitle": "前へ (Shift+Enter)",
    "nextTitle": "次へ (Enter)",
    "closeTitle": "閉じる (Esc)",
    "processExited": "プロセスが終了しました"
  }
}
</i18n>
