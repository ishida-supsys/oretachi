<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted } from "vue";

/**
 * URL ポップアップの URL テキスト。ポップアップ本体から切り出してあるのは、
 * ツールチップを親の再レンダリングから守るため。
 *
 * PrimeVue の tooltip ディレクティブは `updated` フックで無条件に `unbindEvents` を
 * 呼び、その中で表示中のツールチップを DOM から削除する。カーソルは既にこの要素の
 * 内側に居るので `mouseenter` は再発火せず、いったん外へ出て入り直すまで戻らない。
 * ディレクティブが走るのは「宣言したコンポーネント」の更新時なので、親（コピー完了
 * フィードバックなどで再レンダリングする）から分離しておけば、href が変わらない限り
 * 読んでいる途中でツールチップが消えることはない。
 */
const props = defineProps<{ href: string }>();

/**
 * 打ち切られているかを親へ伝える。親（ポップアップ）はこれを見て、
 * 全文が見えていない URL を開くときだけ確認ダイアログを挟む
 */
const emit = defineEmits<{ (e: "update:truncated", value: boolean): void }>();

/** ツールチップを出すまでの待ち。WorktreeHeader のタスクツールチップと同値 */
const TOOLTIP_DELAY_MS = 300;

const spanRef = ref<HTMLElement | null>(null);
/** max-height で打ち切られているか。切られていない URL にツールチップは要らない */
const truncated = ref(false);

const tooltip = computed(() => ({
  value: props.href,
  disabled: !truncated.value,
  showDelay: TOOLTIP_DELAY_MS,
  class: "artifact-link-url-tooltip",
}));

function measure() {
  const el = spanRef.value;
  // 端数の丸めで 1px 差が出るので余裕を持たせる
  truncated.value = !!el && el.scrollHeight > el.clientHeight + 1;
  emit("update:truncated", truncated.value);
}

onMounted(measure);
watch(
  () => props.href,
  () => void nextTick(measure),
);
</script>

<template>
  <span
    ref="spanRef"
    v-tooltip.bottom="tooltip"
    class="link-hover-url"
    :class="{ 'link-hover-url-truncated': truncated }"
    >{{ href }}</span
  >
</template>

<style scoped>
.link-hover-url {
  /* URL は途中に区切りが無く伸びるので折り返す (省略すると確かめる用途に立たない)。
     長すぎる場合だけ高さで打ち切り、切られた分はホバーのツールチップで出す (issue #247) */
  color: #cdd6f4;
  font-family: monospace;
  word-break: break-all;
  max-height: 4.5em;
  overflow: hidden;
}

/* 打ち切られたときだけ「続きがある」ことを cursor で示す */
.link-hover-url-truncated {
  cursor: help;
}
</style>
