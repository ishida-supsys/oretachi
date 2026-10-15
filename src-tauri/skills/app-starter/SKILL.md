---
name: app-starter
description: 権限を申請して動く「アプリ形式」のアーティファクト（manifest 付きの React アーティファクト）を作る。ユーザーが「アプリとして作って」「管理画面を常駐させたい」「ボタンから MCP ツールを呼ぶ常駐ツールが欲しい」等と言ったとき、または同梱の最小アプリで生成から起動までを試したいときに使う。
allowed-tools: mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__oretachi_set_description, Read
---

# app-starter スキル

React アーティファクトに **manifest（`app`）** を付けると、権限を申請して動く「アプリ」になる。
このスキルは同梱の最小アプリ（通知の一覧と、テスト通知の送信）を生成する雛形で、生成物は
そのまま自分のアプリへ書き換えて使う。

## アプリの性質（先に知っておくこと）

- **manifest だけでは何も動かない。** AI が書いた manifest は信頼されない。ユーザーがアーティファクトウィンドウの**起動画面**で起動し、承認ダイアログで許可して初めて権限が有効になる。承認の前に、あなたが API を呼べるようにする手段は無い。
- **宣言したものだけが使える。** manifest に無い権限（ツール・越境先・メモリー）は、承認済みでも常に拒否される。
- **通常のビューアでは動かない。** `app` 付きのアーティファクトは起動画面を出す。実行は承認後の常駐ウィンドウの中（`sandbox` iframe）。Web ビューアでは起動できない。
- 今の版で書ける権限は `mcp`（`tools` と `targets`）と `memory`（`maxBytes` 必須）だけ。`fs` / `db` / `network` / `subWindow` / `wasm` / `aiTerminal` は未対応で、書くとエラーになる。
- コード（content / modules）を更新しても再承認は要らない。**権限（manifest の `permissions`）を変えたときだけ**、起動中なら直ちに再承認のダイアログが出る。承認までは旧権限のまま動き、新しい権限が要る呼び出しは拒否される。

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
    "memory": { "maxBytes": 65536 }
  }
}
```

- `mcp.tools`: 呼びたいツールだけを書く。呼べるのは `artifact` ツールの説明にある一覧の部分集合。`oretachi_write_terminal` / `oretachi_send_keys` / `oretachi_answer_prompt` / `oretachi_add_task` は**ユーザーと同等の権限（任意コマンド実行）**として、承認ダイアログで強調される。本当に必要なときだけ宣言する。
- `mcp.targets`: 他のワークツリーへ届かせたいときだけ書く（省略 = 自ワークツリーのみ）。ワークツリー名 / ID、`*`、`workgroup:<ID か名前>`、`repo:<リポジトリ名>`。**ワイルドカードは最も強い警告になる**（今後作られるワークツリーも含み、メインのクローンの端末にも届く）。名前で書いたワークツリーは承認時に ID へ固定される。越境には、これに加えて既存の購読（`oretachi_subscribe_worktree`）も必要。
- `memory.maxBytes`: メモリーを使うなら必須。メモリー API 本体はまだ無い（後続）。

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
