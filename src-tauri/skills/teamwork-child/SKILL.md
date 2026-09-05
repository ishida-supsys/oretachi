---
name: teamwork-child
description: このワークツリーが親issue(teamwork-parentセッション)によって作成されたsub-issue用ワークツリーであることを認識させ、報告・クローズに関する追加義務のみを課す。通常は自発的に呼び出す必要はなく、teamwork-parentが子ワークツリー作成時のoretachi_add_task呼び出しのpromptで「まずこのスキルを読み込んでください」と明示的に指示することで読み込まれる。
allowed-tools: mcp__plugin_oretachi_oretachi__notify_worktree, mcp__plugin_oretachi_oretachi__oretachi_close_worktree, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status
---

# teamwork-child スキル

このワークツリーは親issueのsub-issueとして作成された。実際の作業内容・進め方は、通常どおり他のスキル・エージェント設定・ユーザー指示に従ってよい。**このスキルはそれらを一切上書きせず、実行フローも規定しない。** 以下の3つの義務のみを追加で負う。

## 義務1: 報告

親issueまたは現在のissueへの報告事項が発生したら、通常の報告作業(issueコメント投稿等)に加えて`notify_worktree`で親ワークツリーへ通知する。

- issueへコメントした場合: 本文は省略し、コメントのURLのみをbodyに入れる。
  ```
  notify_worktree(worktree_name: <自分のワークツリー名>, kind: "general",
    event_kind: "worktree.message", body: "<コメントURL>")
  ```
  - `event_kind: "worktree.message"`を指定することで、親issueワークツリーが購読している場合に配送される(宛先は購読側=親issueワークツリーが決めるため、`worktree_name`は表示トースト用であり配送先には無関係)。
  - 自分のワークツリー名が分からない場合は`oretachi_get_worktree_status(query: <git branch --show-current の結果>)`で確認する。
- ユーザーが購読機能を認識しており、issueコメントを介さず直接ワークツリーへの報告を指示された場合に限り、`notify_worktree`の通知のみでよい(issueコメント投稿は不要)。

## 義務2: 停止条件

issue本文に`## 停止条件`セクションがある場合、そこに書かれた各項目は**人の判定が必須**であり、AIだけで先に進めてはいけない。

1. 該当する状態に達したら、**作業を止めて自ワークツリー上でユーザーに確認する。** `notify_worktree`で自分自身に通知してユーザーを呼び戻す:
   ```
   notify_worktree(worktree_name: <自分のワークツリー名>, kind: "approval",
     body: "<どの停止条件に達したか>")
   ```
2. 確認が取れたら、**どの停止条件をクリアしたかを親へ報告する**(親が計画フロー図のチェックを付けるため)。本文には停止条件のidまたはテキストが分かる形で含める:
   ```
   notify_worktree(worktree_name: <自分のワークツリー名>, kind: "general",
     event_kind: "worktree.message", body: "停止条件クリア: 「<停止条件のテキスト>」→ <ユーザーの判断内容>")
   ```
3. issue本文のチェックボックスも `- [x]` に更新しておくとよい。

**子の停止条件は子の上で解決する。親を止めるためのものではない。** 親は子の停止を受け取っても止まらず流れを継続する仕様なので、ユーザーへの確認は必ず自ワークツリーで行うこと。

## 義務3: 完了時のクローズ

ユーザーからタスクの完了を指示されたら`oretachi_close_worktree`でこのワークツリーを終了する。実際のクローズはUI側のユーザー確認ダイアログを経て確定するため、この呼び出し自体が最終承認ではない。

## 階層構造について

このワークツリーがさらに自分のsub-issue(sub-sub-issue)を持つ場合は`teamwork-parent`スキルを併せて読み込み、そちらの手順に従って自分の子ワークツリーを管理する。その場合も本スキルの義務1〜3はそのまま維持される(親への報告・自分の停止条件・自分自身のクローズ)。
