# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.15.0] - 2026-03-26

### Added
- 設定ビューに言語・MCPサーバー・ウィンドウ・通知音の各オプションを追加
- アプリケーション設定管理（Rustバックエンド + Vue UIコンポーネント）を実装
- ターミナルエミュレーション・包括的なGit連携・AIエージェント管理を実装
- AI駆動タスク生成によるワークツリー管理（MCP連携・構造化出力）を追加
- ホームビューとCatターミナルコンポーネントによるAIエージェントインタラクションを導入

### Fixed
- ボタンとホットキートグルのスタイルを改善

## [0.14.0] - 2026-03-25

### Added
- ホームビューにマスonry レイアウトとドラッグ&ドロップによるワークツリー並べ替えを実装
- タスク/ワークツリーパネル切り替え機能を追加
- `useMasonryLayout` コンポジャブルを追加
- MCP サーバーのグレースフルシャットダウンと再起動準備を実装

### Fixed
- アプリ終了処理とシャットダウン UI を改善
- MCP サーバーシャットダウンタイムアウト時の再起動失敗処理を追加
- MCP サーバー再起動の安定性を改善

## [0.13.0] - 2026-03-25

### Added
- ワークツリーカードのドラッグ&ドロップ並べ替え機能
- ワークツリー管理・ターミナルビュー・設定・タスク実行の初期アプリケーション構造

### Fixed
- 保存順序にないワークツリーの復元時の保持
- Windows PATH 環境変数の展開と IDE 選択の改善

## [0.12.2] - 2026-03-25

### Fixed
- Implement various security hardening measures

## [0.12.1] - 2026-03-25

### Fixed
- Buffer PTY output until session activation to prevent data loss on startup
- Consolidate PTY session setup with per-sessionId buffers
- Offload blocking I/O operations to thread pool for improved responsiveness
- Execute absolute command paths directly on Windows
- Resolve AI agent command paths
- Add concurrency controls and async I/O improvements
- Add generation counter and serialize task execution to prevent race conditions
- Update MCP server status on initialization errors

## [0.12.0] - 2026-03-24

### Added
- Implement task executor to generate AI-driven task plans for worktree operations based on user prompts and system state

### Fixed
- Position gaming border fixed to viewport to remain visible and static relative to the viewport when page content scrolls

[Unreleased]: https://github.com/ishida-supsys/oretachi/compare/0.15.0...HEAD
[0.15.0]: https://github.com/ishida-supsys/oretachi/compare/0.14.0...0.15.0
[0.14.0]: https://github.com/ishida-supsys/oretachi/compare/0.13.0...0.14.0
[0.13.0]: https://github.com/ishida-supsys/oretachi/compare/0.12.2...0.13.0
[0.12.2]: https://github.com/ishida-supsys/oretachi/compare/0.12.1...0.12.2
[0.12.1]: https://github.com/ishida-supsys/oretachi/compare/0.12.0...0.12.1
[0.12.0]: https://github.com/ishida-supsys/oretachi/releases/tag/0.12.0
