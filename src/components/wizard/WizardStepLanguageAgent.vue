<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "vue-i18n";
import { useSettings } from "../../composables/useSettings";
import { setLocale } from "../../i18n";
import type { AiAgentKind } from "../../types/settings";
import { AI_AGENT_LABELS, ALL_AGENT_KINDS, type AiAgentInfo } from "../../constants/aiAgents";

const { t } = useI18n();
const { settings, scheduleSave } = useSettings();

const detectedAgents = ref<AiAgentInfo[]>([]);

onMounted(async () => {
  try {
    detectedAgents.value = await invoke<AiAgentInfo[]>("detect_ai_agents");
  } catch (e) {
    console.error("detect_ai_agents failed:", e);
  }
});

function isAgentDetected(kind: AiAgentKind): boolean {
  return detectedAgents.value.some((a) => a.kind === kind);
}

function onLocaleChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  settings.value.locale = v;
  setLocale(v as "en" | "ja");
  scheduleSave();
}

function onAgentChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  const kind = v ? (v as AiAgentKind) : undefined;
  settings.value.aiAgent = {
    ...(settings.value.aiAgent ?? {}),
    approvalAgent: kind,
    taskAddAgent: kind,
  };
  scheduleSave();
}
</script>

<template>
  <div class="step">
    <div class="section-label">{{ t('languageLabel') }}</div>
    <select class="select-input" :value="settings.locale ?? 'en'" @change="onLocaleChange">
      <option value="en">English</option>
      <option value="ja">日本語</option>
    </select>
    <p class="hint">{{ t('languageHint') }}</p>

    <div class="divider" />

    <div class="section-label">{{ t('agentLabel') }}</div>
    <select
      class="select-input"
      :value="settings.aiAgent?.taskAddAgent ?? settings.aiAgent?.approvalAgent ?? ''"
      @change="onAgentChange"
    >
      <option value="">{{ t('notSet') }}</option>
      <option
        v-for="kind in ALL_AGENT_KINDS"
        :key="kind"
        :value="kind"
        :disabled="!isAgentDetected(kind)"
      >{{ AI_AGENT_LABELS[kind] }}{{ !isAgentDetected(kind) ? t('notDetected') : '' }}</option>
    </select>
    <p class="hint">{{ t('agentDesc') }}</p>

    <div class="detected-row">
      <span
        v-for="kind in ALL_AGENT_KINDS"
        :key="kind"
        class="detected-badge"
        :class="isAgentDetected(kind) ? 'badge--found' : 'badge--missing'"
      >{{ isAgentDetected(kind) ? '✓' : '✗' }} {{ AI_AGENT_LABELS[kind] }}</span>
    </div>

    <p class="note">{{ t('changeLater') }}</p>
  </div>
</template>

<style scoped>
.step {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.section-label {
  font-size: 12px;
  font-weight: 700;
  color: #a6adc8;
  letter-spacing: 0.5px;
}

.select-input {
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 8px 10px;
  font-size: 13px;
  color: #cdd6f4;
  outline: none;
  max-width: 320px;
}

.select-input:focus {
  border-color: #cba6f7;
}

.hint {
  margin: 0;
  font-size: 11.5px;
  color: #6c7086;
}

.divider {
  border-top: 1px solid #313244;
  margin: 10px 0;
}

.detected-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.detected-badge {
  border-radius: 4px;
  padding: 3px 8px;
  font-size: 11px;
  font-family: monospace;
  font-weight: 700;
}

.badge--found {
  background: rgba(166, 227, 161, 0.13);
  color: #a6e3a1;
  border: 1px solid rgba(166, 227, 161, 0.33);
}

.badge--missing {
  background: rgba(108, 112, 134, 0.13);
  color: #6c7086;
  border: 1px solid rgba(108, 112, 134, 0.33);
}

.note {
  margin: 6px 0 0;
  font-size: 11.5px;
  color: #6c7086;
  line-height: 1.7;
}
</style>

<i18n lang="json">
{
  "en": {
    "languageLabel": "Language",
    "languageHint": "The wizard text switches immediately when changed.",
    "agentLabel": "AI Agent",
    "agentDesc": "Used for both auto-approval / commit messages and task execution.",
    "notSet": "Not set",
    "notDetected": " (not detected)",
    "changeLater": "You can proceed without selecting. These can be changed anytime in Settings → Auto Approval (where the two roles can also be set individually)."
  },
  "ja": {
    "languageLabel": "言語 / Language",
    "languageHint": "選択するとこの画面の表示も即座に切り替わります。",
    "agentLabel": "AI エージェント",
    "agentDesc": "自動承認・コミットメッセージ・タスク実行で共通に使用します。",
    "notSet": "未設定",
    "notDetected": " (未検出)",
    "changeLater": "未選択でも次へ進めます。あとから 設定 → 自動承認 でいつでも変更できます（そちらでは2つの用途を個別に設定可能です）。"
  }
}
</i18n>
