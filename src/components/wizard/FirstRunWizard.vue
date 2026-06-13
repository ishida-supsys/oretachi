<script setup lang="ts">
import { ref, computed } from "vue";
import { useI18n } from "vue-i18n";
import WizardStepWelcome from "./WizardStepWelcome.vue";
import WizardStepLanguageAgent from "./WizardStepLanguageAgent.vue";
import WizardStepWorkspace from "./WizardStepWorkspace.vue";
import WizardStepHomeHotkeys from "./WizardStepHomeHotkeys.vue";
import WizardStepTrayHotkeys from "./WizardStepTrayHotkeys.vue";

const { t } = useI18n();

const emit = defineEmits<{
  finish: [];
}>();

const TOTAL_STEPS = 5;
const currentStep = ref(1);

const title = computed(() => t(`stepTitle${currentStep.value}`));

function next() {
  if (currentStep.value < TOTAL_STEPS) {
    currentStep.value++;
  } else {
    emit("finish");
  }
}

function back() {
  if (currentStep.value > 1) currentStep.value--;
}

// スキップも完了扱い (wizardCompleted=true で次回非表示)。
// Esc での誤スキップ (IME キャンセル等) を避けるためキーボード操作は設けない
function skip() {
  emit("finish");
}
</script>

<template>
  <div class="wizard-overlay">
    <div class="wizard">
      <!-- ヘッダー: タイトル + スキップ -->
      <div class="wizard-header">
        <h3 class="wizard-title">{{ title }}</h3>
        <button class="skip-btn" @click="skip">{{ t('skip') }}</button>
      </div>

      <!-- コンテンツ -->
      <div class="wizard-body">
        <Transition name="wizard-step" mode="out-in">
          <WizardStepWelcome v-if="currentStep === 1" />
          <WizardStepLanguageAgent v-else-if="currentStep === 2" />
          <WizardStepWorkspace v-else-if="currentStep === 3" />
          <WizardStepHomeHotkeys v-else-if="currentStep === 4" />
          <WizardStepTrayHotkeys v-else />
        </Transition>
      </div>

      <!-- フッター: 戻る / 進捗ドット / 次へ・完了 -->
      <div class="wizard-footer">
        <div class="footer-side">
          <button v-if="currentStep > 1" class="btn-back" @click="back">{{ t('back') }}</button>
        </div>
        <div class="dots">
          <button
            v-for="i in TOTAL_STEPS"
            :key="i"
            class="dot"
            :class="{ 'dot--active': i === currentStep }"
            @click="currentStep = i"
          />
        </div>
        <div class="footer-side footer-side--right">
          <button class="btn-next" @click="next">
            {{ currentStep === 1 ? t('start') : currentStep === TOTAL_STEPS ? t('finish') : t('next') }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.wizard-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.65);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9000; /* 既存ダイアログより上、gaming-border(9999)/shutdown-overlay(10000) 未満 */
}

.wizard {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 12px;
  width: 680px;
  max-width: 92vw;
  max-height: 92vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 16px 56px rgba(0, 0, 0, 0.5);
}

.wizard-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 24px 0;
  min-height: 24px;
}

.wizard-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: #cba6f7;
}

.skip-btn {
  background: transparent;
  border: none;
  color: #6c7086;
  font-size: 12px;
  cursor: pointer;
  text-decoration: underline;
  padding: 0;
}

.skip-btn:hover {
  color: #a6adc8;
}

.wizard-body {
  padding: 16px 28px 20px;
  overflow-y: auto;
  min-height: 320px;
}

.wizard-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 24px;
  border-top: 1px solid #313244;
}

.footer-side {
  width: 110px;
}

.footer-side--right {
  display: flex;
  justify-content: flex-end;
}

.btn-back {
  background: transparent;
  border: none;
  color: #a6adc8;
  font-size: 13px;
  cursor: pointer;
  padding: 0;
}

.btn-back:hover {
  color: #cdd6f4;
}

.dots {
  display: flex;
  gap: 8px;
}

.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: #45475a;
  border: none;
  cursor: pointer;
  padding: 0;
  transition: background 0.2s;
}

.dot--active {
  background: #cba6f7;
}

.btn-next {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 5px;
  padding: 8px 18px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.btn-next:hover {
  background: #b4befe;
}

/* ステップ切り替えトランジション */
.wizard-step-enter-active,
.wizard-step-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.wizard-step-enter-from {
  opacity: 0;
  transform: translateX(16px);
}

.wizard-step-leave-to {
  opacity: 0;
  transform: translateX(-16px);
}
</style>

<i18n lang="json">
{
  "en": {
    "stepTitle1": "",
    "stepTitle2": "Language & AI Agent",
    "stepTitle3": "Workspace Setup",
    "stepTitle4": "Home Tab",
    "stepTitle5": "Tray Popup",
    "skip": "Skip",
    "back": "← Back",
    "next": "Next →",
    "start": "Get Started →",
    "finish": "Finish"
  },
  "ja": {
    "stepTitle1": "",
    "stepTitle2": "言語と AI エージェント",
    "stepTitle3": "ワークスペースの設定",
    "stepTitle4": "ホームタブ",
    "stepTitle5": "トレイポップアップ",
    "skip": "スキップ",
    "back": "← 戻る",
    "next": "次へ →",
    "start": "はじめる →",
    "finish": "完了"
  }
}
</i18n>
