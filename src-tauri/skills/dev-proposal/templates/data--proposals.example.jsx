
// ── data/proposals テンプレート例 ─────────────────────────────────────────
// スキーマ確認用のサンプル。実際の `data/proposals` は Step 2〜3 の探索結果から新規生成する。
// **企画の唯一のデータソース**で、アーティファクトを開いている間は表示中ロックで更新できない。
// entry-point / components / lib にはカスタマイズ箇所が無く、可変値はすべてここに入れる。
//
// ── META ─────────────────────────────────────────────────────────────────
//   theme       ユーザーが与えたテーマ（原文）
//   repo        'owner/name'。`gh repo view --json nameWithOwner` で取得（推測で書かない）
//   repoUrl     `gh repo view --json url`。`relatedIssues` のリンクに使う。分からなければ省略
//   generatedAt 生成日時の表示用文字列。時刻が分からなければ日付だけでよい（推測で時刻を書かない）
//   sessionId   自セッションの PTY sessionId（`oretachi_list_terminals` の `sessionId`。camelCase）。
//               送信ボタンの宛先。アプリ再起動・タブ再作成で失効する
//   artifactId  このアーティファクトの ID。送信文面に載り、Step 5 の `artifact_store(read)` に使う
//   projectDir  生成セッションの作業ディレクトリ絶対パス。同上
//
// ── PROPOSALS[] ──────────────────────────────────────────────────────────
//   必須: id / title / size('L'|'M'|'S') / effect('high'|'mid'|'low') / summary / evidence
//   任意: risk('high'|'mid'|'low') / scope / outOfScope / doneWhen / tasks / dependsOn / relatedIssues
//   evidence は探索で見つけた根拠（`path` + `line` + `note`）。**根拠の無い一般論の案は出さない**。
exports.META = {
  theme: 'リファクタして',
  repo: 'owner/name',
  repoUrl: 'https://github.com/owner/name',
  generatedAt: '2026-10-07',
  sessionId: 12,
  artifactId: 'dev-proposal-1',
  projectDir: 'C:\\path\\to\\worktree',
};

exports.PROPOSALS = [
  {
    id: 'p1',
    title: '重複した make_command 定義を process_utils へ統合',
    size: 'S',
    effect: 'mid',
    risk: 'low',
    summary: '5 ファイルに同じ CREATE_NO_WINDOW 付きのコマンド生成が重複している。1 か所へ寄せて今後の修正漏れを防ぐ。',
    evidence: [{ path: 'src-tauri/src/foo.rs', line: 120, note: '重複定義が 5 箇所' }],
    scope: ['make_command / make_async_command の統合'],
    outOfScope: ['kill_process_tree の挙動変更'],
    doneWhen: ['重複定義が 0 件', 'cargo check が通る'],
    tasks: ['定義の統合', '呼び出し側の置換'],
    dependsOn: [],
    relatedIssues: [],
  },
  {
    id: 'p2',
    title: '設定画面のフォーム状態を composable へ切り出し',
    size: 'M',
    effect: 'high',
    risk: 'mid',
    summary: '設定画面が 1 ファイルに状態とロジックを抱えている。composable へ分けてテスト可能にする。',
    evidence: [{ path: 'src/components/Settings.vue', line: 1, note: '1500 行超' }],
    scope: ['フォーム状態の抽出'],
    outOfScope: ['見た目の変更'],
    doneWhen: ['Settings.vue が 800 行以下'],
    tasks: ['状態の抽出', 'テスト追加'],
    dependsOn: ['p1'],
    relatedIssues: [123],
  },
];
