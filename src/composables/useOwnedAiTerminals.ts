import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * インスタンス専用 AI ターミナル（#372）の所有情報。Rust の `ai_terminal` レジストリが真実で、
 * ここはその写し（タブのアイコン表示と、自動承認・復元からの除外に使う）。
 * `is_ai_agent` とは別の印で、そちらは PTY の監視で専用端末にも自動で立つ。
 *
 * ウィンドウ（main / sub / tray）ごとにモジュールが別なので、各ウィンドウが `initOwnedAiTerminals` を呼ぶ。
 */
export interface OwnedAiTerminal {
  terminalId: string;
  sessionId: number;
  worktreeId: string;
  artifactId: string;
  appName: string;
  title: string;
}

const owned = ref<OwnedAiTerminal[]>([]);

/** PTY の session_id → 所有端末 */
const ownedBySession = computed(() => new Map(owned.value.map((o) => [o.sessionId, o])));

async function refreshOwnedAiTerminals(): Promise<void> {
  try {
    owned.value = await invoke<OwnedAiTerminal[]>("list_owned_ai_terminals");
  } catch (e) {
    console.warn("[ai-terminal] list_owned_ai_terminals failed:", e);
  }
}

let unlisten: UnlistenFn | null = null;
let initPromise: Promise<void> | null = null;

/** 一覧を読み、変更イベントを購読する。何度呼んでも 1 回だけ */
export function initOwnedAiTerminals(): Promise<void> {
  if (!initPromise) {
    initPromise = (async () => {
      unlisten = await listen("ai-terminal-owned-changed", () => void refreshOwnedAiTerminals());
      await refreshOwnedAiTerminals();
    })();
  }
  return initPromise;
}

export function disposeOwnedAiTerminals(): void {
  unlisten?.();
  unlisten = null;
  initPromise = null;
}

export function useOwnedAiTerminals() {
  return {
    owned,
    ownedBySession,
    /** この PTY セッションがインスタンス専用 AI ターミナルか */
    isOwnedSession: (sessionId: number | null | undefined): boolean =>
      sessionId != null && ownedBySession.value.has(sessionId),
    ownerOfSession: (sessionId: number | null | undefined): OwnedAiTerminal | undefined =>
      sessionId != null ? ownedBySession.value.get(sessionId) : undefined,
    refreshOwnedAiTerminals,
  };
}
