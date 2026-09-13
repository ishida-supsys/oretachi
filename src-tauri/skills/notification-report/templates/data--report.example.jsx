
// ── data/report テンプレート例 ────────────────────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の `data/report` は Step 1 で収集した
// 通知・ワークツリー情報から新規生成する。**レポートの唯一のデータソース**で、
// レポートを開いている間は表示中ロックで更新できない（スナップショット）。
//
// ══ 大原則: カードは画面の写しではなくフォーム（#264）════════════════════
//
// **生データ（hook JSON / 画面のダンプ）をカードに載せない。** それが読めないから
// レポートを作っている。折りたたみで残すのもやらない。生成側が構造化して入れる:
//
//   paragraphs / bullets / fields / links   ← 人が読む本文
//   request                                  ← 「何を聞かれているか」（通知の hook JSON 由来）
//   prompt                                   ← 送信直前の照合用（表示には使わない）
//
// 材料が足りないときは **①ターミナルを読む → ②該当ワークツリーの issue / git /
// アーティファクトを直接見に行く** で埋める。それでも分からない項目は「不明」と
// 明示する（推測を書かない）。
//
// ── META フィールド仕様 ──────────────────────────────────────────────────
//   reportId       (string) : アーティファクトID と同じ値。送信テキストの先頭に入る
//   generatedAtMs  (number) : レポートの基準時刻（epoch ms）。**載せた通知のうち最大の
//                             `createdAt` をそのまま入れる。現在時刻を推測して入れない**
//                             （このセッションには時刻が渡っていない。#220）。
//                             表示用の整形は entry-point 側の `generatedLabel` がやる
//   callerWorktree (string) : レポートを置いたワークツリー名（購読の主体）
//   repoUrl        (string) : リポジトリの URL（`https://github.com/<owner>/<repo>`）。
//                             本文中の `#203` と `issueRef` をこれでリンクにする。
//                             **分からなければ省略する**（当てずっぽうのリンクは張らない。
//                             省略すると `#203` はコード表記のまま出る）。#264
//
// ── NOTIFICATIONS 配列フィールド仕様 ─────────────────────────────────────
// ── カードは 2 種類ある（#228） ──────────────────────────────────────────
//
// `kind` が `worktree.created` / `worktree.closed` のカードは**報告カード**で、
// 人の判断を必要としない（返答 UI が出ない）。判定は `lib/send` の `isReportOnly`
// が `kind` から機械的にやるので、**この配列にフラグを足す必要は無い**。
// 報告カードで埋めるのは次の 7 つだけ:
//
//   id / inboxIds / worktreeName / worktreeId / kind / at / paragraphs
//   （+ 任意で branchName / issueRef / links）
//
// **報告カードでは `sessionId` / `subscribed` / `prompt` / `request` / `desc` /
// `descFallback` / `phase` / `phaseSummary` / `readAt` / `choices` を集めない。**
// 送信経路が無いので使われないうえ、`worktree.closed` は発信元ワークツリーが
// 既に削除済みで `oretachi_get_worktree_status` も `oretachi_read_terminal` も引けない。
// `worktreeName` / `branchName` は**通知本文（`WorktreeClosedBody` /
// `WorktreeCreatedBody`）に焼き付いている値**を使う。
//
// 配列は「要返答カード → 報告カード」の順に置く（`entry-point.jsx` が描画時にも
// 同じ並べ替えをするが、同じ session への 2 枚目を塞ぐ `promptConflicts` は
// **この配列の順**で先頭を生かすので、要返答カードの相対順序は崩さない）。
//
//   id            (string)   : カードの一意キー。inbox メッセージ ID をそのまま使ってよい
//   inboxIds      (string[]) : このカードが束ねる inbox メッセージ ID。**生成時（Step 5.5）に
//                              ack 済み**の記録で、レポート側からは ack しない（#219）
//   worktreeName  (string)   : 発信元ワークツリー名（= 送信先）
//   worktreeId    (string)   : 発信元ワークツリーID。**報告カードも含め全カードで必須**
//                              （#294）。購読の突合と、生成時のトレイ通知クリア
//                              （oretachi_clear_worktree_notification）に使う。
//                              oretachi_poll_inbox の sourceWorktreeId をそのまま入れる
//                              （表示はしない）。worktree.closed でも null にならない
//   sessionId     (number)   : 送信先の PTY セッションID。**null 可**（稼働中 AI 端末なし）。
//                              （このサンプルの `99xxx` は実在しない値。**例をそのまま
//                              動かしても本物の端末へ書き込まないため**にわざと外してある）
//   subscribed    (boolean)  : callerWorktree がこの宛先を購読しているか。false なら送信不可表示
//   issueRef      (string)   : `#187` など。`META.repoUrl` があればリンクになる
//   at            (string)   : 通知の到着時刻（`HH:MM`）
//   kind          (string)   : general / approval / completed / hook / worktree.message
//                              / worktree.created / worktree.closed。
//                              **後ろ 2 つは報告カードになる**（返答 UI が出ない。#228）
//   branchName    (string)   : 報告カード専用。通知本文の `branchName`。無ければ省略可
//   desc          (string)   : ワークツリーの description。**未設定なら null**
//   descFallback  (string)   : desc が null のとき、ターミナルから推定したミッション
//   phase         (string)   : 設計中 / 実装中 / 実装完了 / レビュー対応中 / 停止条件待ち / 不明
//   phaseSummary  (string)   : 現況の 1 行要約（ターミナル読み取りから）
//   readAt        (string)   : ターミナルを読んだ時刻（`HH:MM`）
//   artifacts     (array)    : 発信元ワークツリーに登録されている**URL アーティファクト**
//                              （#265）。`[{ id, title }]`。ワークツリー名の隣の
//                              「📄 アーティファクト」ボタンのポップアップに並ぶ。
//                              `search_artifact` で発信元のものを探し、返り値の `type` が
//                              `text/uri-list` のものだけを入れる。**URL 本体はここに入れない**
//                              （リンク先は `artifact://worktree/<worktreeId>/<id>` で、
//                              開くのは飛んだ先のビューの「ブラウザで開く」ボタン）。
//                              無ければ省略する
//   choices       (string[]) : 候補ボタン。**`prompt.shape` が `"text"` のときだけ使う。**
//                              通知内容から作る。`その他（補足で指示）` はコード側で
//                              常に足されるので**ここに入れない**。
//                              **`!` で始まる候補はコマンド実行になる（#288）** —
//                              宛先のターミナルがシェルモードに入り、`!` の後ろが
//                              そのまま走る（前置きも補足も付かない）。人が中身を
//                              読んで即座に許可できる**読み取り系**だけにする
//
// ── 本文（#264）─────────────────────────────────────────────────────────
//
//   paragraphs (string[]) : 段落。**人が読める整形済みの文**を入れる。
//                           `**強調**` / `` `コード` `` / `#203` / URL は
//                           カードがリンクや装飾に組む（Markdown が素で出ることはない）。
//                           **生 JSON も画面のダンプもここに入れない**
//   bullets    (string[]) : 箇条書き（任意）。同じ記法が使える
//   fields     ([{label, value, code}]) : 明細表（任意）。ツール許可の `tool_input` や
//                           「設問 2 問」のような要約に使う。`code: true` で等幅表示
//   links      ([{kind, label, href}])  : 関連リンク（任意）。
//                           `kind` は `artifact` / `pr` / `issue` / `url`。
//                           **アーティファクトは `artifact://worktree/<worktreeId>/<id>` 形式**。
//                           発信元が作ったアーティファクトは `search_artifact` で探して入れる
//                           （実測で一度も出ていなかった。#264）
//   body       (string)   : **旧形式。** `paragraphs` があれば使われない。
//                           `oretachi_poll_inbox` の `body` は**パース済みの JSON
//                           オブジェクト**なので、ここへ入れてはいけない
//                           （人が読める 1 行は別フィールドの `text`）
//
// ── request（何を聞かれているか。#264）───────────────────────────────────
//
// `approval` の通知本文は `PermissionRequest` フックの JSON で、`tool_name` と
// `tool_input` が丸ごと入っている。**AskUserQuestion なら全設問・全選択肢・
// `description`・`preview` まで全部そこにある。**
// 一方ターミナルの画面は 1 問ずつしか出さず、preview は `✂ N lines hidden` で
// 切られる（実測）。**通知の方が情報量が多い**ので、設問はそちらから起こす。
//
//   request.tool      (string) : `tool_name` そのまま（`Bash` / `AskUserQuestion` / `Edit` …）
//   request.questions (array)  : **`AskUserQuestion` のときだけ。**
//                                `tool_input.questions` を写す:
//                                `[{ header, question, options: [{ label, description, preview }] }]`
//                                **選択肢の順番を変えない。** カードは i 番目を画面の
//                                i+1 番として送る（Claude Code は通知の選択肢をその順で
//                                `1.` から並べ、後ろに `Type something.` /
//                                `Chat about this` を足す）。並べ替えると別の選択肢を確定する
//
// **`request.questions` を入れると、カードは全設問を 1 枚のフォームに出し、
// 送信は `kind: "selectAll"` の 1 回で最後の確定（Submit）まで進む。**
// 入れないと画面から読めた 1 問だけの表示になり、2 問目以降に答えられない。
//
// ツール許可（`Bash` など）は `request.questions` を作らない。承認の選択肢
// （`Yes` / `Yes, and don't ask again` / `No, ...`）は画面にしか無いので `prompt`
// から出す。`tool_input` の中身は `fields` に整形して入れる。
//
// ── prompt フィールド（#215） ────────────────────────────────────────────
//
// **`oretachi_inspect_prompt` の戻り値を編集せずそのまま焼き込む。** 要約したり
// ラベルを言い換えたりしてはいけない（fingerprint を書き換えると照合が必ず外れて
// 何も送れなくなる）。**表示には使わない**（表示は上の本文と `request` から組む）。
//
//   shape        (string)  : "text" / "permission" / "plan" / "askUserQuestion" /
//                            "yesno" / "numbered" / "pager" / "unknown"
//   navigation   (string)  : "arrows" / "digits" / "none"
//   header       (string)  : 問いの見出し
//   context      (string)  : 承認対象。`shape` が "text" 以外のとき画面から取れる範囲
//   questions    (array)   : [{ header, question, multiSelect, options: [{ index, label }],
//                              allowOther, cursorIndex }]
//                            **画面から読めた 1 問ぶんだけ**（複数設問でも 1 問しか入らない）
//   tabs         (array)   : [{ label, answered, isSubmit }]。複数設問のタブバー（#264）。
//                            `☐` が未回答 / `☒` が回答済み / `✔ Submit` が確定タブ
//   escapeHatch  (string)  : "esc" なら ESC で自由入力へ抜けられる
//   truncated    (boolean) : **必須。落とさないこと。** 画面に収まっていない、または
//                            リストがスクロールしていて全項目が描かれていない（#292）
//   fingerprint  (string)  : 画面の同一性キー。**書き換えない**
//   tail         (string)  : 画面末尾。`shape` が "unknown" のとき人に見せる。
//                            `shape: "text"` では自動候補の行に
//                            `⟪自動候補（ユーザー入力ではない）: …⟫` の印が付く
//   pendingInput (string)  : 人が入力欄に打ちかけているテキスト。空なら未入力（#289）。
//                            **`shape: "text"` のときだけ有効**（他は常に空）
//   inputSuggestion (string): Claude Code の自動候補（ゴーストテキスト）。
//                            **ユーザー入力ではない**。要約や summary に混ぜないこと
//   detectedAtMs (number)  : 解析した時刻（epoch ms）
//
// ── 1 セッションにつきキー操作カードは 1 枚だけ ──────────────────────────
//
// ダイアログは 1 つしか無いので、同じ `sessionId` へキー操作カード（shape が "text"
// 以外）を 2 枚向けてはいけない。同じ宛先の通知が複数あるなら**最新 1 件だけ**に
// `prompt` を付け、残りは `prompt: null` + `choices: []` の参考表示に落とす。
// （`lib/send` の `promptConflicts` が 2 枚目以降を機械的に塞ぐが、そもそも作らない）

const META = {
  // 載せた通知の最大 createdAt（= 下の NOTIFICATIONS で最も新しい at: '14:21'）を
  // そのまま ID と基準時刻に使う。両者は必ず同じ通知を指す
  reportId: 'notif-report-1788844860000',
  generatedAtMs: 1788844860000,           // JST 2026-09-08 14:21
  callerWorktree: 'oretachi-vy7f',
  repoUrl: 'https://github.com/ishida-supsys/oretachi',
};

const NOTIFICATIONS = [
  {
    // ── パターン 1: ツール許可（PermissionRequest / Bash）──────────────────
    // hook JSON の `tool_input` を `fields` へ整形する。**生 JSON は載せない。**
    // 選択肢は画面にしか無いので `prompt` から出す（`request.questions` は作らない）
    id: 'inbox-9a1',
    inboxIds: ['inbox-9a1'],
    worktreeName: 'oretachi-xaoe',
    worktreeId: '1788700000000-xaoe',
    sessionId: 99014,
    subscribed: true,
    issueRef: '#187',
    at: '14:21',
    kind: 'approval',
    desc: 'アーティファクトのリポジトリ保管庫への転送 (#187)',
    descFallback: null,
    phase: '停止条件待ち',
    phaseSummary: 'マイグレーション適用の許可待ちで停止中',
    readAt: '14:30',
    paragraphs: ['`oretachi-xaoe` がツールの許可を待って止まっています。'],
    fields: [
      { label: 'ツール', value: 'Bash' },
      { label: 'コマンド', value: 'sqlx migrate run --source ./migrations', code: true },
      { label: '説明', value: 'Apply pending migrations' },
      { label: '作業ディレクトリ', value: 'X:\\devel\\worktree\\oretachi-xaoe', code: true },
    ],
    links: null,
    // ワークツリー名の隣の「📄 アーティファクト」に並ぶ（#265）
    artifacts: [{ id: 'url-pr-187', title: 'PR #187' }],
    choices: [],
    request: { tool: 'Bash' },
    prompt: {
      shape: 'permission',
      navigation: 'arrows',
      header: 'Do you want to proceed?',
      context: 'Bash command\nsqlx migrate run --source ./migrations\nApply pending migrations',
      questions: [
        {
          header: '',
          question: 'Do you want to proceed?',
          multiSelect: false,
          options: [
            { index: 1, label: 'Yes' },
            { index: 2, label: "Yes, and don't ask again for sqlx commands in X:\\devel\\worktree\\oretachi-xaoe" },
            { index: 3, label: 'No, and tell Claude what to do differently (esc)' },
          ],
          allowOther: true,
          cursorIndex: 1,
        },
      ],
      tabs: [],
      escapeHatch: 'esc',
      truncated: false,
      fingerprint: '3f9c1a0b7d2e4856',
      tail: '  Esc to cancel · Tab to amend',
    },
  },
  {
    // ── パターン 2: AskUserQuestion 複数設問（#264）────────────────────────
    // **`request.questions` に通知の全設問を写す。** 画面には 1 問ずつしか出ないが、
    // 通知には全部入っているので、カードは全問を 1 枚のフォームで出せる。
    // `preview` はターミナルでは `✂ N lines hidden` で切られていて読めない
    id: 'inbox-9a3',
    inboxIds: ['inbox-9a3'],
    worktreeName: 'oretachi-orqn',
    worktreeId: '1788720000000-orqn',
    sessionId: 99033,
    subscribed: true,
    issueRef: '#217',
    at: '14:08',
    kind: 'approval',
    desc: null,
    descFallback: 'リンクホバーポップアップの URL 表示（ターミナルから推定）',
    phase: '設計中',
    phaseSummary: '表示方針の設問で停止中。実装は未着手',
    readAt: '14:31',
    paragraphs: [
      '`oretachi-orqn` が **2 問**の設問で止まっています。' +
        '現状は `max-height: 4.5em` + `overflow: hidden` で長い URL が黙って 3 行に切断されています。',
    ],
    fields: [{ label: '設問', value: '2 問（URL表示 / コピー）' }],
    links: [
      {
        kind: 'artifact',
        label: '検討メモを開く',
        href: 'artifact://worktree/1788720000000-orqn/url-popup-notes',
      },
    ],
    choices: [],
    request: {
      tool: 'AskUserQuestion',
      // `tool_input.questions` をそのまま写す。**並べ替えない**
      questions: [
        {
          header: 'URL表示',
          question: 'リンクホバーポップアップで長い URL をどう見せますか？',
          options: [
            {
              label: '上限を広げて + スクロール',
              description:
                'max-height を 4.5em → 12em に広げ overflow: auto にする。実用的な長さの URL はほぼ全文が読め、極端に長いものだけスクロールになる。',
              preview:
                '┌────────────────────────────────────┐\n' +
                '│ https://github.com/ishida-supsys/  │\n' +
                '│ oretachi/issues/217?utm_source=x   │\n' +
                '└────────────────────────────────────┘',
            },
            {
              label: '中略して 1 行に畳む',
              description: 'ホスト名と末尾だけ残して中間を … にする。高さは固定のままだが全体像は掴みにくい。',
              preview:
                '┌────────────────────────────────────┐\n' +
                '│ github.com/…/issues/217?utm_sour…  │\n' +
                '└────────────────────────────────────┘',
            },
          ],
        },
        {
          header: 'コピー',
          question: 'ポップアップに URL のコピーボタンを付けますか？',
          options: [
            { label: '付ける', description: 'クリップボードへコピーするボタンを右上に置く。' },
            { label: '付けない', description: '今回のスコープ外にする。' },
          ],
        },
      ],
    },
    prompt: {
      shape: 'askUserQuestion',
      navigation: 'arrows',
      header: 'リンクホバーポップアップで長い URL をどう見せますか？',
      context: '',
      // 画面から読めるのは**いま開いているタブの 1 問だけ**。送信の照合に使う
      questions: [
        {
          header: 'URL表示',
          question: 'リンクホバーポップアップで長い URL をどう見せますか？',
          multiSelect: false,
          options: [
            { index: 1, label: '上限を広げて + スクロール' },
            { index: 2, label: '中略して 1 行に畳む' },
            { index: 3, label: 'Chat about this' },
          ],
          allowOther: true,
          cursorIndex: 1,
        },
      ],
      tabs: [
        { label: 'URL表示', answered: false, isSubmit: false },
        { label: 'コピー', answered: false, isSubmit: false },
        { label: 'Submit', answered: false, isSubmit: true },
      ],
      escapeHatch: 'esc',
      truncated: false,
      fingerprint: 'c05e77a3b1892d64',
      tail: '  Enter to select · Tab/Arrow keys to navigate · Esc to cancel',
    },
  },
  {
    // ── パターン 3: 自由入力（ダイアログ無し）。候補ボタンを作ってよい ──────
    // エージェントが書いた Markdown はそのまま入れてよい（カードが組んで出す）。
    // issue 番号 / URL / アーティファクトはリンクになる
    id: 'inbox-9a2',
    inboxIds: ['inbox-9a2'],
    worktreeName: 'oretachi-xqle',
    worktreeId: '1788710000000-xqle',
    sessionId: 99021,
    subscribed: true,
    issueRef: '#195',
    at: '14:05',
    kind: 'worktree.message',
    desc: '通知レポート機能の親issue進行管理 (#195)',
    descFallback: null,
    phase: '実装完了',
    phaseSummary: '実装完了・未 commit。セルフレビュー待ち。差分 12 ファイル / 約 900 行',
    readAt: '14:30',
    paragraphs: [
      'PR を分割すべきか判断が欲しいです。#201 のリンク実装と #203 のロック実装を' +
        '**1 本にまとめると差分が約 900 行**になります。',
    ],
    bullets: [
      '分割する場合、`artifact_lock.rs` の変更が両方に跨るので先に #203 を出す必要があります',
      'まとめる場合はレビューが重くなりますが、`automerge` で一度に片付きます',
    ],
    links: [
      {
        kind: 'artifact',
        label: '計画フロー図を開く',
        href: 'artifact://worktree/1788710000000-xqle/teamwork-plan-195',
      },
      { kind: 'pr', label: 'PR #204', href: 'https://github.com/ishida-supsys/oretachi/pull/204' },
    ],
    // `!` 始まりは宛先のターミナルでそのまま走るコマンド（#288）。この通知は
    // 「差分 約 900 行」が判断材料なので、その場で数え直せる読み取り系を 1 つ添える
    choices: ['分割して', '1 本でよい', '詳細を教えて', '!git diff --stat', '保留'],
    request: null,
    prompt: {
      shape: 'text',
      navigation: 'none',
      // `text` では受け手の種類が入る（`[Claude Code の入力欄]` /
      // `[シェルのプロンプト] PS X:\...>`）。fingerprint に効くので**書き換えない**
      header: '[Claude Code の入力欄]',
      context: '',
      questions: [],
      tabs: [],
      escapeHatch: null,
      truncated: false,
      fingerprint: '8b21d5e0c47a9f31',
      tail: '╭───────────╮\n│ >         │\n╰───────────╯',
    },
  },
  {
    // ── パターン 4: 分類不能。**送信ボタンは無効**で tail を見せるだけ ──────
    // 稼働中 AI 端末が無いケース（sessionId が null）も同じく送信不可になる。
    // **ここだけは画面のテキストを出す** — 何を出せばいいのか分からない画面なので、
    // 人がターミナルで何を見ることになるかを示すしかない
    id: 'inbox-9a4',
    inboxIds: ['inbox-9a4'],
    worktreeName: 'oretachi-zlvc',
    worktreeId: '1788730000000-zlvc',
    sessionId: 99041,
    subscribed: true,
    issueRef: '#208',
    at: '13:40',
    kind: 'general',
    desc: 'アーカイブ DB のマイグレーション (#208)',
    descFallback: null,
    phase: '不明',
    phaseSummary: '画面を読み取れなかった',
    readAt: '14:31',
    paragraphs: ['`oretachi-zlvc` が何かの入力待ちで止まっているように見えますが、画面の形状を判別できませんでした。'],
    links: null,
    choices: [],
    request: null,
    prompt: {
      shape: 'unknown',
      navigation: 'none',
      header: '',
      context: '',
      questions: [],
      tabs: [],
      escapeHatch: null,
      truncated: false,
      fingerprint: '5a7e2c9014bd63f8',
      tail: 'Select a profile to continue\n  [use the mouse to pick one]\n',
    },
  },
  {
    // ── 報告カード: ワークツリークローズ（#228）────────────────────────────
    // **発信元がもう存在しない。** `oretachi_poll_inbox` の `sourceWorktreeName` /
    // `sourceWorktreePath` は `null` になるので、名前とブランチは通知本文
    // （`WorktreeClosedBody`）に焼き付いている値から取る。
    // `sessionId` / `subscribed` / `prompt` / `desc` / `phase` は**集めない**
    id: 'inbox-9a6',
    inboxIds: ['inbox-9a6'],
    worktreeName: 'oretachi-htlz',
    worktreeId: '1788690000000-htlz',
    branchName: 'worktree/issue-214',
    at: '13:12',
    kind: 'worktree.closed',
    paragraphs: ["ワークツリー `oretachi-htlz`（ブランチ: `worktree/issue-214`）がクローズされました。"],
  },
  {
    // ── 報告カード: ワークツリー作成（#228）────────────────────────────────
    // 作成直後なので description 未設定・AI 端末未起動が普通。現況は集めない
    id: 'inbox-9a7',
    inboxIds: ['inbox-9a7'],
    worktreeName: 'oretachi-wnqd',
    worktreeId: '1788750000000-wnqd',
    branchName: 'worktree/issue-228',
    at: '13:05',
    kind: 'worktree.created',
    paragraphs: ["`oretachi` にワークツリー `oretachi-wnqd`（ブランチ: `worktree/issue-228`）が作成されました。"],
  },
];

exports.META = META;
exports.NOTIFICATIONS = NOTIFICATIONS;
