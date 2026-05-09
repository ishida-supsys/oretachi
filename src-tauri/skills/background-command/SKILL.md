---
name: background-command
description: pnpm dev / pnpm run tauri dev / next dev / vite / nodemon / cargo watch / docker compose up など長時間常駐する開発サーバ・watcher・background プロセスを起動するときに、bash の run_in_background ではなく oretachi MCP ツール経由で oretachi UI の新しいターミナルタブとして起動するためのスキル。起動後の出力参照・キー入力（vitest の再実行など）にも対応。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_spawn_terminal, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_kill_terminal, mcp__plugin_oretachi_oretachi__oretachi_read_terminal, mcp__plugin_oretachi_oretachi__oretachi_write_terminal, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status
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

### Step 4: 出力ログを参照する

dev サーバのコンパイルエラー確認、vitest の結果確認などに使う。

```
oretachi_read_terminal({
  session_id: <値>,
  max_bytes: 8192   // 省略可。デフォルト 8192。長い場合は 32768 等まで増やす
})
```

戻り値は **ANSI エスケープ除去済みの UTF-8 文字列**（バッファ末尾から `max_bytes` バイト）。リングバッファは 64KiB で、それを超える出力は古い側から破棄される。

### Step 5: ターミナルへ入力する

vitest の単一キー入力（`a` で再実行 / `q` で終了）、対話プロンプトへの応答、PowerShell へのコマンド追加投入などに使う。

```
oretachi_write_terminal({
  session_id: <値>,
  text: "pnpm test\n",
  submit: true   // デフォルト true。PowerShell/conpty 互換に \r へ正規化＋末尾保証する
})

// vitest の press 'a' to re-run all 等、生キーを送りたい時
oretachi_write_terminal({
  session_id: <値>,
  text: "a",
  submit: false  // 改行付与なし、raw 送信
})
```

`submit: true` ならコマンド送信扱い（Enter まで押す）。`submit: false` なら raw 送信。

### Step 6: 停止する場合

`oretachi_list_terminals` で対象の `session_id` を特定してから:

```
oretachi_kill_terminal({ session_id: <値> })  // 強制 kill
```

graceful に終了させたい場合（dev サーバの後始末を走らせたい等）は kill ではなく `oretachi_write_terminal` で `Ctrl-C` 相当のシグナルが必要。現状の write は文字列送信のみで `\x03` の効果は限定的なので、安全策としては:

1. まず `oretachi_write_terminal({ text: "exit", submit: true })` を試す（シェルプロンプトに戻った状態の時のみ有効）
2. それで終わらない常駐プロセスは `oretachi_kill_terminal` で強制 kill

UI のタブは `pty-exit` イベント経由で自動的に消える。

## 注意点

- `command` 中の改行は oretachi 側で `\r` に正規化され、末尾にも `\r` が保証される（PowerShell/conpty が LF だけだと Enter を発火しないため）
- シェルは oretachi のデフォルト（Windows なら PowerShell、それ以外は OS のシェル）。プラットフォーム固有の構文（PowerShell の `;` と bash の `&&` の違い等）に留意
- 同一ワークツリーに連続して `oretachi_spawn_terminal` を呼んでも、内部的には FIFO キューで管理されるためコマンドは取りこぼされない（先入れ先出しで対応するターミナルに流し込まれる）
- 対象ワークツリーが**サブウィンドウ化（detached）**されている場合、`oretachi_spawn_terminal` は明示的に invalid_params エラーを返す。エラー本文に従ってメインウィンドウに戻すよう促す
- 同名ワークツリーが複数あって `worktree_id` を指定しなかった場合は `invalid_params` エラーが返る。エラー本文に候補 ID が含まれているのでそれを使って再試行する
- `oretachi_read_terminal` の戻り値先頭は、リングバッファ容量超過時にバイト境界調整が走るため UTF-8 マルチバイトや ANSI シーケンスの欠片が落とされる。**末尾側は正確、先頭側は近似**と理解する
