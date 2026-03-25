<script setup lang="ts">
import { useSettings } from "../../composables/useSettings";
import HotkeyInput from "../HotkeyInput.vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { settings, scheduleSave } = useSettings();
</script>

<template>
  <div class="field-group">
    <label class="field-label">{{ t('hotkeys.label') }}</label>
    <div class="row-input row-input--inline">
      <input
        id="auto-assign-hotkey"
        type="checkbox"
        class="toggle-checkbox"
        :checked="settings.autoAssignHotkey"
        @change="(e) => { settings.autoAssignHotkey = (e.target as HTMLInputElement).checked; scheduleSave(); }"
      />
      <label for="auto-assign-hotkey" class="inline-label toggle-label">{{ t('hotkeys.autoAssign') }}</label>
    </div>
    <table v-if="settings.hotkeys" class="hotkey-table">
      <thead>
        <tr>
          <th class="hotkey-th">{{ t('hotkeys.action') }}</th>
          <th class="hotkey-th">{{ t('hotkeys.key') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.trayPopup') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.globalTrayPopup"
              @update:model-value="(v) => { settings.hotkeys.globalTrayPopup = v; scheduleSave(); }"
            />
          </td>
        </tr>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.terminalNext') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.terminalNext"
              @update:model-value="(v) => { settings.hotkeys.terminalNext = v; scheduleSave(); }"
            />
          </td>
        </tr>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.terminalPrev') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.terminalPrev"
              @update:model-value="(v) => { settings.hotkeys.terminalPrev = v; scheduleSave(); }"
            />
          </td>
        </tr>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.terminalAdd') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.terminalAdd"
              @update:model-value="(v) => { settings.hotkeys.terminalAdd = v; scheduleSave(); }"
            />
          </td>
        </tr>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.terminalClose') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.terminalClose"
              @update:model-value="(v) => { settings.hotkeys.terminalClose = v; scheduleSave(); }"
            />
          </td>
        </tr>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.trayNext') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.trayNext"
              @update:model-value="(v) => { settings.hotkeys.trayNext = v; scheduleSave(); }"
            />
          </td>
        </tr>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.homeTab') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.homeTab"
              @update:model-value="(v) => { settings.hotkeys.homeTab = v; scheduleSave(); }"
            />
          </td>
        </tr>
        <tr>
          <td class="hotkey-td-label">{{ t('hotkeys.addTask') }}</td>
          <td class="hotkey-td-input">
            <HotkeyInput
              :model-value="settings.hotkeys.addTask"
              @update:model-value="(v) => { settings.hotkeys.addTask = v; scheduleSave(); }"
            />
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "hotkeys": {
      "label": "Hotkeys",
      "autoAssign": "Auto-assign hotkeys",
      "action": "Action",
      "key": "Key",
      "trayPopup": "Show Tray Popup (global)",
      "terminalNext": "Switch terminal: next",
      "terminalPrev": "Switch terminal: prev",
      "terminalAdd": "Add terminal",
      "terminalClose": "Close terminal",
      "trayNext": "Next notification (tray)",
      "homeTab": "Home tab",
      "addTask": "Add task"
    }
  },
  "ja": {
    "hotkeys": {
      "label": "ホットキー",
      "autoAssign": "ホットキー自動割り当て",
      "action": "操作",
      "key": "キー",
      "trayPopup": "Tray Popup 表示 (グローバル)",
      "terminalNext": "ターミナル切り替え: 次",
      "terminalPrev": "ターミナル切り替え: 前",
      "terminalAdd": "ターミナル追加",
      "terminalClose": "ターミナルを閉じる",
      "trayNext": "次の通知へ (トレイ)",
      "homeTab": "ホームタブへ戻る",
      "addTask": "タスク追加"
    }
  }
}
</i18n>

<style scoped>
.toggle-checkbox {
  width: 16px;
  height: 16px;
  accent-color: #cba6f7;
  cursor: pointer;
  flex: unset;
}

.toggle-label {
  cursor: pointer;
}
</style>
