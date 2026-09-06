<script setup lang="ts">
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import { useI18n } from "vue-i18n";
import type { Worktree } from "../types/worktree";
import { useSettings, applyPluginConfig } from "../composables/useSettings";

const { t } = useI18n();

const props = defineProps<{
  worktree: Worktree;
  /** 自動承認の現在値 */
  autoApproval: boolean;
  /** トレイ通知の実効値（ワークツリー個別 > true で解決済み） */
  trayNotification: boolean;
  /** 割り当て済みホットキー文字（未割り当てなら undefined） */
  hotkeyChar?: string;
  /** 自動承認の AI 判定が進行中か（判定中はトグルを触らせない） */
  aiJudging?: boolean;
}>();

const emit = defineEmits<{
  toggleAutoApproval: [worktreeId: string];
  toggleTrayNotification: [worktreeId: string];
  setHotkeyChar: [worktreeId: string];
  close: [];
}>();

const isHome = computed(() => props.worktree.isHome === true);

const reapplyResult = ref("");
const reapplying = ref(false);

/**
 * このワークツリーの .claude/settings.local.json に oretachi プラグイン設定を書き直す。
 * 手編集や削除で MCP / 通知が効かなくなったときの復旧口。
 * ホームは同梱スキルのシードも伴うため専用コマンド経由にする。
 * 親へ emit せずここで完結させる（他の状態に影響しない自己完結の操作のため）。
 */
async function onReapplyPluginConfig() {
  const { settings } = useSettings();
  reapplyResult.value = "";
  reapplying.value = true;
  try {
    if (isHome.value) {
      // overwrite=false: プラグイン設定は毎回マージ書き込みされ、スキルの編集は温存される
      await invoke("setup_home_claude_dir", {
        homePath: props.worktree.path,
        overwrite: false,
      });
    } else {
      const repo = settings.value.repositories.find((r) => r.id === props.worktree.repositoryId);
      await applyPluginConfig(
        props.worktree.path,
        props.worktree.name,
        repo?.notificationHooks ?? [],
      );
    }
    reapplyResult.value = t("reapplyPlugin.done");
  } catch (e) {
    await message(t("reapplyPlugin.failed", { error: e }), { kind: "error" });
  } finally {
    reapplying.value = false;
  }
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('close')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title') }}</h3>
      <p class="dialog-sub">{{ worktree.name }}</p>

      <!-- 自動承認 -->
      <div class="field">
        <label class="checkbox-label">
          <input
            type="checkbox"
            :checked="autoApproval"
            :disabled="aiJudging"
            @change="emit('toggleAutoApproval', worktree.id)"
          />
          <span class="entry-text">{{ t('autoApproval.label') }}</span>
        </label>
        <p class="hint">{{ t('autoApproval.desc') }}</p>
      </div>

      <!-- トレイ通知 -->
      <div class="field">
        <label class="checkbox-label">
          <input
            type="checkbox"
            :checked="trayNotification"
            @change="emit('toggleTrayNotification', worktree.id)"
          />
          <span class="entry-text">{{ t('trayNotification.label') }}</span>
        </label>
        <p class="hint">{{ t('trayNotification.desc') }}</p>
      </div>

      <div class="divider" />

      <!-- ホットキー割り当て -->
      <div class="field">
        <div class="row">
          <button class="btn-secondary" @click="emit('setHotkeyChar', worktree.id)">
            {{ hotkeyChar ? t('hotkey.change') : t('hotkey.assign') }}
          </button>
          <span v-if="hotkeyChar" class="key-badge">Alt+{{ hotkeyChar.toUpperCase() }}</span>
          <span v-else class="hint">{{ t('hotkey.none') }}</span>
        </div>
        <p class="hint">{{ t('hotkey.desc') }}</p>
      </div>

      <!-- プラグイン設定を再適用 -->
      <div class="field">
        <div class="row">
          <button class="btn-secondary" :disabled="reapplying" @click="onReapplyPluginConfig">
            {{ t('reapplyPlugin.label') }}
          </button>
          <span v-if="reapplyResult" class="hint">{{ reapplyResult }}</span>
        </div>
        <p class="hint">{{ t('reapplyPlugin.desc') }}</p>
      </div>

      <div class="dialog-actions">
        <div class="spacer" />
        <button class="btn-primary" @click="emit('close')">{{ t('common.close') }}</button>
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
  width: 520px;
  max-width: 90vw;
  max-height: 88vh;
  overflow-y: auto;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
  margin: 0 0 4px;
}

.dialog-sub {
  font-size: 12px;
  color: #a6adc8;
  margin: 0 0 16px;
  word-break: break-all;
}

.field {
  margin-bottom: 14px;
}

.row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  padding: 4px 6px;
  border-radius: 4px;
}

.checkbox-label:hover {
  background: #313244;
}

.checkbox-label input[type="checkbox"] {
  accent-color: #cba6f7;
  cursor: pointer;
}

.checkbox-label:has(input:disabled) {
  opacity: 0.5;
  cursor: default;
}

.entry-text {
  font-size: 13px;
  color: #cdd6f4;
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

.btn-secondary:hover:not(:disabled) {
  background: #45475a;
}

.btn-secondary:disabled {
  opacity: 0.5;
  cursor: default;
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

.key-badge {
  background: #45475a;
  color: #f9e2af;
  border-radius: 4px;
  padding: 3px 8px;
  font-size: 12px;
  font-family: monospace;
}

.divider {
  height: 1px;
  background: #313244;
  margin: 16px 0;
}

.hint {
  font-size: 11px;
  color: #6c7086;
  margin: 5px 0 0 6px;
  line-height: 1.6;
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
    "title": "Worktree settings",
    "common": {
      "close": "Close"
    },
    "autoApproval": {
      "label": "Auto approval",
      "desc": "Automatically answers the approval prompts of the agent running in this worktree."
    },
    "trayNotification": {
      "label": "Tray notification",
      "desc": "Shows a desktop notification from the tray when this worktree needs attention."
    },
    "hotkey": {
      "assign": "Assign hotkey",
      "change": "Change hotkey",
      "none": "Not assigned",
      "desc": "Switches to this worktree with Alt + the assigned key."
    },
    "reapplyPlugin": {
      "label": "Re-apply plugin settings",
      "desc": "Writes the oretachi plugin settings into .claude/settings.local.json again. Use this if the file was edited or deleted by hand and MCP tools / notifications stopped working. Other keys in the file are preserved.",
      "done": "Re-applied the plugin settings.",
      "failed": "Failed to re-apply the plugin settings: {error}"
    }
  },
  "ja": {
    "title": "ワークツリー設定",
    "common": {
      "close": "閉じる"
    },
    "autoApproval": {
      "label": "自動承認",
      "desc": "このワークツリーで動くエージェントの承認プロンプトに自動で応答します。"
    },
    "trayNotification": {
      "label": "トレイ通知",
      "desc": "このワークツリーが応答待ちになったとき、トレイからデスクトップ通知を出します。"
    },
    "hotkey": {
      "assign": "ホットキーを割り当て",
      "change": "ホットキーを変更",
      "none": "未割り当て",
      "desc": "Alt + 割り当てたキーでこのワークツリーに切り替えます。"
    },
    "reapplyPlugin": {
      "label": "プラグイン設定を再適用",
      "desc": ".claude/settings.local.json に oretachi プラグイン設定を書き直します。手で編集・削除して MCP ツールや通知が効かなくなったときに使います。ファイル内の他のキーは保持されます。",
      "done": "プラグイン設定を再適用しました。",
      "failed": "プラグイン設定の再適用に失敗しました: {error}"
    }
  }
}
</i18n>
