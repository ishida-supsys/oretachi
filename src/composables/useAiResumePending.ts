import { reactive } from "vue";

/**
 * terminalId → 復元待ちの AI エージェント種別（#328）。
 *
 * `pendingAiRestore`（App.vue / SubWindowApp.vue、#157）とは別に持つ表示専用の状態。
 * `pendingAiRestore` は resume 投入判断のために「投入前に消して失敗時に戻す」動きをし、
 * サブウィンドウへ引き渡すとメイン側からは消えてしまうため、そのまま表示に使うと
 * ちらつきや分離ワークツリーでの表示漏れが起きる。
 *
 * モジュールスコープのシングルトンとして持ち、カードコンポーネントは
 * `subscriptionCounts`（useEventSubscriptions）と同じ要領で props リレー無しに直接読む。
 * カード一覧はメインウィンドウにしか無いため、メイン側（App.vue）だけが mark/clear すればよい。
 */
export const resumePendingTerminals = reactive(new Map<number, string>());

/** 復元待ち状態をマークする。agentType はバッジのツールチップ表示に使う */
export function markResumePending(terminalId: number, agentType: string) {
  resumePendingTerminals.set(terminalId, agentType);
}

/** 復元待ち状態を解除する（投入完了・失敗確定・タブ削除のいずれでも呼ぶ） */
export function clearResumePending(terminalId: number) {
  resumePendingTerminals.delete(terminalId);
}
