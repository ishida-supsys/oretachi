<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useSettings } from "../../composables/useSettings";
import { useLoopTick } from "../../composables/useLoopTick";
import { formatHotkey } from "../../composables/useHotkeys";
import WizardHomeMock from "./WizardHomeMock.vue";
import WizardPhaseStrip from "./WizardPhaseStrip.vue";

const { t } = useI18n();
const { settings } = useSettings();

// ── 通知処理フローのループアニメ (24 tick × 220ms ≈ 5.3秒/周) ──
// ベル点灯 → ポップアップ表示 (1/2) → 次へ押下 → (2/2) → 完了押下 → 消灯
const tick = useLoopTick(24, 220);

const bell = computed(() => tick.value >= 3 && tick.value < 20);
const bellPop = computed(() => tick.value >= 3 && tick.value < 6);
const popup = computed(() => tick.value >= 8 && tick.value < 20);
const notifIdx = computed(() => (tick.value < 14 ? 0 : 1));
const buttonPressed = computed(
  () => (tick.value >= 12 && tick.value < 14) || (tick.value >= 18 && tick.value < 20),
);
const phase = computed(() =>
  tick.value < 8 ? 0 : tick.value < 12 ? 1 : tick.value < 18 ? 2 : 3,
);

const notif = computed(() =>
  notifIdx.value === 0
    ? { name: "feature-a", kind: t("kindApproval"), cls: "notif-badge--approval" }
    : { name: "bugfix-b", kind: t("kindCompleted"), cls: "notif-badge--completed" },
);

const phaseItems = computed(() => [t("phase1"), t("phase2"), t("phase3"), t("phase4")]);

const hotkeyRows = computed(() => [
  { desc: t("hkTrayPopup"), key: formatHotkey(settings.value.hotkeys.globalTrayPopup) },
  { desc: t("hkTrayNext"), key: formatHotkey(settings.value.hotkeys.trayNext) },
]);
</script>

<template>
  <div class="step">
    <p class="desc">{{ t('desc') }}</p>

    <!-- ループアニメーション: ベル点灯 → ポップアップ → 次へ → 完了 → 消灯 -->
    <WizardHomeMock :height="160">
      <!-- 通知ベル: 現行 TrayButton.vue と同じ左下の円形ボタン (通知>0 のときのみ表示) -->
      <div
        class="mock-bell"
        :class="{ 'mock-bell--visible': bell, 'mock-bell--pulse': bell }"
      >
        <span
          class="pi pi-bell mock-bell-icon"
          :style="bellPop ? { transform: `rotate(${tick % 2 === 0 ? -14 : 14}deg)` } : undefined"
        />
        <span class="mock-bell-badge">2</span>
      </div>

      <!-- トレイポップアップ (右下からスライドイン) -->
      <div class="mock-popup" :class="{ 'mock-popup--visible': popup }">
        <div class="mock-popup-header">
          <span class="mock-popup-name">{{ notif.name }}</span>
          <span>{{ t('notifCounter', { current: notifIdx + 1, total: 2 }) }}</span>
        </div>
        <div class="mock-popup-body">
          <span class="notif-badge" :class="notif.cls">{{ notif.kind }}</span>
          <div class="mock-popup-lines">──────────────<br />─────────</div>
          <div class="mock-popup-actions">
            <span class="mock-popup-btn">{{ t('toTerminal') }}</span>
            <span class="mock-popup-btn mock-popup-btn--primary" :class="{ 'mock-popup-btn--pressed': buttonPressed }">
              {{ notifIdx === 0 ? t('next') : t('done') }}
            </span>
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

/* ── 通知ベル (TrayButton.vue と同配置: 左下の円形ボタン + 右上バッジ + pulse) ── */
.mock-bell {
  position: absolute;
  bottom: 12px;
  left: 12px;
  width: 40px;
  height: 40px;
  border-radius: 50%;
  background: #313244;
  border: 1px solid #45475a;
  color: #cdd6f4;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  transform: scale(0.4);
  transition: opacity 0.3s, transform 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
}

.mock-bell--visible {
  opacity: 1;
  transform: scale(1);
}

.mock-bell--pulse {
  animation: bell-pulse 2s ease-in-out infinite;
}

@keyframes bell-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(243, 139, 168, 0.4); }
  50% { box-shadow: 0 0 0 8px rgba(243, 139, 168, 0); }
}

.mock-bell-icon {
  font-size: 16px;
  transition: transform 0.2s;
}

.mock-bell-badge {
  position: absolute;
  top: 1px;
  right: 1px;
  min-width: 14px;
  height: 14px;
  border-radius: 7px;
  background: #f38ba8;
  color: #1e1e2e;
  font-size: 9px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 2px;
}

/* ── トレイポップアップのモック ── */
.mock-popup {
  position: absolute;
  right: 10px;
  bottom: 10px;
  width: 220px;
  background: #1e1e2e;
  border: 1px solid #45475a;
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  overflow: hidden;
  opacity: 0;
  transform: translateY(16px);
  transition: all 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
}

.mock-popup--visible {
  opacity: 1;
  transform: translateY(0);
}

.mock-popup-header {
  display: flex;
  justify-content: space-between;
  padding: 6px 10px;
  background: #181825;
  font-size: 10px;
  color: #6c7086;
}

.mock-popup-name {
  font-weight: 700;
  color: #cdd6f4;
}

.mock-popup-body {
  padding: 8px 10px;
}

.notif-badge {
  border-radius: 4px;
  padding: 2px 7px;
  font-size: 9.5px;
  font-family: monospace;
  font-weight: 700;
}

.notif-badge--approval {
  background: rgba(250, 179, 135, 0.13);
  color: #fab387;
  border: 1px solid rgba(250, 179, 135, 0.33);
}

.notif-badge--completed {
  background: rgba(166, 227, 161, 0.13);
  color: #a6e3a1;
  border: 1px solid rgba(166, 227, 161, 0.33);
}

.mock-popup-lines {
  font-size: 9px;
  color: #45475a;
  margin-top: 6px;
}

.mock-popup-actions {
  display: flex;
  gap: 5px;
  margin-top: 8px;
}

.mock-popup-btn {
  flex: 1;
  text-align: center;
  font-size: 10px;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 4px 0;
  color: #a6adc8;
}

.mock-popup-btn--primary {
  font-weight: 700;
  transition: all 0.15s;
}

.mock-popup-btn--pressed {
  background: #cba6f7;
  border-color: #cba6f7;
  color: #1e1e2e;
  transform: scale(0.92);
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
    "desc": "Notifications from the AI (approval requests, completions, ...) pile up on the bell, and you can process them one by one in the tray popup via a global hotkey.",
    "kindApproval": "Approval",
    "kindCompleted": "Completed",
    "notifCounter": "Notification {current} / {total}",
    "toTerminal": "To terminal",
    "next": "Next →",
    "done": "Done",
    "phase1": "The bell lights up on a notification",
    "phase2": "Open the popup with the hotkey",
    "phase3": "\"Next\" steps through them",
    "phase4": "\"Done\" clears & turns it off",
    "hotkeysLabel": "Hotkeys",
    "hkTrayPopup": "Show tray popup (global — works while other apps are focused)",
    "hkTrayNext": "Go to the next notification"
  },
  "ja": {
    "desc": "AI からの通知（承認待ち・完了など）はベルに溜まり、グローバルホットキーのポップアップで順番に処理できます。",
    "kindApproval": "承認待ち",
    "kindCompleted": "完了",
    "notifCounter": "通知 {current} / {total}",
    "toTerminal": "ターミナルへ",
    "next": "次へ →",
    "done": "完了",
    "phase1": "通知が届くとベルが点灯",
    "phase2": "ホットキーでポップアップ表示",
    "phase3": "「次へ」で順に確認",
    "phase4": "「完了」でクリア・消灯",
    "hotkeysLabel": "ホットキー",
    "hkTrayPopup": "トレイポップアップを表示（グローバル・他アプリ使用中でも有効）",
    "hkTrayNext": "次の通知へ移動"
  }
}
</i18n>
