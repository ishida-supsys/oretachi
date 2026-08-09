<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open, ask, message } from "@tauri-apps/plugin-dialog";
import { useI18n } from "vue-i18n";
import {
  useSettings,
  migrateHomeWorktree,
  setupHomeClaudeDir,
} from "../composables/useSettings";
import { useWorktrees } from "../composables/useWorktrees";

const { t } = useI18n();
const { settings, scheduleSave } = useSettings();
const { syncWorktreesFromSettings } = useWorktrees();

const emit = defineEmits<{ close: [] }>();

// 設定タブから移設した2項目。どちらも「ローカル ref に溜めて保存時に反映」ではなく即時反映にしている。
// selectWorktreeBaseDir は migrateHomeWorktree → syncWorktreesFromSettings という
// 副作用の順序に意味があるため、保存ボタン待ちにすると壊れる。
async function selectWorktreeBaseDir() {
  const selected = await open({ directory: true, multiple: false });
  if (typeof selected === "string") {
    settings.value.worktreeBaseDir = selected;
    // ホームワークツリーを生成・追従させ、その .claude/ (プラグイン設定 + 同梱スキル) を用意する。
    // 既存のスキルファイルは上書きしない。
    // scheduleSave が emit する settings-changed は自ウィンドウでは無視されるため、
    // ランタイム配列は自分で同期する（片方だけ変異させると再起動までカードとタブに出ない）。
    if (migrateHomeWorktree(settings.value)) {
      syncWorktreesFromSettings();
    }
    scheduleSave();
    await setupHomeClaudeDir(selected);
  }
}

const reseedResult = ref("");

// プロンプト未設定時に実際に注入される既定値（Rust 側の定義を唯一の出所にする）
const defaultHomeAgentPrompt = ref("");
onMounted(async () => {
  defaultHomeAgentPrompt.value = await invoke<string>("get_default_home_agent_prompt").catch(() => "");
});

/** 同梱スキルを既定内容へ戻す（ユーザーが編集したファイルも上書きする） */
async function reseedHomeSkills() {
  const baseDir = settings.value.worktreeBaseDir;
  if (!baseDir) return;
  const ok = await ask(t("homeAgent.reseedConfirm"), { kind: "warning" });
  if (!ok) return;
  try {
    const count = await invoke<number>("setup_home_claude_dir", { homePath: baseDir, overwrite: true });
    reseedResult.value = t("homeAgent.reseedDone", { count });
  } catch (e) {
    await message(t("homeAgent.reseedFailed", { error: e }), { kind: "error" });
  }
}

function onPromptChange(e: Event) {
  const v = (e.target as HTMLTextAreaElement).value.trim();
  settings.value.homeAgentPrompt = v ? v : undefined;
  scheduleSave();
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('close')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title') }}</h3>

      <div class="field">
        <label class="label">{{ t('worktreeBaseDir.label') }}</label>
        <div class="row">
          <input
            class="text-in"
            :value="settings.worktreeBaseDir"
            readonly
            :placeholder="t('common.notConfigured')"
          />
          <button class="btn-secondary" @click="selectWorktreeBaseDir">
            {{ t('worktreeBaseDir.select') }}
          </button>
        </div>
        <p class="hint">{{ t('worktreeBaseDir.homeDesc') }}</p>
      </div>

      <template v-if="settings.worktreeBaseDir">
        <div class="divider" />

        <div class="field">
          <label class="label">{{ t('homeAgent.label') }}</label>
          <p class="hint hint-top">{{ t('homeAgent.desc') }}</p>
          <textarea
            class="textarea"
            rows="6"
            :value="settings.homeAgentPrompt ?? ''"
            :placeholder="defaultHomeAgentPrompt"
            @change="onPromptChange"
          />
        </div>

        <div class="field">
          <div class="row">
            <button class="btn-secondary" @click="reseedHomeSkills">
              {{ t('homeAgent.reseedSkills') }}
            </button>
            <span v-if="reseedResult" class="hint">{{ reseedResult }}</span>
          </div>
          <p class="hint">{{ t('homeAgent.reseedSkillsDesc') }}</p>
        </div>
      </template>

      <div class="dialog-actions">
        <div class="spacer" />
        <button class="btn-primary" @click="emit('close')">{{ t('closeButton') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 10px;
  padding: 24px;
  width: 560px;
  max-width: 90vw;
  max-height: 88vh;
  overflow-y: auto;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
  margin: 0 0 16px;
}

.field {
  margin-bottom: 14px;
}

.label {
  display: block;
  font-size: 12px;
  color: #a6adc8;
  margin-bottom: 5px;
}

.row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.text-in,
.textarea {
  width: 100%;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 10px;
  font-size: 13px;
  color: #cdd6f4;
  outline: none;
  box-sizing: border-box;
}

.textarea {
  resize: vertical;
  font-family: monospace;
  font-size: 12px;
  line-height: 1.5;
}

.btn-secondary {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.btn-secondary:hover {
  background: #45475a;
}

.btn-primary {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 5px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.divider {
  height: 1px;
  background: #313244;
  margin: 16px 0;
}

.hint {
  font-size: 11px;
  color: #6c7086;
  margin: 5px 0 0;
  line-height: 1.6;
}

.hint-top {
  margin: 0 0 6px;
}

.dialog-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 20px;
}

.spacer {
  flex: 1;
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Home & management agent",
    "closeButton": "Close",
    "common": {
      "notConfigured": "Not configured"
    },
    "worktreeBaseDir": {
      "label": "Worktree base directory",
      "select": "Select",
      "homeDesc": "This directory is also the working directory of the \"home\" worktree, where the management agent runs. No home worktree is created while this is unset."
    },
    "homeAgent": {
      "label": "Home management agent",
      "desc": "Injected into every Claude Code session started in the home worktree (via the SessionStart hook, so it survives /clear and restarts). Just run `claude` there and it becomes the worktree management agent. Leave empty to use the default shown below; the actual procedures live as skills under .claude/skills/ in the base directory.",
      "reseedSkills": "Restore bundled skills",
      "reseedSkillsDesc": "Bundled skills (worktree-cleanup / report / assign) are written once and never overwrite your edits. This button restores them to the defaults; skills you added yourself are left untouched.",
      "reseedConfirm": "Overwrite the bundled skill files with their default contents? Your edits to those files will be lost.",
      "reseedDone": "Restored {count} skill file(s).",
      "reseedFailed": "Failed to restore skills: {error}"
    }
  },
  "ja": {
    "title": "ホーム・管理エージェント設定",
    "closeButton": "閉じる",
    "common": {
      "notConfigured": "未設定"
    },
    "worktreeBaseDir": {
      "label": "ワークツリーの追加先ディレクトリ",
      "select": "選択",
      "homeDesc": "このディレクトリは、管理エージェントが動く「home」ワークツリーの作業ディレクトリにもなります。未設定のあいだ home は作られません。"
    },
    "homeAgent": {
      "label": "ホームの管理エージェント",
      "desc": "home で起動した Claude Code セッションに常時注入されます（SessionStart フック経由。/clear や再起動後も維持）。home で claude を起動するだけでワークツリー管理エージェントになります。空なら薄く表示されている既定値が使われます。実際の手順は追加先ディレクトリの .claude/skills/ にスキルとして置かれます。",
      "reseedSkills": "同梱スキルを再展開",
      "reseedSkillsDesc": "同梱スキル (worktree-cleanup / report / assign) は初回のみ書き出され、以降ユーザーの編集を上書きしません。このボタンは既定内容に戻します。自分で追加したスキルには触れません。",
      "reseedConfirm": "同梱スキルのファイルを既定内容で上書きしますか？ これらのファイルへの編集は失われます。",
      "reseedDone": "{count} 件のスキルファイルを再展開しました。",
      "reseedFailed": "スキルの再展開に失敗しました: {error}"
    }
  }
}
</i18n>
