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

### Step 3: 起動確認 / ステータス確認

`oretachi_list_terminals({ worktree_name: "<name>" })` を呼ぶと、各セッションの以下を含む配列が返る:

- `sessionId` / `cwd` / `isAiAgent` / `worktreeId` / `worktreeName`
- `status`: `"running"` か `"exited"`（シェル本体が終了したか）
- `exitCode`: シェル本体の exit code（`status: "exited"` のときのみ非 null）
- `lastCommandExitCode`: シェル統合が拾った**直近コマンド**の exit code（`pnpm test` の pass/fail 判定などに使える）

直前に追加したセッションが含まれていれば起動成功。シェル本体が `exit` 等で自然終了したセッションは（zombie 化を防ぐため）map から自動的に除去されるので、続けてポーリングすると消える ＝ 終了したと判定できる。**コマンド単位の成否は `lastCommandExitCode` を見るのが基本**で、`pnpm test` のように shell 内で実行したコマンドの結果を観察したい場合に使う。

### Step 4: 出力ログを参照する

dev サーバのコンパイルエラー確認、vitest の結果確認などに使う。

```
// 初回: バッファ末尾から最大 max_bytes バイト
oretachi_read_terminal({
  session_id: <値>,
  max_bytes: 8192   // 省略可。デフォルト 8192。長い場合は 32768 等まで増やす
})

// 連続ポーリング: 前回 cursor 以降の差分のみ取る（推奨）
oretachi_read_terminal({
  session_id: <値>,
  from_cursor: <前回レスポンスの cursor>
})
```

レスポンスは JSON 文字列:

```
{
  "text": "<ANSI 除去済み UTF-8>",
  "cursor": 12345,
  "lostBytes": 0
}
```

- `text`: バッファ内容を ANSI エスケープ除去した UTF-8。
- `cursor`: `text` 末尾の累積バイト位置。次回呼び出しで `from_cursor` に渡すと、それ以降の新規出力だけが返ってくる（重複なし）。
- `lostBytes`: 要求 `from_cursor` がリングバッファ範囲外だった場合の欠落バイト数。`> 0` なら出力が間引かれている（バッファは 64KiB で、それを超える分は古い側から破棄される）。

長期 watch（`pnpm dev` / `vitest --watch`）をポーリングする場合は **必ず `from_cursor` を使う**こと。使わないと毎回末尾 N バイトが返り、AI 側で重複処理が必要になる。

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

#### Ctrl-C / EOF を送る (raw キー送信)

常駐プロセスを graceful に止めたい時は、Ctrl-C を raw 送信する。

```
// Ctrl-C: pnpm dev / vitest --watch 等を graceful 停止
oretachi_write_terminal({
  session_id: <値>,
  text: "",   // 0x03 = ETX
  submit: false
})

// Ctrl-D / EOF: REPL 終了
oretachi_write_terminal({
  session_id: <値>,
  text: "",   // 0x04 = EOT
  submit: false
})
```

停止後シェルプロンプトに戻ったら、続けて `oretachi_write_terminal` で別コマンドを投入できる。常駐プロセスがそれでも止まらない場合のみ次の `oretachi_kill_terminal` で強制 kill する。

### Step 6: 停止する場合

`oretachi_list_terminals` で対象の `session_id` を特定してから:

```
oretachi_kill_terminal({ session_id: <値> })  // 強制 kill
```

graceful 終了は Step 5 の Ctrl-C を優先する。プロセスが Ctrl-C ハンドラを持たない / プロセスがハングしている場合のみ `oretachi_kill_terminal` で強制 kill する。

UI のタブは `pty-exit` イベント経由で自動的に消える。

## 注意点

- `command` 中の改行は oretachi 側で `\r` に正規化され、末尾にも `\r` が保証される（PowerShell/conpty が LF だけだと Enter を発火しないため）
- シェルは oretachi のデフォルト（Windows なら PowerShell、それ以外は OS のシェル）。プラットフォーム固有の構文（PowerShell の `;` と bash の `&&` の違い等）に留意
- 同一ワークツリーに連続して `oretachi_spawn_terminal` を呼んでも、内部的には FIFO キューで管理されるためコマンドは取りこぼされない（先入れ先出しで対応するターミナルに流し込まれる）
- 対象ワークツリーが**サブウィンドウ化（detached）**されている場合、`oretachi_spawn_terminal` は明示的に invalid_params エラーを返す。エラー本文に従ってメインウィンドウに戻すよう促す
- 同名ワークツリーが複数あって `worktree_id` を指定しなかった場合は `invalid_params` エラーが返る。エラー本文に候補 ID が含まれているのでそれを使って再試行する
- `oretachi_read_terminal` の `from_cursor` 未指定モード（末尾 N バイト）と `lostBytes > 0` のときは、UTF-8 / ANSI 境界調整のため先頭が落とされる（**末尾側は正確、先頭側は近似**）。`from_cursor` を渡した連続ポーリングで `lostBytes == 0` のときは、cursor は文字境界済みなので欠落なし
- `lastCommandExitCode` はシェル統合 (bash の `PROMPT_COMMAND` / zsh の `precmd` / pwsh の `prompt`) が出力する OSC 777 を Rust 側 reader が拾う仕組み。シェルプロンプトが新しく描画されるたびに更新される。`pnpm test && echo done` のようなチェーンでは最後のコマンドの結果になる
