<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { useI18n } from "vue-i18n";
import type { ArtifactComment, CommentAnchor } from "../../types/artifactComment";
import {
  boxFromPoints,
  isDrag,
  placePopup,
  type CommentAdapter,
  type Point,
} from "../../utils/commentAnchor";
import { createDomCommentAdapter } from "../../utils/commentDomAdapter";

/**
 * アーティファクトの表示領域に重ねるコメント層。親要素（`position: relative` の描画ルート）いっぱいに
 * 広がり、ピン（アイコン）・ポップアップ・コメントモード中の拾い上げを受け持つ。
 *
 * 位置の拾い上げと解決は `adapter` に任せる（同一文書の DOM / sandbox iframe など描画方式ごとに違う）。
 * 未指定なら親要素の DOM を相手にする。コメントの保存は親が行う（ここは見た目と入力だけ）。
 */
const props = defineProps<{
  /** 表示するコメント（未解決のみ。解決済みは非表示） */
  comments: ArtifactComment[];
  /** コメントモード（クリック / ドラッグで位置を選ぶ） */
  active: boolean;
  adapter?: CommentAdapter;
}>();

const emit = defineEmits<{
  (e: "submit", anchor: CommentAnchor, body: string): void;
  (e: "reply", commentId: string, body: string): void;
  (e: "resolve", commentId: string): void;
  (e: "remove", commentId: string): void;
  (e: "exit"): void;
}>();

const { t } = useI18n();

const layerEl = ref<HTMLElement | null>(null);
const rootEl = computed(() => layerEl.value?.parentElement ?? null);

const POPUP_SIZE = { width: 300, height: 200 };

let domAdapter: CommentAdapter | null = null;
function currentAdapter(): CommentAdapter | null {
  if (props.adapter) return props.adapter;
  if (!domAdapter && rootEl.value) {
    domAdapter = createDomCommentAdapter(
      rootEl.value,
      (el) => !!layerEl.value && layerEl.value.contains(el),
    );
  }
  return domAdapter;
}

// ─── ピンの位置 ────────────────────────────────────────────────────────────

type Located = { x: number; y: number; orphan: boolean } | null;
const positions = ref<Record<string, Located>>({});
let refreshSeq = 0;

async function refresh() {
  const adapter = currentAdapter();
  if (!adapter) return;
  const seq = ++refreshSeq;
  const next = await adapter.locate(props.comments);
  if (seq !== refreshSeq) return;
  positions.value = next;
}

/** 表示する順の連番（ピンの数字）。位置の取れないコメントは出さない */
const pins = computed(() =>
  props.comments
    .map((c, i) => ({ comment: c, index: i + 1, pos: positions.value[c.id] ?? null }))
    .filter((p): p is { comment: ArtifactComment; index: number; pos: NonNullable<Located> } => !!p.pos),
);

let unsubscribe: (() => void) | null = null;

function bindAdapter() {
  unsubscribe?.();
  unsubscribe = currentAdapter()?.subscribe(() => void refresh()) ?? null;
  void refresh();
}

onMounted(() => {
  bindAdapter();
});
onBeforeUnmount(() => {
  unsubscribe?.();
  window.removeEventListener("keydown", onKeydown, true);
});

watch(() => props.comments, () => void refresh(), { deep: true });
watch(
  () => props.adapter,
  () => bindAdapter(),
);

// ─── コメントモード（クリック / ドラッグ） ─────────────────────────────────

const dragStart = ref<Point | null>(null);
const dragEnd = ref<Point | null>(null);
const dragBox = computed(() =>
  dragStart.value && dragEnd.value && isDrag(dragStart.value, dragEnd.value)
    ? boxFromPoints(dragStart.value, dragEnd.value)
    : null,
);

/** 新規コメントの入力中。`at` はポップアップを置く位置（ルート内 px） */
const draft = ref<{ anchor: CommentAnchor; at: Point } | null>(null);
const draftBody = ref("");
const draftInput = ref<HTMLTextAreaElement | null>(null);

function toRootPoint(e: PointerEvent): Point {
  const r = layerEl.value!.getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
}

function onPointerDown(e: PointerEvent) {
  // ピン・ポップアップ・ヒントの操作は拾い上げにしない（層そのものを押したときだけ）
  if (!props.active || e.target !== layerEl.value || e.button !== 0) return;
  e.preventDefault();
  layerEl.value!.setPointerCapture(e.pointerId);
  dragStart.value = toRootPoint(e);
  dragEnd.value = dragStart.value;
  openId.value = null;
}

function onPointerMove(e: PointerEvent) {
  if (!dragStart.value) return;
  dragEnd.value = toRootPoint(e);
}

async function onPointerUp(e: PointerEvent) {
  const start = dragStart.value;
  if (!start) return;
  const end = toRootPoint(e);
  dragStart.value = null;
  dragEnd.value = null;
  const adapter = currentAdapter();
  if (!adapter) return;
  const picked = await adapter.pick(start, isDrag(start, end) ? end : null);
  if (!picked) return;
  const rootRect = layerEl.value!.getBoundingClientRect();
  draft.value = {
    anchor: picked,
    at: placePopup(end, POPUP_SIZE, { width: rootRect.width, height: rootRect.height }),
  };
  draftBody.value = "";
  await nextTick();
  draftInput.value?.focus();
}

function submitDraft() {
  const body = draftBody.value.trim();
  if (!draft.value || !body) return;
  emit("submit", draft.value.anchor, body);
  draft.value = null;
  draftBody.value = "";
}

function cancelDraft() {
  draft.value = null;
  draftBody.value = "";
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  if (draft.value) {
    cancelDraft();
  } else if (openId.value) {
    openId.value = null;
  } else if (props.active) {
    emit("exit");
  } else {
    return;
  }
  e.stopPropagation();
}

onMounted(() => window.addEventListener("keydown", onKeydown, true));

watch(
  () => props.active,
  (active) => {
    if (!active) {
      dragStart.value = null;
      dragEnd.value = null;
      cancelDraft();
    }
  },
);

// ─── スレッド（ピンのポップアップ） ────────────────────────────────────────

const openId = ref<string | null>(null);
const replyBody = ref("");

const openPin = computed(() => pins.value.find((p) => p.comment.id === openId.value) ?? null);
const openPopupAt = computed(() => {
  const pin = openPin.value;
  if (!pin || !layerEl.value) return null;
  const rect = layerEl.value.getBoundingClientRect();
  return placePopup(pin.pos, POPUP_SIZE, { width: rect.width, height: rect.height });
});

// 解決・削除・書き換えでピンが消えたらポップアップも閉じる
watch(openPin, (pin) => {
  if (!pin) openId.value = null;
});

function togglePin(id: string) {
  openId.value = openId.value === id ? null : id;
  replyBody.value = "";
  cancelDraft();
}

function submitReply() {
  const body = replyBody.value.trim();
  if (!openId.value || !body) return;
  emit("reply", openId.value, body);
  replyBody.value = "";
}

function onResolve() {
  if (!openId.value) return;
  emit("resolve", openId.value);
  openId.value = null;
}

function onRemove() {
  if (!openId.value) return;
  emit("remove", openId.value);
  openId.value = null;
}

function authorLabel(author: string): string {
  return author === "ai" ? t("author.ai") : t("author.user");
}

function onTextareaKeydown(e: KeyboardEvent, submit: () => void) {
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    submit();
  }
}
</script>

<template>
  <div
    ref="layerEl"
    class="comment-layer"
    :class="{ active }"
    data-comment-layer
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
  >
    <div v-if="active" class="cl-hint">
      <i class="pi pi-comment" />
      <span>{{ t("hint") }}</span>
      <button type="button" class="cl-hint-exit" @click="emit('exit')">{{ t("exit") }}</button>
    </div>

    <div
      v-if="dragBox"
      class="cl-drag"
      :style="{ left: `${dragBox.left}px`, top: `${dragBox.top}px`, width: `${dragBox.width}px`, height: `${dragBox.height}px` }"
    />

    <button
      v-for="pin in pins"
      :key="pin.comment.id"
      type="button"
      class="cl-pin"
      :class="{ orphan: pin.pos.orphan, open: pin.comment.id === openId }"
      :style="{ left: `${pin.pos.x}px`, top: `${pin.pos.y}px` }"
      :title="pin.pos.orphan ? t('orphan') : pin.comment.body"
      @click.stop="togglePin(pin.comment.id)"
    >
      <i class="pi pi-comment" />
      <span class="cl-pin-index">{{ pin.index }}</span>
    </button>

    <div
      v-if="draft"
      class="cl-popup"
      :style="{ left: `${draft.at.x}px`, top: `${draft.at.y}px`, width: `${POPUP_SIZE.width}px` }"
    >
      <div v-if="draft.anchor.excerpt" class="cl-excerpt">{{ draft.anchor.excerpt }}</div>
      <textarea
        ref="draftInput"
        v-model="draftBody"
        class="cl-textarea"
        rows="3"
        :placeholder="t('draftPlaceholder')"
        @keydown="onTextareaKeydown($event, submitDraft)"
      />
      <div class="cl-actions">
        <button type="button" class="cl-btn" @click="cancelDraft">{{ t("cancel") }}</button>
        <button type="button" class="cl-btn primary" :disabled="!draftBody.trim()" @click="submitDraft">
          {{ t("submit") }}
        </button>
      </div>
    </div>

    <div
      v-if="openPin && openPopupAt"
      class="cl-popup"
      :style="{ left: `${openPopupAt.x}px`, top: `${openPopupAt.y}px`, width: `${POPUP_SIZE.width}px` }"
    >
      <div v-if="openPin.pos.orphan" class="cl-orphan">{{ t("orphan") }}</div>
      <div v-if="openPin.comment.anchor.excerpt" class="cl-excerpt">{{ openPin.comment.anchor.excerpt }}</div>
      <div class="cl-thread">
        <div class="cl-msg">
          <div class="cl-msg-head">{{ authorLabel(openPin.comment.author) }}</div>
          <div class="cl-msg-body">{{ openPin.comment.body }}</div>
        </div>
        <div v-for="r in openPin.comment.replies" :key="r.id" class="cl-msg reply">
          <div class="cl-msg-head">{{ authorLabel(r.author) }}</div>
          <div class="cl-msg-body">{{ r.body }}</div>
        </div>
      </div>
      <textarea
        v-model="replyBody"
        class="cl-textarea"
        rows="2"
        :placeholder="t('replyPlaceholder')"
        @keydown="onTextareaKeydown($event, submitReply)"
      />
      <div class="cl-actions">
        <button type="button" class="cl-btn danger" @click="onRemove">{{ t("remove") }}</button>
        <span class="cl-spacer" />
        <button type="button" class="cl-btn" :disabled="!replyBody.trim()" @click="submitReply">
          {{ t("reply") }}
        </button>
        <button type="button" class="cl-btn primary" @click="onResolve">{{ t("resolve") }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.comment-layer {
  position: absolute;
  inset: 0;
  z-index: 20;
  /* モード外はコンテンツの操作を邪魔しない。ピンとポップアップだけが受ける */
  pointer-events: none;
  overflow: hidden;
}

.comment-layer.active {
  pointer-events: auto;
  cursor: crosshair;
  touch-action: none;
  outline: 2px solid var(--p-primary-color, #89b4fa);
  outline-offset: -2px;
}

.cl-hint,
.cl-pin,
.cl-popup {
  pointer-events: auto;
  cursor: default;
}

.cl-hint {
  position: absolute;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  border-radius: 14px;
  background: var(--p-primary-color, #89b4fa);
  color: var(--p-primary-contrast-color, #11111b);
  font-size: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
}

.cl-hint-exit {
  border: 1px solid currentColor;
  border-radius: 10px;
  background: transparent;
  color: inherit;
  font-size: 11px;
  padding: 0 8px;
  cursor: pointer;
}

.cl-drag {
  position: absolute;
  border: 1.5px dashed var(--p-primary-color, #89b4fa);
  background: rgba(137, 180, 250, 0.15);
  pointer-events: none;
}

.cl-pin {
  position: absolute;
  transform: translate(-50%, -50%);
  width: 26px;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 2px solid #fff;
  border-radius: 50%;
  background: var(--p-primary-color, #89b4fa);
  color: var(--p-primary-contrast-color, #11111b);
  box-shadow: 0 1px 6px rgba(0, 0, 0, 0.5);
  cursor: pointer;
  padding: 0;
}

.cl-pin i {
  font-size: 12px;
}

.cl-pin-index {
  position: absolute;
  top: -7px;
  right: -7px;
  min-width: 15px;
  height: 15px;
  border-radius: 8px;
  background: #11111b;
  color: #fff;
  font-size: 10px;
  line-height: 15px;
  text-align: center;
}

.cl-pin.orphan {
  background: #f9e2af;
  border-style: dashed;
}

.cl-pin.open {
  outline: 2px solid #fff;
}

.cl-popup {
  position: absolute;
  box-sizing: border-box;
  max-height: 80%;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border: 1px solid var(--p-content-border-color, #45475a);
  border-radius: 8px;
  background: var(--p-content-background, #1e1e2e);
  color: var(--p-text-color, #cdd6f4);
  font-size: 12px;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.5);
}

.cl-excerpt {
  padding: 4px 6px;
  border-left: 3px solid var(--p-primary-color, #89b4fa);
  background: rgba(255, 255, 255, 0.05);
  color: var(--p-text-muted-color, #a6adc8);
  font-size: 11px;
  max-height: 3.6em;
  overflow: hidden;
  word-break: break-all;
}

.cl-orphan {
  color: #f9e2af;
  font-size: 11px;
}

.cl-thread {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.cl-msg-head {
  font-weight: 600;
  font-size: 11px;
  color: var(--p-text-muted-color, #a6adc8);
}

.cl-msg.reply {
  padding-left: 10px;
  border-left: 2px solid var(--p-content-border-color, #45475a);
}

.cl-msg-body {
  white-space: pre-wrap;
  word-break: break-word;
}

.cl-textarea {
  width: 100%;
  box-sizing: border-box;
  resize: vertical;
  padding: 6px;
  border: 1px solid var(--p-content-border-color, #45475a);
  border-radius: 4px;
  background: transparent;
  color: inherit;
  font: inherit;
}

.cl-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.cl-spacer {
  flex: 1;
}

.cl-btn {
  padding: 3px 10px;
  border: 1px solid var(--p-content-border-color, #45475a);
  border-radius: 4px;
  background: transparent;
  color: inherit;
  font-size: 12px;
  cursor: pointer;
}

.cl-btn:hover:not(:disabled) {
  background: var(--p-content-hover-background, rgba(255, 255, 255, 0.08));
}

.cl-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.cl-btn.primary {
  background: var(--p-primary-color, #89b4fa);
  border-color: transparent;
  color: var(--p-primary-contrast-color, #11111b);
}

.cl-btn.danger {
  color: #f38ba8;
}
</style>

<i18n lang="json">
{
  "en": {
    "hint": "Comment mode: click a spot, or drag to select an area",
    "exit": "Done",
    "draftPlaceholder": "Write a comment (Ctrl+Enter to send)",
    "replyPlaceholder": "Reply (Ctrl+Enter to send)",
    "submit": "Comment",
    "cancel": "Cancel",
    "reply": "Reply",
    "resolve": "Resolve",
    "remove": "Delete",
    "orphan": "The original element is gone, so this is shown at its last position",
    "author": { "user": "You", "ai": "AI" }
  },
  "ja": {
    "hint": "コメントモード: 位置をクリック、またはドラッグで範囲を選択",
    "exit": "終了",
    "draftPlaceholder": "コメントを入力（Ctrl+Enter で送信）",
    "replyPlaceholder": "返信（Ctrl+Enter で送信）",
    "submit": "コメントする",
    "cancel": "キャンセル",
    "reply": "返信",
    "resolve": "解決",
    "remove": "削除",
    "orphan": "元の要素が見つからないため、最後の位置に表示しています",
    "author": { "user": "あなた", "ai": "AI" }
  }
}
</i18n>
