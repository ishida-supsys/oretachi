<script setup lang="ts">
import { useI18n } from "vue-i18n";
import appIcon from "../../../src-tauri/icons/128x128@2x.png";

const { t } = useI18n();

const APP_NAME = "oretachi";
const letters = APP_NAME.split("");
</script>

<template>
  <div class="welcome">
    <img :src="appIcon" alt="oretachi" class="welcome-icon" draggable="false" />
    <div class="welcome-name">
      <span
        v-for="(ch, i) in letters"
        :key="i"
        class="welcome-letter"
        :style="{ '--i': i }"
      >{{ ch }}</span>
    </div>
    <p class="welcome-caption">{{ t('caption') }}</p>
  </div>
</template>

<style scoped>
.welcome {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 22px;
  padding: 36px 0 16px;
}

/* アイコン: 上方からバウンド落下 → グロー pulse */
.welcome-icon {
  width: 96px;
  height: 96px;
  border-radius: 22px;
  animation:
    icon-drop 0.8s cubic-bezier(0.34, 1.56, 0.64, 1) both,
    icon-glow 3s ease-in-out 0.9s infinite;
  user-select: none;
}

@keyframes icon-drop {
  0% {
    opacity: 0;
    transform: scale(0.3) translateY(-40px);
  }
  70% {
    opacity: 1;
    transform: scale(1.08) translateY(4px);
  }
  100% {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

@keyframes icon-glow {
  0%, 100% {
    box-shadow: 0 0 18px rgba(203, 166, 247, 0.35);
  }
  50% {
    box-shadow: 0 0 36px rgba(203, 166, 247, 0.7);
  }
}

/* アプリ名: 流れるグラデーションテキスト
   親要素に background-clip: text を使うと transform アニメーション付きの
   子 span がクリップ対象から外れて文字が消える (Chromium) ため、
   グラデーションは文字単位のスライス (background-size 800% + position オフセット) で表現する */
.welcome-name {
  font-size: 44px;
  font-weight: 800;
  letter-spacing: 2px;
}

/* 1文字ずつ staggered fade-up + 流れるグラデーション (8文字 "oretachi" 前提) */
.welcome-letter {
  display: inline-block;
  background-image: linear-gradient(90deg, #cba6f7, #89b4fa, #94e2d5, #cba6f7);
  background-size: 800% 100%;
  background-repeat: repeat-x;
  background-position-x: calc(var(--i) * 100% / 7);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  animation:
    letter-up 0.5s ease both,
    letter-flow 5s linear 1s infinite;
  animation-delay: calc(0.4s + var(--i) * 60ms), 1s;
}

@keyframes letter-up {
  from {
    opacity: 0;
    transform: translateY(14px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* グラデーションが背面で1周分流れる (位置%は線形なので末尾=先頭でシームレス) */
@keyframes letter-flow {
  from {
    background-position-x: calc(var(--i) * 100% / 7);
  }
  to {
    background-position-x: calc(var(--i) * 100% / 7 + 800% / 7);
  }
}

/* キャプション: 遅延フェードイン */
.welcome-caption {
  margin: 0;
  font-size: 13.5px;
  color: #a6adc8;
  text-align: center;
  line-height: 1.8;
  white-space: pre-line;
  animation: caption-in 0.8s ease 1.2s both;
}

@keyframes caption-in {
  from { opacity: 0; }
  to { opacity: 1; }
}
</style>

<i18n lang="json">
{
  "en": {
    "caption": "A multi-session terminal manager\nfor git worktrees & AI agents"
  },
  "ja": {
    "caption": "Git ワークツリー & AI エージェントのための\nマルチセッションターミナルマネージャー"
  }
}
</i18n>
