
// ── data/flow テンプレート例 ──────────────────────────────────────
// このファイルが計画フローの唯一のデータソース。teamwork-parentは進捗が変わるたびに
// artifact_module(command:"update"/"rewrite", module_name:"data/flow") でこのファイルを
// 直接更新する。専用の進捗管理マークダウンは使わない — このartifactが唯一の進捗表示手段。
//
// フィールド仕様:
//   REPO_URL: このプロジェクトのリポジトリ URL(例 'https://github.com/<owner>/<repo>')。
//     タスクノードの issue 番号をこの URL 配下の issue へのリンクにするために使う。
//     `gh repo view --json url -q .url` で取れる値をそのまま入れる。
//     空文字のままなら issue 番号は素のテキストのまま出る。**別リポジトリの URL を
//     書き換え忘れると、全ノードが無関係な同番号 issue へリンクしてしまう**ので、
//     「分からないなら空のまま」が正しい(リンクが出ないだけで済む)。
//   StopCondition: { id, text, checked, checkedAt }
//     id: 一意なID。タスクは "sc-<issue番号>-<連番>"、エッジは "sc-e<エッジ番号>-<連番>"
//     text: 人が判定する内容を1行で
//     checked: true = クリア済み / false = 未クリア
//     checkedAt: クリアした日時 'YYYY-MM-DD HH:mm'(checked:true のときのみ)
//   TASKS (default export): [{ id, issueNumber, title, status, branch,
//     stopConditions, x, y }]
//     status: "not_started" | "in_progress" | "blocked" | "waiting_approval" | "done"
//     stopConditions: StopCondition[] — このタスク(=子ワークツリー)の停止条件。
//       子の作業中に発生し、子の上で止まってユーザーに確認する。
//       親はこの停止による子の停止では止まらない(MESSAGESに記録して流れを継続)。
//       sub-issue本文の `## 停止条件` セクションに同じ内容を書いて子へ伝達する。
//     --- 入れ子(sub-issue がさらに sub-issue を持つ場合)の任意フィールド ---
//     kind: 'group' — 「このセッションで同時管理」を選んだ中間 sub-issue。
//       ワークツリーを作らない(oretachi_add_task しない)ので branch は持たず、座標(x/y)も持たない。
//       枠の位置と大きさは entry-point が parentId が一致する子ノードの外接矩形から計算する。
//       status は親が data/flow に書く: 全子 done で done、子が1つでも動けば in_progress。
//       **done は子の状態だけで決まり、中間 issue のクローズ成否には依存しない。**
//     parentId: '<グループの id>' — グループに属する孫タスクが持つ。
//     delegated: true — 「子ワークツリーで teamwork-parent を起動(委任)」を選んだ sub-issue。
//       子ワークツリーが自分で孫を管理する(親はこの sub-issue をリーフとして扱う)。
//   DEPENDENCIES: [{ from, to, kind, stopConditions }]
//     kind: "blocks"(実線) | "informs"(破線)
//     グループも端点にできる。from がグループ = 全子 done で遷移成立。
//     to がグループ = グループ内で内部依存を持たない入口の子を解禁する。
//     stopConditions: StopCondition[] — 親ワークツリーの停止条件。
//       from → to の遷移(= to の子ワークツリーを起動する時点)で親が止まる。
//       未クリアの条件が残っている間、親は to の oretachi_add_task を実行しない。
//   MESSAGES: [{ ts, from, text }] — 受信メッセージログ。新しい順に配列の先頭へ追加する
//     (表示は先頭5件程度)
//
//   ※ 旧スキーマの `requiresUserConfirmation` / `confirmationNote` は `stopConditions` に
//     統合された。新規作成では使わない。`stopConditions` 未定義かつ
//     `requiresUserConfirmation: true` の旧データは `confirmationNote` を未クリア1件として
//     扱うフォールバックが TaskNode 側にあるが、更新のついでに書き換えること。
//
// 座標ガイドライン:
//   列幅: 280px（BOX_WIDTH 220 + 間隔 60）
//   列の開始x: 40, 320, 600, 880, 1160, ...
//   行の高さ: 84 + 間隔40 = 124px間隔
//   ※ 依存関係のトポロジカル・レベル(何段目の依存か)を列に割り当てると見やすい

// CUSTOMIZE: 対象リポジトリの URL を入れる(`gh repo view --json url -q .url`)。
// 例: 'https://github.com/<owner>/<repo>'。空のままならリンクにしない
const REPO_URL = '';

const TASKS = [
  // 停止条件の3フェーズが1枚で見えるサンプル:
  //   #140 = 実行済み(緑) / #141 = 実行中(橙・二重枠) / #142 = 未実行(灰)
  { id: 'task-140', issueNumber: 140, title: '購読イベントAPI整備', status: 'done',
    branch: 'issue-140', x: 40, y: 164,
    stopConditions: [
      { id: 'sc-140-1', text: '公開するイベント種別をユーザーが確認', checked: true, checkedAt: '2026-08-16 09:05' },
    ] },

  { id: 'task-141', issueNumber: 141, title: 'teamwork-parent SKILL実装', status: 'in_progress',
    branch: 'issue-141', x: 320, y: 40,
    stopConditions: [
      { id: 'sc-141-1', text: 'SKILL.mdの進行フローが固まったら確認', checked: true, checkedAt: '2026-08-16 09:40' },
      { id: 'sc-141-2', text: '禁止事項の書き方をユーザーが判断', checked: false },
    ] },

  { id: 'task-142', issueNumber: 142, title: '結合検証', status: 'not_started',
    branch: 'issue-142', x: 600, y: 164,
    stopConditions: [
      { id: 'sc-142-1', text: '追加テーブルのスキーマが確定したら確認', checked: false },
      { id: 'sc-142-2', text: '動作確認の結果をユーザーが見て判断', checked: false },
    ] },

  // 入れ子のサンプル: #143 は孫(#144/#145)を持つ中間 issue で「同時管理」を選んだ → グループ。
  // 孫は連続した同じ行に置く(枠の見出し分の余白は entry-point が上辺に足す)
  { id: 'task-143', issueNumber: 143, title: 'イベント配送の再設計', kind: 'group',
    status: 'not_started' },
  { id: 'task-144', issueNumber: 144, title: '配送キュー実装', status: 'not_started',
    parentId: 'task-143', branch: 'issue-144', x: 880, y: 80 },
  { id: 'task-145', issueNumber: 145, title: '再送ポリシー', status: 'not_started',
    parentId: 'task-143', branch: 'issue-145', x: 1160, y: 80 },

  // 「委任」を選んだ sub-issue: 子ワークツリーが自分で teamwork-parent を走らせる(リーフとして1ノード)
  { id: 'task-146', issueNumber: 146, title: '監視ダッシュボード', status: 'not_started',
    delegated: true, branch: 'issue-146', x: 1500, y: 80 },
];

const DEPENDENCIES = [
  // 実行済み(緑 ☑): この遷移の停止条件はクリア済みなので #141 に着手できている。
  // 未クリアのまま to 側を着手済みにしてはいけない(親の停止条件を飛ばしたことになる)
  { from: 'task-140', to: 'task-141', kind: 'blocks',
    stopConditions: [
      { id: 'sc-e0-1', text: '#140 のAPI仕様を見て #141 の着手可否を判断', checked: true, checkedAt: '2026-08-16 09:20' },
    ] },

  // 未実行(灰): 遷移元 #141 がまだ done でないため、この停止条件には到達していない
  { from: 'task-141', to: 'task-142', kind: 'blocks',
    stopConditions: [
      { id: 'sc-e1-1', text: '#141 の成果を見て #142 の分割方針を再確認', checked: false },
    ] },

  // 実行中(橙): 遷移元 #140 が done = 親が今まさに止まる地点。
  // informs(破線)でも停止条件があれば #142 の起動を止める。
  // #140 と #142 を同じ行に置くことで、この破線が #141 のボックスを横切らないようにしている
  { from: 'task-140', to: 'task-142', kind: 'informs',
    stopConditions: [
      { id: 'sc-e2-1', text: '#140 のイベント名が #142 の検証手順と噛み合うか確認', checked: false },
    ] },

  // 入れ子のサンプル。to がグループ = 入口の子(#144)を解禁、グループ内の内部依存は #144 → #145、
  // from がグループ = 全子 done で #146 へ遷移成立
  { from: 'task-142', to: 'task-143', kind: 'blocks' },
  { from: 'task-144', to: 'task-145', kind: 'blocks' },
  { from: 'task-143', to: 'task-146', kind: 'blocks' },
];

const MESSAGES = [
  { ts: '2026-08-16 10:20', from: '#141', text: 'worktree.message: 実装方針の確認、逸脱なし' },
  { ts: '2026-08-16 10:05', from: '#140', text: 'worktree.closed → done、次タスク(#141)着手' },
];

exports.default = TASKS;
exports.REPO_URL = REPO_URL;
exports.DEPENDENCIES = DEPENDENCIES;
exports.MESSAGES = MESSAGES;
