/** ワークツリー間イベントの購読・配送（issue #120 / #125）に関する型。
 *  Rust 側の `event_delivery.rs` の `SubscriptionView` / `OrphanedGroupView` /
 *  `TerminalIdEntry` と 1:1 で対応する（`serde(rename_all = "camelCase")`）。 */

/** Claude Code 以外のエージェントは Stop フック経路が存在せず、PTY 押し込みでしか
 *  通知を受け取れない（#120 §5.7）。UI ではこれで区別する。 */
export const HOOK_CAPABLE_AGENT = "claude";

/** 購読一覧の1行。 */
export interface SubscriptionView {
  id: string;
  /** 購読者タブ（PTY spawn 時に発番される UUID） */
  subscriberTerminalId: string;
  subscriberWorktreeId: string | null;
  subscriberWorktreeName: string | null;
  /** 購読者タブが生存していれば PTY セッション ID。null なら引き継ぎ待ち */
  subscriberSessionId: number | null;
  /** "claude" / "gemini" / "codex" / "cline"。null は AI エージェント未検出 */
  agentName: string | null;
  targetWorktreeId: string;
  /** クローズ済みの対象は null になる */
  targetWorktreeName: string | null;
  eventKinds: string[];
  /** "turn_end" | "interrupt" | "passive" */
  delivery: string;
  spawnIfClosed: boolean;
  /** "active" | "orphaned" */
  state: string;
  createdAt: number;
  expiresAt: number | null;
  orphanedAt: number | null;
  unacked: number;
  undelivered: number;
}

/** 引き継ぎ待ちの1グループ（死亡した1タブが残した購読と未読）。 */
export interface OrphanedGroupView {
  terminalId: string;
  worktreeId: string;
  worktreeName: string | null;
  orphanedAt: number;
  subscriptions: number;
  pending: number;
}

/** タブごとの未読件数（タブバーのバッジ用）。 */
export interface TerminalUnread {
  sessionId: number;
  terminalId: string;
  agentName: string | null;
  worktreeId: string | null;
  worktreeName: string | null;
  isAiAgent: boolean;
  unacked: number;
}

/** 配送トーストのペイロード（Rust の `event-delivered`）。 */
export interface DeliveredPayload {
  terminalId: string;
  sessionId: number;
  worktreeId: string | null;
  worktreeName: string | null;
  agentName: string | null;
  count: number;
  text: string;
  method: string;
}

/** 端末数上限で自動 spawn を拒否したときの通知（Rust の `event-spawn-rejected`）。 */
export interface SpawnRejectedPayload {
  worktreeId: string;
  worktreeName: string;
  liveSessions: number;
  limit: number;
  pending: number;
}

/** 自動 spawn 要求（Rust の `event-spawn-terminal`）。フロントはタブを作り、
 *  成否にかかわらず `event_spawn_result` で応答しなければならない。 */
export interface SpawnTerminalRequest {
  requestId: string;
  worktreeId: string;
  worktreeName: string;
  command: string;
  pending: number;
}
