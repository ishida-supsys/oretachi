# セットアップスクリプト & 通知設定ガイド

## 概要

oretachi には、**ワークツリー追加時にセットアップスクリプトを自動実行する機能**と、**AI エージェントの処理完了などをフロントエンドへ通知する機能**があります。

- **セットアップスクリプト**: ワークツリーを追加するたびに共通の初期化処理（設定ファイルのコピー、依存パッケージのインストールなど）を自動化できます。
- **通知**: AI エージェントが MCP ツール・HTTP API・CLI のいずれかを使って oretachi に通知を送信し、OS 通知やアプリ内バッジとして表示させることができます。

---

## 1. セットアップスクリプトの設定

### 設定方法

1. oretachi の **設定画面** を開く
2. **リポジトリ** セクションで対象リポジトリを確認する
3. 「実行スクリプト」欄の **選択** ボタンをクリックし、`.ps1`（PowerShell）または `.sh` ファイルを指定する

設定したスクリプトは、そのリポジトリ配下のワークツリーを追加するたびに毎回実行されます。

### 実行タイミング

ワークツリー追加後、ターミナルが起動した直後に対象のターミナルでスクリプトが実行されます。
タスク実行の場合は、スクリプトの完了を待ってから AI エージェントの起動に進みます。

### 環境変数

スクリプト実行時に以下の環境変数が自動的にセットされます。

| 変数名 | 内容 |
|---|---|
| `ORETACHI_REPO_NAME` | リポジトリ名（設定画面で登録した名前） |
| `ORETACHI_WORKTREE_NAME` | 追加したワークツリーの名前 |

### シェル別の内部実行コマンド（参考）

oretachi はシェルの種類を自動判定し、以下の形式でスクリプトを呼び出します。

| シェル | 実行形式 |
|---|---|
| PowerShell / pwsh | `$env:ORETACHI_REPO_NAME="..."; $env:ORETACHI_WORKTREE_NAME="..."; Set-ExecutionPolicy -Scope Process Bypass; & "<path>"` |
| CMD | `set ORETACHI_REPO_NAME=...&& set ORETACHI_WORKTREE_NAME=...&& call "<path>"` |
| sh / bash / zsh | `ORETACHI_REPO_NAME="..." ORETACHI_WORKTREE_NAME="..." sh "<path>"` |

---

## 2. セットアップスクリプトのサンプル

### PowerShell 版

メインリポジトリから共有設定ファイルをワークツリーにコピーするサンプルです。

```powershell
# worktree-setup.ps1
#
# ORETACHI_REPO_NAME    : リポジトリ名
# ORETACHI_WORKTREE_NAME: このワークツリーの名前

param()

$ErrorActionPreference = "Stop"

Write-Output "[setup] START: $env:ORETACHI_WORKTREE_NAME"

# メインリポジトリのルートを取得
$gitCommonDir = git rev-parse --git-common-dir 2>&1
if (-not $gitCommonDir -or $gitCommonDir -match "^fatal") {
    Write-Output "[setup] ERROR: git repository not found"
    exit 1
}

if ([System.IO.Path]::IsPathRooted($gitCommonDir)) {
    $repoRoot = Split-Path -Parent $gitCommonDir
} else {
    $repoRoot = git rev-parse --show-toplevel 2>&1
}

$worktreeRoot = git rev-parse --show-toplevel 2>&1
Write-Output "[setup] repoRoot   : $repoRoot"
Write-Output "[setup] worktreeRoot: $worktreeRoot"

# コピーしたいファイルのリスト（リポジトリルートからの相対パス）
$files = @(
    ".env.local",
    "config/local.json"
)

foreach ($rel in $files) {
    $src = Join-Path $repoRoot $rel
    $dst = Join-Path $worktreeRoot $rel

    if (Test-Path $src) {
        $dstDir = Split-Path -Parent $dst
        if (-not (Test-Path $dstDir)) {
            New-Item -ItemType Directory -Path $dstDir | Out-Null
        }
        Copy-Item -Path $src -Destination $dst -Force
        Write-Output "[setup] Copied: $rel"
    } else {
        Write-Output "[setup] SKIP (not found): $rel"
    }
}

Write-Output "[setup] DONE"
```

### sh 版

```sh
#!/bin/sh
# worktree-setup.sh
#
# ORETACHI_REPO_NAME     : リポジトリ名
# ORETACHI_WORKTREE_NAME : このワークツリーの名前

set -e

echo "[setup] START: ${ORETACHI_WORKTREE_NAME}"

# メインリポジトリのルートを取得
git_common_dir=$(git rev-parse --git-common-dir 2>&1)
if [ -z "$git_common_dir" ]; then
    echo "[setup] ERROR: git repository not found"
    exit 1
fi

case "$git_common_dir" in
    /*) repo_root=$(dirname "$git_common_dir") ;;
    *)  repo_root=$(git rev-parse --show-toplevel) ;;
esac

worktree_root=$(git rev-parse --show-toplevel)
echo "[setup] repoRoot    : ${repo_root}"
echo "[setup] worktreeRoot: ${worktree_root}"

# コピーしたいファイルのリスト（リポジトリルートからの相対パス）
files=".env.local config/local.json"

for rel in $files; do
    src="${repo_root}/${rel}"
    dst="${worktree_root}/${rel}"

    if [ -f "$src" ]; then
        dst_dir=$(dirname "$dst")
        mkdir -p "$dst_dir"
        cp "$src" "$dst"
        echo "[setup] Copied: ${rel}"
    else
        echo "[setup] SKIP (not found): ${rel}"
    fi
done

echo "[setup] DONE"
```

---

## 3. 通知の設定

### OS 通知の有効化

1. oretachi の **設定画面** を開く
2. **「OS 通知を有効にする」** をオンにする
3. OS 側から通知の許可を求めるダイアログが表示された場合は許可する

### 通知の仕組み

AI エージェントや外部スクリプトから oretachi へ通知を送る方法は 3 つあります。

#### 方法 1: CLI（`oretachi.exe --notify`）

oretachi の実行ファイルに `--notify` オプションを渡すことで、実行中の oretachi に通知を送信できます。
セットアップスクリプトの末尾からでも利用可能です。

```
oretachi.exe --notify <ワークツリー名>

# 短縮形
oretachi.exe -n <ワークツリー名>
```

#### 方法 2: HTTP API（`POST /notify`）

oretachi が起動中に公開するローカルの HTTP サーバーに POST リクエストを送ります。

```sh
curl -s -X POST "http://127.0.0.1:<PORT>/notify" \
  -H "Content-Type: application/json" \
  -d "{\"worktree\": \"${ORETACHI_WORKTREE_NAME}\"}"
```

```powershell
Invoke-RestMethod -Uri "http://127.0.0.1:<PORT>/notify" `
  -Method Post `
  -ContentType "application/json" `
  -Body "{`"worktree`": `"$env:ORETACHI_WORKTREE_NAME`"}"
```

#### 方法 3: MCP ツール（`notify_worktree`）

Claude Code などの MCP クライアントから `notify_worktree` ツールを呼び出します。AI エージェントがタスク完了時に自分から通知を送る場合に利用します。

```json
{
  "tool": "notify_worktree",
  "parameters": { "worktree_name": "<ワークツリー名>" }
}
```

---

## 4. 応用例: スクリプト完了後に通知を送る

セットアップスクリプトの末尾で CLI を使って通知を送ることで、初期化が完了したことを oretachi 経由で知らせることができます。

```powershell
# worktree-setup.ps1 の末尾に追記

# --- セットアップ処理（省略） ---

# oretachi へ通知を送る
# $oretachiBin にはインストール先の実行ファイルパスを指定する
$oretachiBin = "$env:LOCALAPPDATA\oretachi\oretachi.exe"
if (Test-Path $oretachiBin) {
    & $oretachiBin --notify $env:ORETACHI_WORKTREE_NAME
    Write-Output "[setup] Notification sent"
} else {
    Write-Output "[setup] oretachi.exe not found, skipping notification"
}
```

```sh
#!/bin/sh
# worktree-setup.sh の末尾に追記

# --- セットアップ処理（省略） ---

# oretachi へ通知を送る
ORETACHI_BIN="${HOME}/.local/bin/oretachi"
if [ -x "$ORETACHI_BIN" ]; then
    "$ORETACHI_BIN" --notify "${ORETACHI_WORKTREE_NAME}"
    echo "[setup] Notification sent"
else
    echo "[setup] oretachi not found, skipping notification"
fi
```
