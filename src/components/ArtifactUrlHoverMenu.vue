<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import Popover from "primevue/popover";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { UrlArtifactEntry } from "../types/artifact";

/**
 * アーティファクトボタン／バッジをラップし、マウスオーバーで URL 一覧を出す。
 * ボタン自体の見た目・クリック挙動には手を出さない（クリックは従来どおり
 * アーティファクトウィンドウを開く）。URL が 0 件なら何も起きない。
 */
const props = defineProps<{
  urls: UrlArtifactEntry[];
}>();

// ルートが span + Popover の複数ノードなので、class 等は明示的にラッパへ流す
defineOptions({ inheritAttrs: false });

const wrapperRef = ref<HTMLElement | null>(null);
const popoverRef = ref<InstanceType<typeof Popover> | null>(null);
const visible = ref(false);
let hideTimer: ReturnType<typeof setTimeout> | null = null;

function cancelHide() {
  if (hideTimer) {
    clearTimeout(hideTimer);
    hideTimer = null;
  }
}

function showMenu() {
  if (props.urls.length === 0 || !wrapperRef.value) return;
  cancelHide();
  if (visible.value) return;
  visible.value = true;
  // Popover.show は event.currentTarget を配置基準にするため、ラッパを渡す
  popoverRef.value?.show({ currentTarget: wrapperRef.value } as unknown as Event, wrapperRef.value);
}

function hideNow() {
  cancelHide();
  if (!visible.value) return;
  visible.value = false;
  popoverRef.value?.hide();
}

/** ボタン → ポップアップへマウスを移す間に閉じないよう猶予を持たせる */
function scheduleHide() {
  cancelHide();
  hideTimer = setTimeout(hideNow, 180);
}

onUnmounted(cancelHide);

async function open(entry: UrlArtifactEntry) {
  hideNow();
  try {
    await openUrl(entry.url);
  } catch (e) {
    console.error("openUrl failed", e);
  }
}
</script>

<template>
  <span
    ref="wrapperRef"
    v-bind="$attrs"
    class="url-hover-wrapper"
    @mouseenter="showMenu"
    @mouseleave="scheduleHide"
    @click="hideNow"
  >
    <slot />
  </span>
  <Popover ref="popoverRef" @hide="visible = false">
    <div class="url-popup" @mouseenter="cancelHide" @mouseleave="scheduleHide">
      <button
        v-for="entry in props.urls"
        :key="entry.id"
        class="popup-item"
        :title="entry.url"
        @click="open(entry)"
      >
        <span class="pi pi-external-link" />
        <span class="popup-item-label">{{ entry.title }}</span>
      </button>
    </div>
  </Popover>
</template>

<style scoped>
/* ラップしたボタンのレイアウトを変えないための素通しコンテナ */
.url-hover-wrapper {
  display: inline-flex;
  align-items: center;
}

.url-popup {
  display: flex;
  flex-direction: column;
  min-width: 180px;
  max-width: 420px;
}

.popup-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  background: none;
  border: none;
  color: var(--p-text-color);
  font-size: 13px;
  cursor: pointer;
  border-radius: 4px;
  text-align: left;
  width: 100%;
}

.popup-item:hover {
  background: var(--p-content-hover-background);
}

.popup-item-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
