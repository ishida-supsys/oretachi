# oretachi — リポジトリ共通の作業ルール

このファイルはリポジトリに追跡されており、すべてのワークツリー / クローンで共有されます。

## auto-merge (`automerge` ラベル)

**`automerge` ラベルの付いた PR は、CI (`type-check` / `cargo-check`) が通り次第、
人手を介さず自動でマージされます。**
`.github/workflows/enable-auto-merge.yml` が `gh pr merge --auto --merge` を代行するためです。
必須チェックは main ruleset の CI 2件だけで、**レビュー承認は要求されません**。

### 自動で有効化される条件

* draft でない **かつ** `automerge` ラベルが付いている PR。
* 発火契機は `opened` / `reopened` / `ready_for_review` / `labeled` の4つ。

### 有効化・解除の非対称性（重要）

* **有効化は最初の1回だけ**です。`synchronize` は拾っていないため、
  auto-merge が解除された状態（人が手で切った / コンフリクトで GitHub 側が解除した等）は
  push しても再武装されません。
  再武装したいときはラベルを外して付け直します（`labeled` が再発火します）。
* **ラベルを外しても auto-merge は解除されません。**
  ラベルは起動契機にすぎず、GitHub 側の auto-merge フラグとは独立です。
  止めるには `gh pr merge --disable-auto <PR>` を実行するか、
  PR 画面の Disable auto-merge を押します。

### AI が守ること

* **`automerge` ラベルを自分で付けてはいけません。** ラベル付与は人の承認ゲートです。
* **ラベルが付いた PR は「出してから直す」ができません。**
  CI が通った瞬間にマージされるため、CI エラー以外の理由
  （レビュー指摘、自分で気づいた修正漏れ、コミットメッセージの直し等）で
  後から追いコミットする猶予はありません。
  → PR を作る前に修正を完了させること。レビューを挟みたい場合は
  ラベルを付けずに出すか、draft で出してください。
* すでに auto-merge が有効な PR で修正が必要になったと気づいた場合は、
  追いコミットの前に **`gh pr merge --disable-auto <PR>` で止める**
  （ラベルを外すだけでは止まりません）。
  すでにマージ済みだった場合は追加 PR で直します。
