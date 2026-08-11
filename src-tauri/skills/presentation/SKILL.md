---
name: presentation
description: インタラクティブな React プレゼンテーションアーティファクトを作成する。キーボード操作によるスライド送り、サムネイル一覧、全画面表示、16:9 自動フィット対応。
allowed-tools: mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__search_artifact, Read, Bash(git branch:*)
---

# presentation スキル

インタラクティブな React プレゼンテーションアーティファクトをテンプレートから作成する。

**機能:**
- キーボード操作によるスライド送り（`←` `→` `Space` `PageUp` `PageDown` `Home` `End`）
- サムネイル一覧オーバーレイ（`G` キー / ⊞ ボタン、クリックでジャンプ）
- 全画面表示（`⛶` ボタン。ブロックされる環境では擬似全画面にフォールバック）
- 16:9 を保ったまま親要素にフィットするスライド枠（小窓でも全画面でも同じ見た目）
- Catppuccin Mocha の操作バー + 進捗バー

## 引数

```
$ARGUMENTS: <artifact-id> [--repo <repo>] [--branch <branch>]
```

- `artifact-id`（必須）: 作成するアーティファクトID（例: `kickoff-deck`）
- `--repo` / `--branch`（任意）: 保存先ワークツリーを明示したい場合のみ、両方セットで指定

## ワークフロー

### Step 1: パラメータ確定

引数を解析する。保存先ワークツリーは、artifact 系ツールに **`project_dir`（現在の作業ディレクトリ）を渡して解決させる**のが既定。HOME タブやリポジトリルートは git ワークツリーではないため `git branch --show-current` が使えず、`--repo`/`--branch` では特定できない。

`--repo` と `--branch` が両方明示された場合のみ、`project_dir` の代わりにそれを渡す。`--repo` だけ与えられた場合は `git branch --show-current` でブランチを補う。

### Step 2: テンプレートを読み込む

このスキルディレクトリの `templates/` フォルダにある以下のファイルを Read で読み込む:

| ファイル | アーティファクトモジュール | カスタマイズ要否 |
|---|---|---|
| `templates/entry-point.jsx` | エントリポイント（content）| `// CUSTOMIZE:` コメント箇所のみ変更 |
| `templates/components--deck.jsx` | `components/deck` | `THEME` のみ変更（配色を変えない場合はそのまま） |
| `templates/slides--slide.example.jsx` | ※スキーマ参照用 | 新規生成 |

### Step 3: 内容分析

ユーザー要件（補足ヒアリング or コードベース調査）からアウトラインを組み立てる。

**スライド一覧:**
各スライドについて定義する:
- `key`: スライドキー（kebab-case、例: `arch-overview`）
- `label`: サムネイル一覧に出す短いラベル（例: `アーキテクチャ概要`）
- `kind`: スライド種別（下記）
- 本文（見出し・箇条書き・図版・コードなど）

**スライド種別:**
| kind | 用途 |
|---|---|
| `title` | 表紙。中央寄せ・フッタなし |
| `bullets` | 箇条書き（1スライド 4〜6 項目まで） |
| `two-col` | 左テキスト + 右図版 |
| `diagram` | 図版中心（インライン SVG） |
| `big` | 数値・キーメッセージ 1 つの強調。話の区切りに置く |
| `code` | コード抜粋（15行程度まで） |
| `quote` | 引用・方針の言い切り |
| `closing` | まとめ |

**構成の目安:**
- 全体で 8〜15 枚。`title` で始まり `closing` で終える
- 2 枚目に `bullets` のアジェンダを置く
- 4〜5 枚ごとに `big` か `quote` を挟んでリズムを作る

### Step 4: テーマ設定

`components/deck` の `THEME` を必要に応じて変更する。既定はライト背景 + mauve アクセント。

**Catppuccin Mocha パレット（アクセント色の選択肢）:**
| 色名 | hex | 推奨用途 |
|---|---|---|
| mauve | `#cba6f7` | 汎用（既定） |
| blue | `#89b4fa` | 技術・アーキテクチャ寄りの発表 |
| green | `#a6e3a1` | 成果報告・リリースノート |
| peach | `#fab387` | 提案・企画 |
| red | `#f38ba8` | 障害報告・ポストモーテム |
| teal | `#94e2d5` | ロードマップ・計画 |

本文色は `#1f2430`、補足は `#6b7280` を維持する（ライト背景でのコントラスト確保）。

### Step 5: スライドモジュール設計

Step 3 のアウトラインに基づき各スライドの実装内容を設計する。
`templates/slides--slide.example.jsx` のパターン 1〜7 を参照してコードを生成。

**共通ルール:**
- 各スライドは `<Slide n={n} total={total} label={label}>` で包む（`title` のみ `footer={false}`）
- サイズはすべて `u(n)` を使う（= スライド幅の n%）。`px` 直書きはしない
- `SlideTitle` / `Bullets` / `TwoCol` / `Big` / `Quote` / `CodeBlock` / `Note` / `Center` を組み合わせる

**種別別ガイドライン:**
- **title**: `Slide footer={false}` + `Center`。大見出し + アクセントライン + サブタイトル + 日付
- **bullets**: `SlideTitle`（`sub` に一言）+ `Bullets`。項目が 5 個を超えるなら `dense`
- **two-col**: `SlideTitle` + `TwoCol`。`ratio` は既定 0.5、テキスト主体なら 0.55〜0.6
- **diagram**: `SlideTitle` + インライン SVG を `viewBox` + `width:'100%'` で配置
- **big**: `Center` + `Big`。値は 6 文字以内に収める
- **code**: `SlideTitle` + `CodeBlock`（`caption` にファイルパス）+ 必要なら `Note`
- **quote**: `SlideTitle` + `Center` + `Quote`
- **closing**: `Center` + 大見出し + `Bullets dense` を 3 項目まで

**サンドボックス制約（必ず守る）:**
- プレビュー iframe の CSP は `default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; img-src data: blob:`
  → **外部画像・外部フォント・CDN スクリプトは一切読み込めない**。図版はインライン SVG か data: URI にする
- 利用できるのは React 18 + JSX/TypeScript のみ（Babel の env / react / typescript プリセット）
- Tailwind ユーティリティクラスも利用可（`@tailwindcss/browser` が同梱済み）。
  ただしテンプレートはインラインスタイルで統一しているので、混在させない

### Step 6: アーティファクト作成

以下の順序で MCP ツールを呼び出す:

**1. エントリポイント作成**
```
artifact(command: "create", id: "<artifact-id>", type: "application/vnd.ant.react",
  title: "<発表タイトル> — プレゼンテーション",
  content: <entry-point.jsx の内容。CUSTOMIZE: Slide imports / DECK_LABEL / SLIDES array / renderSlide switch を変更>)
```

**2. `components/deck` モジュール作成**
```
artifact_module(command: "create", module_name: "components/deck",
  content: <components--deck.jsx の内容。THEME を Step 4 の定義に変更>)
```

**3. 各スライドモジュール作成**（スライド数分だけ繰り返す）
```
artifact_module(command: "create", module_name: "slides/<SlideName>",
  content: <Step 5 の設計に基づいて新規生成>)
```

関連するスライドは同一モジュールにまとめて名前付き export してよい
（例: `slides/IntroSlides` に `TitleSlide` / `AgendaSlide` をまとめる）。
その場合はエントリポイントの require を分割代入に変える。

### Step 7: 検証

```
artifact(command: "outline")
```

で構造確認。エントリポイント + `components/deck` + 各スライドモジュールが揃っていれば完了。
`SLIDES` 配列の `key` と `renderSlide` の分岐、そして実際に作成したモジュールが
1対1で対応しているかを必ず突き合わせること（キーの取りこぼしは空白スライドになる）。

## テンプレートファイルの位置

このスキルファイル（`SKILL.md`）と同じディレクトリの `templates/` フォルダを Read で参照:
- `templates/entry-point.jsx`
- `templates/components--deck.jsx`
- `templates/slides--slide.example.jsx`（各スライド生成時のスキーマ参照用）
