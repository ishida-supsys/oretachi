
// ── data/report テンプレート例 ────────────────────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の `data/report` は Step 1 で収集した
// 通知・ワークツリー情報から新規生成する。**レポートの唯一のデータソース**で、
// レポートを開いている間は表示中ロックで更新できない（スナップショット）。
//
// ── META フィールド仕様 ──────────────────────────────────────────────────
//   reportId       (string) : アーティファクトID と同じ値。送信テキストの先頭に入る
//   generatedAtMs  (number) : レポートの基準時刻（epoch ms）。**載せた通知のうち最大の
//                             `createdAt` をそのまま入れる。現在時刻を推測して入れない**
//                             （このセッションには時刻が渡っていない。#220）。
//                             表示用の整形は entry-point 側の `generatedLabel` がやる
//   callerWorktree (string) : レポートを置いたワークツリー名（購読の主体）
//
// ── NOTIFICATIONS 配列フィールド仕様 ─────────────────────────────────────
//   id            (string)   : カードの一意キー。inbox メッセージ ID をそのまま使ってよい
//   inboxIds      (string[]) : このカードが束ねる inbox メッセージ ID。**生成時（Step 5.5）に
//                              ack 済み**の記録で、レポート側からは ack しない（#219）
//   worktreeName  (string)   : 発信元ワークツリー名（= 送信先）
//   worktreeId    (string)   : 発信元ワークツリーID。**必須**。購読の突合と、生成時の
//                              トレイ通知クリア（oretachi_clear_worktree_notification）に使う。
//                              oretachi_poll_inbox の sourceWorktreeId をそのまま入れる（表示はしない）
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
//   choices       (string[]) : 候補ボタン。**`prompt.shape` が `"text"` のときだけ使う。**
//                              通知内容から作る。`その他（補足で指示）` はコード側で
//                              常に足されるので**ここに入れない**
//   prompt        (object)   : `oretachi_inspect_prompt(session_id)` の戻り値を**そのまま**。
//                              `null` / 省略なら従来の自由入力扱い（= shape "text" と同じ）
//
// ── prompt フィールド（#215） ────────────────────────────────────────────
//
// **`oretachi_inspect_prompt` の戻り値を編集せずそのまま焼き込む。** 要約したり
// ラベルを言い換えたりしてはいけない（宛先の画面に実在しない選択肢を人へ見せると
// 判断を誤らせる。fingerprint を書き換えると照合が必ず外れて何も送れなくなる）。
//
//   shape        (string)  : "text" / "permission" / "plan" / "askUserQuestion" /
//                            "yesno" / "numbered" / "unknown"
//   navigation   (string)  : "arrows"(矢印で ❯ を動かして CR = Claude Code のダイアログ) /
//                            "digits"(数字 + CR = 素の TUI の番号リスト) / "none"(選択肢なし)。
//                            **`shape` とは独立**。見出しが折り返して形状の推定が外れても
//                            キーの種類だけは Claude Code のフッタから決まる
//   header       (string)  : 問いの見出し（`Do you want to proceed?` など）
//   context      (string)  : 承認対象の全文（Bash コマンド / ツール名 / cwd / 設問文）。
//                            カードに全文表示される
//   questions    (array)   : [{ header, question, multiSelect, options: [{ index, label }],
//                              allowOther, cursorIndex }]
//                            `options[].label` が**画面に実在する選択肢**。
//                            `cursorIndex` はいま `❯` が当たっている番号
//   escapeHatch  (string)  : "esc" なら ESC で自由入力へ抜けられる。ただし ESC 経路が
//                            実際に使えるのは permission / plan / askUserQuestion のみ
//                            （`lib/send` の `canEscape`）
//   truncated    (boolean) : **必須。落とさないこと。** true なら「ダイアログが宛先の画面に
//                            収まっておらず、選択肢を全部読めていない」。カードは選択 UI を
//                            出さず警告と画面末尾を表示する。**落とすと「読めた選択肢だけ」を
//                            完全な一覧として提示し、拒否の選択肢を見ないまま承認させる**
//                            （Rust の `plan_keys` が送信自体は止めるが、その後カードは
//                            読み取り専用になり警告も出ないまま死ぬ）
//   fingerprint  (string)  : 画面の同一性キー。`oretachi_answer_prompt` の
//                            `expect_fingerprint` にそのまま渡る。**書き換えない**
//   tail         (string)  : 画面末尾。`shape` が "unknown" / `truncated` のとき人に見せる
//   detectedAtMs (number)  : 解析した時刻（epoch ms）。表示の補助にのみ使う
//
// ── shape ごとの choices / prompt の使い分け ─────────────────────────────
//
//   shape "text"     … `choices` を通知本文から作ってよい（従来どおり）
//   それ以外          … **候補を創作してはいけない。`prompt` を入れるだけ**でカードが
//                       画面の選択肢をそのまま出す。`choices` は無視されるので `[]` にする
//   shape "unknown"  … 送信ボタンが無効になり、`tail` を見せて手動操作へ誘導する
//
// ── 1 セッションにつきキー操作カードは 1 枚だけ ──────────────────────────
//
// ダイアログは 1 つしか無いので、同じ `sessionId` へキー操作カード（shape が "text"
// 以外）を 2 枚向けてはいけない。同じ宛先の通知が複数あるなら**最新 1 件だけ**に
// `prompt` を付け、残りは `prompt: null` + `choices: []` の参考表示に落とす。
// （`lib/send` の `promptConflicts` が 2 枚目以降を機械的に塞ぐが、そもそも作らない）

const META = {
  reportId: 'notif-report-1788867120000', // 載せた通知の最大 createdAt をそのまま使う
  generatedAtMs: 1788867120000,           // = その最大 createdAt（epoch ms）
  callerWorktree: 'oretachi-vy7f',
};

const NOTIFICATIONS = [
  {
    // ── パターン 2: ツール許可ダイアログ（PermissionRequest）で止まっている ──
    // `choices` は空。候補を創作せず、画面の選択肢をそのまま出す
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
    phaseSummary: 'Bash の許可待ちで停止中',
    readAt: '14:30',
    body: 'ツールの許可を待っています: マイグレーションの適用コマンドを実行してよいか判断が欲しい',
    link: null,
    linkLabel: null,
    choices: [],
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
      escapeHatch: 'esc',
      truncated: false,
      fingerprint: '3f9c1a0b7d2e4856',
      tail: '  Esc to cancel · Tab to amend',
    },
  },
  {
    // ── パターン 1: 自由入力（ダイアログ無し）。従来どおり候補を作ってよい ──
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
    prompt: {
      shape: 'text',
      navigation: 'none',
      // `text` では受け手の種類が入る（`[Claude Code の入力欄]` /
      // `[シェルのプロンプト] PS X:\...>`）。fingerprint に効くので**書き換えない**
      header: '[Claude Code の入力欄]',
      context: '',
      questions: [],
      escapeHatch: null,
      truncated: false,
      fingerprint: '8b21d5e0c47a9f31',
      tail: '╭───────────╮\n│ >         │\n╰───────────╯',
    },
  },
  {
    // ── パターン 4: AskUserQuestion。末尾の逃げ道は画面上 `Chat about this` ──
    id: 'inbox-9a3',
    inboxIds: ['inbox-9a3'],
    worktreeName: 'oretachi-orqn',
    worktreeId: '1788720000000-orqn',
    sessionId: 33,
    subscribed: true,
    issueRef: '#120',
    at: '13:55',
    kind: 'approval',
    desc: null,
    descFallback: 'ワークツリー購読と inbox の実装（ターミナルから推定）',
    phase: '設計中',
    phaseSummary: '配送戦略の選択で設問待ち。実装は未着手',
    readAt: '14:31',
    body: '配送戦略をどれにするか確認したい（AskUserQuestion で設問を出して停止中）。',
    link: 'artifact://worktree/1788720000000-orqn/teamwork-plan-120',
    linkLabel: '計画フロー図を開く',
    choices: [],
    prompt: {
      shape: 'askUserQuestion',
      navigation: 'arrows',
      header: 'Which delivery strategy should be the default?',
      context: '配送戦略の既定値',
      questions: [
        {
          header: '',
          question: '配送戦略の既定値\nWhich delivery strategy should be the default?',
          multiSelect: false,
          options: [
            { index: 1, label: 'turn_end (待機中なら押し込み、走行中はターン境界を待つ)' },
            { index: 2, label: 'interrupt (走行中でも即割り込む)' },
            { index: 3, label: 'passive (押し込まない)' },
            { index: 4, label: 'Chat about this' },
          ],
          allowOther: true,
          cursorIndex: 1,
        },
      ],
      escapeHatch: 'esc',
      truncated: false,
      fingerprint: 'c05e77a3b1892d64',
      tail: '  Enter to select · Tab/Arrow keys to navigate · Esc to cancel',
    },
  },
  {
    // ── パターン 11: 分類不能。**送信ボタンは無効**で tail を見せるだけ ──
    // 稼働中 AI 端末が無いケース（sessionId が null）も同じく送信不可になる
    id: 'inbox-9a4',
    inboxIds: ['inbox-9a4'],
    worktreeName: 'oretachi-zlvc',
    worktreeId: '1788730000000-zlvc',
    sessionId: 41,
    subscribed: true,
    issueRef: '#208',
    at: '13:40',
    kind: 'general',
    desc: 'アーカイブ DB のマイグレーション (#208)',
    descFallback: null,
    phase: '不明',
    phaseSummary: '画面を読み取れなかった',
    readAt: '14:31',
    body: '何かの入力待ちで止まっているように見える。',
    link: null,
    linkLabel: null,
    choices: [],
    prompt: {
      shape: 'unknown',
      navigation: 'none',
      header: '',
      context: '',
      questions: [],
      escapeHatch: null,
      truncated: false,
      fingerprint: '5a7e2c9014bd63f8',
      tail: 'Select a profile to continue\n  [use the mouse to pick one]\n',
    },
  },
  {
    // ── 画面に収まっていない許可ダイアログ（`truncated: true`）──────────────
    // 宛先のタブが狭く、見出しと `2.` 以降が画面外へ流れて `1. Yes` しか読めていない。
    // **読めたぶんだけを選択肢として出してはいけない**（人が「Yes しか無い」と誤認して、
    // 拒否の選択肢を見ないまま承認する）。カードは選択 UI を出さず、警告と `tail` を表示し、
    // ESC で抜けて指示を書く経路だけを残す
    id: 'inbox-9a5',
    inboxIds: ['inbox-9a5'],
    worktreeName: 'oretachi-kqtr',
    worktreeId: '1788740000000-kqtr',
    sessionId: 52,
    subscribed: true,
    issueRef: '#212',
    at: '13:20',
    kind: 'approval',
    desc: 'リリーススクリプトの整理 (#212)',
    descFallback: null,
    phase: '停止条件待ち',
    phaseSummary: 'ツール許可待ちで停止中（タブが狭くダイアログが画面に収まっていない）',
    readAt: '14:31',
    body: 'ツールの許可を待っています。',
    link: null,
    linkLabel: null,
    choices: [],
    prompt: {
      shape: 'permission',
      navigation: 'arrows',
      header: '',
      context: '',
      questions: [
        {
          header: '',
          question: '',
          multiSelect: false,
          // 画面から読めたのはこれだけ。**完全な一覧ではない**
          options: [{ index: 1, label: 'Yes' }],
          allowOther: false,
          cursorIndex: 1,
        },
      ],
      escapeHatch: 'esc',
      truncated: true,
      fingerprint: 'd41128ba6c07e395',
      tail: ' ❯ 1. Yes\n\n Esc to cancel · Tab to amend',
    },
  },
];

exports.META = META;
exports.NOTIFICATIONS = NOTIFICATIONS;
