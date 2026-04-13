---
name: domain-model-diagram
description: インタラクティブな React ドメインモデル図アーティファクトを作成する。グリッド/フォーカス2モードビュー、パン/ズーム、カテゴリ色分け対応。
allowed-tools: mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__search_artifact, Read, Bash(git branch:*)
---

# domain-model-diagram スキル

インタラクティブな React ドメインモデル図アーティファクトをテンプレートから作成する。

**機能:**
- グリッドビュー（全エンティティ俯瞰 + パン/ズーム）
- フォーカスビュー（エンティティクリックでラジアル配置 + リレーションラベル表示）
- CSS トランジション付きのスムーズな切り替え
- カテゴリ別色分け凡例

## 引数

```
$ARGUMENTS: <artifact-id> [--repo <repo>] [--branch <branch>]
```

- `artifact-id`（必須）: 作成するアーティファクトID（例: `my-service-diagram`）
- `--repo`（省略時: `oretachi`）: リポジトリ名
- `--branch`（省略時: `git branch --show-current`）: ブランチ名

## ワークフロー

### Step 1: パラメータ確定

引数を解析する。`--branch` が省略された場合は `git branch --show-current` で現在ブランチを取得。

### Step 2: テンプレートを読み込む

このスキルディレクトリ（`SKILL.md` と同じ場所）の `templates/` フォルダにある以下のファイルを Read で読み込む:

| ファイル | アーティファクトモジュール | カスタマイズ要否 |
|---|---|---|
| `templates/entry-point.jsx` | エントリポイント（content）| `// CUSTOMIZE:` コメント箇所のみ変更 |
| `templates/components--EntityBox.jsx` | `components/EntityBox` | `CATEGORY_COLORS`, `LAYER_LABELS` を変更 |
| `templates/components--RelationshipLine.jsx` | `components/RelationshipLine` | そのまま利用 |
| `templates/data--entities.example.jsx` | ※スキーマ参照用 | 新規生成 |
| `templates/data--relationships.example.jsx` | ※スキーマ参照用 | 新規生成 |

### Step 3: ドメイン分析

対象コードベースを調査 or ユーザーヒアリングで以下を特定する:
- エンティティ一覧（名前・フィールド）
- エンティティ間のリレーション（方向・カーディナリティ・ラベル）
- カテゴリ/レイヤー（3〜6 分類を推奨）

### Step 4: カテゴリ定義

カテゴリごとに Catppuccin Mocha パレットから背景色を選ぶ。
`templates/components--EntityBox.jsx` の `CATEGORY_COLORS` / `LAYER_LABELS` 形式に合わせて定義する。

**Catppuccin Mocha パレット（推奨色）:**
| 色名 | hex | 推奨用途 |
|---|---|---|
| red | `#f38ba8` | ドメイン層・コア |
| peach | `#fab387` | ユースケース層 |
| blue | `#89b4fa` | インフラ層・外部依存 |
| green | `#a6e3a1` | プレゼンテーション層・UI |
| yellow | `#f9e2af` | 設定・設定値 |
| mauve | `#cba6f7` | 共通・ユーティリティ |
| teal | `#94e2d5` | イベント・メッセージ |
| sky | `#89dceb` | クライアント・外部API |

テキスト色は常に `#1e1e2e`（暗背景上でのコントラスト確保）。

### Step 5: レイアウト計算

グリッド座標を割り当てる。

**ガイドライン:**
- 列の開始 x: 40, 320, 600, 880, 1160, 1440, ...（列幅 280px）
- 行の高さ見積もり: `26 + fieldCount * 19 + 50` px（次のエンティティのy座標算出に使用）
- カテゴリ同士を空間的にまとめる（関連するカテゴリは隣接列に配置）
- 接続数の多いエンティティはキャンバス中央付近に配置（フォーカスビューの可読性向上）
- `CANVAS_W` = 全エンティティの最大 x + BOX_WIDTH + 40（マージン）
- `CANVAS_H` = 全エンティティの最大 y + 最大高さ + 40（マージン）

### Step 6: アーティファクト作成

以下の順序で MCP ツールを呼び出す:

**1. エントリポイント作成**
```
artifact(command: "create", id: "<artifact-id>", type: "application/vnd.ant.react",
  title: "<ダイアグラムタイトル>",
  content: <entry-point.jsx の内容。CANVAS_W/H・タイトル・zoom を調整>)
```

**2. EntityBox モジュール作成**
```
artifact_module(command: "create", module_name: "components/EntityBox",
  content: <components--EntityBox.jsx の内容。CATEGORY_COLORS/LAYER_LABELS をドメインに合わせて変更>)
```

**3. RelationshipLine モジュール作成**
```
artifact_module(command: "create", module_name: "components/RelationshipLine",
  content: <components--RelationshipLine.jsx をそのまま>)
```

**4. entities モジュール作成**
```
artifact_module(command: "create", module_name: "data/entities",
  content: <Step 3〜5 で定義したエンティティ配列>)
```

**5. relationships モジュール作成**
```
artifact_module(command: "create", module_name: "data/relationships",
  content: <Step 3 で定義したリレーション配列>)
```

### Step 7: 検証

```
artifact(command: "outline")
```

で構造確認。エントリポイント + 4 モジュール（`components/EntityBox`, `components/RelationshipLine`, `data/entities`, `data/relationships`）が揃っていれば完了。

## テンプレートファイルの位置

このスキルファイル（`SKILL.md`）と同じディレクトリの `templates/` フォルダを Read で参照:
- `templates/entry-point.jsx`
- `templates/components--EntityBox.jsx`
- `templates/components--RelationshipLine.jsx`
- `templates/data--entities.example.jsx`（スキーマ確認用）
- `templates/data--relationships.example.jsx`（スキーマ確認用）
