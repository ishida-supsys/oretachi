# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.21.1] - 2026-06-10

### Fixed
- リリースCIを復旧: App.vue で未定義の `debug()` を `logDebug()` に修正し、`vue-tsc --noEmit` の TS2304 ビルド失敗を解消 (#64)

## [0.21.0] - 2026-06-10

### Added
- ワークツリーの description をカードホバー時に展開表示するエリアを追加 (旧ツールチップ表示から変更)
- ExitPlanMode フックでプランを要約し description に自動セットする機能を追加
- description をアーカイブにも保存するよう拡張 (正本は settings.json を維持)

### Changed
- MCP 通知を独立サイドカー `oretachi-notify` に分離 (#63)

### Fixed
- description エリアをヘッダー直下に移動し、上下分離アニメーションに変更
- ExitPlanMode フックを PostToolUse から PermissionRequest に変更
- description の永続化漏れなど bug-review 指摘事項を修正

## [0.20.3] - 2026-06-08

### Fixed
- ハングの根本原因となっていた webview 発ロギング IPC を源流から除去 (#59)
- ログ送出を非 Promise 戻り値に対して耐性化 (bug-review 指摘)
- plugin level 変更を revert し、webview verbose ログを Debug Mode に連動させるよう修正

### Documentation
- 現行アップデートに合わせて README の機能一覧を更新

## [0.20.2] - 2026-06-05

### Added
- ホーム画面のワークツリーカードリストを中央寄せに変更 (余った横スペースを左右均等に配分)
- ワークツリーカードの1行ターミナル表示を最大2つに制限 (3個目以降は flex-wrap で折り返し)

### Fixed
- 分割リーフの表示サイズが通知ウィンドウとメインで一致しない問題を修正 (Splitter sizes を永続化し送信 layout と保存済み cols/rows を整合)
- ハング自動復旧の復旧アクション (reload / WebView 再作成) がアプリ全体をクラッシュさせるため一時無効化 (ping/pong 診断ログは維持)

## [0.20.1] - 2026-06-04

### Fixed
- 複数 AI エージェント同時稼働時に PTY 出力 emit が WebView2 IPC を飽和させメインループが恒久 wedge してアプリがフリーズする問題を修正 (出力 emit を 16ms 周期でコアレッシングし emit 頻度を上限化、pty-output payload を base64 化してサイズを 1/3〜1/4 に削減) (#53)
- flush ループと reader 最終 flush が同一セッションの保留出力を並走 drain する際に出力チャンクの順序が逆転しうる問題を修正 (drain と emit を同一クリティカルセクションにまとめ FIFO 順を保証)
- 出力が drain 速度を持続的に上回る際に保留バッファが無制限に増大しメモリを食い潰す問題に対し上限 (8MB) を設定

## [0.20.0] - 2026-05-10

### Added
- MCP に `oretachi_terminal_spawn` / `oretachi_terminal_list` / `oretachi_terminal_kill` ツールを追加
- MCP に `oretachi_read_terminal` / `oretachi_write_terminal` ツールを追加 (差分読み・status 取得・OSC 777 通知サポート)
- MCP 起動ターミナルを背景ペインへ隔離する仕組みを追加
- AI background コマンドの起動先 (前景/背景) を設定で切替可能に
- `mcp_server` モジュールを App.vue に統合

### Fixed
- PowerShell で Enter キーが正しく送信されない問題を修正
- `pty.kill` / `kill_all` 呼び出しに `source` 引数を追加し、`pty_kill` の発行元をログから特定可能に
- bug-review 指摘 (#1-#5 および追加3件) を修正
- review-terminal-flow 指摘 P1 #1, #2 を修正

### Documentation
- skill: background-command に read/write terminal 手順を追加

## [0.19.10] - 2026-04-29

### Fixed
- Add second-stage WebView recreate fallback when heartbeat is unresponsive for 95s (close + rebuild without killing PTY)
- Require `source` on `pty_manager::kill` / `kill_all` so the issuer of `pty_kill` can be identified in logs
- Add `AI_JUDGING_IN_FLIGHT` counter with `InFlightGuard` and surface `aiInFlight` in heartbeat pong/unresponsive logs
- Extend MCP notify debounce key to `(worktree, kind)` and apply hook=3s only, leaving general/completed/custom kinds unthrottled to avoid swallowing intentional notifications
- Skip AI judgment in `runApprovalLoop` when the last 60 lines show no approval prompt to reduce log noise
- Call `show()` / `set_focus()` after WebView rebuild so the recreated window actually becomes visible
- Make `recreate_attempted` an `Arc<AtomicBool>` and reset after 60s on failure / 300s on missing pong to prevent permanent dead-end after a failed or stalled recreation
- Switch WebView teardown from `close()` to `destroy()` and retry build once after 1s to avoid wry async-destroy race
- Track MCP notify last-sent timestamps as `Option<Instant>` to avoid theoretical underflow on Linux right after boot
- Introduce a recreate generation counter so backoff timers from prior cycles cannot reset state set by a newer recreate cycle

## [0.19.9] - 2026-04-21

### Fixed
- Add `blockedMs` to heartbeat payload and switch to native reload for WebView hang recovery
- Add 180s intermediate sign to unresponsive heartbeat log and reset unresponsive state on ping emit failure so subsequent 300s logs are not suppressed
- Move `startEventLoopMonitor()` before pong listener registration to avoid missing blocked-time measurements until the first pong

## [0.19.8] - 2026-04-16

### Added
- Auto-detect `.tsbuildinfo` files when adding a worktree and include them in the copy list

### Fixed
- Unify `notify_worktree` MCP tool hook notifications via broadcast channel to prevent WebView IPC freeze
- Exclude packages from `.tsbuildinfo` detection inside `node_modules`, targeting only cache files

## [0.19.7] - 2026-04-15

### Fixed
- Fix invalid export in TerminalView.vue script setup

## [0.19.6] - 2026-04-15

### Added
- Add WebView hang diagnostics feature

## [0.19.5] - 2026-04-14

### Added
- Include Claude skills in generated plugin

## [0.19.4] - 2026-04-13

### Fixed
- Prevent WebView freeze by filtering hook notifications from webview

## [0.19.3] - 2026-04-12

### Added
- Add debug mode to control log verbosity
- Identify AI agent session IDs and display in tab tooltips

### Fixed
- Cleanup terminalAiSessions on terminal close in useWorktreeFrameBundles

## [0.19.2] - 2026-04-12

### Fixed
- Fix plugin.json, hooks.json and .mcp.json format issues
- Use directory source object format in marketplace.json plugin source
- Use absolute path in marketplace.json plugin source

## [0.19.1] - 2026-04-12

### Fixed
- Generate marketplace.json to fix Claude plugin load error

## [0.19.0] - 2026-04-12

### Added
- Replace direct hook injection with Claude Code plugin system

### Fixed
- Address Claude review feedback for plugin system

## [0.18.3] - 2026-04-11

### Added
- Broadcast lifecycle hooks through MCP notifications

### Fixed
- Notify MCP client when worktree is archived
- Share worktree removal flow with MCP archive handling
- Extract shared worktree removal core

## [0.18.2] - 2026-04-10

### Added
- Add modular React artifact support with MCP flow integration

### Fixed
- Address review feedback for React artifact MCP flow

## [0.18.1] - 2026-04-10

### Added
- Annotate React artifact type with Tailwind CSS availability in MCP tool description
- Load Tailwind browser runtime in artifact viewer

### Fixed
- Move @tailwindcss/browser to dependencies for production builds

## [0.18.0] - 2026-04-10

### Added
- Add React artifact viewer
- Add Ctrl+P quick open file palette in code-reviewer

### Fixed
- Retry worktree removal after killing external processes
- Skip MCP port file cleanup when server is disabled

## [0.17.9] - 2026-04-09

### Added
- Add option to pull from remote before adding a worktree

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

[Unreleased]: https://github.com/ishida-supsys/oretachi/compare/0.21.1...HEAD
[0.21.1]: https://github.com/ishida-supsys/oretachi/compare/0.21.0...0.21.1
[0.21.0]: https://github.com/ishida-supsys/oretachi/compare/0.20.3...0.21.0
[0.20.3]: https://github.com/ishida-supsys/oretachi/compare/0.20.2...0.20.3
[0.20.2]: https://github.com/ishida-supsys/oretachi/compare/0.20.1...0.20.2
[0.20.1]: https://github.com/ishida-supsys/oretachi/compare/0.20.0...0.20.1
[0.20.0]: https://github.com/ishida-supsys/oretachi/compare/0.19.10...0.20.0
[0.19.10]: https://github.com/ishida-supsys/oretachi/compare/0.19.9...0.19.10
[0.19.9]: https://github.com/ishida-supsys/oretachi/compare/0.19.8...0.19.9
[0.19.8]: https://github.com/ishida-supsys/oretachi/compare/0.19.7...0.19.8
[0.19.7]: https://github.com/ishida-supsys/oretachi/compare/0.19.6...0.19.7
[0.19.6]: https://github.com/ishida-supsys/oretachi/compare/0.19.5...0.19.6
[0.19.5]: https://github.com/ishida-supsys/oretachi/compare/0.19.4...0.19.5
[0.19.4]: https://github.com/ishida-supsys/oretachi/compare/0.19.3...0.19.4
[0.19.3]: https://github.com/ishida-supsys/oretachi/compare/0.19.2...0.19.3
[0.19.2]: https://github.com/ishida-supsys/oretachi/compare/0.19.1...0.19.2
[0.19.1]: https://github.com/ishida-supsys/oretachi/compare/0.19.0...0.19.1
[0.19.0]: https://github.com/ishida-supsys/oretachi/compare/0.18.3...0.19.0
[0.18.3]: https://github.com/ishida-supsys/oretachi/compare/0.18.2...0.18.3
[0.18.2]: https://github.com/ishida-supsys/oretachi/compare/0.18.1...0.18.2
[0.18.1]: https://github.com/ishida-supsys/oretachi/compare/0.18.0...0.18.1
[0.18.0]: https://github.com/ishida-supsys/oretachi/compare/0.17.9...0.18.0
[0.17.9]: https://github.com/ishida-supsys/oretachi/compare/0.17.8...0.17.9
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
