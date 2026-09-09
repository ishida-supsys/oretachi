<script setup lang="ts">
import { ref, nextTick, onMounted, onBeforeUnmount } from "vue";
import { useI18n } from "vue-i18n";
import type { ArtifactLinkRect } from "../../utils/artifactFrameLink";

/**
 * アーティファクト本文中のリンクにマウスを乗せたときの URL ポップアップ。
 *
 * 本文は AI が自由に書けるためリンクテキストは飛び先を表さない。開く前に URL を
 * 確かめたい / URL だけ他所へ貼りたい、という用途に応えるのがこれ（markdown は
 * 親ドキュメント、html / react は sandbox iframe の中で、どちらもリンクの飛び先を
 * 見る手段が無い）。
 *
 * ただし「開く前に確かめる」が成立するのは markdown ビューだけ。ここが href を
 * `<a>` から直接読み、実際に開くのも同じ値だからである。html / react ビューの URL は
 * iframe 内のスクリプトが postMessage で申告した値で、アーティファクトの JS と同じ
 * レルムで動くため任意の値を名乗れる（代わりに iframe のリンクはそもそも開けない。
 * sandbox が外部遷移を塞いでいて、親へ渡るのは `artifact:` だけ）。
 *
 * 座標は呼び出し側がリンクのビューポート座標で渡す。position: fixed で body へ
 * teleport するのは、markdown ビューの overflow や iframe の枠で切られないため。
 * 状態とタイマーはこのコンポーネントが持ち、呼び出し側は showFor / scheduleHide を
 * 叩くだけでよい（ArtifactUrlHoverMenu と同じ持ち方）。
 */
const { t } = useI18n();

/** リンク → ポップアップへマウスを移す間に閉じないための猶予。ArtifactUrlHoverMenu と同値 */
const GRACE_MS = 180;
/** リンクとポップアップの隙間 / 画面端との余白 */
const GAP = 6;
const MARGIN = 8;
/** コピー完了フィードバックを戻すまで。ArtifactUrlView と同値 */
const COPIED_MS = 1500;
/**
 * 出す URL の長さの上限。これを超える href はポップアップを出さない。
 * 折り返し表示なので長さがそのままレイアウト計算量になり、本文（= 信用できない）が
 * 巨大な href を書けば重くなる。この長さを超える URL は目で確かめる用途に立たない
 */
const MAX_HREF_LENGTH = 4096;

const boxRef = ref<HTMLElement | null>(null);
const href = ref("");
/** ポップアップ自身にマウスが乗っているか。iframe からの閉じる要求と競合するため必要 */
const hovering = ref(false);
const visible = ref(false);
const copied = ref(false);
const left = ref(0);
const top = ref(0);

let hideTimer: ReturnType<typeof setTimeout> | null = null;
let copyTimer: ReturnType<typeof setTimeout> | null = null;

function cancelHide() {
  if (hideTimer) {
    clearTimeout(hideTimer);
    hideTimer = null;
  }
}

/** 明示的に閉じる（コンテンツ差し替え・スクロールなど）。ホバー中でも閉じる */
function hideNow() {
  cancelHide();
  hovering.value = false;
  visible.value = false;
}

/**
 * リンクから離れたときに呼ぶ。ポップアップ自身へマウスが乗れば取り消される。
 *
 * 猶予明けにもう一度 hovering を見るのは、iframe 内のリンクでは「離れた」通知が
 * postMessage 経由で遅れて届き、ポップアップの mouseenter（= cancelHide）より
 * 後に scheduleHide が走りうるため。乗っている間は閉じない。
 */
function scheduleHide() {
  cancelHide();
  hideTimer = setTimeout(() => {
    hideTimer = null;
    if (!hovering.value) hideNow();
  }, GRACE_MS);
}

/**
 * リンク直下にポップアップを出す。
 * @param rawHref `getAttribute("href")` の生値。解決済みの `.href` は相対パスを
 *   webview の URL 基準に化けさせるので使わない（utils/externalLink.ts と同じ方針）
 * @param rect リンクのビューポート座標
 */
function showFor(rawHref: string, rect: ArtifactLinkRect) {
  const url = rawHref.trim();
  if (!url || url.length > MAX_HREF_LENGTH) return;
  cancelHide();
  // 別のリンクへ移ったらコピー済み表示は持ち越さない（別 URL なのに「コピーしました」に見える）
  if (url !== href.value) {
    copied.value = false;
    if (copyTimer) {
      clearTimeout(copyTimer);
      copyTimer = null;
    }
  }
  href.value = url;
  left.value = rect.left;
  top.value = rect.top + rect.height + GAP;
  visible.value = true;
  void nextTick(() => clampIntoViewport(rect));
}

/** 画面外へはみ出す場合だけ寄せる / リンクの上へ回す */
function clampIntoViewport(rect: ArtifactLinkRect) {
  const box = boxRef.value;
  if (!box || !visible.value) return;
  const { offsetWidth: w, offsetHeight: h } = box;
  if (left.value + w > window.innerWidth - MARGIN) {
    left.value = window.innerWidth - MARGIN - w;
  }
  if (top.value + h > window.innerHeight - MARGIN) {
    const above = rect.top - h - GAP;
    top.value = above >= MARGIN ? above : window.innerHeight - MARGIN - h;
  }
  // 上・左の外へ出さない（呼び出し側から負の座標が来ても画面内に留める）
  left.value = Math.max(MARGIN, left.value);
  top.value = Math.max(MARGIN, top.value);
}

function onEnter() {
  hovering.value = true;
  cancelHide();
}

function onLeave() {
  hovering.value = false;
  scheduleHide();
}

async function copy() {
  try {
    await navigator.clipboard.writeText(href.value);
    copied.value = true;
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => {
      copied.value = false;
      copyTimer = null;
    }, COPIED_MS);
  } catch (e) {
    console.error("copy failed", e);
  }
}

// リサイズするとリンクが動いて座標が合わなくなる。追従させるより閉じる方が素直
onMounted(() => window.addEventListener("resize", hideNow));

onBeforeUnmount(() => {
  window.removeEventListener("resize", hideNow);
  cancelHide();
  if (copyTimer) clearTimeout(copyTimer);
});

defineExpose({ showFor, scheduleHide, cancelHide, hideNow });
</script>

<template>
  <Teleport to="body">
    <div
      v-if="visible"
      ref="boxRef"
      class="link-hover-popup"
      :style="{ left: `${left}px`, top: `${top}px` }"
      @mouseenter="onEnter"
      @mouseleave="onLeave"
    >
      <span class="link-hover-url">{{ href }}</span>
      <!-- click.stop: リンク本体のクリック（開く / artifact: 遷移）へ伝播させない -->
      <button
        type="button"
        class="link-hover-copy"
        :title="copied ? t('copied') : t('copy')"
        :aria-label="copied ? t('copied') : t('copy')"
        @click.stop.prevent="copy"
      >
        <i :class="copied ? 'pi pi-check' : 'pi pi-copy'" />
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
.link-hover-popup {
  position: fixed;
  /* mermaid の全画面オーバーレイ (10001) より下、md-editor-v3 の sticky ヘッダー
     (10000) より上。オーバーレイ表示中は呼び出し側が閉じている */
  z-index: 10000;
  display: flex;
  align-items: flex-start;
  gap: 8px;
  max-width: 420px;
  padding: 6px 8px;
  border: 1px solid #45475a;
  border-radius: 4px;
  background: #1e1e2e;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  font-size: 12px;
  line-height: 1.5;
}

.link-hover-url {
  /* URL は途中に区切りが無く伸びるので折り返す (省略すると確かめる用途に立たない)。
     長すぎる場合だけ高さで打ち切る */
  color: #cdd6f4;
  font-family: monospace;
  word-break: break-all;
  max-height: 4.5em;
  overflow: hidden;
}

.link-hover-copy {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: 1px solid #45475a;
  border-radius: 3px;
  background: #313244;
  color: #cdd6f4;
  cursor: pointer;
}

.link-hover-copy:hover {
  background: #45475a;
}
</style>

<i18n lang="json">
{
  "en": {
    "copy": "Copy URL",
    "copied": "Copied"
  },
  "ja": {
    "copy": "URL をコピー",
    "copied": "コピーしました"
  }
}
</i18n>
