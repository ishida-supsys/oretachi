---
name: worktree-cleanup
description: 不要になったワークツリーを棚卸しして削除する。マージ済み・未コミット変更なし・ターミナルが停止しているものを削除候補として提示し、ユーザーの承認を得てから削除する。「ワークツリーの掃除」「使っていないワークツリーを消したい」「棚卸し」などの依頼で使う。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_inspect_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_read_terminal, mcp__plugin_oretachi_oretachi__oretachi_close_worktree
---

# worktree-cleanup スキル

ワークツリー追加先ディレクトリ配下のワークツリー群を棚卸しし、不要になったものを
**ユーザーの承認を得てから**削除する。

## 手順

1. `oretachi_get_worktree_status` で全ワークツリーを取得する。
   `isHome: true`（ホーム自身）と `isRepository: true`（リポジトリのルート）は対象外。
2. 各ワークツリーについて `oretachi_inspect_worktree` を呼び、
   `dirtyCount` / `mergedInto` / `lastCommitAt` を取得する。
3. ターミナルが残っているものは `oretachi_list_terminals` で状態を確認し、
   停止しているものは `oretachi_read_terminal` で末尾を読んで停止理由を把握する。
4. 下記の判定基準で分類し、**表にして提示する**。根拠（マージ先・dirty 数・最終コミット）を必ず添える。
5. ユーザーが承認したものだけ `oretachi_close_worktree` を呼ぶ。

## 判定基準

| 区分 | 条件 |
|---|---|
| 削除推奨 | 対象ブランチにマージ済み・`dirtyCount == 0`・ターミナルが停止済み |
| 要確認 | 未マージだが 30 日以上更新なし・`dirtyCount == 0` |
| 残す | `dirtyCount > 0`、またはターミナルが動作中 |

## 提示フォーマット

```
区分      ワークツリー      ブランチ            根拠
削除推奨  oretachi-k3p1     fix/tray-focus      merged→main / dirty 0 / 12 日前
要確認    oretachi-z0f4     worktree/spike-ui   未マージ / 31 日更新なし
残す      oretachi-jaqo     worktree/home-...   dirty 3
```

そのうえで「削除推奨の N 件を削除しますか？」と尋ね、返答を待つ。

## 禁止事項

- ユーザーの承認なしに `oretachi_close_worktree` を呼ばない
- `dirtyCount > 0` のワークツリーを削除推奨に入れない
- 「要確認」を勝手に削除推奨へ格上げしない
- ホーム（`isHome: true`）とリポジトリ（`isRepository: true`）を削除しようとしない（サーバー側でも拒否される）

## 削除時のオプション

`oretachi_close_worktree` には次を渡せる。ユーザーの意向を確認してから決める。

- `merge_to`: 削除前にマージするターゲットブランチ
- `delete_branch`: ブランチも削除するか（マージ済みなら通常 true）
- `force_branch`: 未マージブランチを強制削除するか（既定は使わない）
