<div align="center">
  <img src="src-tauri/icons/128x128.png" alt="oretachi アイコン" width="128" />

  # oretachi

  **Git ワークツリーと AI コーディングエージェントのための、マルチセッション・ターミナルマネージャー**

  <img width="1915" height="908" alt="Image" src="https://github.com/user-attachments/assets/13f3152b-f087-4dff-82c3-7898398e74b1" />

  [English README](README.md)
</div>

---

## 機能一覧

- **マルチターミナル管理** — PTY ベースのターミナルエミュレータ、タブ切り替え・分割ペイン（縦横リサイズ対応）、ターミナル内検索
- **Git Worktree 管理** — ワークツリーの作成・削除、複数リポジトリの管理
- **タスク実行** — Issue/PR の URL や自由記述からワークツリーを自動生成し、AI エージェントでタスクを並列実行
- **AI 自動承認** — ターミナル出力の承認プロンプトを検出し、AIが安全性を判定して自動承認
- **AI エージェント対応** — Claude Code・Gemini CLI・Codex CLI・Cline CLI を自動検出、プロセスツリー解析で AI セッションを識別
- **コードレビューワー** — 内蔵のDiffビューワー、ファイルツリー・コミット履歴表示
- **サブウィンドウ** — ワークツリーを独立ウィンドウに分離
- **通知システム** — MCP・REST API 経由でワークツリーへ通知を送信、トレイに未読数バッジ表示
- **内蔵 MCP サーバー** — Streamable HTTP による MCP プロトコル対応、リポジトリ・ワークツリー情報の提供、AI エージェントから直接利用可能
- **ホットキー** — `Alt`+キーでワークツリーに即座にフォーカス、ワークツリー追加時の自動割り当て、キーバインドは自由にカスタマイズ可能
- **IDE 連携** — Cursor・VS Code・Antigravity を自動検出、内蔵 CodeReviewer も選択可能

### タスク実行

IssueやPullRequestのURLや具体的な修正内容等のタスクを送信すると、自動でワークツリーを生成して対応を開始します。

https://github.com/user-attachments/assets/be44a731-a25e-41e6-ac03-502bf7a651eb

### トレイポップアップ

通知を受信したワークツリーをトレイポップアップで順番に確認することができます。

https://github.com/user-attachments/assets/23a0e4b5-6586-41ed-8d30-af0e4b641a36

### コードビューワー

IDEの選択にCodeReviewerを選択すると、内蔵のビューワーが起動し、レビューを実施することができます。

<img width="1200" height="834" alt="Image" src="https://github.com/user-attachments/assets/b6c53230-6675-4ff4-ba6d-8f78a85fbcc9" />

## インストール

Releaseタブからインストーラーをダウンロードしてインストールしてください。

手動ビルドする場合：

```bash
# リポジトリをクローン
git clone https://github.com/ishida-supsys/oretachi.git
cd oretachi

# フロントエンド依存関係のインストール
pnpm install

# ビルド
pnpm tauri build
```

ビルドされたインストーラーは `src-tauri/target/release/bundle/` に生成されます。

## 開発

```bash
pnpm tauri dev
```

ログはプラットフォーム固有のアプリログディレクトリに出力されます。

## ライセンス

MIT — 詳細は [LICENSE](LICENSE) を参照してください。
