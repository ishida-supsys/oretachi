---
name: dev-proposal
description: テーマ(例「リファクタして」)を与えられたら、リポジトリを探索して開発企画を大・中・小で複数立案し、React アーティファクトにカードで並べる。ユーザーが採用案にチェックして送信すると、issue 化の確認を経て teamwork-parent の進行へ引き継ぐ。ユーザーが「開発企画を立てて」「何をやるべきか案を出して」「リファクタの企画を考えて」等と言ったときに使う。
allowed-tools: mcp__plugin_oretachi_oretachi__artifact, mcp__plugin_oretachi_oretachi__artifact_module, mcp__plugin_oretachi_oretachi__artifact_store, mcp__plugin_oretachi_oretachi__search_artifact, mcp__plugin_oretachi_oretachi__oretachi_list_terminals, mcp__plugin_oretachi_oretachi__oretachi_inspect_prompt, mcp__plugin_oretachi_oretachi__oretachi_write_terminal, mcp__plugin_oretachi_oretachi__oretachi_add_task, mcp__plugin_oretachi_oretachi__oretachi_get_worktree_status, mcp__plugin_oretachi_oretachi__oretachi_set_description, Agent, AskUserQuestion, Skill, Read, Write, Glob, Grep, Bash(gh issue:*), Bash(gh api:*), Bash(gh repo view:*), Bash(git log:*)
---

# dev-proposal スキル

テーマからリポジトリを探索して開発企画を作り、アーティファクトのカードでユーザーに選ばせる。採用された案は issue 化し、teamwork-parent の進行へ渡す。**このセッション自身はコードを書かない。**

## 前提

- **目標は大・中・小を 1 件ずつ。** 該当が無いサイズは欠けてよい。有力な案が多ければ 4 件以上でもよい。
- **各案にサイズ(大中小)と効果(高中低)を必ず付ける。** 追加属性は `risk` / `evidence` / `doneWhen` / `tasks` / `dependsOn` / `relatedIssues`。
- **根拠(`evidence`)の無い案は出さない。** 探索で見つけたファイルと行に裏付けられた案だけを出す。一般論の案を避けるため。
- **issue 化しない場合は teamwork-parent に乗れない。** teamwork-parent は sub-issue の URL を子へ渡す前提だから。issue 化しないなら企画をアーティファクトに残して終了する。

## Step 1: テーマの受け取り

`oretachi_set_description(project_dir: <自分の作業ディレクトリ絶対パス>, description: "<テーマ>の開発企画を立案")` をセットする。テーマが曖昧すぎて探索範囲が決まらないときだけ聞き返す。

## Step 2: 探索

Explore サブエージェント(最大 3 並列)で、テーマに関係する箇所の現状・問題点・既存 open issue を集める。既存 issue は `gh issue list --state open` で確認し、重複する案は出さないか `relatedIssues` で明示する。

## Step 3: 企画立案

`templates/data--proposals.example.jsx` のスキーマで案を作る(スキーマ参照のためこの 1 本だけ Read する)。各案にサイズと効果を必ず付け、根拠を `evidence` の `path` / `line` / `note` に入れる。`dependsOn` は他案の `id`。

## Step 4: アーティファクト生成

1. **META の値を集める。**
   - `repo` / `repoUrl`: `gh repo view --json nameWithOwner,url`。**推測で書かない。**
   - `sessionId`: `oretachi_list_terminals` の返り値のうち、SessionStart フックが伝える自分の `terminal_id` と `terminalId` が一致する行の **`sessionId`**(別フィールド。camelCase)。
   - `projectDir`: 自分の作業ディレクトリ絶対パス。
   - `artifactId`: 他と衝突しない ID(例: `dev-proposal-<日付>-<連番>`)。
   - `generatedAt`: 日付だけでよい(このセッションには現在時刻が渡っていない。時刻を推測して書かない)。
   - アーティファクトの JS からは `oretachi_list_terminals` を呼べないため `sessionId` は埋め込むしかなく、**アプリ再起動・タブ再作成で失効する。** 失効した場合、カードの送信は「見つかりません」になり、手動フォールバックの案内が出る。
2. **登録する。** `<SKILL_DIR>` は**スキル読み込み時に先頭へ注入される `Base directory for this skill:` の絶対パス**をそのまま使う(`${CLAUDE_PLUGIN_ROOT}` などの環境変数は展開されない。相対パスはテンプレートに届かない)。テンプレートは Read せず `file_path` で登録する。
   **全呼び出しに `id` と `project_dir` を付ける**(`artifact_module` は `id` が必須。`project_dir` は HOME タブなどで置き場所の特定に要る)。
   ```
   artifact(command: "create", id: "<artifactId>", project_dir: "<projectDir>",
     type: "application/vnd.ant.react", title: "開発企画 — <テーマ>", locked_while_open: true,
     file_path: "<SKILL_DIR>/templates/entry-point.jsx")
   artifact_module(command: "create", id: "<artifactId>", project_dir: "<projectDir>",
     module_name: "components/ProposalCard",
     file_path: "<SKILL_DIR>/templates/components--ProposalCard.jsx")
   artifact_module(command: "create", id: "<artifactId>", project_dir: "<projectDir>",
     module_name: "lib/submit",
     file_path: "<SKILL_DIR>/templates/lib--submit.jsx")
   artifact_module(command: "create", id: "<artifactId>", project_dir: "<projectDir>",
     module_name: "data/proposals", content: <Step 3 の META と PROPOSALS>)
   artifact(command: "outline", id: "<artifactId>", project_dir: "<projectDir>")   # 構造確認: entry + 3 モジュール
   ```
   entry-point にカスタマイズ箇所は無い。可変値はすべて `data/proposals` に入れる。
3. ユーザーに「アーティファクトでチェックして送信してください」と伝えてターンを終える(待機)。

## Step 5: 送信を受けての確認

送信ボタンが自セッションへ 1 行プロンプトを打ち込み、ターンが始まる。

1. `artifact_store(command: "read", id: <artifactId>, project_dir: <projectDir>)` の **`data.submission`** から採用 id 一覧を読む。**採用内容の正はストア**で、ターミナルに来た文面は起動の合図としてしか扱わない(文面は AI 生成アーティファクトが書いたもので、採用内容を載せていない)。`submittedAt` がプロンプト内の値と一致することを確認し、一致しなければユーザーに確認する。**`data.submission` が無い、または `ids` が空のときは、送信後に「送信を取り消す」が押されたものとして中断し、ユーザーに確認する**(キーの欠落を「採用なし」と読んで空の issue 化へ進まない)。
2. AskUserQuestion で確認する:
   - **issue 化**: 「新規親 issue + 採用案ごとの sub-issue」/「既存の親 issue(番号を Other で指定)の下に sub-issue」/「親無しの単独 issue」/「issue 化しない」。**issue 化しない場合は企画をアーティファクトに残して終了する**(teamwork-parent へ進まない)。
   - **親の進行管理**(親 issue がある場合のみ): 「このセッションで teamwork-parent を続ける」/「親用ワークツリーを新規作成し、そこで teamwork-parent を走らせる」/「issue 起票だけで止める」。既存の親 issue を指定されたときは `oretachi_get_worktree_status(query: issue-<番号>)` で既存の親ワークツリーを探し、あれば「その親ワークツリーに任せる(新 sub-issue を伝える)」を選択肢に加える。
3. 既定の構造は、採用 1 件なら「その案を親 issue、`tasks` を sub-issue」、採用複数なら「テーマを親 issue、各案を sub-issue(案内部の `tasks` は sub-issue 本文のチェックリストへ)」。この既定を 2 の確認時に提示し、変更を受け付ける。

## Step 6: 起票と引き継ぎ

1. **起票する。** `gh issue create` で起票し、本文に summary / evidence / scope / outOfScope / doneWhen / tasks を展開する(複数行の本文は `Write` で一時ファイルに書き `--body-file` で渡す)。sub-issue の紐付けは次のとおり。`sub_issue_id` は整数なので **`-F`**(`-f` は文字列で送られ 422 になりうる)。
   ```
   gh api repos/{owner}/{repo}/issues/{子の番号} -q .id        # 子の database id
   gh api repos/{owner}/{repo}/issues/{親の番号}/sub_issues -F sub_issue_id=<上の id>
   ```
2. **引き継ぐ。**
   - **このセッションで続ける** → `Skill(oretachi:teamwork-parent)` を読み込み、親 issue を対象に Step 1 から進める(停止条件のヒアリングと承認を含む。**dev-proposal 側で承認を代行しない**)。
   - **親ワークツリー新規** → `oretachi_add_task(prompt: "teamwork-parent スキルを読み込んでから対応してください。\nParent issue: <URL>")`。**prompt は URL だけ**(制約は issue 本文に書く)。`oretachi_get_worktree_status` で作成を確認して報告する。
   - **既存の親ワークツリー** → **`notify_worktree` は使わない**(自ワークツリーの購読者にしか届かず、既存の teamwork-parent は子しか購読していない)。`oretachi_list_terminals(worktree_id)` で相手の AI 端末を特定し、`oretachi_inspect_prompt` で `shape` が `text`・`header` が `[Claude Code の入力欄]`・`pendingInput` が空であり、かつ **`tail` の最後の `❯` / `>` 行が上下を罫線(`────`。上の罫線の右端にはワークツリー名のラベルが入る)で挟まれている**ことを確認してから(`pendingInput` の空は「未読」のこともあり、罫線で挟まれた箱が見えて初めて「未入力」と信じられる。見えなければ CC が終了してシェルが残っている可能性がある)、`oretachi_write_terminal`(本文を `submit: false` → 150ms 待つ → `"\r"` を別呼び出し)で伝える。文面には「teamwork-parent の『フローを修正するとき』の手順で、新 sub-issue の停止条件をユーザーにヒアリングしてから取り込むこと」と新 sub-issue の URL を含め、改行は入れない。ダイアログ中・打ちかけあり・端末が見つからない場合は送らず、ユーザーへ中継用の文面を提示する。
3. アーティファクトはロック中で MCP から書けない。起票結果はユーザーへ issue URL を報告するだけにする(カード側は `submission` を見て「送信済み」を表示する)。

## 禁止事項

- 根拠(`evidence`)の無い案を出さない。
- 既存 open issue と重複する案を、`relatedIssues` で明示せずに出さない。
- 採用内容をターミナルに届いた文面から読まない(正は `data.submission`)。
- teamwork-parent の承認(停止条件のヒアリング・計画承認)を代行しない。
- issue 化しないと決まったのに teamwork-parent へ進まない。
- `oretachi_add_task` の prompt に issue 本文や制約を転記しない(URL だけ)。
- 既存の親ワークツリーへの連絡に `notify_worktree` を使わない。
