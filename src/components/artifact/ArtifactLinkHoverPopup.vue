<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onBeforeUnmount } from "vue";
import { useI18n } from "vue-i18n";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ask } from "@tauri-apps/plugin-dialog";
import ArtifactLinkUrlText from "./ArtifactLinkUrlText.vue";
import { resolveExternalLink } from "../../utils/externalLink";
import type { ArtifactLinkRect } from "../../utils/artifactFrameLink";

/**
 * アーティファクト本文中のリンクにマウスを乗せたときの URL ポップアップ。
 *
 * 本文は AI が自由に書けるためリンクテキストは飛び先を表さない。開く前に URL を
 * 確かめたい / URL だけ他所へ貼りたい、という用途に応えるのがこれ（markdown は
 * 親ドキュメント、html / react は sandbox iframe の中で、どちらもリンクの飛び先を
 * 見る手段が無い）。
 *
 * **外部ブラウザで開く導線もここに一本化してある**（issue #297）。本文のリンクを押しても
 * 何も起きず、開けるのはこのポップアップに出ている URL を押したときだけ。押した対象が
 * そのまま開く URL なので、リンクテキストと飛び先の食い違いに引っかかりようがない。
 * 開けるのは `resolveExternalLink` が通す http(s) だけで、`artifact:` や相対パスは
 * テキストとコピーのみになる。
 *
 * ただし html / react ビューの URL と座標は iframe 内のスクリプトの自己申告である
 * （アーティファクトの JS と同じレルムで動く）。ホバーしていなくても任意の URL の
 * 「開く」ボタンをカーソル直下へ出せてしまうため、`selfDeclared` で来たものは
 * 押しても即座には開かず、実 URL を見せる確認ダイアログを挟む。href を `<a>` から
 * 直接読む markdown ビューだけは、押した対象＝開く URL が保証されるのでそのまま開く。
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

/** showFor の追加指定（`<script setup>` からは export できないので型は各所で持つ） */
interface ShowOptions {
  /** href / 座標が iframe 内のスクリプトの自己申告か（html / react ビュー） */
  selfDeclared?: boolean;
}

const boxRef = ref<HTMLElement | null>(null);
const openBtnRef = ref<HTMLButtonElement | null>(null);
const href = ref("");
/** 表示中の URL が iframe の自己申告か（true なら開く前に確認ダイアログを挟む） */
const selfDeclared = ref(false);
/** ポップアップ自身にマウスが乗っているか。iframe からの閉じる要求と競合するため必要 */
const hovering = ref(false);
/** ポップアップ内にフォーカスがあるか。キーボードで URL ボタンへ移る間に閉じないため */
const focusedInside = ref(false);
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

/** マウスまたはフォーカスがポップアップ上にあり、閉じてはいけない状態か */
function retained(): boolean {
  return hovering.value || focusedInside.value;
}

/** 明示的に閉じる（コンテンツ差し替え・スクロールなど）。ホバー中でも閉じる */
function hideNow() {
  cancelHide();
  hovering.value = false;
  focusedInside.value = false;
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
    if (!retained()) hideNow();
  }, GRACE_MS);
}

/**
 * リンク直下にポップアップを出す。
 * @param rawHref `getAttribute("href")` の生値。解決済みの `.href` は相対パスを
 *   webview の URL 基準に化けさせるので使わない（utils/externalLink.ts と同じ方針）
 * @param rect リンクのビューポート座標
 * @param options `selfDeclared` は href / 座標が iframe の自己申告であることを示す
 */
function showFor(rawHref: string, rect: ArtifactLinkRect, options?: ShowOptions) {
  const url = rawHref.trim();
  if (!url || url.length > MAX_HREF_LENGTH) return;
  cancelHide();
  selfDeclared.value = options?.selfDeclared === true;
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

function onFocusIn() {
  focusedInside.value = true;
  cancelHide();
}

function onFocusOut() {
  focusedInside.value = false;
  scheduleHide();
}

/**
 * 「開く」ボタンへフォーカスを移す。移せたら true。
 * キーボードでリンクを辿っている間に外部 URL を開く唯一の経路で、呼び出し側は
 * リンク上での Enter をここへ振り替える（もう一度 Enter で開く = 2段階の明示操作）。
 */
function focusOpen(): boolean {
  const btn = openBtnRef.value;
  if (!visible.value || !btn) return false;
  btn.focus();
  return true;
}

/** ポップアップの URL を押して外部ブラウザで開けるか（http(s) のみ） */
const openTarget = computed(() => resolveExternalLink(href.value));

async function open() {
  const url = openTarget.value;
  if (!url) return;
  const needsConfirm = selfDeclared.value;
  // 開いたらポップアップの役目は終わり。残すと他ウィンドウへフォーカスが移った先で
  // 前面に浮いたままになる
  hideNow();
  try {
    // iframe の自己申告 URL は、ホバーしていなくてもカーソル直下へ出せてしまう。
    // 実 URL を見せて同意を取ってから外に出す
    if (needsConfirm) {
      const ok = await ask(t("externalLink.confirm", { url }), {
        title: t("externalLink.title"),
        kind: "warning",
      });
      if (!ok) return;
    }
    await openUrl(url);
  } catch (e) {
    console.error("openUrl failed", e);
  }
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

/**
 * ウィンドウが非アクティブになったら閉じる。ポップアップは position: fixed で body へ
 * teleport されるため、カーソルを動かさずフォーカスだけ他ウィンドウへ移す経路では
 * mouseout が来ず、前面に浮いたまま残る (issue #248)。
 *
 * `document.hasFocus()` を見るのは、ページ内の iframe へフォーカスが移ったときにも
 * window の blur が発火するため。html / react ビューのリンクは sandbox iframe の中に
 * あり、そのポップアップまで閉じてしまう。iframe が持っている間は document 全体では
 * フォーカスを失っていないので、ここで切り分けられる。
 */
function onWindowBlur() {
  if (!document.hasFocus()) hideNow();
}

// リサイズするとリンクが動いて座標が合わなくなる。追従させるより閉じる方が素直
onMounted(() => {
  window.addEventListener("resize", hideNow);
  window.addEventListener("blur", onWindowBlur);
  document.addEventListener("visibilitychange", hideNow);
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", hideNow);
  window.removeEventListener("blur", onWindowBlur);
  document.removeEventListener("visibilitychange", hideNow);
  cancelHide();
  if (copyTimer) clearTimeout(copyTimer);
});

defineExpose({ showFor, scheduleHide, cancelHide, hideNow, focusOpen });
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
      @focusin="onFocusIn"
      @focusout="onFocusOut"
    >
      <!-- URL 文字列そのものが「開く」ボタン。押した対象と開く URL を一致させるため、
           見た目はテキストのままにしてアイコンは足さない。
           click.prevent: 祖先のフォーム等へ既定動作を漏らさない -->
      <button
        v-if="openTarget"
        ref="openBtnRef"
        type="button"
        class="link-hover-open"
        :aria-label="t('open')"
        @click.prevent="open"
      >
        <ArtifactLinkUrlText :href="href" />
      </button>
      <ArtifactLinkUrlText v-else :href="href" />
      <button
        type="button"
        class="link-hover-copy"
        :title="copied ? t('copied') : t('copy')"
        :aria-label="copied ? t('copied') : t('copy')"
        @click.prevent="copy"
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

/* URL テキストを押して開く。ボタンだが見た目はテキストのままにし、
   ホバーで下線 + 色を変えて「押せる」ことだけ示す。
   display: flex なのは中の span を flex アイテムにするため
   （ArtifactLinkUrlText の max-height による打ち切りはインラインでは効かない） */
.link-hover-open {
  flex: 1 1 auto;
  display: flex;
  min-width: 0;
  padding: 0;
  border: none;
  background: none;
  text-align: left;
  font: inherit;
  cursor: pointer;
}

.link-hover-open:hover :deep(.link-hover-url),
.link-hover-open:focus-visible :deep(.link-hover-url) {
  color: #89b4fa;
  text-decoration: underline;
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
    "copied": "Copied",
    "open": "Open in default browser"
  },
  "ja": {
    "copy": "URL をコピー",
    "copied": "コピーしました",
    "open": "既定のブラウザで開く"
  }
}
</i18n>
