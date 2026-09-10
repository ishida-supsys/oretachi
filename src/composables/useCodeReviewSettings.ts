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
  const { settings, flushSave } = useSettings();

  const resolved = computed(
    () => ({ ...CODE_REVIEW_DEFAULTS, ...settings.value.codeReview }),
  );

  /**
   * 設定を1件更新して即座に書き込む。
   *
   * デバウンス（`scheduleSave`）にしないのは、コードレビューウィンドウが
   * `settings-changed` を受けて `settings.value` を丸ごと差し替えるため（#261）。
   * 保留中のタイマーは発火時点の `settings.value` を読むので、デバウンス中に
   * リロードが着弾すると**巻き戻った値**をそのまま保存してしまう。
   */
  function update<K extends keyof CodeReviewSettings>(key: K, value: CodeReviewSettings[K]) {
    settings.value.codeReview = { ...settings.value.codeReview, [key]: value };
    void flushSave();
  }

  return { resolved, update };
}
