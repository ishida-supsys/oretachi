<script setup lang="ts">
import { ref, watch, nextTick, onMounted } from "vue";

/**
 * URL ポップアップの URL テキスト。長すぎて max-height で打ち切られたときは、
 * ネイティブのツールチップ（title）で URL 全文を出す。
 *
 * 打ち切りは「開く」操作を妨げない（確認ダイアログも挟まない）。見えない部分は
 * ポップアップ中の URL へマウスを乗せれば全文で確かめられる、という分担。
 */
const props = defineProps<{ href: string }>();

const spanRef = ref<HTMLElement | null>(null);
/** max-height で打ち切られているか。切られていない URL にツールチップは要らない */
const truncated = ref(false);

function measure() {
  const el = spanRef.value;
  // 端数の丸めで 1px 差が出るので余裕を持たせる
  truncated.value = !!el && el.scrollHeight > el.clientHeight + 1;
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
    :title="truncated ? href : undefined"
    class="link-hover-url"
    >{{ href }}</span
  >
</template>

<style scoped>
.link-hover-url {
  /* URL は途中に区切りが無く伸びるので折り返す (省略すると確かめる用途に立たない)。
     長すぎる場合だけ高さで打ち切り、切られた分はネイティブのツールチップ (title) で出す (issue #247) */
  color: #cdd6f4;
  font-family: monospace;
  word-break: break-all;
  max-height: 4.5em;
  overflow: hidden;
}
</style>
