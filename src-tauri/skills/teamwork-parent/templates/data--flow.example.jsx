
// ── data/flow テンプレート例 ──────────────────────────────────────
// このファイルが計画フローの唯一のデータソース。teamwork-parentは進捗が変わるたびに
// artifact_module(command:"update"/"rewrite", module_name:"data/flow") でこのファイルを
// 直接更新する。専用の進捗管理マークダウンは使わない — このartifactが唯一の進捗表示手段。
//
// フィールド仕様:
//   TASKS (default export): [{ id, issueNumber, title, status, branch,
//     requiresUserConfirmation, confirmationNote, x, y }]
//     status: "not_started" | "in_progress" | "blocked" | "waiting_approval" | "done"
//   DEPENDENCIES: [{ from, to, kind }] — kind: "blocks"(実線) | "informs"(破線)
//   MESSAGES: [{ ts, from, text }] — 受信メッセージログ。新しい順に配列の先頭へ追加する
//     (表示は先頭5件程度)
//
// 座標ガイドライン:
//   列幅: 280px（BOX_WIDTH 220 + 間隔 60）
//   列の開始x: 40, 320, 600, 880, 1160, ...
//   行の高さ: 84 + 間隔40 = 124px間隔
//   ※ 依存関係のトポロジカル・レベル(何段目の依存か)を列に割り当てると見やすい

const TASKS = [
  { id: 'task-140', issueNumber: 140, title: '購読イベントAPI整備', status: 'done',
    branch: 'issue-140', requiresUserConfirmation: false, x: 40, y: 40 },

  { id: 'task-141', issueNumber: 141, title: 'teamwork-parent SKILL実装', status: 'in_progress',
    branch: 'issue-141', requiresUserConfirmation: false, x: 320, y: 40 },

  { id: 'task-142', issueNumber: 142, title: '結合検証', status: 'not_started',
    branch: 'issue-142', requiresUserConfirmation: true,
    confirmationNote: '動作確認の結果をユーザーが見て判断', x: 600, y: 40 },
];

const DEPENDENCIES = [
  { from: 'task-140', to: 'task-141', kind: 'blocks' },
  { from: 'task-141', to: 'task-142', kind: 'blocks' },
];

const MESSAGES = [
  { ts: '2026-08-16 10:20', from: '#141', text: 'worktree.message: 実装方針の確認、逸脱なし' },
  { ts: '2026-08-16 10:05', from: '#140', text: 'worktree.closed → done、次タスク(#141)着手' },
];

exports.default = TASKS;
exports.DEPENDENCIES = DEPENDENCIES;
exports.MESSAGES = MESSAGES;
