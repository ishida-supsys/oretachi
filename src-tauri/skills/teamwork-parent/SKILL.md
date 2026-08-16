---
name: teamwork-parent
description: 親issueがsub-issueに分割された場合の進行管理を自動化する。ユーザーが「チームワークスキルを使って」「sub-issueを立てて分担して」等、既存または今後作成するsub-issue群の進行をこのセッションに任せたいときに使う。呼び出し後このセッションは「チームワークセッション」となり、sub-issueのワークツリーを購読し、進行可能なタスクを自動で子ワークツリーとして生成・監視し、全sub-issueがクローズしたら完了報告する。このセッション自身が誰かのsub-issueとして作られている場合は teamwork-child の義務と併用する。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_add_task, mcp__plugin_oretachi_oretachi__oretachi_subscribe_worktree, mcp__plugin_oretachi_oretachi__oretachi_unsubscribe_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_subscriptions, mcp__plugin_oretachi_oretachi__oretachi_poll_inbox, mcp__plugin_oretachi_oretachi__oretachi_ack_message, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_inspect_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_repository, mcp__plugin_oretachi_oretachi__notify_worktree, mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__search_artifact, Read, Write, Edit, Glob, Grep, Bash(gh issue:*), Bash(gh api:*), Bash(git branch:*)
---

# teamwork-parent スキル

このセッションは以後「チームワークセッション」として、sub-issueの分担・進行管理に徹する。実際のコード実装は各sub-issue用の子ワークツリーに委ね、このセッション自身はコードを書かない。

## Step 1: 計画とユーザー承認

1. 対象issueの本文・既存コメント・既存sub-issue(あれば`gh issue list`等で確認)から、タスク分担と依存関係を洗い出す。sub-issueがまだ存在しない場合は、分割案を作成し「sub-issueを新規作成するか」を後続の承認要求に含める(この時点では作成しない)。
2. 各sub-issueに決定的なブランチ名を採番する(例: `issue-<番号>`)。検証sub-issue等、完了前にユーザー確認が必要なものを識別する。
3. 3章の手順で計画フロー図をartifactとして作成する(状態はすべて`not_started`でよい)。**この計画フローartifactが以後の唯一の進捗管理データになる**(別途マークダウン等は作らない)。
4. ユーザーに承認を求める。承認内容には次を含める:
   - sub-issue新規作成の要否(対象がある場合)
   - 計画フロー(依存関係・検証ポイント・ユーザーが介入するタイミング)
   **承認が得られるまでStep2(`oretachi_add_task`の呼び出し)を行わない。**

## Step 2: 子ワークツリー作成(承認不要、開始・完了トリガーで自動)

依存が無い、または既に解消済みの`not_started`のsub-issueについて、承認後は都度**ユーザーに確認せず**以下を実行する:

1. `oretachi_add_task`を呼ぶ。**promptの冒頭に必ず`teamwork-child`スキルの読み込み指示を入れ、本文はsub-issueのURL参照のみに留める**(issue本文を転記しない):
   ```
   oretachi_add_task(prompt: "teamwork-child スキルを読み込んでから対応してください。\nSub-issue: <URL>")
   ```
2. `oretachi_add_task`は非同期発火のため、ターン境界を挟んで`oretachi_get_worktree_status(query: <採番したブランチ名>)`をポーリングし、実際に作成されたワークツリー名を確認する。
3. `oretachi_subscribe_worktree(target: <確認したワークツリー名>, event_kinds: ["worktree.closed", "worktree.created", "worktree.message"])`で購読する。
4. 4章の手順で計画フローartifactの`data/flow`モジュールを更新し、対象タスクの`status`を`in_progress`にし、`branch`が実際の値と一致していることを確認する。

## Step 3: 子ワークツリーイベントへの対応

- **`worktree.closed`**: 対応するsub-issueの`status`を`data/flow`更新で`done`にする。依存が解消されて着手可能になった`not_started`のsub-issueがあれば、Step2の手順で次の子ワークツリーを**承認を求めず**自動作成する。
- **`worktree.message`**: `oretachi_poll_inbox`で内容を確認し、`oretachi_ack_message`で既読化する。
  - issueコメントのURLのみの場合は必要に応じて参照し、ユーザーの判断が必要な内容かどうかを見極める。
  - ユーザー判断が必要な内容だけを提示し、それ以外は自動で流れを継続する。
- すべてのイベントを4章の手順で`data/flow`の`MESSAGES`に追記する(新しい順に先頭へ追加)。
- 状態に迷ったら`oretachi_list_subscriptions`で購読状態を確認してよい。

## Step 4: 完了判定

全sub-issueが`done`になったら:
- ユーザーに完了を報告し、作業を停止する。
- **このワークツリー自身が誰かのsub-issueである場合**(teamwork-childの義務を負っている場合)は、続けて`teamwork-child`スキルの完了報告手順(親issueへの報告 → `oretachi_close_worktree`の承認)に従う。

## 禁止事項

- Step1の承認前に`oretachi_add_task`を呼ばない。
- sub-issueの本文をそのまま`oretachi_add_task`のpromptに転記しない(URLのみ)。
- `oretachi_add_task`のprompt冒頭の`teamwork-child`読み込み指示を省略しない(子ワークツリーが正しいスキルを読み込む唯一の経路のため)。
- ユーザー判断が不要なメッセージでユーザーの手を止めない。
- 進捗を別ファイル(マークダウン等)へ二重に記録しない。進捗の唯一の記録先は計画フローartifactの`data/flow`。

---

# 計画フロー図artifactモジュール

## 引数

```
$ARGUMENTS: <artifact-id> [--repo <repo>] [--branch <branch>]
```

- `artifact-id`(必須): 作成するアーティファクトID(例: `teamwork-plan-139`)
- `--repo` / `--branch`(任意): 保存先ワークツリーを明示したい場合のみ、両方セットで指定

## テンプレートを読み込む

このスキルディレクトリ(`SKILL.md`と同じ場所)の`templates/`フォルダにある以下のファイルをReadで読み込む:

| ファイル | アーティファクトモジュール | カスタマイズ要否 |
|---|---|---|
| `templates/entry-point.jsx` | エントリポイント(content) | `// CUSTOMIZE:` コメント箇所のみ変更 |
| `templates/components--TaskNode.jsx` | `components/TaskNode` | そのまま利用 |
| `templates/components--DependencyEdge.jsx` | `components/DependencyEdge` | そのまま利用 |
| `templates/data--flow.example.jsx` | `data/flow` | ※スキーマ参照用、新規生成 |

**`data/flow`が計画フローの唯一のデータソース(TASKS/DEPENDENCIES/MESSAGESの3つをこの1ファイルにまとめる)。** Step2/Step3で進捗が変わるたびに、このモジュールを`artifact_module(command:"update", ...)`で直接更新する。専用の進捗管理マークダウン等は作らない。

## 表示操作

domain-model-diagramと同様、ドラッグでパン・スクロールでズームイン/アウトができる(マウスホイールで0.2〜2.5倍)。右上の「⟲ 表示をリセット」ボタンで初期表示に戻せる。

## レイアウト計算

Step1で洗い出したsub-issue一覧・依存関係から座標を割り当てる。

**ガイドライン:**
- 依存関係のトポロジカル・レベル(何段目の依存か)を列に割り当てる(依存が無いタスクは列0)
- 列の開始x: 40, 320, 600, 880, 1160, ...(列幅280px)
- 同じ列内のタスクは行間124px(84 + 間隔40)で縦に並べる
- `CANVAS_W` = 最大タスクx + 220(BOX_WIDTH) + 40(マージン)
- `CANVAS_H` = 最大タスクy + 84(BOX_HEIGHT) + 40(マージン)

## アーティファクト作成

以下の順序でMCPツールを呼び出す:

**1. エントリポイント作成**
```
artifact(command: "create", id: "<artifact-id>", type: "application/vnd.ant.react",
  title: "チームワーク計画フロー — #<親issue番号>",
  content: <entry-point.jsx の内容。CANVAS_W/H・タイトルを調整>)
```

**2. `components/TaskNode` モジュール作成**
```
artifact_module(command: "create", module_name: "components/TaskNode",
  content: <components--TaskNode.jsx をそのまま>)
```

**3. `components/DependencyEdge` モジュール作成**
```
artifact_module(command: "create", module_name: "components/DependencyEdge",
  content: <components--DependencyEdge.jsx をそのまま>)
```

**4. `data/flow` モジュール作成**
```
artifact_module(command: "create", module_name: "data/flow",
  content: <Step1で分析したsub-issue一覧・依存関係。MESSAGESは空配列でよい>)
```

## 検証

```
artifact(command: "outline")
```

で構造確認。エントリポイント + 3モジュール(`components/TaskNode`, `components/DependencyEdge`, `data/flow`)が揃っていれば完了。

## 進捗の反映(`data/flow`の更新方法)

進捗管理マークダウン等の別ファイルは使わず、`data/flow`モジュール自体を`artifact_module(command:"update", ...)`で直接書き換えることで進捗を反映する。`old_str`/`new_str`はモジュール内で一意になる部分文字列を選ぶ。

**タスク状態の更新**(Step2でsubscribe完了時、Step3でworktree.closed受信時など):
```
artifact_module(command: "update", module_name: "data/flow",
  old_str: "status: 'not_started', branch: 'issue-142'",
  new_str: "status: 'in_progress', branch: 'issue-142'")
```

**メッセージログの追記**(worktree.message受信時。新しい順に配列先頭へ追加):
```
artifact_module(command: "update", module_name: "data/flow",
  old_str: "const MESSAGES = [",
  new_str: "const MESSAGES = [\n  { ts: '<ISO8601>', from: '#<issue番号>', text: '<要約>' },")
```

これにより、ユーザーはoretachi UIで計画フローartifactを開くだけで最新の進捗を確認できる。
