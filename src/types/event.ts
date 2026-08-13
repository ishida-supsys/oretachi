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
  /** DB に入っている生の target。ワイルドカードなら "*" / "workgroup:<id>" / "repo:<name>" */
  targetWorktreeId: string;
  /** 厳密一致 target のワークツリー名。クローズ済み・ワイルドカードでは null */
  targetWorktreeName: string | null;
  /** "worktree" | "all" | "workgroup" | "repo"（#126）。
   *  **「クローズ済み」の判定はこれを見ること。** targetWorktreeName が null であることだけを
   *  根拠にすると、ワイルドカード購読がすべて「クローズ済み」と誤表示される。 */
  targetKind: string;
  /** 人間向けの表示名。"*" は null（UI 側でローカライズする） */
  targetLabel: string | null;
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

/** 端末数が webview ハングの危険域にある状態で自動 spawn したときの通知
 *  （Rust の `event-spawn-warning`）。
 *
 *  **spawn は止めていない。** `threshold` は拒否の上限ではなく警告のしきい値で、
 *  実際の歯止めは PTY セッション数のハードリミット側にある。 */
export interface SpawnWarningPayload {
  worktreeId: string;
  worktreeName: string;
  liveSessions: number;
  threshold: number;
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
