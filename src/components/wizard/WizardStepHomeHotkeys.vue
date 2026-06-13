<script setup lang="ts">
import { computed } from "vue";
import { platform } from "@tauri-apps/plugin-os";
import { useI18n } from "vue-i18n";
import { useSettings } from "../../composables/useSettings";
import { useLoopTick } from "../../composables/useLoopTick";
import { formatHotkey } from "../../composables/useHotkeys";
import WizardHomeMock from "./WizardHomeMock.vue";
import WizardPhaseStrip from "./WizardPhaseStrip.vue";

const { t } = useI18n();
const { settings } = useSettings();

const isMac = platform() === "macos";

// ── タスク追加フローのループアニメ (26 tick × 220ms ≈ 5.7秒/周) ──
// idle → ダイアログ出現 → タイプ → 追加押下 → ダイアログ閉じ + カード出現
const tick = useLoopTick(26, 220);

const dialogVisible = computed(() => tick.value >= 4 && tick.value <= 16);
const pressed = computed(() => tick.value >= 14 && tick.value <= 15);
const added = computed(() => tick.value >= 18);
const phase = computed(() =>
  tick.value < 4 ? 0 : tick.value < 14 ? 1 : tick.value < 18 ? 2 : 3,
);

const typedText = computed(() => {
  const full = t("taskText");
  if (tick.value < 6) return "";
  const len = Math.min(full.length, Math.ceil(((tick.value - 5) / 8) * full.length));
  return full.slice(0, len);
});

const phaseItems = computed(() => [t("phase1"), t("phase2"), t("phase3"), t("phase4")]);

const hotkeyRows = computed(() => [
  { desc: t("hkAddTask"), key: formatHotkey(settings.value.hotkeys.addTask) },
  { desc: t("hkFocusWorktree"), key: `${isMac ? "⌥" : "Alt+"}[${t("assignedChar")}]` },
  { desc: t("hkHomeTab"), key: formatHotkey(settings.value.hotkeys.homeTab) },
]);
</script>

<template>
  <div class="step">
    <p class="desc">{{ t('desc') }}</p>

    <!-- ループアニメーション: ダイアログ表示 → 入力 → 追加 → ワークツリー出現 -->
    <WizardHomeMock :height="150">
      <template #cards>
        <div class="mock-card-new" :class="{ 'mock-card-new--visible': added }">
          <div class="mock-card-new-name">fix-login ✦</div>
          <div class="mock-card-new-lines">─────────<br />────────────</div>
        </div>
      </template>

      <!-- タスク追加ダイアログ -->
      <div class="mock-dialog-overlay" :class="{ 'mock-dialog-overlay--visible': dialogVisible }">
        <div class="mock-dialog" :class="{ 'mock-dialog--visible': dialogVisible }">
          <div class="mock-dialog-title">{{ t('mockDialogTitle') }}</div>
          <div class="mock-textarea">
            {{ typedText }}<span class="mock-caret" :class="{ 'mock-caret--off': tick % 2 === 1 }">▍</span>
          </div>
          <div class="mock-dialog-actions">
            <span class="mock-btn-cancel">{{ t('mockCancel') }}</span>
            <span class="mock-btn-add" :class="{ 'mock-btn-add--pressed': pressed }">{{ t('mockAdd') }}</span>
          </div>
        </div>
      </div>
    </WizardHomeMock>

    <WizardPhaseStrip :items="phaseItems" :active="phase" />

    <div class="section-label">{{ t('hotkeysLabel') }}</div>
    <div v-for="row in hotkeyRows" :key="row.desc" class="hotkey-row">
      <span class="hotkey-desc">{{ row.desc }}</span>
      <kbd class="kbd">{{ row.key }}</kbd>
    </div>
  </div>
</template>

<style scoped>
.step {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.desc {
  margin: 0;
  font-size: 13px;
  color: #cdd6f4;
  line-height: 1.8;
}

.section-label {
  margin-top: 4px;
  font-size: 12px;
  font-weight: 700;
  color: #a6adc8;
  letter-spacing: 0.5px;
}

/* ── 新規ワークツリーカード (ポップイン) ── */
.mock-card-new {
  flex: 1;
  border: 1px dashed #cba6f7;
  border-radius: 6px;
  padding: 8px 10px;
  background: rgba(203, 166, 247, 0.08);
  align-self: flex-start;
  opacity: 0;
  transform: scale(0.6);
  transition: all 0.45s cubic-bezier(0.34, 1.56, 0.64, 1);
}

.mock-card-new--visible {
  opacity: 1;
  transform: scale(1);
}

.mock-card-new-name {
  font-size: 11px;
  font-weight: 700;
  color: #cba6f7;
}

.mock-card-new-lines {
  font-size: 9px;
  color: #45475a;
  margin-top: 3px;
}

/* ── タスク追加ダイアログのモック ── */
.mock-dialog-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  opacity: 0;
  pointer-events: none;
  transition: all 0.3s;
}

.mock-dialog-overlay--visible {
  background: rgba(0, 0, 0, 0.5);
  opacity: 1;
}

.mock-dialog {
  width: 280px;
  background: #1e1e2e;
  border: 1px solid #45475a;
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  padding: 10px 12px;
  transform: scale(0.85) translateY(8px);
  transition: transform 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
}

.mock-dialog--visible {
  transform: scale(1) translateY(0);
}

.mock-dialog-title {
  font-size: 11px;
  font-weight: 700;
  color: #cba6f7;
  margin-bottom: 6px;
}

.mock-textarea {
  border: 1px solid #45475a;
  border-radius: 5px;
  padding: 6px 8px;
  font-size: 11px;
  color: #cdd6f4;
  min-height: 30px;
  background: #313244;
  line-height: 1.5;
}

.mock-caret {
  color: #cba6f7;
}

.mock-caret--off {
  opacity: 0;
}

.mock-dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 8px;
}

.mock-btn-cancel {
  font-size: 10.5px;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 4px 10px;
  color: #a6adc8;
}

.mock-btn-add {
  font-size: 10.5px;
  border-radius: 4px;
  padding: 4px 14px;
  color: #1e1e2e;
  font-weight: 700;
  background: #cba6f7;
  transition: all 0.15s;
}

.mock-btn-add--pressed {
  background: #b4befe;
  transform: scale(0.92);
  box-shadow: 0 0 0 3px rgba(203, 166, 247, 0.35);
}

/* ── ホットキー行 ── */
.hotkey-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border: 1px solid #313244;
  border-radius: 6px;
  background: #181825;
}

.hotkey-desc {
  font-size: 12.5px;
  color: #cdd6f4;
}

.kbd {
  border: 1px solid #45475a;
  border-bottom-width: 2px;
  border-radius: 4px;
  padding: 3px 9px;
  font-size: 12px;
  font-family: monospace;
  background: #313244;
  color: #cdd6f4;
  font-weight: 700;
  white-space: nowrap;
}
</style>

<i18n lang="json">
{
  "en": {
    "desc": "From the home tab, add a task and the AI will create a worktree and start working on it right away.",
    "taskText": "Fix the bug on the login screen",
    "mockDialogTitle": "Add Task",
    "mockCancel": "Cancel",
    "mockAdd": "Add",
    "phase1": "Open the Add Task dialog",
    "phase2": "Type what you want done",
    "phase3": "Press \"Add\"",
    "phase4": "A worktree is created & runs",
    "hotkeysLabel": "Hotkeys",
    "hkAddTask": "Add a task",
    "hkFocusWorktree": "Jump to a worktree",
    "hkHomeTab": "Back to the home tab",
    "assignedChar": "key"
  },
  "ja": {
    "desc": "ホームタブからタスクを追加すると、AI がワークツリーを作成してそのまま実行を始めます。",
    "taskText": "ログイン画面のバグを修正して",
    "mockDialogTitle": "タスクを追加",
    "mockCancel": "キャンセル",
    "mockAdd": "追加",
    "phase1": "タスク追加ウィンドウを表示",
    "phase2": "やりたいことを入力",
    "phase3": "「追加」を押す",
    "phase4": "ワークツリーが作られ実行開始",
    "hotkeysLabel": "ホットキー",
    "hkAddTask": "タスクを追加",
    "hkFocusWorktree": "各ワークツリーへ移動",
    "hkHomeTab": "ホームタブへ戻る",
    "assignedChar": "割当文字"
  }
}
</i18n>
