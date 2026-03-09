<script setup lang="ts">
import { computed } from "vue";
import { useCodeReviewSettings } from "../../composables/useCodeReviewSettings";
import HotkeyInput from "../HotkeyInput.vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { resolved, update } = useCodeReviewSettings();

const fontSize = computed({
  get: () => resolved.value.monacoFontSize,
  set: (v) => update("monacoFontSize", v),
});

const minimap = computed({
  get: () => resolved.value.monacoMinimap,
  set: (v) => update("monacoMinimap", v),
});

const wordWrap = computed({
  get: () => resolved.value.monacoWordWrap,
  set: (v) => update("monacoWordWrap", v),
});

const lineNumbers = computed({
  get: () => resolved.value.monacoLineNumbers,
  set: (v) => update("monacoLineNumbers", v),
});

const chatHotkey = computed({
  get: () => resolved.value.chatHotkey,
  set: (v) => update("chatHotkey", v),
});

const autoOpenReview = computed({
  get: () => resolved.value.autoOpenReviewOnDiff,
  set: (v) => update("autoOpenReviewOnDiff", v),
});
</script>

<template>
  <div class="h-full overflow-y-auto p-6 text-surface-100">
    <h2 class="text-base font-semibold text-surface-200 mb-6">{{ t("title") }}</h2>

    <!-- Monaco Editor 設定 -->
    <section class="mb-8">
      <h3 class="text-xs font-semibold text-surface-400 uppercase tracking-wider mb-4">{{ t("section.monaco") }}</h3>
      <div class="space-y-4">
        <div class="flex items-center gap-4">
          <label class="w-40 text-sm text-surface-300 shrink-0">{{ t("monaco.fontSize") }}</label>
          <input
            v-model.number="fontSize"
            type="number"
            min="8"
            max="32"
            class="w-20 bg-surface-800 border border-surface-600 rounded px-2 py-1 text-sm text-surface-100 outline-none focus:border-primary-500"
          />
        </div>
        <div class="flex items-center gap-4">
          <label class="w-40 text-sm text-surface-300 shrink-0">{{ t("monaco.minimap") }}</label>
          <input
            type="checkbox"
            :checked="minimap"
            class="w-4 h-4 accent-primary-500"
            @change="minimap = ($event.target as HTMLInputElement).checked"
          />
        </div>
        <div class="flex items-center gap-4">
          <label class="w-40 text-sm text-surface-300 shrink-0">{{ t("monaco.wordWrap") }}</label>
          <input
            type="checkbox"
            :checked="wordWrap === 'on'"
            class="w-4 h-4 accent-primary-500"
            @change="wordWrap = ($event.target as HTMLInputElement).checked ? 'on' : 'off'"
          />
        </div>
        <div class="flex items-center gap-4">
          <label class="w-40 text-sm text-surface-300 shrink-0">{{ t("monaco.lineNumbers") }}</label>
          <input
            type="checkbox"
            :checked="lineNumbers === 'on'"
            class="w-4 h-4 accent-primary-500"
            @change="lineNumbers = ($event.target as HTMLInputElement).checked ? 'on' : 'off'"
          />
        </div>
      </div>
    </section>

    <!-- チャットボタン ホットキー -->
    <section class="mb-8">
      <h3 class="text-xs font-semibold text-surface-400 uppercase tracking-wider mb-4">{{ t("section.chat") }}</h3>
      <div class="flex items-center gap-4">
        <label class="w-40 text-sm text-surface-300 shrink-0">{{ t("chat.hotkey") }}</label>
        <HotkeyInput v-model="chatHotkey" />
      </div>
    </section>

    <!-- 自動オープン -->
    <section class="mb-8">
      <h3 class="text-xs font-semibold text-surface-400 uppercase tracking-wider mb-4">{{ t("section.behavior") }}</h3>
      <div class="flex items-center gap-4">
        <label class="w-40 text-sm text-surface-300 shrink-0">{{ t("behavior.autoOpenReview") }}</label>
        <input
          type="checkbox"
          :checked="autoOpenReview"
          class="w-4 h-4 accent-primary-500"
          @change="autoOpenReview = ($event.target as HTMLInputElement).checked"
        />
        <span class="text-xs text-surface-500">{{ t("behavior.autoOpenReviewDesc") }}</span>
      </div>
    </section>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "title": "Code Reviewer Settings",
    "section": {
      "monaco": "Monaco Editor",
      "chat": "Chat Button",
      "behavior": "Behavior"
    },
    "monaco": {
      "fontSize": "Font Size",
      "minimap": "Show Minimap",
      "wordWrap": "Word Wrap",
      "lineNumbers": "Line Numbers"
    },
    "chat": {
      "hotkey": "Chat Hotkey"
    },
    "behavior": {
      "autoOpenReview": "Auto Open Review Tab",
      "autoOpenReviewDesc": "Automatically open the code review tab when there are changes"
    }
  },
  "ja": {
    "title": "コードレビュー設定",
    "section": {
      "monaco": "Monaco Editor",
      "chat": "チャットボタン",
      "behavior": "動作"
    },
    "monaco": {
      "fontSize": "フォントサイズ",
      "minimap": "ミニマップを表示",
      "wordWrap": "ワードラップ",
      "lineNumbers": "行番号を表示"
    },
    "chat": {
      "hotkey": "チャットホットキー"
    },
    "behavior": {
      "autoOpenReview": "レビュータブを自動で開く",
      "autoOpenReviewDesc": "コードレビューを開いたときに変更差分があれば自動でレビュータブを開く"
    }
  }
}
</i18n>
