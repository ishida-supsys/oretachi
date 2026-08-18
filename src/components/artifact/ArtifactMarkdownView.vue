<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { useI18n } from "vue-i18n";
import { MdPreview, config } from "md-editor-v3";
import "md-editor-v3/lib/preview.css";
import mermaid from "mermaid";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ask } from "@tauri-apps/plugin-dialog";
import PanZoomCanvas from "./PanZoomCanvas.vue";
import { mermaidConfig, sanitizeMermaidSvg } from "../../utils/mermaidTheme";
import { createPanZoom, type PanZoomController } from "../../utils/panZoom";
import { resolveExternalLink } from "../../utils/externalLink";

const props = defineProps<{
  content: string;
}>();

const { t, locale } = useI18n();

// md-editor-v3 は既定で mermaid を CDN から非同期ロードするため、初回パースに間に合わず
// mermaid ブロックがソースのまま表示されてしまう。バンドル済みインスタンスを注入して
// 確実に描画させ、あわせて ArtifactMermaidView と同じテーマを共有する。
// md-editor-v3 はテーマ切替のたびに instance.initialize({ theme: "dark" }) を自前で呼ぶため、
// mermaidConfig フックでこちらの設定を上書き適用しておく
mermaid.initialize(mermaidConfig);
// enableZoom: md-editor-v3 標準のピン式パンズーム。ホイールが「加算 0.02/ノッチ」固定で
// 極端に鈍く、刻み幅を変える設定もないため無効化し、同じピン UI を createPanZoom で
// 実装し直す (アーティファクト側と同じ乗算ズーム = 同じ操作感にする)
config({
  editorExtensions: { mermaid: { instance: mermaid, enableZoom: false } },
  mermaidConfig: (base: any) => ({ ...base, ...mermaidConfig }),
});

const root = ref<HTMLElement | null>(null);
const canvas = ref<InstanceType<typeof PanZoomCanvas> | null>(null);
const overlaySvg = ref("");

// 装飾済みの mermaid ブロックを示す目印 (MutationObserver の再入で二重付与しないため)
// class ではなく属性を使う: md-editor-v3 の CSS が [class=md-editor-mermaid] の完全一致で
// アクションバーのスタイルを当てているため、class を足すと崩れる
const DECORATED = "data-mermaid-expandable";

// md-editor-v3 のアクションバー (コピー) と同じ lucide 系アイコンで見た目を揃える
const ICON_ATTRS = `xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"`;
const EXPAND_ICON = `<svg ${ICON_ATTRS} class="lucide lucide-maximize md-editor-icon"><path d="M8 3H5a2 2 0 0 0-2 2v3"></path><path d="M21 8V5a2 2 0 0 0-2-2h-3"></path><path d="M3 16v3a2 2 0 0 0 2 2h3"></path><path d="M16 21h3a2 2 0 0 0 2-2v-3"></path></svg>`;
// ピン (インラインのパンズーム有効/無効) は md-editor-v3 標準と同じ pin / pin-off アイコン
const PIN_OFF_ICON = `<svg ${ICON_ATTRS} class="lucide lucide-pin-off md-editor-icon"><path d="M12 17v5"></path><path d="M15 9.34V7a1 1 0 0 1 1-1a2 2 0 0 0 0-4H7.89"></path><path d="m2 2 20 20"></path><path d="M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h11"></path></svg>`;
const PIN_ICON = `<svg ${ICON_ATTRS} class="lucide lucide-pin md-editor-icon"><path d="M12 17v5"></path><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"></path></svg>`;

// ピン留め中のブロックとそのパンズーム制御
const pinned = new Map<HTMLElement, PanZoomController>();

/** インラインのパンズームを ON/OFF する (md-editor-v3 標準のピンと同じ操作) */
function togglePin(block: HTMLElement, btn: HTMLElement) {
  const active = pinned.get(block);
  if (active) {
    active.destroy();
    pinned.delete(block);
    block.removeAttribute("data-grab");
    const svg = block.querySelector<SVGElement>("svg");
    if (svg) svg.style.transform = "";
    btn.innerHTML = PIN_OFF_ICON;
    btn.title = t("panZoom.pin");
    return;
  }
  const svg = block.querySelector<SVGElement>("svg");
  if (!svg) return;
  // transform-origin: top left と overflow: hidden は md-editor-v3 の CSS が
  // すでに当てているので、こちらは transform を書くだけでよい
  const controller = createPanZoom(block, () => svg, {
    onChange: (s) => {
      svg.style.transform = `translate(${s.x}px, ${s.y}px) scale(${s.zoom})`;
    },
  });
  pinned.set(block, controller);
  block.setAttribute("data-grab", ""); // [data-grab] で grab カーソルになる
  btn.innerHTML = PIN_ICON;
  btn.title = t("panZoom.unpin");
}

/**
 * 挿入済みボタンのツールチップを貼り直す。
 * DOM に直接書いた title は locale 切替に追従しないため、切替時に呼ぶ
 */
function refreshTitles() {
  root.value?.querySelectorAll<HTMLElement>("[data-mermaid-btn]").forEach((btn) => {
    if (btn.dataset.mermaidBtn === "fullscreen") {
      btn.title = t("panZoom.fullscreen");
      return;
    }
    const block = btn.closest<HTMLElement>(".md-editor-mermaid");
    btn.title = block && pinned.has(block) ? t("panZoom.unpin") : t("panZoom.pin");
  });
}

/** ピン留めを全解除する (コンテンツ差し替え・アンマウント時) */
function unpinAll() {
  pinned.forEach((controller) => controller.destroy());
  pinned.clear();
}

/** 描画済みの mermaid ブロックにピン / 全画面ボタンを付ける */
function decorate() {
  const el = root.value;
  if (!el) return;
  el.querySelectorAll<HTMLElement>(`.md-editor-mermaid:not([${DECORATED}])`).forEach((block) => {
    // mermaid が未描画のブロック (ソースのまま) は対象外
    if (!block.querySelector("svg")) return;
    block.setAttribute(DECORATED, "");

    // コピーボタンと重ならないよう、md-editor-v3 のアクションバーに相乗りする。
    // まだ生成されていなければこちらで作る (md-editor-v3 側も既存のバーを再利用する実装)
    let action = block.querySelector<HTMLElement>(".md-editor-mermaid-action");
    if (!action) {
      block.insertAdjacentHTML("beforeend", '<div class="md-editor-mermaid-action"></div>');
      action = block.querySelector<HTMLElement>(".md-editor-mermaid-action");
    }
    if (!action) return;
    // バー上でのドラッグでパンが始まらないようにする (createPanZoom は [data-ui] を無視する)
    action.setAttribute("data-ui", "");

    const pinBtn = document.createElement("span");
    pinBtn.className = "mermaid-action-btn";
    pinBtn.dataset.mermaidBtn = "pin";
    pinBtn.title = t("panZoom.pin");
    pinBtn.innerHTML = PIN_OFF_ICON;
    pinBtn.addEventListener("click", () => togglePin(block, pinBtn));
    action.appendChild(pinBtn);

    const expandBtn = document.createElement("span");
    expandBtn.className = "mermaid-action-btn";
    expandBtn.dataset.mermaidBtn = "fullscreen";
    expandBtn.title = t("panZoom.fullscreen");
    expandBtn.innerHTML = EXPAND_ICON;
    expandBtn.addEventListener("click", () => openOverlay(block));
    action.appendChild(expandBtn);
  });
}

async function openOverlay(block: HTMLElement) {
  const svg = block.querySelector<SVGElement>("svg");
  if (!svg) return;
  // ピン留めで拡大/移動していると svg 自身に inline transform が載っている。
  // そのまま持ち込むと全画面側のキャンバスの transform と二重にかかり、
  // fitToView (transform を含まない offsetWidth で測る) も破綻するため落とす
  const clone = svg.cloneNode(true) as SVGElement;
  clone.style.transform = "";
  clone.style.transformOrigin = "";
  overlaySvg.value = sanitizeMermaidSvg(clone.outerHTML);
  await nextTick();
  canvas.value?.fitToView();
}

function closeOverlay() {
  overlaySvg.value = "";
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && overlaySvg.value) {
    closeOverlay();
  }
}

/**
 * 本文中のリンククリックを横取りする。
 * 素通しするとこのウィンドウの webview 自身が外部サイトへ遷移してしまい、
 * Tauri の特権ドキュメントが差し替わったうえ戻る手段もなくなる。
 * キャプチャ段階で取るのは md-editor-v3 側のハンドラより先に止めるため。
 */
function onLinkClick(e: MouseEvent) {
  const anchor = (e.target as Element | null)?.closest?.("a[href]");
  if (!anchor) return;
  const href = anchor.getAttribute("href");
  // 見出しアンカーは md-editor-v3 のページ内スクロールに任せる
  if (href?.startsWith("#")) return;
  // ここから先は開く/開かないに関わらず webview を遷移させない
  e.preventDefault();
  e.stopPropagation();
  const url = resolveExternalLink(href);
  if (!url) return; // 相対パス・ローカルパス・非 http スキームは何もしない
  void confirmAndOpen(url);
}

async function confirmAndOpen(url: string) {
  try {
    // アーティファクト本文は AI が自由に書けるためリンクテキストは信用できない。
    // 実 URL を見せて同意を取ってから外に出す
    const ok = await ask(t("externalLink.confirm", { url }), {
      title: t("externalLink.title"),
      kind: "warning",
    });
    if (!ok) return;
    await openUrl(url);
  } catch (e) {
    console.error("openUrl failed", e);
  }
}

// mermaid の描画は非同期で、md-editor-v3 がブロック要素を差し替えるため DOM 変化を監視する
let observer: MutationObserver | null = null;

onMounted(() => {
  decorate();
  if (root.value) {
    observer = new MutationObserver(decorate);
    observer.observe(root.value, { childList: true, subtree: true });
    root.value.addEventListener("click", onLinkClick, true);
    // 中クリックは click ではなく auxclick で飛ぶ。塞がないとここだけ遷移が残る
    root.value.addEventListener("auxclick", onLinkClick, true);
  }
  window.addEventListener("keydown", onKeydown);
});

onBeforeUnmount(() => {
  observer?.disconnect();
  // capture 有無を揃えないと解除されない
  root.value?.removeEventListener("click", onLinkClick, true);
  root.value?.removeEventListener("auxclick", onLinkClick, true);
  window.removeEventListener("keydown", onKeydown);
  unpinAll();
});

watch(locale, refreshTitles);

watch(() => props.content, () => {
  closeOverlay();
  // 差し替えでブロック要素自体が入れ替わるため、古いピンの購読を解除しておく
  unpinAll();
  nextTick(decorate);
});
</script>

<template>
  <div ref="root" class="markdown-view">
    <MdPreview :modelValue="content" theme="dark" language="ja-JP" :style="{ padding: '1rem 1.5rem' }" />

    <!-- プレビュー内の stacking context / containing block から切り離すため body へ出す -->
    <Teleport to="body">
      <div v-if="overlaySvg" class="mermaid-overlay">
        <PanZoomCanvas ref="canvas">
          <div class="overlay-svg" v-html="overlaySvg" />
        </PanZoomCanvas>
        <button
          type="button"
          class="overlay-close"
          data-ui
          :title="t('common.close')"
          @click="closeOverlay"
        >
          <span class="pi pi-times" />
        </button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.markdown-view {
  overflow-y: auto;
  height: 100%;
  box-sizing: border-box;
  /* 見出しアンカーの着地をスムーズに。親の .content-body は overflow: hidden なので
     フラグメント遷移で実際にスクロールするのはこの要素 */
  scroll-behavior: smooth;
}

/* OS 側でアニメーション抑制を選んでいる場合は即座にジャンプさせる */
@media (prefers-reduced-motion: reduce) {
  .markdown-view {
    scroll-behavior: auto;
  }
}

/* 全画面ボタンは md-editor-v3 のアクションバーに入るので、位置とアイコンの装飾は
   向こうの CSS に任せ、こちらはカーソルとホバー色だけ足す。
   ボタンは JS で後から挿入するため :deep で当てる */
.markdown-view :deep(.mermaid-action-btn) {
  display: inline-flex;
  cursor: pointer;
}

.markdown-view :deep(.mermaid-action-btn:hover) {
  color: #cdd6f4;
}

.mermaid-overlay {
  position: fixed;
  inset: 0;
  /* md-editor-v3 のコードブロックヘッダーが position: sticky + z-index: 10000 で、
     同 fullscreen も 10000。それらより前面に出す
     (このウィンドウには App.vue の shutdown-overlay(10000) は出ないため競合しない) */
  z-index: 10001;
  background: rgba(30, 30, 46, 0.97);
}

.overlay-svg :deep(svg) {
  display: block;
  max-width: none;
}

.overlay-close {
  position: absolute;
  top: 12px;
  right: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border: 1px solid #313244;
  border-radius: 6px;
  background: rgba(24, 24, 37, 0.85);
  color: #a6adc8;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

.overlay-close:hover {
  background: #313244;
  color: #cdd6f4;
}
</style>
