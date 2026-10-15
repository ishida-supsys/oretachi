<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { AppGrantRequest } from "../types/appInstance";
import { buildPermissionView } from "../utils/appPermissionView";

const { t } = useI18n();

const props = defineProps<{
  request: AppGrantRequest;
  /** 他にも待っている要求の数（このダイアログの後に続く） */
  waiting: number;
  busy: boolean;
}>();

const emit = defineEmits<{
  approve: [];
  reject: [];
}>();

const view = computed(() =>
  buildPermissionView(props.request.resolved, props.request.previous, props.request.dangerousTools),
);

/** 最も強い警告（ワイルドカードの越境先・既定拒否パスの解除・専用 AI ターミナル）を含むときは、確認チェックを必須にする */
const understood = ref(false);
watch(
  () => props.request.requestId,
  () => (understood.value = false),
);
const canApprove = computed(() => !props.busy && (!view.value.hasCritical || understood.value));

const isChange = computed(() => props.request.kind === "change");
const hasAnyRow = computed(() => view.value.rows.length > 0);
const hasWildcard = computed(() => view.value.rows.some((r) => r.wildcard));
const hasSensitive = computed(() => view.value.rows.some((r) => r.kind === "fsSensitive" || r.kind === "dbSensitive"));
const hasAiTerminal = computed(() => view.value.rows.some((r) => r.kind === "aiTerminal"));
</script>

<template>
  <div class="dialog-overlay">
    <div class="dialog" role="dialog" aria-modal="true">
      <h3 class="dialog-title">
        {{ isChange ? t("titleChange", { name: request.appName }) : t("titleStart", { name: request.appName }) }}
      </h3>

      <div class="meta">
        <div><span class="label">{{ t("worktree") }}</span> {{ request.worktreeName }}</div>
        <div>
          <span class="label">{{ t("artifact") }}</span> {{ request.artifactTitle || request.artifactId }}
          <code class="id">{{ request.artifactId }}</code>
        </div>
        <div v-if="request.appVersion"><span class="label">{{ t("version") }}</span> v{{ request.appVersion }}</div>
        <div v-if="request.source?.skill">
          <span class="label">{{ t("source") }}</span> {{ request.source.skill }}
          <template v-if="request.source.version"> (v{{ request.source.version }})</template>
        </div>
        <div v-if="request.appDescription" class="desc">{{ request.appDescription }}</div>
      </div>

      <p class="notice">{{ isChange ? t("noticeChange") : t("noticeStart") }}</p>

      <div class="perm-box">
        <h4>{{ isChange ? t("permsChange") : t("perms") }}</h4>

        <ul v-if="hasAnyRow" class="perm-list">
          <li
            v-for="r in view.rows"
            :key="r.kind + r.value"
            :class="[r.severity, { added: r.added }]"
          >
            <span v-if="r.added" class="badge-new">{{ t("added") }}</span>
            <template v-if="r.kind === 'tool'">
              <span class="kind">{{ t("kindTool") }}</span>
              <code>{{ r.value }}</code>
              <div v-if="r.severity === 'danger'" class="why">{{ t("dangerTool") }}</div>
            </template>
            <template v-else-if="r.kind === 'target'">
              <span class="kind">{{ t("kindTarget") }}</span>
              <strong>{{ r.label }}</strong>
              <div v-if="r.wildcard" class="why">{{ t("wildcardWarning") }}</div>
              <div v-else class="why neutral">{{ t("pinnedToId") }}</div>
            </template>
            <template v-else-if="r.kind === 'aiTerminal'">
              <span class="kind">{{ t("kindAiTerminal") }}</span>
              <strong>{{ t("aiTerminalSummary", { spec: r.value }) }}</strong>
              <div class="why">{{ t("aiTerminalWarning") }}</div>
            </template>
            <template v-else-if="r.kind === 'fs'">
              <span class="kind">{{ r.access === "write" ? t("kindFsWrite") : t("kindFsRead") }}</span>
              <code class="path">{{ r.label }}</code>
              <div class="why neutral">
                {{ r.scheme === "worktree" ? t("fsWorktree", { name: request.worktreeName }) : t("fsArtifact") }}
              </div>
              <div v-if="r.access === 'write'" class="why">{{ t("fsWriteWarning") }}</div>
              <div v-else-if="r.scheme === 'worktree'" class="why neutral">{{ t("fsReadNote") }}</div>
            </template>
            <template v-else-if="r.kind === 'fsSensitive'">
              <span class="kind">{{ t("kindFsSensitive") }}</span>
              <strong>{{ t("fsSensitiveTitle") }}</strong>
              <div class="why">{{ t("fsSensitiveWarning") }}</div>
            </template>
            <template v-else-if="r.kind === 'db'">
              <span class="kind">{{ r.access === "write" ? t("kindDbWrite") : t("kindDbRead") }}</span>
              <code class="path">{{ r.label }}</code>
              <div class="why neutral">
                {{ r.scheme === "worktree" ? t("fsWorktree", { name: request.worktreeName }) : t("fsArtifact") }}
              </div>
              <div v-if="r.access === 'write'" class="why">{{ t("dbWriteWarning") }}</div>
              <div v-else class="why neutral">{{ t("dbReadNote") }}</div>
            </template>
            <template v-else-if="r.kind === 'dbSensitive'">
              <span class="kind">{{ t("kindDbSensitive") }}</span>
              <strong>{{ t("dbSensitiveTitle") }}</strong>
              <div class="why">{{ t("dbSensitiveWarning") }}</div>
            </template>
            <template v-else>
              <span class="kind">{{ t("kindMemory") }}</span>
              {{ t("memoryMax", { bytes: r.value }) }}
            </template>
          </li>
        </ul>
        <p v-else class="none">{{ t("noPerms") }}</p>

        <p v-if="view.rows.some((r) => r.kind === 'tool') && view.ownWorktreeOnly" class="own-only">
          {{ t("ownOnly", { name: request.worktreeName }) }}
        </p>

        <div v-if="view.removed.length > 0" class="removed">
          <div class="removed-title">{{ t("removed") }}</div>
          <ul>
            <li v-for="r in view.removed" :key="r.kind + r.value">
              <s v-if="r.kind === 'target'">{{ r.label }}</s>
              <s v-else-if="r.kind === 'memory'">{{ t("memoryMax", { bytes: r.value }) }}</s>
              <s v-else-if="r.kind === 'aiTerminal'">{{ t("aiTerminalSummary", { spec: r.value }) }}</s>
              <s v-else-if="r.kind === 'fs' || r.kind === 'db'"><code>{{ r.kind }} {{ r.access }} {{ r.label }}</code></s>
              <s v-else><code>{{ r.value }}</code></s>
            </li>
          </ul>
        </div>
      </div>

      <label v-if="view.hasCritical" class="understand">
        <input v-model="understood" type="checkbox" />
        <span>
          <template v-if="hasWildcard">{{ t("understand") }}</template>
          <template v-if="hasWildcard && hasSensitive"> </template>
          <template v-if="hasSensitive">{{ t("understandSensitive") }}</template>
          <template v-if="hasAiTerminal && (hasWildcard || hasSensitive)"> </template>
          <template v-if="hasAiTerminal">{{ t("understandAiTerminal") }}</template>
        </span>
      </label>

      <p v-if="isChange" class="hint">{{ t("changeHint") }}</p>
      <p v-if="waiting > 0" class="hint">{{ t("waiting", { count: waiting }) }}</p>

      <div class="dialog-actions">
        <button class="btn-cancel" :disabled="busy" @click="emit('reject')">
          {{ isChange ? t("rejectChange") : t("reject") }}
        </button>
        <button class="btn-approve" :disabled="!canApprove" @click="emit('approve')">
          {{ isChange ? t("approveChange") : t("approve") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.7);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 300;
}
.dialog {
  background: #1e1e2e;
  border: 1px solid #45475a;
  border-radius: 10px;
  padding: 22px 24px;
  width: 520px;
  max-width: 94vw;
  max-height: 90vh;
  overflow-y: auto;
  color: #cdd6f4;
}
.dialog-title {
  font-size: 16px;
  font-weight: 600;
  margin: 0 0 12px;
  color: #f9e2af;
}
.meta {
  font-size: 12px;
  line-height: 1.7;
  margin-bottom: 10px;
}
.meta .label {
  display: inline-block;
  min-width: 78px;
  color: #6c7086;
}
.meta .id {
  margin-left: 6px;
  color: #6c7086;
  font-size: 11px;
}
.meta .desc {
  margin-top: 4px;
  color: #a6adc8;
}
.notice {
  margin: 0 0 12px;
  padding: 8px 12px;
  border-radius: 4px;
  background: #3a331e;
  color: #f9e2af;
  font-size: 12px;
}
.perm-box {
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 10px 12px;
  background: #181825;
  margin-bottom: 12px;
}
.perm-box h4 {
  margin: 0 0 8px;
  font-size: 12px;
  color: #a6adc8;
}
.perm-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.perm-list li {
  font-size: 13px;
  padding: 6px 8px;
  border-radius: 4px;
  border-left: 3px solid #45475a;
  background: #1e1e2e;
}
.perm-list li.danger {
  border-left-color: #fab387;
  background: #2a2119;
}
.perm-list li.critical {
  border-left-color: #f38ba8;
  background: #2f1b22;
}
.perm-list li.added {
  outline: 1px solid #89b4fa;
}
.kind {
  display: inline-block;
  margin-right: 8px;
  font-size: 11px;
  color: #6c7086;
}
.perm-list code {
  font-family: monospace;
}
.badge-new {
  float: right;
  font-size: 10px;
  padding: 0 6px;
  border-radius: 3px;
  background: #89b4fa;
  color: #1e1e2e;
}
.why {
  margin-top: 3px;
  font-size: 11px;
  color: #fab387;
}
.critical .why {
  color: #f38ba8;
  font-weight: 600;
}
.why.neutral {
  color: #6c7086;
  font-weight: normal;
}
.own-only,
.none {
  margin: 8px 0 0;
  font-size: 12px;
  color: #6c7086;
}
.removed {
  margin-top: 10px;
  font-size: 12px;
  color: #6c7086;
}
.removed ul {
  margin: 2px 0 0;
  padding-left: 18px;
}
.understand {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 12px;
  font-size: 12px;
  color: #f38ba8;
  cursor: pointer;
}
.understand input {
  margin-top: 2px;
  accent-color: #f38ba8;
}
.hint {
  margin: 0 0 10px;
  font-size: 11px;
  color: #6c7086;
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.btn-cancel {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  cursor: pointer;
}
.btn-cancel:hover:not(:disabled) {
  background: #45475a;
}
.btn-approve {
  background: #a6e3a1;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}
.btn-approve:hover:not(:disabled) {
  background: #94d68f;
}
.btn-approve:disabled,
.btn-cancel:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

<i18n lang="json">
{
  "en": {
    "titleStart": "Allow \"{name}\" to run with these permissions?",
    "titleChange": "\"{name}\" is asking for different permissions",
    "worktree": "Worktree",
    "artifact": "Artifact",
    "version": "Version",
    "source": "Created from",
    "noticeStart": "This app's manifest was written by an AI. Nothing below is granted until you approve it here, and the app can never use anything that is not listed.",
    "noticeChange": "The permissions in the manifest changed. The app keeps running with the previously approved permissions until you decide; calls that need the new ones are rejected.",
    "perms": "Permissions requested",
    "permsChange": "New permissions",
    "added": "new",
    "kindTool": "MCP tool",
    "kindTarget": "Reach worktree",
    "kindMemory": "Memory",
    "kindAiTerminal": "AI terminal",
    "aiTerminalSummary": "Open dedicated AI terminals ({spec})",
    "aiTerminalWarning": "Starts an agent that runs with the same authority as you (it can run arbitrary commands and edit files in this worktree). The app can type into it, read its output and answer its permission dialogs. This is the strongest permission.",
    "kindFsRead": "Read files",
    "kindFsWrite": "Write files",
    "kindFsSensitive": "Files",
    "kindDbRead": "Read database (read-only)",
    "kindDbWrite": "Read and write database",
    "kindDbSensitive": "Database",
    "dbWriteWarning": "The app can create SQLite database files here and run any SQL on them (create, change and delete data). It cannot ATTACH other files, VACUUM INTO a file or load extensions.",
    "dbReadNote": "Opened read-only: existing databases can be queried, but nothing can be written or created. Everything stored inside is readable.",
    "dbSensitiveTitle": "Allowed to put databases in files that run commands (.git/, .claude/, .github/workflows, ...)",
    "dbSensitiveWarning": "Normally refused, even read-only: .git/, .github/, .claude/, .vscode/, .idea/, .husky/, .devcontainer/, .mcp.json, .envrc, CLAUDE.md, AGENTS.md and similar agent/tool config files. A database there is a file the tools may read or run. This is the strongest permission.",
    "dangerTool": "Same authority as you (can run arbitrary commands in a terminal or start agents)",
    "wildcardWarning": "Includes worktrees created in the future, and reaches terminals in the main clone as well. This is the strongest permission.",
    "pinnedToId": "Fixed to this worktree by its ID at approval time (renaming it does not change the target)",
    "memoryMax": "Persistent memory up to {bytes} bytes",
    "fsWorktree": "Inside the worktree \"{name}\" (this app's own worktree). Everything outside this folder, including through links, is refused.",
    "fsArtifact": "This app's private storage area. It is deleted together with the artifact.",
    "fsWriteWarning": "The app can create, overwrite and delete files here.",
    "fsReadNote": "Reading is not filtered: secrets stored inside (such as .env or .git/config) are readable too.",
    "fsSensitiveTitle": "Allowed to write to files that run commands (.git/hooks, .claude/, .github/workflows, ...)",
    "fsSensitiveWarning": "Normally refused: .git/, .github/, .claude/, .vscode/, .idea/, .husky/, .devcontainer/, .mcp.json, .envrc, CLAUDE.md, AGENTS.md and similar agent/tool config files. Writing there lets the app make commands run later with your authority (git hooks, Claude Code hooks, CI, editor tasks). This is the strongest permission.",
    "noPerms": "No permissions requested. The app cannot call any API.",
    "ownOnly": "MCP calls reach only \"{name}\" (this app's own worktree).",
    "removed": "No longer requested",
    "understand": "I understand this lets the app reach worktrees I did not choose individually.",
    "understandSensitive": "I understand this lets the app run commands with my authority by editing hook and workflow files.",
    "understandAiTerminal": "I understand this lets the app start an agent that runs with my authority and drive it.",
    "changeHint": "Denying stops the app.",
    "waiting": "{count} more approval request(s) are waiting.",
    "approve": "Allow and start",
    "reject": "Cancel",
    "approveChange": "Allow",
    "rejectChange": "Deny and stop the app"
  },
  "ja": {
    "titleStart": "「{name}」をこの権限で実行しますか？",
    "titleChange": "「{name}」が権限の変更を求めています",
    "worktree": "ワークツリー",
    "artifact": "アーティファクト",
    "version": "バージョン",
    "source": "生成元",
    "noticeStart": "このアプリの manifest は AI が書いたものです。ここで承認するまで下記は何も許可されず、一覧に無いものは一切使えません。",
    "noticeChange": "manifest の権限が変わりました。判断するまで、アプリは以前に承認した権限のまま動作し、新しい権限が要る呼び出しは拒否されます。",
    "perms": "申請されている権限",
    "permsChange": "新しい権限",
    "added": "追加",
    "kindTool": "MCP ツール",
    "kindTarget": "届くワークツリー",
    "kindMemory": "メモリー",
    "kindAiTerminal": "AI ターミナル",
    "aiTerminalSummary": "専用の AI ターミナルを開く（{spec}）",
    "aiTerminalWarning": "あなたと同じ権限で動くエージェントを起動します（このワークツリーで任意のコマンド実行やファイル編集ができます）。アプリはそのエージェントへ入力し、出力を読み、許可ダイアログに回答できます。最も強い権限です。",
    "kindFsRead": "ファイルの読み取り",
    "kindFsWrite": "ファイルの書き込み",
    "kindFsSensitive": "ファイル",
    "kindDbRead": "データベースの読み取り（読み取り専用）",
    "kindDbWrite": "データベースの読み書き",
    "kindDbSensitive": "データベース",
    "dbWriteWarning": "アプリはここに SQLite のデータベースを作り、任意の SQL（データの作成・変更・削除）を実行できます。他のファイルの ATTACH・VACUUM INTO・拡張のロードはできません。",
    "dbReadNote": "読み取り専用で開きます。既存のデータベースを検索できますが、書き込みも新規作成もできません。中にあるデータはすべて読めます。",
    "dbSensitiveTitle": "コマンドが走るファイル（.git/・.claude/・.github/workflows など）の場所へのデータベース配置を許可",
    "dbSensitiveWarning": "通常は読み取り専用でも拒否されるパスです: .git/ / .github/ / .claude/ / .vscode/ / .idea/ / .husky/ / .devcontainer/ / .mcp.json / .envrc / CLAUDE.md / AGENTS.md など、エージェントやツールの設定ファイル。ここにデータベースがあると、ツールが読んだり実行したりしうるファイルになります。最も強い権限です。",
    "dangerTool": "あなたと同等の権限です（端末で任意のコマンドを実行したり、エージェントを起動したりできます）",
    "wildcardWarning": "今後作られるワークツリーも含み、メインのクローンの端末にも届きます。最も強い権限です。",
    "pinnedToId": "承認時点の ID でこのワークツリーに固定されます（改名しても対象は変わりません）",
    "memoryMax": "永続メモリー 最大 {bytes} バイト",
    "fsWorktree": "ワークツリー「{name}」（このアプリ自身のワークツリー）の中だけです。このフォルダの外は、リンク経由でも拒否されます。",
    "fsArtifact": "このアプリ専用の保存領域です。アーティファクトを削除すると一緒に消えます。",
    "fsWriteWarning": "アプリはここでファイルを作成・上書き・削除できます。",
    "fsReadNote": "読み取りは絞り込まれません。範囲内にある秘密（.env や .git/config など）も読めます。",
    "fsSensitiveTitle": "コマンドが走るファイル（.git/hooks・.claude/・.github/workflows など）への書き込みを許可",
    "fsSensitiveWarning": "通常は拒否されるパスです: .git/ / .github/ / .claude/ / .vscode/ / .idea/ / .husky/ / .devcontainer/ / .mcp.json / .envrc / CLAUDE.md / AGENTS.md など、エージェントやツールの設定ファイル。ここへ書けると、あなたの権限で後からコマンドを実行させられます（git フック・Claude Code のフック・CI・エディタのタスク）。最も強い権限です。",
    "noPerms": "権限は申請されていません。アプリはどの API も呼べません。",
    "ownOnly": "MCP 呼び出しは「{name}」（このアプリ自身のワークツリー）だけに届きます。",
    "removed": "申請から外れたもの",
    "understand": "個別に選んでいないワークツリーにもアプリが届くことを理解しました。",
    "understandSensitive": "フックやワークフローのファイルを書き換えて、私の権限でコマンドを実行できてしまうことを理解しました。",
    "understandAiTerminal": "アプリが私の権限で動くエージェントを起動し、操作できることを理解しました。",
    "changeHint": "拒否するとアプリは停止します。",
    "waiting": "ほかに {count} 件の承認要求が待っています。",
    "approve": "許可して起動",
    "reject": "キャンセル",
    "approveChange": "許可",
    "rejectChange": "拒否してアプリを停止"
  }
}
</i18n>
