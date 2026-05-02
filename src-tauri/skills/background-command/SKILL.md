---
name: background-command
description: pnpm dev / pnpm run tauri dev / next dev / vite / nodemon / cargo watch / docker compose up など長時間常駐する開発サーバ・watcher・background プロセスを起動するときに、bash の run_in_background ではなく oretachi MCP ツール経由で oretachi UI の新しいターミナルタブとして起動するためのスキル。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_spawn_terminal, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_kill_terminal, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status
---

# background-command スキル

長時間常駐するコマンド（開発サーバ・ファイル監視・watch ビルド・HTTP サーバ等）を実行するとき、Claude Code の `bash` tool の `run_in_background` を使うのではなく、oretachi の MCP ツールで oretachi UI 上に新しいターミナルタブを追加してそこで実行する。

これにより:

- 出力ログが xterm.js でユーザーに可視化される
- ユーザーが任意タイミングで Ctrl-C / kill できる
- 他ワークツリーへの切替・detach 等の oretachi 既存 UI が活用できる
- Claude Code 終了後もプロセスを残せる

## いつこのスキルを使うか

**使う**（長時間常駐するコマンド）:

- `pnpm dev`, `pnpm run dev`, `pnpm run tauri dev`, `npm start`, `yarn dev`
- `next dev`, `vite`, `nuxt dev`, `astro dev`
- `nodemon`, `tsc --watch`, `cargo watch`, `cargo run`（HTTP サーバ等）
- `docker compose up`（`-d` なし）, `rails s`, `flask run`, `python -m http.server`
- その他、Ctrl-C しない限り終了しないコマンド全般

**使わない**（短命コマンド or 出力をすぐ後処理したいケース）:

- `pnpm test`, `pnpm run build`, `cargo build`, `cargo check`, `tsc --noEmit`, `eslint .`, `prettier --check`
- ビルド・テスト・lint・型チェック等、終了コードを判定したいもの
- ユーザーが明示的に「bash で background 実行して」と指示した場合
- 出力を即 grep / 解析したい短命スクリプト（`curl`, `git log`, `ls` 等）

判断基準: **「実行後ずっと起動し続けるか？」**「Yes」ならこのスキル、「No」なら通常の bash。

## 手順

### Step 1: ワークツリーを特定する

`oretachi_get_worktree_status` を呼び、現在の作業ディレクトリ（PWD）と一致する worktree の `name` と `id` を取得する。

複数の worktree が同名だった場合は `id` も使う。

### Step 2: ターミナルを起動してコマンドを流し込む

```
oretachi_spawn_terminal({
  worktree_name: "<Step 1 で取得した name>",
  worktree_id: "<必要なら id を指定>",
  command: "pnpm dev",
  title: "pnpm dev",            // タブ表示名
  reason: "ローカルサーバ起動"  // ログ用メモ（任意）
})
```

戻り値は `"新規ターミナルの追加リクエストを送信しました"` のみ（fire-and-forget）。

oretachi UI に新しいタブが追加され、`pnpm dev` がそのターミナルで実行される。

### Step 3: 起動確認（必要な場合）

数秒待ってから `oretachi_list_terminals({ worktree_name: "<name>" })` を呼ぶと、`session_id` / `cwd` / `isAiAgent` を含む配列が返る。直前に追加したセッションが含まれていれば起動成功。

### Step 4: 停止する場合

`oretachi_list_terminals` で対象の `session_id` を特定してから:

```
oretachi_kill_terminal({ session_id: <値> })
```

UI のタブは `pty-exit` イベント経由で自動的に消える。

## 注意点

- `command` は末尾に改行が無くても、oretachi 側で `\n` が自動付与される
- シェルは oretachi のデフォルト（Windows なら PowerShell、それ以外は OS のシェル）。プラットフォーム固有の構文（PowerShell の `;` と bash の `&&` の違い等）に留意
- `pendingScripts` 機構の制限により、同一ワークツリーに連続して `oretachi_spawn_terminal` を呼ぶ場合は前のコマンド投入完了まで待つこと。並行投入すると先に積まれたコマンドが上書きされる可能性がある
- 同名ワークツリーが複数あって `worktree_id` を指定しなかった場合は `invalid_params` エラーが返る。エラー本文に候補 ID が含まれているのでそれを使って再試行する
