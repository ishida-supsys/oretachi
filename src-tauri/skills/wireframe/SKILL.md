---
name: wireframe
description: インタラクティブな React ワイヤーフレームアーティファクトを作成する。エンティティバインディング表示（W コンポーネント）、画面遷移図（OverviewScreen）、デスクトップ全画面レイアウト対応。
allowed-tools: mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__search_artifact, Read, Bash(git branch:*)
---

# wireframe スキル

インタラクティブな React ワイヤーフレームアーティファクトをテンプレートから作成する。

**機能:**
- タブ切り替えによる複数画面プレビュー（デスクトップ全画面レイアウト）
- W コンポーネントによるエンティティバインディング表示（ホバー or 「パラメータ表示」ボタンで常時表示）
- 画面遷移 SVG 図（クリックで対応タブにジャンプ）
- Catppuccin Mocha ダークテーマのタブバー

## 引数

```
$ARGUMENTS: <artifact-id> [--repo <repo>] [--branch <branch>]
```

- `artifact-id`（必須）: 作成するアーティファクトID（例: `my-app-wireframe`）
- `--repo`（省略時: `oretachi`）: リポジトリ名
- `--branch`（省略時: `git branch --show-current`）: ブランチ名

## ワークフロー

### Step 1: パラメータ確定

引数を解析する。`--branch` が省略された場合は `git branch --show-current` で現在ブランチを取得。

### Step 2: テンプレートを読み込む

このスキルディレクトリの `templates/` フォルダにある以下のファイルを Read で読み込む:

| ファイル | アーティファクトモジュール | カスタマイズ要否 |
|---|---|---|
| `templates/entry-point.jsx` | エントリポイント（content）| `// CUSTOMIZE:` コメント箇所のみ変更 |
| `templates/components--W.jsx` | `components/W` | `EC` エンティティカラーマップのみ変更 |
| `templates/components--layout.jsx` | `components/layout` | そのまま利用 |
| `templates/components--primitives.jsx` | `components/primitives` | そのまま利用 |
| `templates/screens--OverviewScreen.example.jsx` | ※スキーマ参照用 | 新規生成 |
| `templates/screens--screen.example.jsx` | ※スキーマ参照用 | 新規生成 |

### Step 3: 画面・ドメイン分析

ユーザー要件（補足ヒアリング or コードベース調査）から以下を特定する:

**画面一覧:**
各画面について定義する:
- `key`: タブキー（kebab-case、例: `task-list`）
- `label`: タブ表示名（例: `Task List`）
- `type`: `list` | `detail` | `form` | `auth` | `overview`（常に1つ含む）
- 表示フィールドとエンティティバインディング（例: `Task.title`, `Task.status`）
- アクションボタンとサービスメソッドバインディング（例: `TaskService.createTask(title, dueDate)`）
- ナビゲーション先（どのタブキーに遷移するか）

**エンティティ一覧:**
- エンティティ名（例: `User`, `Task`）と所属カテゴリ
- 主要フィールド（W コンポーネントの `f` prop で参照するもの）
- サービスクラス（例: `TaskService`, `UserService`）も同様に定義

**画面遷移グラフ:**
- `{ from, to, label, isBack }` のリスト
- `isBack: true` は戻り方向（破線表示）

### Step 4: エンティティカラーマップ定義

`components/W` の `EC` オブジェクトを定義する。エンティティ名 → Catppuccin Mocha 色の対応を作成。

**Catppuccin Mocha パレット（推奨色）:**
| 色名 | hex | 推奨用途 |
|---|---|---|
| red | `#f38ba8` | ドメインコアエンティティ |
| peach | `#fab387` | サービス・ユースケース層 |
| blue | `#89b4fa` | インフラ・アダプター層 |
| green | `#a6e3a1` | プレゼンテーション・UI |
| yellow | `#f9e2af` | 値オブジェクト・列挙型 |
| mauve | `#cba6f7` | 共通・ユーティリティ |
| teal | `#94e2d5` | イベント・メッセージ |
| sky | `#89dceb` | 外部クライアント・API |

### Step 5: 画面モジュール設計

Step 3 の分析に基づき各画面の実装内容を設計する。
`templates/screens--screen.example.jsx` のスキーマを参照して各画面のコードを生成。

**画面タイプ別ガイドライン:**

- **list**: Frame + 右端に新規作成ボタン + Card 配列。各カードにタイトル・バッジ・メタ情報を Row で横並び
- **detail**: Frame + 右端に編集ボタン + 複数 Card。最後の Card にアクションボタン Row
- **form**: Frame + maxWidth 640 のフォームコンテナ + WInput/WTextarea/WSelect + 送信/キャンセルボタン Row
- **auth**: Frame + maxWidth 480 の中央配置コンテナ + メールアドレス/パスワード入力 + 送信ボタン
- **overview**: `templates/screens--OverviewScreen.example.jsx` のスキーマを参照して SVG 遷移図を新規生成

### Step 6: アーティファクト作成

以下の順序で MCP ツールを呼び出す:

**1. エントリポイント作成**
```
artifact(command: "create", id: "<artifact-id>", type: "application/vnd.ant.react",
  title: "<アプリ名> — ワイヤーフレーム",
  content: <entry-point.jsx の内容。CUSTOMIZE: Screen imports / TABS array / Screen render switch を変更>)
```

**2. `components/W` モジュール作成**
```
artifact_module(command: "create", module_name: "components/W",
  content: <components--W.jsx の内容。EC エンティティカラーマップをStep 4の定義に変更>)
```

**3. `components/layout` モジュール作成**
```
artifact_module(command: "create", module_name: "components/layout",
  content: <components--layout.jsx をそのまま>)
```

**4. `components/primitives` モジュール作成**
```
artifact_module(command: "create", module_name: "components/primitives",
  content: <components--primitives.jsx をそのまま>)
```

**5. `screens/OverviewScreen` モジュール作成**
```
artifact_module(command: "create", module_name: "screens/OverviewScreen",
  content: <Step 3の画面リスト・遷移グラフから新規生成>)
```

**6. 各画面モジュール作成**（画面数分だけ繰り返す）
```
artifact_module(command: "create", module_name: "screens/<ScreenName>",
  content: <Step 5 の設計に基づいて新規生成>)
```

複数の画面が定義されている場合、同一の export ファイルにまとめることができる（例: 認証系は `screens/AuthScreens` に `LoginScreen` / `RegisterScreen` を名前付き export）。

### Step 7: 検証

```
artifact(command: "outline")
```

で構造確認。エントリポイント + 4 つの固定モジュール（`components/W`, `components/layout`, `components/primitives`, `screens/OverviewScreen`）+ 各画面モジュールが揃っていれば完了。

## テンプレートファイルの位置

このスキルファイル（`SKILL.md`）と同じディレクトリの `templates/` フォルダを Read で参照:
- `templates/entry-point.jsx`
- `templates/components--W.jsx`
- `templates/components--layout.jsx`
- `templates/components--primitives.jsx`
- `templates/screens--OverviewScreen.example.jsx`（OverviewScreen 生成時のスキーマ参照用）
- `templates/screens--screen.example.jsx`（各画面生成時のスキーマ参照用）
