<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import type { SubscriptionView } from "../types/event";
import { subscriptions } from "../composables/useEventSubscriptions";
import { isExactWorktreeTarget } from "../utils/subscriptionCounts";

/** ワークツリーカードの購読バッジから開く購読関係のダイアログ（issue #137）。
 *
 *  outgoing（このワークツリーが購読している先）と incoming（このワークツリーを購読している元）を
 *  並べ、どちらの行からでもフォーカス先を選べる。フォーカスそのものは呼び出し側
 *  （App.vue → `useAppHotkeys` の `focusWorktree`）が担う。 */

const { t } = useI18n();

const props = defineProps<{
  worktreeId: string;
  worktreeName?: string;
}>();

const emit = defineEmits<{
  /** 選ばれたワークツリーへフォーカスする */
  focus: [worktreeId: string];
  cancel: [];
}>();

/** ダイアログの1行。`focusId` が null の行は選択できない。 */
interface Row {
  key: string;
  label: string;
  /** 購読者タブの PTY セッション ID。同じワークツリーの複数タブが同じ先を購読すると
   *  ラベルだけでは行を見分けられないので、購読パネルと同じ `#N` バッジを出す */
  sessionId: number | null;
  agentName: string | null;
  /** ワイルドカード購読（`*` / `workgroup:` / `repo:`）の種別。厳密一致なら null */
  wildcardKind: string | null;
  /** 相手のワークツリーが settings から消えている（= クローズ済み） */
  closed: boolean;
  orphaned: boolean;
  unacked: number;
  /** フォーカス先。定まらない行は null（＝選択不可） */
  focusId: string | null;
}

/** 購読対象の表示文字列。`*` は表示名を持たないのでローカライズした文言を出す（#126）。 */
function targetText(s: SubscriptionView): string {
  if (s.targetKind === "all") return t("targetAll");
  return s.targetLabel ?? s.targetWorktreeName ?? s.targetWorktreeId;
}

/** このワークツリーのタブが張っている購読（pi-share-alt 側）。ワイルドカードもここに出る。 */
const outgoing = computed<Row[]>(() =>
  subscriptions.value
    .filter((s) => s.subscriberWorktreeId === props.worktreeId)
    .map((s) => ({
      key: s.id,
      label: targetText(s),
      sessionId: s.subscriberSessionId,
      agentName: s.agentName,
      // 「クローズ済み」は厳密一致 target でのみ意味を持つ。targetWorktreeName が null で
      // あることだけを根拠にすると、ワイルドカード購読が全部誤表示される（#134）
      wildcardKind: isExactWorktreeTarget(s) ? null : s.targetKind,
      closed: isExactWorktreeTarget(s) && !s.targetWorktreeName,
      orphaned: s.state === "orphaned",
      unacked: s.unacked,
      // ワイルドカードはフォーカス先が定まらないので選択不可。クローズ済みも同様
      focusId: isExactWorktreeTarget(s) && s.targetWorktreeName ? s.targetWorktreeId : null,
    })),
);

/** このワークツリーを**厳密一致で**対象にしている購読（pi-bolt 側）。 */
const incoming = computed<Row[]>(() =>
  subscriptions.value
    .filter((s) => isExactWorktreeTarget(s) && s.targetWorktreeId === props.worktreeId)
    .map((s) => ({
      key: s.id,
      label: s.subscriberWorktreeName ?? s.subscriberWorktreeId ?? "-",
      sessionId: s.subscriberSessionId,
      agentName: s.agentName,
      wildcardKind: null,
      // 購読者ワークツリーが settings から消えていると名前が引けない。outgoing 側と同じく
      // 選択不可にする。ここを truthy な生 ID のまま渡すと、選択できるように見えて
      // `focusWorktree` が `worktrees` から引けず**無言で何も起きない**行になる。
      closed: !s.subscriberWorktreeName,
      orphaned: s.state === "orphaned",
      unacked: s.unacked,
      focusId: s.subscriberWorktreeName ? s.subscriberWorktreeId : null,
    })),
);

/** 2セクションは行の見た目が同じなので、マークアップを1つにまとめて回す。
 *  アイコンはカードの購読バッジと揃える（pi-share-alt = 購読している側 / pi-bolt = 届く側）。
 *  （outgoing にだけワイルドカードの種別バッジが出るが、incoming 側は `wildcardKind` が常に null） */
const sections = computed(() => [
  { key: "outgoing", icon: "pi-share-alt", title: t("outgoing"), rows: outgoing.value },
  { key: "incoming", icon: "pi-bolt", title: t("incoming"), rows: incoming.value },
]);

function onSelect(row: Row): void {
  if (!row.focusId) return;
  emit("focus", row.focusId);
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key !== "Escape") return;
  event.preventDefault();
  event.stopPropagation();
  emit("cancel");
}

// capture で拾う（ターミナル側のキーハンドラより先に Escape を消費する）
onMounted(() => window.addEventListener("keydown", onKeydown, true));
onUnmounted(() => window.removeEventListener("keydown", onKeydown, true));
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title') }}</h3>
      <p class="dialog-sub">{{ worktreeName ?? worktreeId }}</p>

      <section v-for="section in sections" :key="section.key" class="section">
        <h4 class="section-title">
          <i class="pi section-icon" :class="section.icon" aria-hidden="true" />
          {{ section.title }} ({{ section.rows.length }})
        </h4>
        <p v-if="section.rows.length === 0" class="empty">{{ t('none') }}</p>
        <ul v-else class="row-list">
          <li v-for="row in section.rows" :key="row.key">
            <button
              class="row"
              :class="{ 'row-disabled': !row.focusId }"
              :disabled="!row.focusId"
              :title="row.focusId ? t('focusHint') : t('notFocusable')"
              @click="onSelect(row)"
            >
              <span class="row-label">{{ row.label }}</span>
              <!-- 同じワークツリーの複数タブが同じ先を購読すると、ラベルだけでは
                   行を見分けられない（購読パネルと同じ #N / エージェント名を出す） -->
              <span v-if="row.sessionId !== null" class="tab-badge">#{{ row.sessionId }}</span>
              <span v-if="row.agentName" class="badge badge-agent">{{ row.agentName }}</span>
              <span v-if="row.wildcardKind" class="badge badge-wildcard">
                {{ t(`targetKind.${row.wildcardKind}`) }}
              </span>
              <span v-if="row.closed" class="badge badge-muted">{{ t('targetClosed') }}</span>
              <span v-if="row.orphaned" class="badge badge-orphaned" :title="t('orphanedHint')">
                {{ t('orphaned') }}
              </span>
              <span v-if="row.unacked > 0" class="badge badge-unread">{{ row.unacked }}</span>
            </button>
          </li>
        </ul>
      </section>

      <p class="hint">{{ t('wildcardHint') }}</p>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 10px;
  padding: 24px;
  width: 460px;
  max-width: 90vw;
  max-height: 80vh;
  overflow-y: auto;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
  margin: 0 0 4px;
}

.dialog-sub {
  font-size: 13px;
  color: #a6adc8;
  margin: 0 0 16px;
}

.section {
  margin-bottom: 16px;
}

.section-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: #cdd6f4;
  margin: 0 0 6px;
}

/* カードの購読バッジと同じ配色にして、どちらの方向かを一目で結び付ける */
.section-icon {
  font-size: 11px;
  color: #94e2d5;
}

.empty {
  font-size: 12px;
  color: #6c7086;
  margin: 0 0 4px;
}

.row-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  text-align: left;
  background: #181825;
  border: 1px solid #313244;
  border-radius: 4px;
  padding: 6px 10px;
  font-size: 13px;
  color: #cdd6f4;
  cursor: pointer;
}

.row:hover:not(.row-disabled) {
  border-color: #89b4fa;
}

.row-disabled {
  cursor: default;
  color: #6c7086;
}

.row-label {
  font-family: monospace;
  /* バッジが増えても label 側だけが縮んで省略されるようにする */
  min-width: 0;
  flex: 0 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tab-badge {
  flex-shrink: 0;
  font-family: monospace;
  font-size: 11px;
  color: #6c7086;
}

.badge {
  flex-shrink: 0;
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 4px;
  white-space: nowrap;
}

.badge-agent {
  background: #313244;
  color: #a6e3a1;
}

.badge-wildcard {
  background: #313244;
  color: #94e2d5;
}

.badge-muted {
  background: #313244;
  color: #6c7086;
}

.badge-orphaned {
  background: #45324a;
  color: #fab387;
}

.badge-unread {
  background: #45324a;
  color: #f38ba8;
  margin-left: auto;
}

.hint {
  font-size: 11px;
  color: #6c7086;
  margin: 0 0 12px;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.btn-cancel {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  cursor: pointer;
}

.btn-cancel:hover {
  background: #45475a;
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Subscriptions",
    "outgoing": "Subscribing to",
    "incoming": "Subscribed by",
    "none": "None",
    "focusHint": "Focus this worktree",
    "notFocusable": "This row has no single worktree to focus",
    "targetAll": "All worktrees",
    "targetClosed": "closed",
    "orphaned": "awaiting handover",
    "orphanedHint": "The subscribing tab is gone. Messages keep piling up and are handed over to the next AI agent started in the same worktree.",
    "wildcardHint": "Wildcard subscriptions (all / group / repository) are listed but cannot be focused, and are not counted in the incoming badge.",
    "targetKind": {
      "all": "all",
      "workgroup": "group",
      "repo": "repository"
    }
  },
  "ja": {
    "title": "購読関係",
    "outgoing": "このワークツリーが購読している先",
    "incoming": "このワークツリーを購読している元",
    "none": "なし",
    "focusHint": "このワークツリーにフォーカスする",
    "notFocusable": "フォーカス先のワークツリーが定まりません",
    "targetAll": "すべてのワークツリー",
    "targetClosed": "クローズ済み",
    "orphaned": "引き継ぎ待ち",
    "orphanedHint": "購読していたタブがありません。メッセージは溜まり続け、同じワークツリーで次に AI エージェントが立ち上がったときに引き継がれます",
    "wildcardHint": "ワイルドカード購読（全体 / グループ / リポジトリ）は表示のみで、フォーカスできません。「購読している元」の件数にも含まれません。",
    "targetKind": {
      "all": "全体",
      "workgroup": "グループ",
      "repo": "リポジトリ"
    }
  }
}
</i18n>
