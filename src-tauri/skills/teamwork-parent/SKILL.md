---
name: teamwork-parent
description: 親issueがsub-issueに分割された場合の進行管理を自動化する。ユーザーが「チームワークスキルを使って」「sub-issueを立てて分担して」等、既存または今後作成するsub-issue群の進行をこのセッションに任せたいときに使う。呼び出し後このセッションは「チームワークセッション」となり、sub-issueのワークツリーを購読し、進行可能なタスクを自動で子ワークツリーとして生成・監視し、全sub-issueがクローズしたら完了報告する。このセッション自身が誰かのsub-issueとして作られている場合は teamwork-child の義務と併用する。
allowed-tools: mcp__plugin_oretachi_oretachi__oretachi_add_task, mcp__plugin_oretachi_oretachi__oretachi_subscribe_worktree, mcp__plugin_oretachi_oretachi__oretachi_unsubscribe_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_subscriptions, mcp__plugin_oretachi_oretachi__oretachi_poll_inbox, mcp__plugin_oretachi_oretachi__oretachi_ack_message, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_inspect_worktree, mcp__plugin_oretachi_oretachi__oretachi_list_repository, mcp__plugin_oretachi_oretachi__notify_worktree, mcp__plugin_oretachi_oretachi__oretachi_set_tray_notification, mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__search_artifact, Read, Write, Edit, Glob, Grep, Bash(gh issue:*), Bash(gh api:*), Bash(git branch:*)
---

# teamwork-parent スキル

このセッションは以後「チームワークセッション」として、sub-issueの分担・進行管理に徹する。実際のコード実装は各sub-issue用の子ワークツリーに委ね、このセッション自身はコードを書かない。

## Step 1: 計画とユーザー承認

1. **まず自分自身のワークツリーのトレイ通知をオフにする**(承認を待つ必要はない。自分のノイズを止めるだけで他ワークツリーの通知には影響しないため):
   ```
   oretachi_set_tray_notification(project_dir: <自分の作業ディレクトリ絶対パス>, enabled: false)
   ```
2. 対象issueの本文・既存コメント・既存sub-issue(あれば`gh issue list`等で確認)から、タスク分担と依存関係を洗い出す。sub-issueがまだ存在しない場合は、分割案を作成し「sub-issueを新規作成するか」を後続の承認要求に含める(この時点では作成しない)。
3. 各sub-issueに決定的なブランチ名を採番する(例: `issue-<番号>`)。
4. **停止条件をヒアリングする。** 分割案と同時に、次の2種類の停止条件の候補を挙げてユーザーに提示し、回答を得る:
   - **タスクの停止条件**(子ワークツリーが自分の作業中に止まる項目) — 例: 「追加テーブルのスキーマが確定したら確認」「動作確認の結果を見て判断」
   - **エッジの停止条件**(次タスクへ進む遷移で親自身が止まる項目) — 例: 「#141 の成果を見て #142 の分割方針を再確認」
   AIが候補を出し、ユーザーが取捨選択・追加する形で確定させる。**ユーザーの回答を得てから**`data/flow`にセットすること。
5. 5章の手順で計画フロー図をartifactとして作成する(状態はすべて`not_started`、停止条件は確定した内容を`checked: false`で入れる)。**この計画フローartifactが以後の唯一の進捗管理データになる**(別途マークダウン等は作らない)。
6. ユーザーに承認を求める。承認内容には次を含める:
   - sub-issue新規作成の要否(対象がある場合)
   - 計画フロー(依存関係・ユーザーが介入するタイミング)
   - **各タスク/エッジに設定した停止条件の一覧**
   **承認が得られるまでStep2(`oretachi_add_task`の呼び出し)を行わない。**

### フローを修正するとき

タスクの追加・削除や依存関係の変更を行う場合は、**その差分について停止条件を改めてユーザーに聞く**。新しいタスクにはそのタスクの停止条件を、新しいエッジにはその遷移の停止条件を、Step1と同じ手順でヒアリングしてから`data/flow`を更新する。既存の停止条件をそのまま流用する・不要と判断する、といった推測はしない。

## Step 2: 子ワークツリー作成(承認不要、開始・完了トリガーで自動)

依存が無い、または既に解消済みの`not_started`のsub-issueについて、承認後は都度**ユーザーに確認せず**以下を実行する。

**ただし、そのタスクへ入るエッジに未クリアの停止条件が1つでもあれば、この手順に入らない。** その場合はStep3の「親自身の停止条件」の扱いに従い、`notify_worktree`でユーザーを呼び戻して回答を得てからにする。

1. **そのタスクの停止条件をsub-issue本文に書き込む。** `data/flow`のタスク側`stopConditions`を、sub-issue本文の末尾に`## 停止条件`セクションとしてチェックリストで追記する(既にあれば内容を同期する):
   ```markdown
   ## 停止条件

   - [ ] 追加テーブルのスキーマが確定したら確認する
   - [ ] 動作確認の結果をユーザーが見て判断する
   ```
   `gh issue edit <番号> --body-file <一時ファイル>`等で本文を更新する。**これが子へ停止条件を伝える唯一の経路**(`oretachi_add_task`のpromptはURLのみのため、本文に書かないと子は停止条件を知れない)。エッジの停止条件は親自身のものなのでsub-issue本文には書かない。
2. `oretachi_add_task`を呼ぶ。**promptの冒頭に必ず`teamwork-child`スキルの読み込み指示を入れ、本文はsub-issueのURL参照のみに留める**(issue本文を転記しない):
   ```
   oretachi_add_task(prompt: "teamwork-child スキルを読み込んでから対応してください。\nSub-issue: <URL>")
   ```
3. `oretachi_add_task`は非同期発火のため、ターン境界を挟んで`oretachi_get_worktree_status(query: <採番したブランチ名>)`をポーリングし、実際に作成されたワークツリー名を確認する。
4. `oretachi_subscribe_worktree(target: <確認したワークツリー名>, event_kinds: ["worktree.closed", "worktree.created", "worktree.message"])`で購読する。
5. 5章の手順で計画フローartifactの`data/flow`モジュールを更新し、対象タスクの`status`を`in_progress`にし、`branch`が実際の値と一致していることを確認する。

## Step 3: 子ワークツリーイベントへの対応

- **`worktree.closed`**: 対応するsub-issueの`status`を`data/flow`更新で`done`にする。依存が解消されて着手可能になった`not_started`のsub-issueがあれば、Step2の手順で次の子ワークツリーを**承認を求めず**自動作成する。
- **`worktree.message`**: `oretachi_poll_inbox`で内容を確認し、`oretachi_ack_message`で既読化する。
  - issueコメントのURLのみの場合は必要に応じて参照し、ユーザーの判断が必要な内容かどうかを見極める。
  - ユーザー判断が必要な内容だけを提示し、それ以外は自動で流れを継続する。
  - **受信内容が「子が自分の停止条件で止まった」ことを示す場合、親は停止も`notify_worktree`もしない。** 子の停止条件は子ワークツリー上でユーザーが解決するものなので、親は`MESSAGES`に記録して流れを継続する(親がここで止まると、子と親の二重で人を呼ぶことになる)。
  - **子から停止条件のクリア報告を受けたら、`data/flow`の該当条件を`checked: true`にして`checkedAt`を入れる。** 報告本文に含まれる停止条件のidまたはテキストで対象を特定する。

### 親自身の停止条件(エッジ)への対応

`from`が`done`になり、次タスク`to`を起動しようとする時点で、そのエッジに未クリアの停止条件があれば:

1. **`oretachi_add_task`を実行しない。**
2. `notify_worktree`でユーザーを呼び戻し、該当する停止条件の内容を提示して回答を求める。
3. 回答を得たら`data/flow`のそのエッジの条件を`checked: true` / `checkedAt`に更新する。
4. すべてクリアになってからStep2の手順で子ワークツリーを作成する。

複数の停止条件がある場合は、すべてクリアされるまで起動しない。
- **ユーザーの判断・操作が必要になったときは必ず`notify_worktree`を呼ぶ。** Step 1 でトレイ通知をオフにしているため、テキストを出力するだけではユーザーは気付けない。`notify_worktree`はトレイ通知オフでも常に通る唯一の呼び戻し経路。
- すべてのイベントを4章の手順で`data/flow`の`MESSAGES`に追記する(新しい順に先頭へ追加)。
- 状態に迷ったら`oretachi_list_subscriptions`で購読状態を確認してよい。

## Step 4: 完了判定

全sub-issueが`done`になったら:
- `oretachi_set_tray_notification(project_dir: <自分の作業ディレクトリ絶対パス>, enabled: true)`を呼び、トレイ通知をオンへ戻す(ワークツリーは完了後も残って再利用されうるため、オフのまま放置しない)。ワークツリー作成時にトレイ通知オフが焼き込まれていた場合は、`enabled: false`のままにするかをユーザーに確認してから戻すこと。
- ユーザーに完了を報告し、作業を停止する。
- **このワークツリー自身が誰かのsub-issueである場合**(teamwork-childの義務を負っている場合)は、続けて`teamwork-child`スキルの完了報告手順(親issueへの報告 → `oretachi_close_worktree`の承認)に従う。

## トレイ通知のノイズ対策(原則)

チームワークセッションは子ワークツリーのイベントを購読して待機と再開を繰り返すため、自分自身のフック由来通知(`Stop`→`completed` / `PermissionRequest`→`approval` 等)がトレイに溢れやすい。**原則として親ワークツリー(このセッション)はトレイ通知をオフにし、人の判断・操作が必要なときにのみ明示的に`notify_worktree`で通知する。** Step 1 の最初に`oretachi_set_tray_notification(enabled: false)`で自分自身をオフにすること。

`enabled`を省略して呼ぶと「未設定」に戻る(実効値は`true` = 通知する)。ワークグループの`trayNotification`は**新規ワークツリー作成時の初期値**でしかなく、フォールバック先にはならないので、省略呼び出しは「作成時の設定へ戻す」ことにはならない点に注意する(Step 4)。

**対象は自分自身のワークツリーだけ**。子ワークツリーは承認待ちをユーザーに見せる必要があるため、通知はオンのままにする(子側の設定に触れない)。

オフにしても次は変わらない:

- 自動承認は動き続ける(通知イベント自体は流れており、抑制されるのはトレイへの提示だけ)。
- **ユーザー判断が必要なときは`notify_worktree`を明示的に呼ぶ。トレイ通知がオフでもこれだけは通る**(このツール経由の通知は`kind`にかかわらず常にトレイへ出る)。

したがって Step 3 で「ユーザー判断が必要な内容だけを提示する」際は、テキスト出力に頼らず`notify_worktree`を呼んでユーザーを呼び戻すこと。

## 禁止事項

- Step1の承認前に`oretachi_add_task`を呼ばない。
- sub-issueの本文をそのまま`oretachi_add_task`のpromptに転記しない(URLのみ)。
- `oretachi_add_task`のprompt冒頭の`teamwork-child`読み込み指示を省略しない(子ワークツリーが正しいスキルを読み込む唯一の経路のため)。
- ユーザー判断が不要なメッセージでユーザーの手を止めない。
- トレイ通知をオフにしたまま、ユーザー判断が必要な場面で`notify_worktree`を省略しない(ユーザーが永久に気付けなくなる)。
- 子ワークツリーのトレイ通知を`oretachi_set_tray_notification`でオフにしない(承認待ちが見えなくなる)。
- **親の停止条件(エッジ)を飛ばして次タスクを起動しない。** 未クリアの停止条件が残っているエッジの先にあるタスクは、`oretachi_add_task`の対象にしない。
- 子の停止条件による子の停止を、親の停止として扱わない(親は止まらず流れを継続する)。
- 停止条件をユーザーに聞かずにAIの判断だけで設定・変更・削除しない。
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
| `templates/lib--stopConditions.jsx` | `lib/stopConditions` | そのまま利用 |
| `templates/data--flow.example.jsx` | `data/flow` | ※スキーマ参照用、新規生成 |

**`data/flow`が計画フローの唯一のデータソース(TASKS/DEPENDENCIES/MESSAGESの3つをこの1ファイルにまとめる)。** Step2/Step3で進捗が変わるたびに、このモジュールを`artifact_module(command:"update", ...)`で直接更新する。専用の進捗管理マークダウン等は作らない。

## 表示操作

domain-model-diagramと同様、ドラッグでパン・スクロールでズームイン/アウトができる(マウスホイールで0.2〜2.5倍)。右上の「⟲」ボタンで初期表示に戻せる。

フロー図を覆わないよう、四隅の欄は**既定ですべて畳まれている**。四隅のチップ(左上=進捗、右上=`⟲`/`?`、左下=凡例、右下=状況)をクリックすると展開し、同時に開くのは1つだけ。`Esc`キーまたは背景クリックで閉じる。右下の「状況」チップには畳んだ状態でも `▸<次に着手可能数> ⏸<要ユーザー確認数>` のバッジが出る。

停止条件を持つノード・依存線には `⏸`(未クリアあり) / `☑`(全クリア済み) のバッジが付き、**ホバーすると停止条件のtodoリストがポップアップする**(対象は白枠で強調される)。色は3フェーズを表す: 灰=未実行(まだそのタスク/遷移に到達していない)、橙=実行中(今まさに人の判定待ち)、緑=実行済み。

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

**4. `lib/stopConditions` モジュール作成**
```
artifact_module(command: "create", module_name: "lib/stopConditions",
  content: <lib--stopConditions.jsx をそのまま>)
```

**5. `data/flow` モジュール作成**
```
artifact_module(command: "create", module_name: "data/flow",
  content: <Step1で分析したsub-issue一覧・依存関係。MESSAGESは空配列でよい>)
```

## 検証

```
artifact(command: "outline")
```

で構造確認。エントリポイント + 4モジュール(`lib/stopConditions`, `components/TaskNode`, `components/DependencyEdge`, `data/flow`)が揃っていれば完了。

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

**停止条件のクリア**(子からクリア報告を受けたとき、または親自身の停止条件にユーザーが回答したとき):
```
artifact_module(command: "update", module_name: "data/flow",
  old_str: "{ id: 'sc-142-1', text: '追加テーブルのスキーマが確定したら確認', checked: false }",
  new_str: "{ id: 'sc-142-1', text: '追加テーブルのスキーマが確定したら確認', checked: true, checkedAt: '2026-09-05 14:30' }")
```

これにより、ユーザーはoretachi UIで計画フローartifactを開くだけで最新の進捗と、どこで人の判断待ちになっているかを確認できる。
