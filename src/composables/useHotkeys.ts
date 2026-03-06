import { onMounted, onUnmounted } from "vue";
import { platform } from "@tauri-apps/plugin-os";
import type { HotkeyBinding } from "../types/settings";

const isMac = platform() === "macos";

export interface HotkeyAction {
  binding: HotkeyBinding;
  handler: () => void;
}

export function matchesHotkey(event: KeyboardEvent, binding: HotkeyBinding): boolean {
  if (!!binding.ctrl !== event.ctrlKey) return false;
  if (!!binding.meta !== event.metaKey) return false;
  if (!!binding.shift !== event.shiftKey) return false;
  if (!!binding.alt !== event.altKey) return false;
  // key の比較: 単一文字は小文字で比較
  const bindKey = binding.key.length === 1 ? binding.key.toLowerCase() : binding.key;
  const evKey = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  return bindKey === evKey;
}

export function formatHotkey(binding: HotkeyBinding): string {
  const parts: string[] = [];
  if (binding.ctrl) parts.push(isMac ? "⌃" : "Ctrl");
  if (binding.meta) parts.push(isMac ? "⌘" : "Win");
  if (binding.shift) parts.push(isMac ? "⇧" : "Shift");
  if (binding.alt) parts.push(isMac ? "⌥" : "Alt");
  parts.push(binding.key);
  return parts.join(isMac ? "" : "+");
}

export function bindingToAccelerator(binding: HotkeyBinding): string {
  const parts: string[] = [];
  if (binding.ctrl) parts.push("Ctrl");
  if (binding.meta) parts.push("Cmd");
  if (binding.shift) parts.push("Shift");
  if (binding.alt) parts.push("Alt");
  parts.push(binding.key.length === 1 ? binding.key.toUpperCase() : binding.key);
  return parts.join("+");
}

export function eventToBinding(event: KeyboardEvent): HotkeyBinding {
  return {
    ctrl: event.ctrlKey || undefined,
    meta: event.metaKey || undefined,
    shift: event.shiftKey || undefined,
    alt: event.altKey || undefined,
    key: event.key,
  };
}

/**
 * window keydown をキャプチャフェーズで登録するホットキーリスナー。
 * getActions はリアクティブな設定変更を反映できるようゲッター関数で渡す。
 */
export function useHotkeyListener(getActions: () => HotkeyAction[]) {
  function onKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return;
    const actions = getActions();
    for (const action of actions) {
      if (matchesHotkey(event, action.binding)) {
        event.preventDefault();
        event.stopPropagation();
        action.handler();
        return;
      }
    }
  }

  onMounted(() => {
    window.addEventListener("keydown", onKeydown, true);
  });

  onUnmounted(() => {
    window.removeEventListener("keydown", onKeydown, true);
  });
}
