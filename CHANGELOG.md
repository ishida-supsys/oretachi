# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.17.8] - 2026-04-08

### Added
- Move artifact count from button badge to inline indicator in header

### Fixed
- Split bare-repo-risk note into its own bullet in ai_judge prompt
- Allow cd-to-CWD compound git commands in auto-approval
- Change Codex CLI default model to gpt-5.4-mini for ChatGPT account compatibility

## [0.17.7] - 2026-04-07

### Added
- Enable copy/paste via mouse in terminal

### Fixed
- Add --skip-git-repo-check to Codex CLI invocation
- Use capture phase for mousedown to get selection before xterm.js clears it

## [0.17.6] - 2026-04-07

### Added
- Display artifact count indicators on worktree card and header
- Optimize worktree rendering and task synchronization

### Fixed
- Prevent app freeze when close button is clicked on Mac
- Update CodexCli invocation to use exec subcommand
- Use withDefaults to correct boolean prop casting for traffic light buttons

## [0.17.5] - 2026-04-03

### Fixed
- Support image paste in Claude Code terminal
- Restore Ctrl+V paste handler with double-paste prevention
- Improve async stability by moving blocking I/O to spawn_blocking and adding timeouts for network/lock operations
- Use consecutive timeout counter to detect dead TCP half-open connections in MCP broadcast
- Recover from poisoned mutex in MCP server to keep timeout counts working
- Validate audio file extension and reject path traversal in copy_custom_sound

## [0.17.4] - 2026-04-02

### Added
- Disable right-click context menu on all windows

### Changed
- Remove manual Ctrl+V paste handler in terminal

## [0.17.3] - 2026-04-01

### Added
- Add macOS-style traffic light window controls

### Fixed
- Fix project list settings being empty

## [0.17.2] - 2026-03-31

### Added
- Add macOS support for system sounds
- Improve task tooltip to show full content with smaller font

### Fixed
- Restore worktree when branch deletion fails due to not fully merged

## [0.17.1] - 2026-03-29

### Added
- Improve MCP API key display with PrimeVue Password component

### Fixed
- Always merge well-known paths regardless of login shell success

## [0.17.0] - 2026-03-29

### Added
- Show worktree task details in tray popup tooltip
- Add copy feedback for API key field
- Add remote access toggle for MCP server in settings
- Implement API key authentication for MCP server
- Integrate PrimeVue tooltip for worktree task details
- Add worktree duplicate feature
- Inherit Claude Code session when creating new worktree

### Fixed
- Adjust remote access text and toggle label style

## [0.16.1] - 2026-03-28

### Added
- PTY管理システムにAIエージェントプロセス検出・監視機能を実装
- Tauriバックエンドのコア設定・PTY管理・ライブラリモジュールを実装

### Fixed
- xterm.js Terminal初期化時の未定義cols/rowsをスキップ
- macOSでログインシェルからPATHを補完してAIエージェントを検出

## [0.16.0] - 2026-03-28

### Added
- Notify MCP clients when a worktree is added
- Add commit file and diff viewer for git history
- Make AI timeout configurable from settings tab
- Add worktree archive functionality
- Broadcast active tasks to all windows for real-time tooltip sync
- Add task tooltips to worktree headers
- Implement task persistence, search, and infinite scroll

### Fixed
- Prevent double destroy race and re-entry on tray popup close
- Synchronize task offset with database persistence
- Refine task loading, updating, and optimistic removal

### Changed
- Split task persistence and search into useTaskPersistence composable
- Add inter-window task data synchronization

## [0.15.1] - 2026-03-27

### Fixed
- Catターミナルの描画安定性を改善

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

[Unreleased]: https://github.com/ishida-supsys/oretachi/compare/0.17.8...HEAD
[0.17.8]: https://github.com/ishida-supsys/oretachi/compare/0.17.7...0.17.8
[0.17.7]: https://github.com/ishida-supsys/oretachi/compare/0.17.6...0.17.7
[0.17.6]: https://github.com/ishida-supsys/oretachi/compare/0.17.5...0.17.6
[0.17.5]: https://github.com/ishida-supsys/oretachi/compare/0.17.4...0.17.5
[0.17.4]: https://github.com/ishida-supsys/oretachi/compare/0.17.3...0.17.4
[0.17.3]: https://github.com/ishida-supsys/oretachi/compare/0.17.2...0.17.3
[0.17.2]: https://github.com/ishida-supsys/oretachi/compare/0.17.1...0.17.2
[0.17.1]: https://github.com/ishida-supsys/oretachi/compare/0.17.0...0.17.1
[0.17.0]: https://github.com/ishida-supsys/oretachi/compare/0.16.1...0.17.0
[0.16.1]: https://github.com/ishida-supsys/oretachi/compare/0.16.0...0.16.1
[0.16.0]: https://github.com/ishida-supsys/oretachi/compare/0.15.1...0.16.0
[0.15.1]: https://github.com/ishida-supsys/oretachi/compare/0.15.0...0.15.1
[0.15.0]: https://github.com/ishida-supsys/oretachi/compare/0.14.0...0.15.0
[0.14.0]: https://github.com/ishida-supsys/oretachi/compare/0.13.0...0.14.0
[0.13.0]: https://github.com/ishida-supsys/oretachi/compare/0.12.2...0.13.0
[0.12.2]: https://github.com/ishida-supsys/oretachi/compare/0.12.1...0.12.2
[0.12.1]: https://github.com/ishida-supsys/oretachi/compare/0.12.0...0.12.1
[0.12.0]: https://github.com/ishida-supsys/oretachi/releases/tag/0.12.0
