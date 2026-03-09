import { computed } from "vue";
import { useSettings } from "./useSettings";
import type { CodeReviewSettings, HotkeyBinding } from "../types/settings";

export const CODE_REVIEW_DEFAULTS: Required<CodeReviewSettings> = {
  monacoFontSize: 13,
  monacoMinimap: true,
  monacoWordWrap: "off",
  monacoLineNumbers: "on",
  chatHotkey: { ctrl: true, key: "l" } as HotkeyBinding,
  autoOpenReviewOnDiff: true,
};

export function useCodeReviewSettings() {
  const { settings, scheduleSave } = useSettings();

  const resolved = computed(
    () => ({ ...CODE_REVIEW_DEFAULTS, ...settings.value.codeReview }),
  );

  function update<K extends keyof CodeReviewSettings>(key: K, value: CodeReviewSettings[K]) {
    settings.value.codeReview = { ...settings.value.codeReview, [key]: value };
    scheduleSave();
  }

  return { resolved, update };
}
