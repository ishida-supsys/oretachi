---
name: notification-auto-dismiss
description: 購読しているワークツリーから届いた Stop 通知のうち、バックグラウンド処理（shell / subagent / monitor など）の完了待ちで止まっているだけのもの、および自動承認が承認したあとに残った approval 通知を自動で見分け、1 行報告してトレイ通知を外す。人の入力が要る通知はトレイに残す。ユーザーが「バックグラウンド待ちの通知は自動で消して」「完了待ちだけの通知を片付けて」等と言ったときに使う。notification-report と同じタブでは併用しない。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_list_subscriptions, mcp__plugin_oretachi_oretachi__oretachi_subscribe_worktree, mcp__plugin_oretachi_oretachi__oretachi_poll_inbox, mcp__plugin_oretachi_oretachi__oretachi_ack_message, mcp__plugin_oretachi_oretachi__oretachi_list_worktree_notifications, mcp__plugin_oretachi_oretachi__oretachi_clear_worktree_notification, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_inspect_prompt
---

# notification-auto-dismiss スキル

購読対象のワークツリーから届く `Stop` 通知のうち、**バックグラウンド処理の完了待ちで止まっているだけ**のものを判定し、報告を 1 行出してからトレイ通知を外す。人の入力が要る通知はトレイに残す。

対象は2種類ある。**Stop 通知**（Step 1〜4）と、**自動承認が承認したあとに残った approval バッジ**（Step 5）。後者は `oretachi_poll_inbox` を使わず、トレイのバッジ（`oretachi_list_worktree_notifications`）から直接判定する。

## 前提

- **起動する場所:** ワークツリー・home・リポジトリのどのタブからでもよい。対象は**そのタブの購読範囲**（`*` / `workgroup:<ID>` / `repo:<名前>` / ワークツリー名）。
- **notification-report と同じタブでは併用しない。** どちらも `oretachi_poll_inbox`（terminal 単位の inbox）を消費するため、片方が取ったぶんをもう片方が見られなくなる。別タブで同じワークツリーを購読していれば、同じ通知がそちらのレポートに載ることはある（仕様）。
- **このスキルを動かしているタブ自身**の Stop も通知バッジを出すが、何もしない（自分の通知は判定対象にしない）。
- **報告先はこのタブの端末出力だけ。** artifact など別の保存先は作らない。
- **迷ったら「入力要」にする。** 誤って外した通知は取り戻せないが、誤って残した通知は人が開けば済む。

## Step 0: 購読を整える

自分の terminal_id（セッション開始時に注入されたもの）で `oretachi_list_subscriptions` を呼び、状態に応じて扱う。

| 状態 | やること |
|---|---|
| 購読が無い | 対象をユーザーに聞く（home なら `*` を提案）。**勝手に張らない** |
| 既存の購読の `event_kinds` に `completed` が無い | `oretachi_subscribe_worktree` で張り直して `completed` を追加する。`(terminal_id, target)` で UNIQUE に**上書き**されるので、**既存の kinds との和集合**を渡す |
| `delivery` が `passive` | ユーザーに断ってから `turn_end` に直す。`passive` だと新着が届いてもこのタブが動かず、手で呼び直すまで片付かない |

次の前提を知っておくこと。

- **自動承認との関係:** 宛先ワークツリーの自動承認設定は配送に影響しない。`completed` は自動承認が ON のタブにも本文なしの告知として届く（本文は `oretachi_poll_inbox` のツール出力で読む）。
- **events.db の書き込み増:** `*` で `completed` を購読すると、全ワークツリーの Stop が events.db に書かれる。
- **対象外:** リポジトリ設定で Stop を `completed` 以外の kind に割り当てている場合は、このスキルには届かない。

## Step 1: 収集

`oretachi_poll_inbox(terminal_id: <自分の terminal_id>)` を **1 回**呼ぶ。返り値がこの回の**母集合**で、あとから足さない。

- 本文を JSON として読み、`hook_event_name == "Stop"` のものだけを判定対象にする。
- それ以外（Stop 以外 / JSON として読めない本文）は判定せず、Step 4 で ack だけする。
- 自分自身のワークツリー由来（`sourceWorktreeId` が自分）の Stop は判定対象にしない（ack だけ）。

## Step 2: 判定

Stop 1 件ごとに判定する。次を**すべて**満たすものだけを「バックグラウンド待ち」とする。

1. 本文が JSON として読めて、**`background_tasks` キーが存在する**。キーが無い、または切り詰めで判定できないなら「入力要」。
2. `background_tasks` か `session_crons` が**空でない**。`background_tasks_omitted` が 1 以上のときも「空でない」に数える（件数を減らして切り詰められただけで、実際には複数ある）。
3. `last_assistant_message` が、次のどれにも**当たらない**。
   - 人への質問・選択肢の提示
   - 承認依頼・「〜してよいか」の確認
   - 作業完了の報告と次の指示待ち

`background_tasks` が空でないことは**必要条件であって十分条件ではない**。`pnpm run tauri dev` のような常駐プロセスを裏で動かしたまま、人に質問して止まっているセッションでも配列は空にならない。だから 3 を必ず読む。

本文に `_truncated: true` があるときは、`last_assistant_message` が先頭と末尾だけに中略されている（`…（中略）…`）。残った先頭・末尾から判断し、**読み切れず判断がつかなければ「入力要」**にする。人への質問は末尾に来やすいので、末尾は特に丁寧に読む。

### 判断例

| `last_assistant_message` | 判定 |
|---|---|
| 「ビルドの完了を待ちます」「結果が来たら続けます」 | バックグラウンド待ち |
| 「サブエージェントのレビューが終わるまで待機します」 | バックグラウンド待ち |
| 「待っている間に A と B どちらにするか決めてください」 | **入力要**（選択を求めている） |
| 「実装が完了しました。次は何をしますか？」 | **入力要**（完了報告と次の指示待ち） |
| 「この方針で進めてよいですか？」 | **入力要**（承認依頼） |
| 「テストが失敗しました。どう直しますか？」 | **入力要** |
| 判断に迷う・文面が途切れて読めない | **入力要** |

## Step 3: クリア

ワークツリー（`sourceWorktreeId`）ごとに、次の手順で処理する。

1. **母集合に含まれるそのワークツリーの Stop が全件**バックグラウンド待ちのときだけ進む。1 件でも入力要が混ざるなら、そのワークツリーはクリアしない。
2. `oretachi_list_worktree_notifications` で、そのワークツリーの `count` と `kind` を見る。
3. `count` がそのワークツリーのバックグラウンド待ちの件数と一致し、かつ `kind == "completed"` のときだけ、`expected_count` / `expected_kind` を付けて `oretachi_clear_worktree_notification` を呼ぶ。

   ```
   oretachi_clear_worktree_notification(
     worktree_id: <sourceWorktreeId>,
     expected_count: <手順 2 の count>,
     expected_kind: "completed"
   )
   ```
4. 一致しない、または結果が `skipped: true` のときは**クリアせず残す**。判定している間に同じワークツリーへ approval や本物の入力待ちが積まれた可能性があるため。

条件を付けずに呼ばない。無条件クリアは、判定中に積まれた本物の通知まで消す。

## Step 4: ack と報告

- 母集合は**全件 ack** する（`oretachi_ack_message`）。入力要と判定したものはトレイに残っているので、人はそちらで気づける。
- 端末への出力は **1 件 1 行**にする。
  - 外した通知: `<worktreeName>: <background_tasks の description を要約> の完了待ちのため通知を外しました`
  - 残した通知: `<worktreeName>: 入力要（<理由>）のため残しました`
  - 条件付きクリアが `skipped` だったとき: `<worktreeName>: 判定中に別の通知が積まれたため残しました`
- 母集合が 0 件なら「新着の Stop 通知はありません」と 1 行だけ出す。

## Step 5: 自動承認が承認したあとに残った approval バッジ

oretachi の自動承認は、許可ダイアログを承認しても、同じダイアログに由来する後続のフック（約 6 秒遅れの Notification など）を「未承認」として数え、トレイに approval バッジを残すことがある。承認後は Claude Code が動いているので人の入力は要らない。Step 1〜4 とは独立に、**購読範囲のワークツリー**へ次を適用する。

1. `oretachi_list_worktree_notifications` を呼び、`kind == "approval"` で `lastAutoApprovedAt` が null でない item だけを候補にする。`lastAutoApprovedAt` は自動承認が実際にダイアログを承認した直近時刻で、アプリ再起動で消える。null なら人の入力待ちかもしれないので候補にしない。
2. 候補ごとに、次を**すべて**満たすものだけを「承認後の残留」とする。1 つでも欠けるか迷ったら残す。
   - `oretachi_get_worktree_status` の `autoApproval` が `true`。
   - `lastAutoApprovedAt - 1000 <= firstNotifiedAt <= lastAutoApprovedAt + 30000`（ミリ秒）。バッジが承認の直後に積まれたことの確認。承認より前から残っているバッジは別の入力待ちかもしれない。
   - そのワークツリーの全ターミナル（`oretachi_list_terminals` の `worktreeId` が一致するもの）について `oretachi_inspect_prompt` の `shape` が `"text"`。`permission` / `plan` / `askUserQuestion` / `yesno` / `numbered` が 1 つでもあれば、いま人の回答待ちのダイアログが出ているので残す。`unknown` / `pager` も判断できないので残す。
3. Step 3 と同じく、`expected_count`（item の `count`）と `expected_kind: "approval"` を付けて `oretachi_clear_worktree_notification` を呼ぶ。`skipped: true` ならクリアせず残す。
4. **Step 3 でそのワークツリーを残すと決めた（Stop の入力要が混ざる）場合は、Step 5 でもクリアしない。** `count` が Stop と approval の合算で `kind` は最後に来た方になるので、条件が一致しても片方が人の入力待ちである可能性が残る。

プラン承認（ExitPlanMode）と AskUserQuestion は自動承認の対象外で、ダイアログは出たままなので、2 のダイアログ判定で必ず残る。

報告は 1 件 1 行: `<worktreeName>: 自動承認済み（承認後に届いた重複通知）のため通知を外しました`。母集合の概念が無いので ack は不要。Step 4 の「新着の Stop 通知はありません」は Step 5 の結果と関係なく、Stop が 0 件のときだけ出す。
