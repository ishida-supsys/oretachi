---
name: notification-report
description: 購読しているワークツリーから通知が溜まったときに、関連する通知の一覧を読んでレポートアーティファクトを作成する。人はレポートを見るだけで、ターミナルを1つずつ開かずに溜まった通知へ一括でクイックに返答できる。ホームタブや teamwork-parent の親ワークツリーからの利用を想定。ユーザーが「通知をまとめて確認したい」「溜まった通知にまとめて返したい」等と言ったときに使う。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_list_subscriptions, mcp__plugin_oretachi_oretachi__oretachi_subscribe_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_worktree_notifications, mcp__plugin_oretachi_oretachi__oretachi_poll_inbox, mcp__plugin_oretachi_oretachi__oretachi_ack_message, mcp__plugin_oretachi_oretachi__oretachi_clear_worktree_notification, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_read_terminal, mcp__plugin_oretachi_oretachi__oretachi_inspect_prompt, mcp__plugin_oretachi_oretachi__notify_worktree, mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__artifact_store, mcp__plugin_oretachi_oretachi__search_artifact, Read, Glob, Grep
---

# notification-report スキル

購読しているワークツリーから届いた通知を 1 枚のレポートアーティファクトにまとめる。ユーザーはレポート上のボタン + 補足プロンプトで一括返答し、返答は**各ワークツリーの AI 端末へ直接届く**。

## 前提

- **返答の宛先は各通知の発信元ワークツリーの AI 端末。** アーティファクトの JS から `oretachi_write_terminal` を他ワークツリー宛に呼べるが、許可条件は**レポートを置いたワークツリーがその宛先ワークツリーを購読していること**（#211）。向きは購読者側が呼び出し元で、宛先側が自分を購読しているだけでは通らない。
- **レポートはスナップショット。** 開いている間に届いた通知はそのレポートに足さず、次のレポートへ回す。表示中に本体を書き換えて人が見ている画面を動かす方が有害。
- **返答状態はサイドカー（`artifact_store`）に持つ。** 本体 JSON に持つと、送信後に「返答済み」へ書き換える操作が自分自身の表示中ロックに弾かれる。ビューア由来の書き込みはロックの対象外。

## Step 1: 収集

### 1-1. 自分の購読を確認する

```
oretachi_list_subscriptions(terminal_id: <セッション開始時に注入された terminal_id>)
```

ここに出てくる `target` が**送信できる宛先の範囲**を決める。ワイルドカード（`*` / `workgroup:<ID>` / `repo:<名前>`）も許可対象で、`event_kinds` は問わない。

### 1-2. 通知本文を取る（**ここがレポートの母集合**）

```
oretachi_poll_inbox(terminal_id: <自分の terminal_id>)
```

返ってきた未 ack メッセージが**レポートに載せる通知そのもの**。`id` が inbox メッセージ ID で、送信成功後の ack に使う。**この時点では ack しない**（ユーザーが返答してから ack する）。

`sourceWorktreeId` を 1-1 の購読対象と突き合わせ、購読していないワークツリー由来のものは落とす（載せても返答を送れない）。

**発信元ワークツリーは `sourceWorktreeId` / `sourceWorktreeName` を必ずここから取る（#218）。**
以降のステップ（1-4 の `oretachi_get_worktree_status` / `oretachi_list_terminals`）へ渡す宛先は
**この 2 つだけを使い、通知本文やターミナル出力から名前を推測しない。** 名前を取り違えると
`oretachi_list_terminals` が別のワークツリー（または該当なし）を返し、`sessionId` が `null` の
カード（「稼働中の AI 端末が見つからなかったため送信できません」）になる。実際にこれが起きていた。
`sourceWorktreeName` が `null` のメッセージは発信元が settings から消えている（クローズ済み）ので、
返答を送れない。レポートに載せない。

ユーザーが「購読外も含めて全部見たい」と明示した場合だけ、`oretachi_subscribe_worktree` で購読を張ってから含める。**購読は勝手に張らない** — 張ることはそのワークツリーの端末への書き込みを許すことなので、ユーザーの指示なしに範囲を広げない。

### 1-3. トレイ通知は補助情報として使う（母集合にしない）

```
oretachi_list_worktree_notifications()
```

**これを母集合にしてはいけない。** このツールが見る `NotificationRegistry` はフロントから `sync_notification_state` で全置換されるトレイバッジの写しで、`oretachi_poll_inbox` / `oretachi_ack_message` が見る sqlite の event_db とは**完全に別のストア**。`oretachi_clear_worktree_notification` はトレイ側しかクリアしない。

つまり「トレイ通知が出ているワークツリー」で inbox を絞ると、**バッジだけ先にクリアされたワークツリーの未 ack メッセージがレポートから漏れる。**

使ってよいのは「どのワークツリーに人の注意が向いているか」という補助情報として。カードの並び順を決めたり、`firstNotifiedAt` を参考にしたりする用途に留める。

なお**このツールは全ワークツリー分を返す**（ワークツリースコープが効かない）ので、参照するときも 1-1 の購読対象へ絞ること。

### 1-4. 「そのワークツリーが何なのか」を集める

通知本文だけでは、人はどのワークツリーが何をしていてどの段階なのか分からない。カードごとに次の 2 つを必ず埋める。

**(a) ミッション**

```
oretachi_get_worktree_status(query: <1-2 の sourceWorktreeName>)
```

返る各エントリの `id` を `sourceWorktreeId` と突き合わせて**同名の別ワークツリーを取り違えない**
（`query` は部分一致なので、名前が前方一致する別ワークツリーも返る）。

`description` が入っていればそれを使う。**未設定なら `null` を入れ**、代わりに 1-4(b) のターミナル出力からミッションを推定して `descFallback` に書く（カードには `[description 未設定]` バッジ付きで出る）。

**(b) 現況**

```
oretachi_list_terminals(worktree_id: <1-2 の sourceWorktreeId>)   → isAiAgent: true / status: "running" の sessionId
oretachi_read_terminal(session_id: <上で得た sessionId>, max_bytes: 8192)
```

**絞り込みは `worktree_id` で行う（`worktree_name` ではない）。** 名前指定は同名ワークツリーが
あるとエラーになり、綴りが 1 文字違うだけで「該当なし」になる。ID なら `oretachi_poll_inbox` が
返した値をそのまま渡せる（#218）。

返り値のフィールド名は **`sessionId`**（camelCase）。`session_id` というキーは無い。

**0 件だったときに `sessionId: null` へ直行しないこと。** このツールの絞り込みは
**各 PTY の `cwd` から解決したワークツリー**で行われる（`resolve_worktree_by_cwd` は
`cwd` に前方一致するワークツリーのうち最も深いものを採る）。そのため
**cwd をワークツリー外へ移した生存 AI 端末や、そのワークツリーの下にネストして登録された
別ワークツリーへ吸われた端末は、ID 指定でも結果から落ちる。** 0 件のときは:

1. 絞り込みなしで `oretachi_list_terminals()` を呼ぶ
2. `isAiAgent: true` / `status: "running"` かつ `cwd` が 1-2 の `sourceWorktreePath` 配下に
   あるものを探す（`worktreeId` が別の値になっていても、そこで走っているのは
   その発信元の端末）
3. それでも無ければ本当に AI 端末が無い（素のシェルだけ / タブを閉じた）

ID 指定がエラーになるのは「その ID の登録が無い」＝発信元がクローズ済みの場合だけなので、
そのときは 1-2 の判断（レポートに載せない）へ戻る。

読んだ出力から**フェーズと 1 行要約**にまとめる。フェーズは次のいずれか（カード側の色分けがこの文字列に対応している）:

| phase | 使う場面 |
|---|---|
| `設計中` | 設計・調査中。まだコードを書いていない |
| `実装中` | 実装作業中 |
| `実装完了` | 実装が終わっている（未 commit / PR 作成済み などは要約に書く） |
| `レビュー対応中` | セルフレビューやレビュー指摘の反映中 |
| `停止条件待ち` | 停止条件でユーザー確認待ちで止まっている |
| `不明` | 読み取れなかった |

要約は「実装完了・未 commit。セルフレビュー待ち。差分 12 ファイル / 約 900 行」のように、**次に何が必要か分かる粒度**で 1 行。読んだ時刻を `readAt` に入れる（スナップショットなので、いつ時点かが分からないと判断を誤る）。

この (a)(b) は**レポート生成時のこのセッションが MCP クライアントとして呼ぶ**ので、他ワークツリーの端末も読める（アーティファクトの JS からの `read_terminal` にかかる購読条件は適用されない）。

### 1-4(c). 「いまどんな問いで止まっているか」を取る（**必須**）

```
oretachi_inspect_prompt(session_id: <1-4(b) と同じ値>)
```

**戻り値をそのまま `prompt` フィールドへ焼き込む。** 要約・言い換え・整形をしてはいけない。

- `options[].label` は**宛先の画面に実在する選択肢そのもの**。言い換えると人が「画面に無い選択肢」を選ぶことになる
- `fingerprint` は送信直前の照合キー。書き換えると照合が必ず外れて何も送れなくなる
- `cursorIndex` はいま `❯` が当たっている番号。キー列（矢印の回数）がここから決まる
- `shape` が `text` のときの `header` は**受け手の種類**（`[Claude Code の入力欄]` /
  `[シェルのプロンプト] PS X:\...>`）。fingerprint に混ざっており、「レポート生成時は CC の
  入力欄だったが送信時には CC が終了してシェルのプロンプトだけが残っている」状況を `stale` で
  弾くために使う。**書き換えないこと**
- `truncated` が `true` なら**ダイアログが宛先の画面に収まっておらず、選択肢を全部読めていない**。
  カードは選択 UI を出さず、警告と画面末尾を表示して ESC 経路だけを残す。**このフィールドを
  落とすと「読めた選択肢だけ」が完全な一覧として提示され、拒否の選択肢を見ないまま承認させる**
  （実測: 7 行のタブで `options` が `[{1, "Yes"}]` だけになった）
- `navigation` は選択肢の選び方（`arrows` / `digits` / `none`）。**`shape` とは独立**で、
  Claude Code のフッタという構造的な手がかりから決まる。狭いターミナルで見出しが折り返して
  `shape` の推定が外れても、キーの種類だけは間違えない（実測: 13 桁では
  `Do you want to proceed?` が 3 行に、`Esc to cancel · Tab to amend` が 4 行に割れる）

**`oretachi_read_terminal` のテキストから問いを読み取ろうとしないこと。** Claude Code は
カーソル移動で差分描画するので、ANSI を除去したバイト列には再描画の断片しか残らない
（実測: 選択肢を 1 つ動かした 4 バイトは `strip_ansi` 後に空文字列になる）。
`oretachi_inspect_prompt` は出力履歴を VT エミュレータへ流し直した**画面グリッド**を見る。

`oretachi_read_terminal` は 1-4(b) の「現況の 1 行要約」を作るためにこれまでどおり使う。
役割が違う（要約は流れたログから、問いの形状は現在の画面から）。

### 1-4(d). 問いのパターンと回答手段

`shape` ごとにカードの見た目と送信経路が変わる。**候補を創作してよいのは `text` のときだけ。**

| shape | 画面上の特徴 | カードの UI | 送られるキー |
|---|---|---|---|
| `text` | 入力欄だけ（ダイアログ無し） | 候補ボタン + 補足プロンプト（従来どおり） | 本文 → 150ms → CR |
| `permission` | `Do you want to proceed?` + 番号付き選択肢 | **画面の選択肢そのまま**のラジオ + 承認対象の全文 | 矢印で `❯` を動かして CR |
| `plan` | `Would you like to proceed?` + `No, keep planning` | 同上 | 同上 |
| `askUserQuestion` | 設問 + 番号付き選択肢 + `Chat about this`、フッタ `Enter to select · Tab/Arrow keys to navigate` | 同上 | 同上 |
| `yesno` | `(y/N)` / `[Y/n]`（シェル側の gh / npm / git など） | `y` / `n` ボタン | `y` or `n` → CR |
| `numbered` | 素の TUI の `1) foo` | 画面の選択肢そのままのラジオ | 数字 → CR |
| `unknown` | 分類できないが入力待ちらしい | **送信ボタン無効。** 画面末尾を読み取り専用で表示 | 送らない |

`escapeHatch` が `"esc"` のカードには「**ESC で抜けて指示を書く**」欄が別に出る。
`No, and tell Claude what to do differently` を選ぶのと同じ着地点で、選択肢の文言に依存しない。
ただし ESC 経路が実際に使えるのは `permission` / `plan` / `askUserQuestion` の 3 形状のみ
（`yesno` / `numbered` では `escapeHatch` が立っていても使えない）。

**選択は数字キーではなく矢印 + CR。** 実測で確認済み: 許可ダイアログへ Down を 2 回送ってから
CR を送ると、3 番目の選択肢が確定した。`cursorIndex` から目標までの移動量を Rust 側が計算するので、
選択肢が 10 件以上あっても同じ手順で通る（数字キーだと 2 桁を打てない）。

素の TUI の番号リスト（`navigation: "digits"`）だけは行入力ベースなので数字 + CR。
この振り分けは `navigation` が持っている。

### 1-4(e). 1 セッションにつきキー操作カードは 1 枚だけ

**ダイアログは 1 つしか無い。** 同じ `sessionId` へ `shape` が `text` 以外のカードを 2 枚
向けると、2 枚目は必ず `stale`（1 枚目で画面が変わっている）になるか、最悪の場合
**1 枚目の回答が別の問いへ撃ち込まれる**。

同じ宛先の通知が複数あるなら、**最新 1 件だけ**に `prompt` を付け、残りは
`prompt: null` + `choices: []` の参考表示へ落とす。
（`lib/send` の `promptConflicts` が 2 枚目以降を機械的に塞ぐが、そもそも作らない）

### 1-4(f). 複数選択・複数設問の扱い

- **`multiSelect` は画面から判別できない**（単一選択との差が画面に出ない）。`questions[].multiSelect`
  は常に `false` で、カードも単一選択のラジオになる。**複数選ばせたい設問は人がターミナルで操作する**
- **複数設問の 2 問目以降はそのレポートでは答えられない。** 1 問答えると画面が次の設問へ変わるので、
  `answer_prompt` の `afterShape` が `askUserQuestion` のままなら「まだ設問が残っている」と
  カードに出る。続きは次のレポートで答える

### 1-5. 送信先の session_id を決める

1-4(b) で得た「そのワークツリーで走っている AI エージェント端末」の `sessionId` をそのまま `sessionId` に入れる。見つからなければ `null` を入れる（カードが「稼働中の AI 端末が見つからなかったため送信できません」になる）。

**`null` を入れる前に 1-4(b) の 0 件時の手順（絞り込みなしで引き直して `cwd` を見る）を
必ず通す。** そのカードは人が押せないので、レポートの価値がそのぶん失われる。実測で起きていた
`null` はどれも「宛先ワークツリー名を推測して `oretachi_list_terminals(worktree_name: ...)` が
空を返した」ケースで、端末は生きていた（#218）。本当に AI 端末が無いのか
（素のシェルだけ / タブを閉じた）を `oretachi_list_terminals` の返り値そのもので
確認してから `null` にすること。

アーティファクトからは `oretachi_list_terminals` が呼べないため、`session_id` は**ここで焼き込むしかない**。アプリ再起動やタブ再作成で失効するので、失効したらレポートを作り直す。

## Step 2: 既存レポートの確認（新規レポート作成プロトコル）

新しいレポートを作る前に必ずこの順で確認する。

1. `search_artifact(query: "通知レポート", project_dir: <自分の作業ディレクトリ>)` で既存レポートを探し、**更新日時が最も新しいもの**を選ぶ
2. そのレポートのサイドカーを読む
   ```
   artifact_store(command: "read", id: <レポートID>, project_dir: <自分の作業ディレクトリ>)
   ```
   `answers` に未送信の通知が残っていれば**未送信のレポート**。送信済みなら新規作成へ進む
3. 未送信なら、そのレポートの `data/report` へ今回の通知を追記しようとする
   ```
   artifact_module(command: "update", id: <レポートID>, module_name: "data/report", ...)
   ```
4. **表示中ロックでエラーになったら新しいレポートを作る。** エラー文言は「`locked_while_open` が立っており、いま oretachi のビューアで開かれているため書き込めません」。**リトライしない** — 開いている限り何度試しても失敗する

**一意性を保証する専用機構は入れない。** 最悪でも未送信レポートが 2 つできるだけで、次のエージェントは最新を見て追記するので収束する。強制力は指示文ではなく表示中ロックのエラー（ツール側の機構）にある。

## Step 3: テンプレートを読み込む

このスキルディレクトリ（`SKILL.md` と同じ場所）の `templates/` フォルダを Read で読み込む。

| ファイル | アーティファクトモジュール | カスタマイズ要否 |
|---|---|---|
| `templates/entry-point.jsx` | エントリポイント（content） | `// CUSTOMIZE:` 箇所のみ |
| `templates/components--NotificationCard.jsx` | `components/NotificationCard` | そのまま利用 |
| `templates/lib--send.jsx` | `lib/send` | そのまま利用 |
| `templates/data--report.example.jsx` | `data/report` | ※スキーマ参照用、新規生成 |

## Step 4: 候補ボタンを作る（`shape` が `text` のときだけ）

**`shape` が `text` 以外のカードでは候補を創作してはいけない。** `prompt` を焼き込むだけで、
カードが `options[].label`（宛先の画面に実在する選択肢）をそのまま出す。`choices` は無視されるので `[]` にする。

実在しない選択肢を人へ見せると、「押したのに画面と違う」「押した内容が届かない」という
形で判断を誤らせる。#215 の要点はここ。

以下は `shape` が `text`（ダイアログ無しで入力待ち）のカードにだけ当てはまる。

`choices` は**通知内容から作る**。その通知に対して人が返しそうな短い返答を 2〜4 個。

- 承認を求めている通知 → `了解` / `この方針で進めて` / `別案も見たい` / `保留`
- 二択の判断を求めている通知 → その二択をそのまま（`分割して` / `1 本でよい`）+ `詳細を教えて` / `保留`
- 事実確認を求めている通知 → `クリア済み` / `未クリア` / `詳細を教えて` / `保留`
- 内容から候補を作れない場合は `choices: []` でよい（`その他` と補足プロンプトだけになる）

**`その他（補足で指示）` は `choices` に入れない。** コード側（`components/NotificationCard`）が常に末尾へ足す。選ぶと補足プロンプトが必須になり、空なら送信対象から外れる。

## Step 5: アーティファクト作成

**`locked_while_open: true` を必ず付ける**（付けないと、別のエージェントが人の入力中にレポートを書き換えられる）。

**1. エントリポイント作成**
```
artifact(command: "create", id: "notif-report-<YYYYMMDD-HHMM>",
  type: "application/vnd.ant.react",
  title: "通知レポート — <生成時刻>",
  locked_while_open: true,
  project_dir: <自分の作業ディレクトリ>,
  content: <entry-point.jsx。CUSTOMIZE 箇所のみ調整>)
```

**2. `lib/send` モジュール作成**（そのまま）
```
artifact_module(command: "create", id: <同じID>, module_name: "lib/send",
  project_dir: <自分の作業ディレクトリ>, content: <lib--send.jsx をそのまま>)
```

**3. `components/NotificationCard` モジュール作成**（そのまま）
```
artifact_module(command: "create", id: <同じID>, module_name: "components/NotificationCard",
  project_dir: <自分の作業ディレクトリ>, content: <components--NotificationCard.jsx をそのまま>)
```

**4. `data/report` モジュール作成**（Step 1 で集めた内容から新規生成）
```
artifact_module(command: "create", id: <同じID>, module_name: "data/report",
  project_dir: <自分の作業ディレクトリ>, content: <META + NOTIFICATIONS>)
```

**5. 検証**
```
artifact(command: "outline", id: <同じID>, project_dir: <自分の作業ディレクトリ>)
```
エントリポイント + 3 モジュール（`lib/send`, `components/NotificationCard`, `data/report`）が揃っていれば完了。

## Step 6: ユーザーへ提示

レポートを作ったらユーザーに知らせる。トレイ通知をオフにしている親ワークツリーではテキスト出力だけでは気付けないので、`notify_worktree` を使う。

```
notify_worktree(worktree_name: <自分のワークツリー名>, kind: "general",
  body: "通知レポート <ID> を作成しました（未返答 N 件）")
```

**ここで通知をクリアしない。** 宛先のトレイ通知は、ユーザーが返答を送った時点で
**レポート自身が** `oretachi_clear_worktree_notification` を呼んで落とす（`lib/send` の
`clearNotifications`。#218）。レポートを開くのは AI セッションが終わったあとのことが多く、
生成側のセッションがクリアする経路は当てにできないので、生成時にも生成後にも
このセッションからクリアしないこと。

## 送信の仕組み（レポート側の挙動）

実装は `lib/send` と `entry-point` にある。読む人向けの要約:

1. ユーザーが選択肢（または候補ボタン + 補足）を選び、「選択した N 件へ送信」を押す
2. **通知 1 件ごとに**、形状に応じた経路で送る
   - `shape` が `text` … `oretachi_write_terminal` を 2 回（本文 → 150ms → CR）
   - それ以外 … `oretachi_answer_prompt(session_id, expect_fingerprint, kind, ...)` を 1 回
3. 1 件ごとにサイドカーへ結果を書く（途中で閉じても「どこまで届いたか」が残る）
4. 全件終わったら `sent` 分の inbox ID をまとめて `oretachi_ack_message`。**失敗は許容**して「ack 不可」を表示する
5. `sent` になった宛先ワークツリーの未確認通知を `oretachi_clear_worktree_notification`
   （`worktree_id` 指定）で落とす。**失敗は許容**して「クリア失敗」を表示する

### ack とトレイ通知クリアは別のストア（#218）

`oretachi_ack_message` が触るのは sqlite の event_db（inbox の行）で、トレイバッジ /
ホームのカードの件数はフロントが持つ**別の写し**（`NotificationRegistry`）。
**ack だけではバッジが残る。** 残ると、返答済みのワークツリーがトレイポップアップの
巡回に出続け、人が同じ通知を何度も見ることになる（これが #217 の項目4）。

クリアには AI セッションの稼働が要らない（`worktree_id` を明示するので
`resolve_subscriber` のフォールバックに倒れない）。許可条件は `write_terminal` と同じ
#211 の購読なので、**返答を送れた宛先なら必ず通る**。

トレイポップアップが開いたまま外からクリアされた場合は、メインウィンドウが
`tray-notification-cleared` を投げてポップアップの一覧からそのカードを取り除く
（ポップアップの一覧は開いた時点のスナップショットなので、これが無いと消えない）。
遷移中・ダイアログ表示中・アーカイブ依頼中は取り除きを予約に溜めて後で流す
（割り込むと確定待ちのダイアログが別のワークツリーを指す）。

**クリアの粒度はワークツリー単位。** `NotificationRegistry` に通知 1 件ごとの粒度が無いため、
レポートは「そのワークツリーのカードが**全部** `sent` になった宛先」だけをクリアする。
一部だけ送った時点でクリアすると、未返答のカードが残っているのにバッジが消えて
人が気づく導線が失われる。

### ダイアログ経路の安全弁（#215）

`oretachi_answer_prompt` は**送信直前に宛先の画面を読み直して `expect_fingerprint` と照合し、
一致しなければ何も送らない**（`stale`）。照合とキー送信の間は PTY セッション単位の
書き込みロックで直列化されているので、間に別の write（他の通知の押し込み / 別の
`write_terminal`）が割り込むこともない。

これが無いと「人が手でダイアログを消したあとにレポートの送信を押して、その後に開いた
**別の許可ダイアログを承認してしまう**」が起きる。fingerprint には `❯` の位置も入っているので、
矢印の移動量が変わる状況も `stale` として弾かれる。

`status` は 7 種類:

| status | 意味 | 再送 |
|---|---|---|
| `sent` | 送信して画面が変わった | 不要 |
| `unverified` | キーは送ったが画面が変わらなかった | **してはいけない**（矢印が二重に動く） |
| `stale` | 画面が変わっていたので**何も送っていない** | **してはいけない**。レポートを作り直す |
| `unsupported` | その形状にその回答は送れない。**何も送っていない** | 不可 |
| `pastedOnly` | キー列の途中 / Enter だけ失敗。入力状態が中途半端 | **してはいけない** |
| `failed` | 何も送れていない | 自由入力のカードだけ可 |

**ダイアログのカードは一度送ったら結果に関わらず読み取り専用になる。** カードに再送ボタンは
出ない（画面が変わっているか矢印が既に動いているので、同じ回答が別の選択肢を確定しうる）。

### 本文と Enter は別の呼び出しに分ける

**Claude Code は同じ読み取りチャンクに来た CR を送信として扱わない。** 本文の一部として入力欄に取り込むだけなので、`text + CR` を 1 回で書くと「テキストは届くのにターンが始まらない」状態になる（実機で確認済み。`event_delivery::write_push` に同じ現象の記録がある）。

`lib/send` の `sendOne` はこう分けている:

```js
await callTool('oretachi_write_terminal', { session_id, text, submit: false });
await new Promise(r => setTimeout(r, 150));
await callTool('oretachi_write_terminal', { session_id, text: '\r', submit: false });
```

`oretachi_write_terminal(submit: true)` 側も同じ分割をするよう直してあるが、**`submit: false` で自分で分けておけば古い oretachi でも正しく送信でき、ツールの submit 実装に依存しない。**

### 送信テキストは必ず 1 行

`submit: true` は `\n` を `\r` へ正規化するため、**複数行を渡すと行ごとに送信され、宛先の AI エージェントへプロンプトがばらばらに飛ぶ。** `lib/send` の `flatten()` が改行を `" / "` へ畳んでいるので、テキスト生成を書き換えるときも 1 行を維持すること。

`flatten()` は **ESC を含む C0 制御文字も空白へ置換する。** 本文は通知の中身、補足はユーザー入力で、ブラケットペーストで囲んでいないため、ESC がそのまま届くと宛先の TUI へ任意のエスケープシーケンスを流せてしまう。ここを外さないこと。

### 出自の断り書きは自前で入れる

`normalize_artifact_tool_params` が出自を自動で前置するのは `notify_worktree` の `body` と `oretachi_add_task` の `prompt` だけで、`write_terminal` の `text` には何も付かない。`lib/send` の `buildReplyText` が先頭に断り書きを入れているので消さない。

## 制約

- **`oretachi_ack_message` / `oretachi_poll_inbox` / `notify_worktree`（`kind: "worktree.message"` 指定）はアーティファクトからは AI セッション稼働中しか使えない。** `terminal_id` を取らないので、レポートを置いたワークツリーで走行中の AI エージェント端末が**ちょうど 1 つ**でないと失敗する。AI セッション終了後にユーザーがレポートを触る場合は常に失敗するので、**ack の失敗を前提に設計してある**（返答自体は届く）。`oretachi_write_terminal` / `oretachi_answer_prompt` / `oretachi_clear_worktree_notification` は宛先を明示するのでこの制約を受けない。
- **表示中ロックが守るのは「そのウィンドウでいま表示している 1 件」だけ。** ウィンドウが開いたままでもユーザーが別のアーティファクトへ切り替えるとロックは外れる。レポートはユーザーがそのページに留まっている前提で扱う。
- **`multiSelect` の設問と複数設問の 2 問目以降はレポートから答えられない（#215）。** 複数選択は画面から単一選択と判別できず、トグルキーを推測して送ると意図しない選択を確定しうるため、単一選択の 1 つ選んで CR だけを提供する。複数設問は 1 問答えると画面が次へ変わるので、続きは次のレポートに回る（カードに「まだ設問が残っています」と出る）。どちらも人がターミナルを開いて操作するのが確実。
- **`shape` の判定はレポート生成時点のスナップショット。** 生成後に宛先が進んでダイアログが消えていれば、送信時に `stale` になって何も送られない（安全側に倒れる）。
- **ホーム / リポジトリ擬似ワークツリー宛はワイルドカード購読からしか許可が出ない。** 名前指定の購読ができないため、これらからの通知に返答したい場合は `*` / `repo:` 購読が必要。

## 禁止事項

- **`locked_while_open: true` を付け忘れない。** 付けないと人の入力中に裏から書き換えられる。
- **返答状態を本体 JSON（`content` / `data/report`）に持たない。** 自分自身のロックで書き換えられなくなる。
- **レポートを開いている間に本体へ追記しようとしてリトライしない。** 開いている限り成功しないので、新しいレポートを作る。
- **購読していないワークツリーをレポートに載せない。** 返答を送れないカードになる。ユーザーの指示なしに購読を張って範囲を広げるのもしない。
- **通知本文を要約してカードに載せない。** 人の判断材料なので全文を入れる（カードは全文をそのまま表示する。折りたたみは無い）。
- **`その他（補足で指示）` を `choices` に入れない。** コード側が足すので二重になる。
- **送信テキストに改行を入れない。** 行ごとに送信されて宛先のエージェントへプロンプトが分割して飛ぶ。
- **本文と Enter を 1 回の `write_terminal` でまとめない。** テキストは届くのにターンが始まらない。
- **`flatten()` の制御文字除去を外さない。** 宛先の TUI へエスケープシーケンスを注入できてしまう。キー列は Rust 側（`oretachi_answer_prompt`）が組むので、JS から生のキーを流さない。
- **通知のクリアを生成側のセッションでやらない。** レポート自身が送信成功時に呼ぶ（#218）。生成時にクリアすると、人が捌く前にバッジが消える。
- **宛先ワークツリー名を推測しない。** `oretachi_poll_inbox` の `sourceWorktreeId` / `sourceWorktreeName` だけを使う。推測した名前で `oretachi_list_terminals` を引くと空が返り、`sessionId: null` の押せないカードになる（#218）。

### ダイアログ関連（#215）

- **ダイアログで止まっている宛先へ自由テキストを送らない。** テキストはダイアログに吸われ、末尾の CR が意図しない選択肢（許可ダイアログの既定は `1. Yes`）の確定として解釈される。判定は `oretachi_inspect_prompt` の `shape` で行う（`text` 以外なら全部ダイアログ）。
- **`shape` が `text` 以外のとき、候補ボタンの文字列を創作しない。** `options[].label`（画面に実在する選択肢）だけを出す。`choices` は `[]` にする。
- **`oretachi_inspect_prompt` の戻り値を編集しない。** 要約・言い換え・整形はどれも駄目。特に `fingerprint` を書き換えると照合が必ず外れて何も送れなくなる。
- **フィールドを取捨選択して転記しない。** 戻り値のオブジェクトを**丸ごと**入れる。特に `truncated` を落とすと、画面に収まっていないダイアログの「読めた選択肢だけ」を完全な一覧として人へ見せ、拒否の選択肢を見ないまま承認させることになる。
- **`stale` をリトライしない。** 画面が変わっているので何度試しても同じ。レポートを作り直す導線を出す。
- **`unverified` / `pastedOnly` を再送しない。** 既に送ったキーで `❯` が動いているので、同じ回答をもう一度送ると別の選択肢を確定しうる。
- **`unknown` に推測でキーを送らない。** カードは送信ボタン無効 + `tail` の表示にして、ターミナルでの手動操作へ誘導する。
- **1 セッションに対しキー操作カードを 2 枚以上作らない。** ダイアログは 1 つしか無い。同じ宛先の通知が複数あれば最新 1 件だけに `prompt` を付ける。
- **`oretachi_read_terminal` のテキストからダイアログを読み取ろうとしない。** Claude Code はカーソル移動で差分描画するので、ANSI を除去すると再描画の断片しか残らない。問いの形状は `oretachi_inspect_prompt` から取る。
