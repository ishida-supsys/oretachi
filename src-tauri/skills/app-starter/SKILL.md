---
name: app-starter
description: 権限を申請して動く「アプリ形式」のアーティファクト（manifest 付きの React アーティファクト）を作る。ユーザーが「アプリとして作って」「管理画面を常駐させたい」「ボタンから MCP ツールを呼ぶ常駐ツールが欲しい」等と言ったとき、または同梱の最小アプリで生成から起動までを試したいときに使う。
allowed-tools: mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__artifact_file, mcp__plugin_oretachi_oretachi__oretachi_set_description, Read
---

# app-starter スキル

React アーティファクトに **manifest（`app`）** を付けると、権限を申請して動く「アプリ」になる。
このスキルは同梱の最小アプリ（通知の一覧と、テスト通知の送信）を生成する雛形で、生成物は
そのまま自分のアプリへ書き換えて使う。

## アプリの性質（先に知っておくこと）

- **manifest だけでは何も動かない。** AI が書いた manifest は信頼されない。ユーザーがアーティファクトウィンドウの**起動画面**で起動し、承認ダイアログで許可して初めて権限が有効になる。承認の前に、あなたが API を呼べるようにする手段は無い。
- **宣言したものだけが使える。** manifest に無い権限（ツール・越境先・メモリー）は、承認済みでも常に拒否される。
- **通常のビューアでは動かない。** `app` 付きのアーティファクトは起動画面を出す。実行は承認後の常駐ウィンドウの中（`sandbox` iframe）。Web ビューアでは起動できない。
- 今の版で書ける権限は `mcp`（`tools` と `targets`）、`memory`（`maxBytes` 必須）、`fs`、`db`（下記）だけ。`network` / `subWindow` / `wasm` / `aiTerminal` は未対応で、書くとエラーになる。
- コード（content / modules）を更新しても再承認は要らず、起動中なら**承認を保ったまま iframe がリロードされる**（フルリロード。メモリー以外の JS の状態は失われる）。**権限（manifest の `permissions`）を変えたときだけ**、起動中なら直ちに再承認のダイアログが出る。承認までは旧権限のまま動き（リロードもされない）、新しい権限が要る呼び出しは拒否される。承認でリロード、拒否で停止。

## manifest

`<SKILL_DIR>/app.json` が雛形。

```json
{
  "name": "表示名",
  "description": "説明",
  "version": "1.0.0",
  "permissions": {
    "mcp": {
      "tools": ["oretachi_list_worktree_notifications", "notify_worktree"],
      "targets": []
    },
    "memory": { "maxBytes": 65536 },
    "fs": {
      "read": ["worktree://docs/", "artifact://"],
      "write": ["worktree://out/", "artifact://data/"]
    }
  }
}
```

- `mcp.tools`: 呼びたいツールだけを書く。呼べるのは `artifact` ツールの説明にある一覧の部分集合。`oretachi_write_terminal` / `oretachi_send_keys` / `oretachi_answer_prompt` / `oretachi_add_task` は**ユーザーと同等の権限（任意コマンド実行）**として、承認ダイアログで強調される。本当に必要なときだけ宣言する。
- `mcp.targets`: 他のワークツリーへ届かせたいときだけ書く（省略 = 自ワークツリーのみ）。ワークツリー名 / ID、`*`、`workgroup:<ID か名前>`、`repo:<リポジトリ名>`。**ワイルドカードは最も強い警告になる**（今後作られるワークツリーも含み、メインのクローンの端末にも届く）。名前で書いたワークツリーは承認時に ID へ固定される。越境には、これに加えて既存の購読（`oretachi_subscribe_worktree`）も必要。
- `memory.maxBytes`: メモリーを使うなら必須（1〜1048576 バイト。書き込み後の JSON のバイト数で数える）。宣言が無い・0 以下・上限超過の manifest は拒否される。承認ダイアログに最大サイズが出る。上限を下げる変更も再承認になり、既存データが新しい上限を超えていても消えない（収まらない書き込みだけ拒否、読み取りは可）。

- `fs`: ファイルの読み書き。`read` / `write` に**ディレクトリのプレフィックス**を `worktree://<相対>` / `artifact://<相対>` で並べる（`worktree://` だけ = ワークツリー全体）。**`write` は `read` を含意しない**（書いて読み返すなら両方に書く）。生の絶対パス・`..`・`C:` や `:` を含む名前・Windows の予約名（`CON` `NUL` `COM1` …）・末尾のドット/空白は manifest ごと拒否される。承認ダイアログには**解決済みの絶対パス**と読み書きの別が出る。書き込みは「あなたと同等の権限」寄りの扱いなので、必要な範囲だけ宣言する。
  - `worktree://` はこのアプリを置いたワークツリーの中だけ。ホーム / リポジトリのタブに置いたアプリは使えない。
  - `artifact://` はこのアプリ専用の保存領域（`app_data/artifact-files/<ワークツリー>/<アプリ id>/`）。**アーティファクトを削除すると一緒に消える。転送・エクスポート・インポートでは持ち出されない**（移した先で `artifact_file put` し直す）。
  - **既定で書き込みを拒否するパス**（`worktree://` の任意の階層。大文字小文字は無視）: `.git` / `.github` / `.claude` / `.vscode` / `.idea` / `.husky` / `.devcontainer` / `.mcp.json` / `.envrc` / `.codex` / `.gemini` / `.cursor` / `.githooks` / `.gitmodules` / `.cargo` / `.npmrc` / `.yarnrc.yml` / `.pre-commit-config.yaml` / `CLAUDE.md` / `CLAUDE.local.md` / `AGENTS.md`。ここへ書けると、あなたの権限で後からコマンドが走る（git フック・Claude Code のフック・CI・エディタのタスク）。**読み取りは絞り込まれない**（範囲内の `.env` や `.git/config` も読める。`read` は必要な範囲だけにする）。これらを `write` に含む manifest は、**`"allowSensitivePaths": true` を足さない限りエラー**になる。足した manifest は承認ダイアログで**最も強い警告**が出る。本当に必要でない限り足さない。
  - リンク（junction・symlink）の先が範囲外なら拒否される。上限: 1 回の読み書き 8MiB、一覧 1000 件。

- `db`: SQLite のデータベース。形は `fs` と同じ（`read` / `write` に `worktree://…` / `artifact://…` のディレクトリのプレフィックス、`allowSensitivePaths`）で、**DB ファイルを置ける場所**を決める。承認ダイアログには解決済みの絶対パスが出る。`fs` の申請とは独立で、`db` を申請しても `fs` の権限にはならない（逆も同じ）。
  - `write` のスコープの DB は読み書き・作成ができる。`read` のスコープの DB は**読み取り専用で開く**（新規作成も書き込みもできない。`query` で `INSERT … RETURNING` を書いても拒否される）。書いて読み返すなら `write` だけでよい（DB では `write` が読み取りを含む）。
  - **既定拒否パス（上の一覧）は、DB では `read` でも使えない**（DB の副産物の `-journal` が同じ場所に出るため）。含む manifest は `allowSensitivePaths: true` を足さない限りエラー。
  - 同じ DB へ複数のアプリが書くのは避ける（ロックの競合で `database is locked` になりうる）。`artifact://` に専用の DB を置くのが無難。

## データベース API（`db`）

`require('oretachi')` の `db` が使える。パスは `fs` と同じ形。DB ファイルが無ければ、`write` のスコープなら作られる。

```js
import { db } from 'oretachi';
const P = 'artifact://app.db';
await db.execute(P, 'CREATE TABLE IF NOT EXISTS todo (id INTEGER PRIMARY KEY, title TEXT, done INTEGER)');
const { rowsAffected, lastInsertRowid } = await db.execute(P, 'INSERT INTO todo (title, done) VALUES (?, ?)', ['牛乳', 0]);
const { columns, rows } = await db.query(P, 'SELECT id, title FROM todo WHERE done = ?', [0]);   // rows は行ごとの配列
await db.close(P);   // 任意。接続は host が持ち、停止まで開いたままでもよい
```

- パラメータは `null` / 真偽値（0・1 になる）/ 数値 / 文字列 / `{ $blob: '<base64>' }`。BLOB は結果でも `{ $blob: '<base64>' }`。**2^53 を超える整数は JSON の数値では正確に扱えない**（TEXT で持つ）。
- 接続は 1 つのアプリ・1 つの DB につき 1 本なので、`BEGIN` と `COMMIT` を別の呼び出しに分けても同じ接続で実行される（途中で例外になったら `ROLLBACK` すること）。接続はリロードをまたいで残り、アプリの停止で閉じる。
- **使えない SQL**（拒否は Promise の reject と、起動画面のデバッグログ）: `ATTACH DATABASE`（空文字列の一時 DB を除く）、`VACUUM INTO`、`load_extension()`、`PRAGMA temp_store_directory` / `data_store_directory`。通常の `VACUUM` は使える。
- 上限: 1 回の実行 5 秒（超えると中断）、`query` の結果は 10,000 行・約 8MiB まで、同時に開ける DB は 1 アプリ 8 個まで。結果が 0 行のときは `columns` が空になる。
- **DB の中身は信頼しない入力として扱う**（`artifact://` の DB は `artifact_file put` で配れるし、`worktree://` の DB は AI や他のツールが書ける）。

## ファイル API（`fs`）

`require('oretachi')` の `fs` が使える。パスは `worktree://…` / `artifact://…` だけ。拒否・失敗は Promise の reject（理由はエラーメッセージ）で、起動画面のデバッグログにも残る。

```js
import { fs } from 'oretachi';
const text = await fs.readText('worktree://docs/readme.md');
const bytes = await fs.readBytes('artifact://engine.wasm');          // Uint8Array
await fs.writeText('worktree://out/result.json', JSON.stringify(x), { createDirs: true });
await fs.writeBytes('artifact://data/cache.bin', bytes, { createDirs: true });
const { entries, truncated } = await fs.list('worktree://docs/');    // 直下だけ。{ name, kind: 'file'|'dir'|'other', size, modifiedMs }
await fs.remove('artifact://data/cache.bin');                         // ファイルだけ（ディレクトリは不可）
```

- 書き込みは一時ファイル経由の置き換えで、途中で落ちても壊れたファイルは残らない。`createDirs` を付けないと、親ディレクトリが無いときは失敗する。
- **ファイルの中身は信頼しない入力として扱う**（ワークツリーのファイルは AI や他のツールが書ける）。

## 同梱バイナリを `artifact://` へ置く（`artifact_file`）

wasm や初期 DB のようなバイナリは、`artifact` / `artifact_module` の `file_path`（UTF-8 テキスト 1MB まで）では渡せない。`artifact_file` でスキル同梱のファイルを `artifact://` へコピーする（1 ファイル 32MB まで）。コピー元に読めるのは、ワークツリー追加先ディレクトリ配下と oretachi プラグインディレクトリ配下だけ。

```
artifact_file(command: "put", id: "<アプリ id>", project_dir: <作業ディレクトリ>,
  path: "artifact://engine.wasm", file_path: "<SKILL_DIR>/assets/engine.wasm")
artifact_file(command: "list", id: "<アプリ id>", project_dir: <作業ディレクトリ>)
artifact_file(command: "delete", id: "<アプリ id>", project_dir: <作業ディレクトリ>, path: "artifact://engine.wasm")
```

- アプリのアーティファクトが先に存在している必要がある（`create` の後に `put`）。承認は要らない（あなたはもともとユーザーと同等の権限を持つ）。
- アプリ側から読むには、manifest の `fs.read` に `artifact://` を宣言しておく。DB として開くなら `db` に `artifact://` を宣言する（初期 DB を `put` しておき、`write` のスコープで開けば読み書きできる）。

## メモリー API

`require('oretachi')` の `useMemory` / `getMemory` / `setMemory` / `setMemoryKey` / `clearMemory` / `subscribeMemory` が使える（通常の React アーティファクトと同じ形）。保存先は host 側のストックで、停止・アプリ再起動・ウィンドウを閉じる・リロードをまたいで残る。コードを更新してリロードされても、直前に書いた値（debounce 中のものも）は新しい iframe に引き継がれる。

- 書き込みは host が毎回サイズを検査し、`maxBytes` を超えると Promise が reject される（デバッグログにも残る）。`memory` を宣言していないアプリは、読み書きとも拒否される（初期値は常に空）。
- AI セッションからの `artifact_store` の write / delete も、同じ `maxBytes` が強制される。書き込まれた値は起動中のアプリへ即座に反映される。**メモリーの中身は信頼しない入力として扱うこと**（AI や他の経路が書ける。`dangerouslySetInnerHTML` や eval に渡さない）。

## 手順

1. `oretachi_set_description` でこのワークツリーの目的を 1 行セットする。
2. 同梱の最小アプリを生成する。`<SKILL_DIR>` は「Base directory for this skill」に出ているパス。ファイルを Read して書き戻さず、`file_path` / `app_file_path` で渡す:

   ```
   artifact(command: "create", id: "<アプリ id>", project_dir: <作業ディレクトリ>,
     type: "application/vnd.ant.react", title: "<タイトル>",
     file_path: "<SKILL_DIR>/templates/entry-point.jsx",
     app_file_path: "<SKILL_DIR>/app.json")
   ```

   自分のアプリにするときは `entry-point.jsx` を元に `content` を書き、`app_file_path` の代わりに `app`（manifest の JSON）を直接渡してもよい。
3. 生成したら、ユーザーに「アーティファクトウィンドウの起動画面で **起動** を押し、承認ダイアログで権限を確認して許可してください」と伝える。**あなたが起動することはできない。**
4. 動作確認はユーザーに頼む。起動画面のデバッグログに、許可・拒否された呼び出しが残る。

## 更新するとき

- **コードだけ**: `artifact(command: "update" / "rewrite")` や `artifact_module` で更新する。再承認は要らない。
- **権限を変える**: `artifact(command: "rewrite", app_file_path or app: …)` で manifest を書き換える。起動中なら再承認のダイアログがユーザーに出る。権限が狭まる変更でも出る。
- **このスキルが更新されても、既存のアプリは自動では変わらない。** 新しい版にしたいときは、同じ id へもう一度 `create` する。
- manifest は外せない。別の種類に作り直したいときは、先に `delete` して別 id で作る。
