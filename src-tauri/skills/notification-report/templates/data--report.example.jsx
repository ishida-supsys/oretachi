
// ── data/report テンプレート例 ────────────────────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の `data/report` は Step 1 で収集した
// 通知・ワークツリー情報から新規生成する。**レポートの唯一のデータソース**で、
// レポートを開いている間は表示中ロックで更新できない（スナップショット）。
//
// ── META フィールド仕様 ──────────────────────────────────────────────────
//   reportId       (string) : アーティファクトID と同じ値。送信テキストの先頭に入る
//   generatedAt    (string) : 生成時刻（`YYYY-MM-DD HH:MM` か `HH:MM`）
//   callerWorktree (string) : レポートを置いたワークツリー名（購読の主体）
//
// ── NOTIFICATIONS 配列フィールド仕様 ─────────────────────────────────────
//   id            (string)   : カードの一意キー。inbox メッセージ ID をそのまま使ってよい
//   inboxIds      (string[]) : この カードが束ねる inbox メッセージ ID。送信成功後に ack する
//   worktreeName  (string)   : 発信元ワークツリー名（= 送信先）
//   worktreeId    (string)   : 発信元ワークツリーID（購読の突合に使う。表示はしない）
//   sessionId     (number)   : 送信先の PTY セッションID。**null 可**（稼働中 AI 端末なし）
//   subscribed    (boolean)  : callerWorktree がこの宛先を購読しているか。false なら送信不可表示
//   issueRef      (string)   : `#187` など。無ければ省略可
//   at            (string)   : 通知の到着時刻（`HH:MM`）
//   kind          (string)   : general / approval / completed / hook
//   desc          (string)   : ワークツリーの description。**未設定なら null**
//   descFallback  (string)   : desc が null のとき、ターミナルから推定したミッション
//   phase         (string)   : 設計中 / 実装中 / 実装完了 / レビュー対応中 / 停止条件待ち / 不明
//   phaseSummary  (string)   : 現況の 1 行要約（ターミナル読み取りから）
//   readAt        (string)   : ターミナルを読んだ時刻（`HH:MM`）
//   body          (string)   : 通知本文。**要約せず全文を入れる**（人の判断材料）
//   link          (string)   : 子アーティファクトへの `artifact://` リンク。無ければ null
//   linkLabel     (string)   : リンクの表示名
//   choices       (string[]) : 候補ボタン。通知内容から作る。`その他（補足で指示）` は
//                              コード側で常に足されるので**ここに入れない**

const META = {
  reportId: 'notif-report-20260908-1432',
  generatedAt: '2026-09-08 14:32',
  callerWorktree: 'oretachi-vy7f',
};

const NOTIFICATIONS = [
  {
    id: 'inbox-9a1',
    inboxIds: ['inbox-9a1'],
    worktreeName: 'oretachi-xaoe',
    worktreeId: '1788700000000-xaoe',
    sessionId: 14,
    subscribed: true,
    issueRef: '#187',
    at: '14:21',
    kind: 'approval',
    desc: 'アーティファクトのリポジトリ保管庫への転送 (#187)',
    descFallback: null,
    phase: '停止条件待ち',
    phaseSummary: '転送先スコープの確定待ちで停止中。実装は未着手',
    readAt: '14:30',
    body: '停止条件クリア: [sc-187-1]「追加テーブルのスキーマが確定したら確認」→ 現行スキーマのまま進めてよいか判断が欲しい',
    link: 'artifact://worktree/1788700000000-xaoe/schema-review-187',
    linkLabel: 'スキーマ比較アーティファクトを開く',
    choices: ['了解', 'この方針で進めて', '別案も見たい', '保留'],
  },
  {
    id: 'inbox-9a2',
    inboxIds: ['inbox-9a2'],
    worktreeName: 'oretachi-xqle',
    worktreeId: '1788710000000-xqle',
    sessionId: 21,
    subscribed: true,
    issueRef: '#195',
    at: '14:08',
    kind: 'approval',
    desc: '通知レポート機能の親issue進行管理 (#195)',
    descFallback: null,
    phase: '実装完了',
    phaseSummary: '実装完了・未 commit。セルフレビュー待ち。差分 12 ファイル / 約 900 行',
    readAt: '14:30',
    body: 'PR を分割すべきか判断が欲しい。#201 のリンク実装と #203 のロック実装を 1 本にまとめると差分が約 900 行になる。',
    link: null,
    linkLabel: null,
    choices: ['分割して', '1 本でよい', '詳細を教えて', '保留'],
  },
  {
    // description 未設定 + AI 端末が居ないケース（送信不可としてカードに出る）
    id: 'inbox-9a3',
    inboxIds: ['inbox-9a3'],
    worktreeName: 'oretachi-orqn',
    worktreeId: '1788720000000-orqn',
    sessionId: null,
    subscribed: true,
    issueRef: '#120',
    at: '13:55',
    kind: 'general',
    desc: null,
    descFallback: 'ワークツリー購読と inbox の実装（ターミナルから推定）',
    phase: '実装中',
    phaseSummary: 'event_db のマイグレーション作成中。テストは未実行',
    readAt: '14:31',
    body: 'worktree.closed を受けたが、停止条件 sc-120-2 がまだ未チェックのまま。実態としてクリア済みか確認したい。',
    link: 'artifact://worktree/1788720000000-orqn/teamwork-plan-120',
    linkLabel: '計画フロー図を開く',
    choices: ['クリア済み', '未クリア', '詳細を教えて', '保留'],
  },
];

exports.META = META;
exports.NOTIFICATIONS = NOTIFICATIONS;
