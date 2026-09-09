<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import { buildHtmlSrcdoc } from "../../utils/htmlArtifactSrcdoc";
import ArtifactLinkHoverPopup from "./ArtifactLinkHoverPopup.vue";
import {
  readArtifactLinkHoverMessage,
  readArtifactNavigateMessage,
  type ArtifactLinkHover,
} from "../../utils/artifactFrameLink";
import { applyFrameLinkHover } from "../../utils/artifactLinkHover";
import {
  mergeCspViolations,
  readArtifactCspViolationMessage,
  type ArtifactCspViolation,
} from "../../utils/artifactCspViolation";

const props = defineProps<{
  content: string;
}>();

const emit = defineEmits<{
  (e: "navigate", href: string): void;
}>();

const frame = ref<HTMLIFrameElement | null>(null);
const linkPopup = ref<InstanceType<typeof ArtifactLinkHoverPopup> | null>(null);
const violations = ref<ArtifactCspViolation[]>([]);
const truncated = ref(false);
const detailsOpen = ref(false);

// nonce を含むので content ごとに1度だけ組む
const frameDoc = computed(() => buildHtmlSrcdoc(props.content));

// srcdoc が変われば iframe は読み込み直しになるので、前の文書の違反は捨てる。
// 差し替え直前に仕掛けられた遅延メッセージは nonce 不一致で弾かれる
watch(frameDoc, () => {
  violations.value = [];
  truncated.value = false;
  detailsOpen.value = false;
  // 読み込み直しでホバー中のリンクも消えるので、URL ポップアップも閉じる
  linkPopup.value?.hideNow();
});

// sandbox の opaque origin では event.origin が "null" になり検証に使えないため、
// 送信元は contentWindow の同一性で判定する
function onMessage(event: MessageEvent) {
  const href = readArtifactNavigateMessage(event, frame.value);
  if (href) {
    emit("navigate", href);
    return;
  }
  const hover = readArtifactLinkHoverMessage(event, frame.value);
  if (hover) {
    onHover(hover);
    return;
  }
  const report = readArtifactCspViolationMessage(event, frame.value, frameDoc.value.nonce);
  if (!report) return;
  violations.value = mergeCspViolations(violations.value, report.violations);
  if (report.truncated) truncated.value = true;
}

function describe(v: ArtifactCspViolation): string {
  const uri = v.blockedUri || "(インライン)";
  return v.directive ? `${v.directive} — ${uri}` : uri;
}

/** iframe 内の座標で来たホバー通知を、親のビューポート座標へ直してポップアップへ渡す */
function onHover(hover: ArtifactLinkHover) {
  applyFrameLinkHover(hover, frame.value, linkPopup.value);
}

onMounted(() => window.addEventListener("message", onMessage));
onBeforeUnmount(() => window.removeEventListener("message", onMessage));
</script>

<template>
  <div class="html-view">
    <div v-if="violations.length > 0" class="csp-banner">
      <div class="csp-banner-head">
        <span class="csp-banner-text">
          CSP で {{ violations.length }} 種類{{ truncated ? "以上" : "" }}
          の読み込み・通信をブロックしました
        </span>
        <button type="button" class="csp-banner-toggle" @click="detailsOpen = !detailsOpen">
          {{ detailsOpen ? "閉じる" : "詳細" }}
        </button>
      </div>
      <ul v-if="detailsOpen" class="csp-banner-list">
        <li v-for="(v, i) in violations" :key="`${v.directive}|${v.blockedUri}|${i}`">
          {{ describe(v) }}
        </li>
      </ul>
    </div>
    <iframe
      ref="frame"
      :srcdoc="frameDoc.srcdoc"
      sandbox="allow-scripts"
      class="html-iframe"
    />
    <ArtifactLinkHoverPopup ref="linkPopup" />
  </div>
</template>

<style scoped>
.html-view {
  height: 100%;
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.csp-banner {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px 10px;
  border: 1px solid rgba(243, 139, 168, 0.35);
  background: rgba(243, 139, 168, 0.1);
  border-radius: 4px;
  font-size: 12px;
}

.csp-banner-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.csp-banner-text {
  flex: 1;
  min-width: 0;
}

.csp-banner-toggle {
  flex: 0 0 auto;
  padding: 2px 8px;
  border: 1px solid rgba(243, 139, 168, 0.45);
  border-radius: 3px;
  background: transparent;
  color: inherit;
  font-size: 11px;
  cursor: pointer;
}

.csp-banner-toggle:hover {
  background: rgba(243, 139, 168, 0.15);
}

.csp-banner-list {
  margin: 0;
  padding-left: 18px;
  max-height: 140px;
  overflow-y: auto;
  font-family: monospace;
  font-size: 11px;
  line-height: 1.7;
  word-break: break-all;
}

.html-iframe {
  flex: 1;
  min-height: 0;
  border: none;
  background: #fff;
  border-radius: 4px;
}
</style>
