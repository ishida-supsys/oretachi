---
name: notification-report
description: 購読しているワークツリーから通知が溜まったときに、関連する通知の一覧を読んでレポートアーティファクトを作成する。人はレポートを見るだけで、ターミナルを1つずつ開かずに溜まった通知へ一括でクイックに返答できる。ホームタブや teamwork-parent の親ワークツリーからの利用を想定。ユーザーが「通知をまとめて確認したい」「溜まった通知にまとめて返したい」等と言ったときに使う。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_list_subscriptions, mcp__plugin_oretachi_oretachi__oretachi_subscribe_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_worktree_notifications, mcp__plugin_oretachi_oretachi__oretachi_poll_inbox, mcp__plugin_oretachi_oretachi__oretachi_ack_message, mcp__plugin_oretachi_oretachi__oretachi_clear_worktree_notification, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_read_terminal, mcp__plugin_oretachi_oretachi__notify_worktree, mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__artifact_store, mcp__plugin_oretachi_oretachi__search_artifact, Read, Glob, Grep
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
oretachi_get_worktree_status(query: <ワークツリー名>)
```

`description` が入っていればそれを使う。**未設定なら `null` を入れ**、代わりに 1-4(b) のターミナル出力からミッションを推定して `descFallback` に書く（カードには `[description 未設定]` バッジ付きで出る）。

**(b) 現況**

```
oretachi_list_terminals(worktree_name: <ワークツリー名>)   → isAiAgent: true / status: "running" の session_id
oretachi_read_terminal(session_id: <上で得た値>, max_bytes: 8192)
```

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

### 1-5. 送信先の session_id を決める

1-4(b) で得た「そのワークツリーで走っている AI エージェント端末」の `session_id` をそのまま `sessionId` に入れる。見つからなければ `null` を入れる（カードが「稼働中の AI 端末が見つからなかったため送信できません」になる）。

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

## Step 4: 候補ボタンを作る

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

**ここで通知をクリアしない。** `oretachi_clear_worktree_notification` はユーザーが返答を送り終えたあとに呼ぶ（レポートを作った時点では、まだ人が捌いていない）。

## 送信の仕組み（レポート側の挙動）

実装は `lib/send` と `entry-point` にある。読む人向けの要約:

1. ユーザーが候補ボタン + 補足を選び、「選択した N 件へ送信」を押す
2. **通知 1 件ごとに** その宛先へ `oretachi_write_terminal` を 2 回呼ぶ（本文 → 150ms → CR）。宛先の端末が別々なので 1 本にまとめられない
3. 1 件ごとにサイドカーへ結果を書く（途中で閉じても「どこまで届いたか」が残る）
4. 全件終わったら成功分の inbox ID をまとめて `oretachi_ack_message`。**失敗は許容**して「ack 不可」を表示する

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

- **`oretachi_ack_message` / `oretachi_poll_inbox` / `notify_worktree`（`event_kind` 付き）はアーティファクトからは AI セッション稼働中しか使えない。** `terminal_id` を取らないので、レポートを置いたワークツリーで走行中の AI エージェント端末が**ちょうど 1 つ**でないと失敗する。AI セッション終了後にユーザーがレポートを触る場合は常に失敗するので、**ack の失敗を前提に設計してある**（返答自体は届く）。
- **表示中ロックが守るのは「そのウィンドウでいま表示している 1 件」だけ。** ウィンドウが開いたままでもユーザーが別のアーティファクトへ切り替えるとロックは外れる。レポートはユーザーがそのページに留まっている前提で扱う。
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
- **`flatten()` の制御文字除去を外さない。** 宛先の TUI へエスケープシーケンスを注入できてしまう。
- **レポートを作った時点で通知をクリアしない。** ユーザーが返答し終えてから `oretachi_clear_worktree_notification` を呼ぶ。
