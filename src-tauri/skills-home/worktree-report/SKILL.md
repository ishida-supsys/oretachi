---
name: worktree-report
description: 全ワークツリーの状況を1つのレポートにまとめて報告する。作業内容(description)・ブランチ・git 状態・ターミナルの稼働状況・PR 状態を横断的に集約する。「今どうなってる」「状況を教えて」「ワークツリー一覧」などの依頼で使う。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_inspect_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_read_terminal, mcp__plugin_oretachi_oretachi__oretachi_list_repository, mcp__plugin_oretachi_oretachi__oretachi_show_worktree, Bash
---

# worktree-report スキル

ワークツリー群の現状を 1 画面で把握できるレポートにまとめる。

## 手順

1. `oretachi_get_worktree_status` で全ワークツリーを取得する（`isHome` / `isRepository` は除外）。
   `query` を渡せば name / branchName / description の部分一致で絞り込める。
   各エントリの `workgroupName` が所属ワークグループ（表示名）。レポートはこれで章立てする。
2. `oretachi_inspect_worktree` で各ワークツリーの git 状態を取得する。
3. `oretachi_list_terminals` でターミナルの稼働状況（running / exited、AI エージェントの有無）を取得する。
4. 必要なら `gh pr list --json number,title,headRefName,state` で PR 状態を突き合わせる。
   ブランチ名で紐付ける。`gh` が無い環境ではこの列を省略する。
5. `workgroupName` ごとにグループ分けし、下記フォーマットで表を出力する。
   最後に注意が必要なものを 2〜3 行でまとめる。

## 出力フォーマット

ワークグループごとに見出しを立て、その配下に表を出す。
グループが 1 つしかない場合は見出しを省略してよい。

```
## グループ(1)

ワークツリー      ブランチ              状態     端末   PR      作業内容
oretachi-jaqo     worktree/home-...     dirty 3  claude #112    ホームターミナルの実装
oretachi-k3p1     fix/tray-focus        clean    —      #111 M  トレイのフォーカス修正

## リリース準備

ワークツリー      ブランチ              状態     端末   PR      作業内容
oretachi-9x2b     release/0.29.0        clean    —      #120 O  0.29.0 のリリース作業
```

- 状態: `clean` / `dirty N` / `merged→<branch>`
- 端末: 動作中のプロセス名。停止済みなら `—`
- PR: 番号と状態（O=OPEN, M=MERGED, C=CLOSED）

## ワークツリーを見せる

「〜の様子を見せて」「〜を開いて」と言われたら `oretachi_show_worktree` でフォーカスを移す。
レポート中に特定のワークツリーへ誘導したいときにも使える。

## まとめの観点

- 未コミット変更を抱えたまま長く放置されているもの
- PR がマージ済みなのに残っているもの（`worktree-cleanup` の候補）
- エージェントが承認待ちで止まっているもの

## 禁止事項

- レポート作成のためにワークツリーの内容を変更しない（読み取り専用の作業）
