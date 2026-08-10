---
name: worktree-assign
description: 新しい作業のためにワークツリーを作成し、AI エージェントにタスクを割り当てる。「〜をやりたい」「〜を直して」といった作業依頼を受けたときに、適切なリポジトリを選んで oretachi_add_task でワークツリー作成とエージェント起動を任せる。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_list_repository, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_add_task, mcp__plugin_oretachi_oretachi__notify_worktree
---

# worktree-assign スキル

新しい作業を受け取り、ワークツリーを作ってエージェントに割り当てる。

## 手順

1. `oretachi_list_repository` で登録リポジトリと `branchNamePattern` を確認する。
2. `oretachi_get_worktree_status` で、**同じ作業のワークツリーが既に無いか**確認する。
   description やブランチ名が近いものがあれば、新規作成せずそちらを使うようユーザーに提案する。
3. 作業内容が曖昧なら、着手前に 1〜2 点だけ確認する（対象リポジトリ・スコープ）。
4. `oretachi_add_task` にプロンプトを渡す。oretachi 側が
   ワークツリー作成 → セットアップスクリプト実行 → エージェント起動まで非同期で行う。

## プロンプトの書き方

`oretachi_add_task` に渡すプロンプトは、割り当て先のエージェントがそのまま読む指示になる。

- 対象リポジトリ名を明記する
- 何を達成すれば完了かを 1 行で書く
- 既知の制約（触ってはいけない箇所、参照すべきファイル）があれば添える

## 禁止事項

- 重複するワークツリーを確認せずに作らない
- 作業内容が全く不明なまま `oretachi_add_task` を呼ばない
