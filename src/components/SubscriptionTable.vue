<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { OrphanedGroupView, SubscriptionView } from "../types/event";
import { HOOK_CAPABLE_AGENT } from "../types/event";

const { t } = useI18n();

const props = defineProps<{
  items: SubscriptionView[];
  /** 引き継ぎ待ちグループ。購読行が既に消えているもの（対象クローズ後の未読だけ）も含む */
  orphanedGroups: OrphanedGroupView[];
  /** 引き継ぎ先の候補: 生存している AI エージェント端末 */
  agentTerminals: { sessionId: number; worktreeId: string | null; label: string }[];
}>();

const emit = defineEmits<{
  unsubscribe: [subscriptionId: string];
  rebind: [payload: { worktreeId: string; deadTerminalId: string; sessionId: number }];
  ackAll: [terminalId: string];
}>();

function formatDate(ts: number | null): string {
  return ts ? new Date(ts).toLocaleString() : "-";
}

/** 購読対象の表示文字列。`*` は表示名を持たないのでローカライズした文言を出す（#126）。 */
function targetText(item: SubscriptionView): string {
  if (item.targetKind === "all") return t("targetAll");
  return item.targetLabel ?? item.targetWorktreeId;
}

/** そのワークツリーで引き継ぎ先に選べる生存 AI 端末。 */
function candidatesFor(worktreeId: string) {
  return props.agentTerminals.filter((c) => c.worktreeId === worktreeId);
}

/** 引き継ぎ待ちグループのうち、購読行が1つも残っていないもの。
 *  `worktree.closed` は配送直後に購読行が消えるので、未読だけが宙に浮くこの形が本命。 */
const inboxOnlyGroups = computed(() =>
  props.orphanedGroups.filter((g) => g.subscriptions === 0 && g.pending > 0),
);

function onRebind(worktreeId: string, deadTerminalId: string, event: Event): void {
  const select = event.target as HTMLSelectElement;
  const sessionId = Number(select.value);
  // 選択値は毎回プレースホルダへ戻す。引き継ぎに失敗して行が残ったとき、同じ候補を
  // 選び直しても change が発火せず、二度選び替えないと再試行できなくなるため。
  select.value = "";
  if (!Number.isFinite(sessionId) || sessionId <= 0) return;
  emit("rebind", { worktreeId, deadTerminalId, sessionId });
}
</script>

<template>
  <div class="subscription-table-wrapper">
    <table v-if="items.length > 0" class="subscription-table">
      <thead>
        <tr>
          <th>{{ t('colSubscriber') }}</th>
          <th>{{ t('colTarget') }}</th>
          <th>{{ t('colDelivery') }}</th>
          <th>{{ t('colState') }}</th>
          <th>{{ t('colUnread') }}</th>
          <th>{{ t('colCreatedAt') }}</th>
          <th class="col-actions"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="item in items" :key="item.id">
          <td class="cell-name">
            {{ item.subscriberWorktreeName ?? item.subscriberWorktreeId ?? '-' }}
            <span v-if="item.subscriberSessionId !== null" class="tab-badge">
              #{{ item.subscriberSessionId }}
            </span>
            <!-- フックは Claude Code 固有。非 CC は Stop 経路が無く PTY 押し込みだけで届く -->
            <span
              v-if="item.agentName && item.agentName !== HOOK_CAPABLE_AGENT"
              class="badge badge-warn"
              :title="t('nonCcHint')"
            >{{ item.agentName }} / {{ t('ptyOnly') }}</span>
            <span v-else-if="item.agentName" class="badge badge-agent">{{ item.agentName }}</span>
          </td>
          <td class="cell-target">
            <span class="branch-badge">{{ targetText(item) }}</span>
            <!-- 「クローズ済み」は厳密一致 target でのみ意味を持つ。ワイルドカード購読は
                 対象ワークツリー名を持たないので、名前の有無だけで判定すると全部誤表示になる -->
            <span
              v-if="item.targetKind === 'worktree' && !item.targetWorktreeName"
              class="badge badge-muted"
            >{{ t('targetClosed') }}</span>
            <span v-else-if="item.targetKind !== 'worktree'" class="badge badge-wildcard">
              {{ t(`targetKind.${item.targetKind}`) }}
            </span>
            <span v-for="kind in item.eventKinds" :key="kind" class="badge badge-kind">
              {{ kind.replace('worktree.', '') }}
            </span>
          </td>
          <td class="cell-delivery">
            {{ item.delivery }}
            <span v-if="item.spawnIfClosed" class="badge badge-spawn" :title="t('spawnHint')">
              {{ t('spawnIfClosed') }}
            </span>
          </td>
          <td class="cell-state">
            <template v-if="item.state === 'orphaned'">
              <span class="badge badge-orphaned" :title="t('orphanedHint')">{{ t('orphaned') }}</span>
              <select
                v-if="item.subscriberWorktreeId && candidatesFor(item.subscriberWorktreeId).length > 0"
                class="rebind-select"
                :title="t('rebindHint')"
                @change="onRebind(item.subscriberWorktreeId!, item.subscriberTerminalId, $event)"
              >
                <option value="">{{ t('rebindTo') }}</option>
                <option
                  v-for="c in candidatesFor(item.subscriberWorktreeId)"
                  :key="c.sessionId"
                  :value="c.sessionId"
                >{{ c.label }}</option>
              </select>
              <!-- 自動引き継ぎを同じ AI セッションに限ったので、手動引き継ぎが唯一の逃げ道に
                   なった。候補が無いときに何も出さないと「badge だけあって操作できない」に
                   見えるため、下の未読テーブルと同じく理由を出す -->
              <span v-else class="muted">{{ t('noCandidate') }}</span>
            </template>
            <span v-else class="badge badge-active">{{ t('active') }}</span>
          </td>
          <td class="cell-unread">
            <template v-if="item.unacked > 0">
              <span class="unread-badge">{{ item.unacked }}</span>
              <button
                class="btn-ack"
                :title="t('ackAllTitle')"
                @click="emit('ackAll', item.subscriberTerminalId)"
              >
                <span class="pi pi-check" />
              </button>
            </template>
            <span v-else class="muted">-</span>
          </td>
          <td class="cell-date">{{ formatDate(item.createdAt) }}</td>
          <td class="cell-actions">
            <button class="btn-delete" :title="t('unsubscribeTitle')" @click="emit('unsubscribe', item.id)">
              <span class="pi pi-trash" />
            </button>
          </td>
        </tr>
      </tbody>
    </table>

    <!-- 購読行が消えたあとに残った未読（対象クローズ後にアプリを再起動した場合など）。
         これを出さないと「消えたように見えて実は保持されている」状態が不可視になる。 -->
    <div v-if="inboxOnlyGroups.length > 0" class="orphan-section">
      <div class="orphan-title">{{ t('pendingHandover') }}</div>
      <table class="subscription-table">
        <thead>
          <tr>
            <th>{{ t('colSubscriber') }}</th>
            <th>{{ t('colUnread') }}</th>
            <th>{{ t('colOrphanedAt') }}</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="g in inboxOnlyGroups" :key="g.terminalId">
            <td class="cell-name">{{ g.worktreeName ?? g.worktreeId }}</td>
            <td class="cell-unread"><span class="unread-badge">{{ g.pending }}</span></td>
            <td class="cell-date">{{ formatDate(g.orphanedAt) }}</td>
            <td>
              <select
                v-if="candidatesFor(g.worktreeId).length > 0"
                class="rebind-select"
                :title="t('rebindHint')"
                @change="onRebind(g.worktreeId, g.terminalId, $event)"
              >
                <option value="">{{ t('rebindTo') }}</option>
                <option v-for="c in candidatesFor(g.worktreeId)" :key="c.sessionId" :value="c.sessionId">
                  {{ c.label }}
                </option>
              </select>
              <span v-else class="muted">{{ t('noCandidate') }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.subscription-table-wrapper {
  width: 100%;
  overflow-x: auto;
}

.subscription-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}

.subscription-table thead tr {
  background: #181825;
  border-bottom: 1px solid #313244;
}

.subscription-table th {
  padding: 8px 12px;
  text-align: left;
  font-size: 11px;
  font-weight: 600;
  color: #6c7086;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  white-space: nowrap;
}

.subscription-table tbody tr {
  border-bottom: 1px solid #1e1e2e;
  transition: background 0.1s;
}

.subscription-table tbody tr:hover {
  background: #181825;
}

.subscription-table td {
  padding: 9px 12px;
  color: #cdd6f4;
  vertical-align: middle;
}

.cell-name {
  font-weight: 500;
}

.branch-badge {
  font-family: monospace;
  font-size: 12px;
  background: #313244;
  padding: 2px 8px;
  border-radius: 4px;
  color: #cdd6f4;
}

.badge {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 4px;
  margin-left: 6px;
  white-space: nowrap;
}

.badge-agent {
  background: #313244;
  color: #a6e3a1;
}

.badge-warn {
  background: #45324a;
  color: #f9e2af;
}

.badge-orphaned {
  background: #45324a;
  color: #fab387;
}

.badge-active {
  background: #313244;
  color: #89b4fa;
}

.badge-muted {
  background: #313244;
  color: #6c7086;
}

.badge-spawn {
  background: #313244;
  color: #f5c2e7;
}

/* ワイルドカード target（* / workgroup: / repo:）の種別バッジ */
.badge-wildcard {
  background: #313244;
  color: #94e2d5;
}

/* 購読中のイベント種別。通知種別(kind)とは別物であることを画面でも区別できるようにする */
.badge-kind {
  background: #1e1e2e;
  color: #cdd6f4;
  border: 1px solid #45475a;
}

.tab-badge {
  font-family: monospace;
  font-size: 11px;
  color: #6c7086;
  margin-left: 6px;
}

.unread-badge {
  display: inline-block;
  min-width: 18px;
  text-align: center;
  background: #f38ba8;
  color: #11111b;
  font-size: 11px;
  font-weight: 700;
  padding: 1px 6px;
  border-radius: 9px;
}

.rebind-select {
  margin-left: 6px;
  background: #1e1e2e;
  color: #cdd6f4;
  border: 1px solid #313244;
  border-radius: 4px;
  font-size: 11px;
  padding: 1px 4px;
}

.cell-date {
  color: #6c7086;
  font-size: 12px;
  white-space: nowrap;
}

.muted {
  color: #6c7086;
}

.orphan-section {
  margin-top: 18px;
}

.orphan-title {
  font-size: 11px;
  font-weight: 600;
  color: #fab387;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  padding: 6px 12px;
}

.col-actions,
.cell-actions {
  width: 40px;
  text-align: center;
}

.btn-ack {
  background: none;
  border: none;
  padding: 2px 5px;
  margin-left: 4px;
  border-radius: 4px;
  color: #6c7086;
  cursor: pointer;
  font-size: 11px;
  transition: color 0.15s, background 0.15s;
}

.btn-ack:hover {
  color: #a6e3a1;
  background: #313244;
}

.btn-delete {
  background: none;
  border: none;
  padding: 4px 6px;
  border-radius: 4px;
  color: #6c7086;
  cursor: pointer;
  font-size: 13px;
  transition: color 0.15s, background 0.15s;
}

.btn-delete:hover {
  color: #f38ba8;
  background: #313244;
}
</style>

<i18n lang="json">
{
  "en": {
    "colSubscriber": "Subscriber",
    "colTarget": "Target",
    "colDelivery": "Delivery",
    "colState": "State",
    "colUnread": "Unread",
    "colCreatedAt": "Created",
    "colOrphanedAt": "Orphaned",
    "unsubscribeTitle": "Remove subscription",
    "ackAllTitle": "Mark all unread messages for this tab as read",
    "active": "active",
    "orphaned": "awaiting handover",
    "orphanedHint": "The subscribing tab is gone. Messages keep piling up. Automatic handover only happens for the same AI session (a conversation resumed with --resume), so pick a target tab to hand it to another session (kept for 7 days).",
    "rebindTo": "Hand over to...",
    "rebindHint": "Hand this subscription and its unread messages over to a live agent tab. Automatic handover is limited to the same AI session, so use this to pass it to a different session.",
    "noCandidate": "no live agent tab",
    "pendingHandover": "Unread messages awaiting handover",
    "targetClosed": "closed",
    "targetAll": "All worktrees",
    "targetKind": {
      "all": "wildcard",
      "workgroup": "workgroup",
      "repo": "repository"
    },
    "ptyOnly": "PTY push only",
    "nonCcHint": "Only Claude Code has hooks. Other agents have no Stop hook, so messages are delivered by writing into the PTY.",
    "spawnIfClosed": "auto spawn",
    "spawnHint": "Starts a new tab with an agent when unread messages pile up and no tab is open"
  },
  "ja": {
    "colSubscriber": "購読者",
    "colTarget": "購読対象",
    "colDelivery": "配送戦略",
    "colState": "状態",
    "colUnread": "未読",
    "colCreatedAt": "登録日時",
    "colOrphanedAt": "待機開始",
    "unsubscribeTitle": "購読を解除",
    "ackAllTitle": "このタブの未読をすべて既読にする",
    "active": "有効",
    "orphaned": "引き継ぎ待ち",
    "orphanedHint": "購読していたタブがありません。メッセージは溜まり続けます。自動で引き継がれるのは同じ AI セッション（--resume で再開した会話）だけなので、別のセッションへ渡すには引き継ぎ先を選んでください（7日間保持）",
    "rebindTo": "引き継ぎ先...",
    "rebindHint": "この購読と未読メッセージを、生存しているエージェント端末へ引き継ぐ。自動引き継ぎは同じ AI セッションに限られるので、別のセッションへ渡すにはこれを使う",
    "noCandidate": "エージェント端末なし",
    "pendingHandover": "引き継ぎ待ちの未読メッセージ",
    "targetClosed": "クローズ済み",
    "targetAll": "全ワークツリー",
    "targetKind": {
      "all": "ワイルドカード",
      "workgroup": "ワークグループ",
      "repo": "リポジトリ"
    },
    "ptyOnly": "PTY 押し込みのみ",
    "nonCcHint": "フックは Claude Code 固有です。他のエージェントには Stop フックが無いため、PTY への押し込みでのみ通知が届きます",
    "spawnIfClosed": "自動 spawn",
    "spawnHint": "タブが無い状態で未読が溜まったら、新しいタブを立ててエージェントを起動します"
  }
}
</i18n>
