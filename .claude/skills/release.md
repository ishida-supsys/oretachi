# Release Skill

バージョンをバンプし、CHANGELOG.md を更新してコミットする。

## 使い方

引数なし、または以下を指定:
- `patch` / `minor` / `major` — バンプ種別
- `1.2.3` のような具体的なバージョン番号

## 手順

### 1. 現在バージョンと次バージョンを決定

`package.json` から現在バージョンを読み取る。

引数で明示的にバージョンが指定された場合はそれを使う。

引数がなければ git log を分析してバンプ種別を提案し、**ユーザーに確認を取ってから進む**:
- `BREAKING CHANGE` または `feat!:` / `fix!:` → major バンプを提案
- `feat:` コミットがあれば → minor バンプを提案
- `fix:` / `perf:` のみ → patch バンプを提案
- それ以外 → patch バンプを提案

提案時は以下の形式でユーザーに示す:
```
コミット分析の結果、minor バンプを提案します。

  現在: 0.12.0 → 次バージョン: 0.13.0

よろしいですか？ (別のバージョンを指定することもできます)
```

ユーザーが承認または別バージョンを指定してから次のステップに進む。

### 2. git log を取得

```bash
git log $(git describe --tags --abbrev=0 2>/dev/null || git rev-list --max-parents=0 HEAD)..HEAD --pretty=format:"%s"
```

コミットメッセージを以下のセクションに分類:
- `feat:` → **Added**
- `fix:` → **Fixed**
- `perf:` → **Changed** (パフォーマンス改善)
- `refactor:` → **Changed**
- `docs:` → **Documentation**
- `ci:` / `chore:` / `build:` / `test:` → **Other** (省略可)
- `BREAKING CHANGE` / `!:` → 各セクション冒頭に ⚠️ で記載

コミット本文から重要な説明を抽出してリスト項目を簡潔に書く（英語または日本語、既存 CHANGELOG に合わせる）。

### 3. git remote URL を取得

```bash
git remote get-url origin
```

GitHub URL に変換（例: `https://github.com/owner/repo`）。SSH形式 (`git@github.com:owner/repo.git`) も変換する。

### 4. CHANGELOG.md を更新

`taiki-e/parse-changelog` が期待する Keep a Changelog 形式で書く。

CHANGELOG.md が存在しない場合は新規作成。存在する場合は `## [Unreleased]` セクションの直後に新バージョンセクションを挿入。

フォーマット例:
```markdown
# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [X.Y.Z] - YYYY-MM-DD

### Added
- 新機能の説明

### Fixed
- バグ修正の説明

### Changed
- 変更内容の説明

[Unreleased]: https://github.com/owner/repo/compare/X.Y.Z...HEAD
[X.Y.Z]: https://github.com/owner/repo/compare/OLD.VER...X.Y.Z
```

**重要なルール:**
- 末尾にバージョン比較リンクを追加・更新する
- `Merge branch '...'` コミットは無視する
- `chore: release` や `chore: bump version` コミットは無視する
- セクションが空の場合はそのセクション見出しを省略する
- 今日の日付を使う

### 5. バージョン番号を3ファイルで更新

以下の3ファイルのバージョン文字列を新バージョンに書き換える:
- `package.json` — `"version": "X.Y.Z"`
- `src-tauri/Cargo.toml` — `version = "X.Y.Z"` (package セクション)
- `src-tauri/tauri.conf.json` — `"version": "X.Y.Z"`

### 6. Cargo.lock を再生成

`Cargo.toml` のバージョンが変わると `Cargo.lock` も更新が必要。
`cargo generate-lockfile` は全依存を解決しなおすため、代わりにパッケージ名を指定して最小限の更新に留める:

```bash
cd src-tauri && cargo update --precise X.Y.Z --package oretachi
```

このコマンドで `Cargo.lock` 内の `oretachi` パッケージエントリのみ新バージョンに更新される。

### 7. コミット

```bash
git add CHANGELOG.md package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock
git commit -m "chore: release X.Y.Z"
```

コミット後、次のリリース手順を案内する:
```
git tag X.Y.Z
git push origin main --tags
```

## 注意事項

- コミット前にユーザーに変更内容を確認する
- push はユーザーが明示的に指示するまで行わない
