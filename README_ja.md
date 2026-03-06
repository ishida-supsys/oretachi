<div align="center">
  <img src="src-tauri/icons/128x128.png" alt="oretachi アイコン" width="128" />

  # oretachi

  **Git ワークツリーと AI コーディングエージェントのための、マルチセッション・ターミナルマネージャー**

  [English README](README.md)
</div>

---

## 機能一覧

- **マルチターミナル管理** — PTY ベースのターミナルエミュレータ（xterm.js + WebGL）、タブ切り替え・分割ペインに対応
- **Git Worktree 管理** — ワークツリーの作成・削除、複数リポジトリの管理、Git LFS 対応
- **AI 自動承認** — ターミナル出力の承認プロンプトを検出し、AI（Claude）が安全性を判定して自動承認
- **サブウィンドウ** — ワークツリーを独立ウィンドウに分離、終了時の状態を保存し次回起動時に復元
- **通知システム** — MCP・REST API・CLI 経由でワークツリーへ通知を送信、トレイに未読数バッジ表示
- **内蔵 MCP サーバー** — Streamable HTTP による MCP プロトコル対応、AI コーディングエージェントから直接利用可能
- **ホットキー** — `Alt`+キーでワークツリーに即座にフォーカス、キーバインドは自由にカスタマイズ可能
- **IDE 連携** — Cursor・VS Code・Antigravity を自動検出してワークツリーをオープン

## 技術スタック

| レイヤー | 技術 |
|---|---|
| デスクトップフレームワーク | Tauri 2 |
| バックエンド | Rust, portable-pty, axum, rmcp |
| フロントエンド | Vue 3, TypeScript, Vite |
| ターミナル | xterm.js (WebGL レンダラー) |
| UI | Tailwind CSS 4, PrimeVue 4 |

## 必要な環境

- [Node.js](https://nodejs.org/)（LTS 推奨）
- [pnpm](https://pnpm.io/)
- [Rust ツールチェーン](https://rustup.rs/)

## インストール

```bash
# リポジトリをクローン
git clone https://github.com/YOUR_USERNAME/oretachi.git
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
