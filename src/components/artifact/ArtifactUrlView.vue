<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { openUrl } from "@tauri-apps/plugin-opener";

const props = defineProps<{
  content: string;
}>();

const { t } = useI18n();

/** content は URL 1行。前後の空白と余分な行は落とす */
const url = computed(() => props.content.trim().split(/\r?\n/)[0]?.trim() ?? "");
const isOpenable = computed(() => /^https?:\/\/\S+$/.test(url.value));

const copied = ref(false);
let copyTimer: ReturnType<typeof setTimeout> | null = null;

async function open() {
  if (!isOpenable.value) return;
  try {
    await openUrl(url.value);
  } catch (e) {
    console.error("openUrl failed", e);
  }
}

async function copy() {
  try {
    await navigator.clipboard.writeText(url.value);
    copied.value = true;
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => { copied.value = false; }, 1500);
  } catch (e) {
    console.error("copy failed", e);
  }
}
</script>

<template>
  <div class="url-view">
    <span class="pi pi-link url-icon" />
    <!-- href はスキーム検証を通ったものだけ。アーティファクト本文は AI が自由に書けるため、
         javascript: などが特権ドキュメントのリンク先に載らないようにする -->
    <a class="url-text" :href="isOpenable ? url : undefined" @click.prevent="open">{{ url }}</a>
    <div class="url-actions">
      <button class="btn-url btn-open" :disabled="!isOpenable" @click="open">
        <i class="pi pi-external-link" />
        <span>{{ t("openInBrowser") }}</span>
      </button>
      <button class="btn-url" @click="copy">
        <i :class="copied ? 'pi pi-check' : 'pi pi-copy'" />
        <span>{{ copied ? t("copied") : t("copy") }}</span>
      </button>
    </div>
    <span v-if="!isOpenable" class="url-warning">{{ t("invalidUrl") }}</span>
  </div>
</template>

<style scoped>
.url-view {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  padding: 32px;
  overflow: auto;
}

.url-icon {
  font-size: 32px;
  color: #89b4fa;
}

.url-text {
  font-family: monospace;
  font-size: 13px;
  color: #89b4fa;
  text-align: center;
  word-break: break-all;
  max-width: 100%;
  cursor: pointer;
  text-decoration: none;
}

.url-text:hover {
  text-decoration: underline;
}

.url-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.btn-url {
  display: flex;
  align-items: center;
  gap: 6px;
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.btn-url:hover:not(:disabled) {
  background: #45475a;
}

.btn-url:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-open {
  border-color: #89b4fa;
  color: #89b4fa;
}

.url-warning {
  font-size: 11px;
  color: #f38ba8;
}
</style>

<i18n lang="json">
{
  "en": {
    "openInBrowser": "Open in browser",
    "copy": "Copy",
    "copied": "Copied",
    "invalidUrl": "This artifact does not contain a valid http(s) URL"
  },
  "ja": {
    "openInBrowser": "ブラウザで開く",
    "copy": "コピー",
    "copied": "コピーしました",
    "invalidUrl": "有効な http(s) URL が入っていません"
  }
}
</i18n>
