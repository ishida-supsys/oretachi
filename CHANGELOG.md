# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.30.0] - 2026-09-04

### Added
- ワークグループ単位でトレイ通知を切り替える `trayNotification` 設定と `/notify` の tray フラグを追加 (#153, #154)
- トレイ通知トグルの UI を追加した (#155)
- トレイ通知を切り替える MCP ツールを追加し、teamwork-parent スキルを対応させた (#158)
- 既存ワークツリーへ `trayNotification` を一度だけ移行焼き込みするようにした (#171)
- 再起動時に AI ターミナルのセッションを自動復元するようにした (#157)
- codex の対話セッション ID を rollout から解決するようにした (#157)
- タスク完了後にホームタブへ自動復帰する設定を追加 (#156)

### Fixed
- 自動承認 ON のときに明示的な `notify_worktree` が消える2経路を塞いだ (#168)
- 預かり分の取り出し順と通知の重複を修正 (#168)
- 自動承認経路の通知も tray=false で抑制するよう修正 (#154)
- tray=false の通知が debounce 窓を消費しないよう修正 (#161)
- トレイ通知の実効値を `useWorkgroups.groupOf` 経由で解決するよう修正 (#155)
- Rust 由来の null を未設定として扱うよう修正 (#155)
- HomeView の `toggle-tray-notification` の重複と中継漏れを修正 (#155)
- 復元タブのフラグ分離と resume 投入の取りこぼしを修正 (#157)
- 自動ホーム復帰の発火条件を厳格化した (#156)
- 未割り当ての Alt+英数字をサブウィンドウから PTY へ透過するよう修正 (#150)

### Changed
- ワークグループ解決を `resolve_workgroup` に1本化した (#153)
- `trayNotification` のワークグループ設定を作成時の初期値として扱うよう変更 (#171)

## [0.29.2] - 2026-08-20

### Added
- トレイポップアップのフッターに 前へ / ウィンドウで開く / アーカイブ化して次へ を追加 (#129)

### Fixed
- 無関係なセッションが他ワークツリーのイベントを拾う問題を修正 (#149)
- 削除中に並びが変わると無関係なワークツリーが画面から消える問題を修正 (#149)
- トレイの ▾ メニューの切れとウィンドウ前面化の失敗を修正 (#129)

### Changed
- トレイの 次へ / 完了 と ▾ ボタンの色を緑で統一した (#129)

## [0.29.1] - 2026-08-19

### Fixed
- タブ死亡で生まれた引き継ぎ待ちを走行中のエージェントへ即座に引き取らせるよう修正 (#147)
- 購読イベントの自動セッション再開が不発になる問題を修正 (#147)
- 計画フローの四隅オーバーレイを折りたたみ式にした (#148)
- マークダウンのリンクを webview で開かずブラウザで開くよう修正 (#146)

## [0.29.0] - 2026-08-16

### Added
- teamwork-parent/teamwork-child スキルを追加し、サブイシュー管理をエージェント間で分担できるようにした (#145)
- ワークツリービューのヘッダにも購読バッジを表示するようにした (#138)
- 購読バッジの方向をアイコンで表すようにした (#138)
- 配送トーストを廃止し、カードに購読バッジを追加した (#138)
- `worktree.created` / `worktree.message` イベントと target ワイルドカードを追加 (#126)
- Stop フックによるターン境界配送 + UserPromptSubmit での取りこぼし回収を追加 (#124)
- サブウィンドウ / トレイでの購読配送を可視化した (#130)
- 購読の再バインド + 待機中への PTY 押し込み + 暴走防止 UI を追加 (#125)
- event_db と購読/inbox で `worktree.closed` を配送するようにした (#123)
- `terminal_id` を発番し env / hook / MCP へ貫通させた (#122)
- `oretachi_add_task` にワークグループ指定を追加 (#116)
- HOMEタブ周辺の挙動を修正 (#112)
- URL 型アーティファクトを追加 (#113)
- HOME/リポジトリ擬似ワークツリーでもアーティファクトを作成できるようにした (#111)

### Fixed
- サブエージェント内部発火の通知を抑制するよう修正 (#141)
- 自動 spawn したタブへ初期プロンプトと `--resume` を渡すよう修正 (#138)
- 自由文メッセージが本文を運ばず告知だけになるよう修正 (#138)
- bug-review 指摘対応 — ↓ 行の無反応クリック / クローズ時の emit 欠落 ほか (#138)
- bug-review 指摘対応 — 自動承認ガードの経路漏れ / 注入本文の総量上限 ほか (#126)
- bug-review 指摘対応 — イベント発行失敗のトースト巻き添え ほか (#126)
- 連鎖の深さを `worktree.message` だけで数えるよう修正 (#126)
- 押し込み側の自動承認判定も行単位にするよう修正 (#126)
- bug-review 指摘対応 — 自動承認判定の行単位化 / 引き継ぎ探索の抑止 (#124)
- bug-review 指摘対応 — 毎プロンプトの ack 催促 / HMR ポート追従 (#124)
- 注入本文を渡せてから `delivered_at` を打つよう修正 (#124)
- bug-review 指摘対応 — 購読パネルのヘッダ誤表示 / 更新要求の取りこぼし / spawn タブの引き継ぎ二重取り ほか (#125)
- バリデータ指摘対応 — 保持期限の継承 / 切り詰め分の誤打刻 / 生存タブの orphaned 固定 ほか (#125)
- セルフレビュー指摘対応 — passive の巻き込み配送 / 出力静穏の優先 / spawn 上限の同一tick素通り / 未使用 ack 経路 (#125)
- レビュー指摘対応 — 継承した `terminal_id` の除去と emit の対称化 (#122)

### Changed
- 端末数超過で自動 spawn を拒否せず警告するように変更 (#138)

## [0.28.0] - 2026-08-10

### Added
- ワークツリー追加先ディレクトリを `isHome` 付きの擬似ワークツリー「ホーム」として追加し、タブ・フレーム分割・セッション永続化にそのまま載せた (#110)
- ホームで claude を起動するだけでワークツリー管理エージェントとして振る舞うよう、プロンプトを SessionStart フック経由で注入する方式に統一 (#110)
- 管理業務を `<baseDir>/.claude/skills/` の同梱スキルとして定義 (worktree-cleanup / worktree-report / worktree-assign)。ユーザーが編集・追加できる (#110)
- MCP に `oretachi_inspect_worktree` を新設 (dirtyCount / mergedInto / lastCommitAt / ahead・behind) (#110)
- 登録済みリポジトリを擬似ワークツリー化し、リポジトリ root でターミナルを開けるようにした (#110)
- ワークグループバー先頭にリポジトリチップを追加し、Ctrl+PgUp/PgDown のローテーションにも参加させた (#110)
- ホームタブをドロップダウン化し、リポジトリのタブをその中へ畳んだ。畳んだエントリの通知件数はボタンに合算表示する (#110)
- リポジトリ一覧を masonry カード表示にし、`HomeSettingsDialog` へホーム関連設定を移設 (#110)

### Fixed
- plan モードで oretachi の MCP ツールが毎回承認要求される問題を修正。read-only ツールへ `annotations.readOnlyHint` を付与した (#109)
- MCP の `oretachi_close_worktree` でブランチが既に存在しない場合にエラーダイアログが出る問題を修正。`delete_branch` を冪等化し、二重クローズの in-flight ガードを追加 (#108)
- ワークツリー削除後にブランチ削除だけ失敗したケースを専用の警告として区別 (#108)
- `detect_base_branch` が実在する ref のみ返すよう修正。ローカルに main が無い環境で mergedInto / ahead / behind が常に取得できなかった (#110)
- `oretachi_list_terminals` の cwd 逆引きを最長プレフィックス一致に変更し、全端末がホーム判定になる問題を解消 (#110)
- ワークグループ削除フローからホームを除外し、削除が黙って失敗する問題を修正 (#110)
- baseDir 選択時に `syncWorktreesFromSettings` を呼ぶようにし、初回ウィザード直後にホームがタブ・カードへ出ない問題を修正 (#110)
- リポジトリ登録解除時にターミナル kill・承認判定プロセスの cancel・自動承認マップの掃除を行うよう修正 (#110)
- タブバーの横スクロールバーを掴めない問題を修正 (#110)

## [0.27.1] - 2026-08-07

### Fixed
- MCP 経由の `oretachi_close_worktree` で未マージ／squash マージ済みブランチの削除が失敗し、ワークツリーが復元されてクローズが進まない問題を修正 (#107)
- クローズ結果を MCP 呼び出し元へ返すようにし、成功／キャンセル／失敗を区別できるようにした (#107)

## [0.27.0] - 2026-08-04

### Added
- mermaid/SVG アーティファクトのパンズーム対応と可読性改善 (#103)
- CSV/TSV ビューアとプレゼンテーションスキルを追加 (#105)
- グループ systemPrompt を SessionStart フックで Claude Code セッションに常時注入 (#104)
- ホームタブにリポジトリ一覧を追加しアーティファクトを恒久保存可能にする (#106)

### Fixed
- 横スクロールでズームインしてしまう問題とホイール倍率の暴走を修正 (#103)
- width="100%" 指定の SVG アーティファクトがクリップされる問題を修正 (#103)
- 全画面ビューアーがコードブロックのヘッダーに隠れる問題を修正 (#103)
- リポジトリ側の転送/削除を worktree 単位と別 kind に分離し日次サマリの誤集計を解消 (#106)
- 恒久保存ディレクトリ名を sha256 先頭 128bit に変更し Windows のパス長上限を回避 (#106)
- 壊れた JSON が 1 件あるとリポジトリアーティファクト一覧全体が失敗する問題を修正 (#106)
- RepositoryPanel のリスナリークと、リロード後にアーティファクトウィンドウが無反応になる問題を修正 (#106)

### Changed
- mermaidTheme を components 配下から utils へ移動 (#103)

## [0.26.0] - 2026-07-23

### Added
- プランモード以外でも description を更新可能にする (#102)

## [0.25.3] - 2026-07-21

### Added
- MCP ターミナル追加時にサブウィンドウへ自動移行する設定を追加 (#58)

### Fixed
- 多数端末時の webview ハングを対策 (#101)
- single-instance ガードを導入し本番アプリの重複起動を防止 (#101)
- waitForTerminalReady が同期 ready を取りこぼし常に 5 秒待機していた問題を修正 (#58)

### Changed
- 端末の可視状態管理を useTerminalVisibility に切り出し (#101)
- useOretachiTerminalForBackground のデフォルトを false に変更 (#101)

## [0.25.2] - 2026-07-14

### Added
- 「IDEで開く」にファイルエクスプローラーの選択肢を追加 (#100)

### Fixed
- ファイルエクスプローラーの選択肢をダイアログ末尾に移動 (#100)

## [0.25.1] - 2026-07-13

### Fixed
- userConfig 非依存化で CC 2.1.207 の通知 hook フォーマットエラーを解消 (#99)

## [0.25.0] - 2026-06-25

### Added
- 猫ターミナル左下に CPU/メモリ/ネットワーク使用状況を表示

### Fixed
- メトリクスリスナーのリークを防止
- ワークグループの追加後続処理が失敗した際にカウントと表示が分裂する問題を解消
- commit 後の失敗時にタスク中断と MCP 通知の整合性を回復

## [0.24.6] - 2026-06-23

### Fixed
- アーティファクト一覧にスクロールバーを追加
- ワークグループ切替時の auto-animate でカードが重複/漏れする問題を解消

## [0.24.5] - 2026-06-20

### Added
- 通知有りワークツリーを含むワークグループのチップに赤枠を表示

### Fixed
- 高速切替によるワークツリー分裂を解消

## [0.24.4] - 2026-06-19

### Added
- ホームタブでワークグループを循環切替するホットキーを追加

### Fixed
- Windows で `on_before_exit` に kill-on-close 解除を移し、アップデートが不発になる問題を解消
- MCP 停止を install 後に移し、install 失敗時に MCP が停止したまま残る問題を回避

## [0.24.3] - 2026-06-19

### Fixed
- tao #1215 による UI スレッド再入デッドロック（ハング）を解消 (#89)
- PR URL のブランチ名 fetch をやめ、タスク生成のタイムアウトを解消

### Changed
- フロント `@tauri-apps` を 2.11 系へ揃える (#90)

## [0.24.2] - 2026-06-18

### Added
- タスク実行ダイアログに追加先ワークグループ選択を追加

### Fixed
- relaunch 時に Job の kill-on-close を解除し、アップデート時にインストーラを巻き込んで終了する問題を防止 (#88)
- タスク生成の MCP 設定に認証ヘッダを追加しタイムアウトを解消 (#87)

## [0.24.1] - 2026-06-18

### Fixed
- ClaudeCode の Auto モードを `bypassPermissions` ではなく `auto` にマッピング (#86)

## [0.24.0] - 2026-06-18

### Added
- ワークグループ機能を追加。複数のワークツリーをグループとしてまとめて管理できる (#84)
- `ORETACHI_PLUGIN_OVERWRITE` 環境変数で dev 時の Claude プラグイン上書きを制御できるよう追加 (#85)

### Fixed
- アプリ終了処理を確実化し、MCP 固定ポートの掴みっぱなしや孤児 WebView2 プロセスを防止
- Job オブジェクト割当失敗時に Job ハンドルを CloseHandle してリークを防止
- ホーム画面のワークツリーカードで残留 transform による列ズレ/gap を修正し、FLIP クリーンアップを堅牢化
- ワークグループの i18n で `{{PROMPT}}` をリテラル補間としてエスケープ

## [0.23.1] - 2026-06-17

### Added
- メインスレッドハングの真因捕捉計装を追加

### Fixed
- ホーム画面からのターミナル追加で孤児化・タブ不整合を解消
- フォアグラウンド追加で背景ペインにタブが紛れ込むのを防止
- cancel_worktree_remove に breadcrumb 計装を追加

## [0.23.0] - 2026-06-14

### Added
- タスク追加時のブランチ名パターンをリポジトリ単位で設定可能に (#76)
- トレイポップアップに description 情報バーを追加 (#77)

### Fixed
- ブランチ名パターンの構文例を i18n から外し HTML 誤検出を回避
- description 情報バーの高さを横幅確定後に再計測
- `ORETACHI_FORCE_WIZARD` が `.env.development.local` で効かない問題を修正

## [0.22.0] - 2026-06-13

### Added
- 初回起動ウィザードを追加。5ステップ (ウェルカム → 言語・AIエージェント選択 → ワークスペース設定 → ホームタブ説明 → トレイポップアップ説明) のオンボーディングを実装。既存ユーザーはアップグレード時に表示されない (#74)
- UIスケール設定 (通常/大/特大) を追加。webview ズームでターミナル内テキスト以外の全UIを 1.2倍/1.44倍に拡大可能。ターミナルは xterm fontSize を補正し物理グリフサイズを維持 (#73)

### Fixed
- ウィザード表示中のアプリ内ホットキー無効化、Esc スキップ廃止、アップデート確認の保留など、ウィザードのバグレビュー指摘を修正
- UIスケールの実適用ズームを共有し、setZoom 失敗時のターミナル縮小やトレイ過大サイズを防止。トレイポップアップ表示中の uiScale 変更にも追従

## [0.21.5] - 2026-06-11

### Fixed
- インストーラ/アップデータ経由で起動した際に RedirectionGuard 緩和策が子プロセスへ遺伝し、pnpm install 等のジャンクショントラバースが失敗する問題を修正。起動最初期に検出し explorer.exe 経由で再起動する (#70)
- 同期 PTY コマンドが tao メインスレッドをブロックし WebView が恒久フリーズしうる問題を解消。pty_write を writer スレッド + キュー化し、pty_spawn/resize/kill を spawn_blocking 化、kill_process_tree にタイムアウトを追加。メインスレッド watchdog も追加
- ターミナルのロック構造・キュー有界化・resize 順序保証に関するバグレビュー指摘を修正 (入力キューの有界化、kill のロック解放順、resize の直列化、watchdog の時刻基準を Instant に変更)
- コードレビューのフォルダ展開でロード中の再入により同一ディレクトリが二重フェッチされる問題を防止
- コードレビューのファイルツリーに gitignore 対象ファイル (.claude/CLAUDE.md 等) が表示されない問題を修正。遅延読み込みに変更し、QuickOpen も .gitignore 記載ファイルを含めるよう変更

## [0.21.4] - 2026-06-10

### Added
- ワークツリーカードの description をクリックで開閉でき、状態をワークツリー毎に保存するよう変更 (#68)

### Fixed
- 全表示ON中はカードクリックで開閉状態を書き換えないよう抑止
- カード名ドラッグ直後の click による description 誤トグルを抑止
- 表示内容が無いカードのクリックでは description 状態を保存しないよう修正

## [0.21.3] - 2026-06-10

### Fixed
- PTY 書き込みバッファがオクルージョン時に無制限増大する問題を解消 (#66)

## [0.21.2] - 2026-06-10

### Fixed
- クロスコンパイル時に target triple でサイドカーをビルドするよう修正 (#65)

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

[Unreleased]: https://github.com/ishida-supsys/oretachi/compare/0.30.0...HEAD
[0.30.0]: https://github.com/ishida-supsys/oretachi/compare/0.29.2...0.30.0
[0.29.2]: https://github.com/ishida-supsys/oretachi/compare/0.29.1...0.29.2
[0.29.1]: https://github.com/ishida-supsys/oretachi/compare/0.29.0...0.29.1
[0.29.0]: https://github.com/ishida-supsys/oretachi/compare/0.28.0...0.29.0
[0.28.0]: https://github.com/ishida-supsys/oretachi/compare/0.27.1...0.28.0
[0.27.1]: https://github.com/ishida-supsys/oretachi/compare/0.27.0...0.27.1
[0.27.0]: https://github.com/ishida-supsys/oretachi/compare/0.26.0...0.27.0
[0.26.0]: https://github.com/ishida-supsys/oretachi/compare/0.25.3...0.26.0
[0.25.3]: https://github.com/ishida-supsys/oretachi/compare/0.25.2...0.25.3
[0.25.2]: https://github.com/ishida-supsys/oretachi/compare/0.25.1...0.25.2
[0.25.1]: https://github.com/ishida-supsys/oretachi/compare/0.25.0...0.25.1
[0.25.0]: https://github.com/ishida-supsys/oretachi/compare/0.24.6...0.25.0
[0.24.6]: https://github.com/ishida-supsys/oretachi/compare/0.24.5...0.24.6
[0.24.5]: https://github.com/ishida-supsys/oretachi/compare/0.24.4...0.24.5
[0.24.4]: https://github.com/ishida-supsys/oretachi/compare/0.24.3...0.24.4
[0.24.3]: https://github.com/ishida-supsys/oretachi/compare/0.24.2...0.24.3
[0.24.2]: https://github.com/ishida-supsys/oretachi/compare/0.24.1...0.24.2
[0.24.1]: https://github.com/ishida-supsys/oretachi/compare/0.24.0...0.24.1
[0.24.0]: https://github.com/ishida-supsys/oretachi/compare/0.23.1...0.24.0
[0.23.1]: https://github.com/ishida-supsys/oretachi/compare/0.23.0...0.23.1
[0.23.0]: https://github.com/ishida-supsys/oretachi/compare/0.22.0...0.23.0
[0.22.0]: https://github.com/ishida-supsys/oretachi/compare/0.21.5...0.22.0
[0.21.5]: https://github.com/ishida-supsys/oretachi/compare/0.21.4...0.21.5
[0.21.4]: https://github.com/ishida-supsys/oretachi/compare/0.21.3...0.21.4
[0.21.3]: https://github.com/ishida-supsys/oretachi/compare/0.21.2...0.21.3
[0.21.2]: https://github.com/ishida-supsys/oretachi/compare/0.21.1...0.21.2
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
