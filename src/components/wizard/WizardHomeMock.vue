<script setup lang="ts">
// ④⑤のループアニメ用: ホーム画面のミニモック (タブバー + ワークツリーカード2枚)
// slot "cards" : カード列に追加する要素 (④の新規ワークツリーカード)
// slot default : モック上のオーバーレイ要素 (④のダイアログ、⑤のベル/ポップアップ)
defineProps<{
  height?: number;
}>();
</script>

<template>
  <div class="home-mock" :style="{ height: `${height ?? 150}px` }">
    <div class="mock-tabbar">
      <span class="mock-app">oretachi</span>
      <span class="pi pi-cog mock-cog" />
      <span class="mock-divider" />
      <span class="mock-tab">feature-a <span class="mock-hotkey">A</span></span>
      <span class="mock-tab">bugfix-b <span class="mock-hotkey">B</span></span>
    </div>
    <div class="mock-cards">
      <div class="mock-card">
        <div class="mock-card-name">feature-a</div>
        <div class="mock-card-lines">─────────<br />────────────</div>
      </div>
      <div class="mock-card">
        <div class="mock-card-name">bugfix-b</div>
        <div class="mock-card-lines">─────────<br />────────────</div>
      </div>
      <slot name="cards" />
    </div>
    <slot />
  </div>
</template>

<style scoped>
.home-mock {
  position: relative;
  border: 1px solid #313244;
  border-radius: 8px;
  overflow: hidden;
  background: #11111b;
}

.mock-tabbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 12px;
  background: #181825;
  border-bottom: 1px solid #313244;
  font-size: 11px;
  color: #6c7086;
}

.mock-app {
  font-weight: 700;
  color: #cba6f7;
}

.mock-cog {
  font-size: 11px;
}

.mock-divider {
  width: 1px;
  height: 12px;
  background: #313244;
}

.mock-tab {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.mock-hotkey {
  font-size: 8px;
  padding: 1px 4px;
  border-radius: 3px;
  font-family: monospace;
  background: rgba(203, 166, 247, 0.15);
  color: #cba6f7;
  border: 1px solid rgba(203, 166, 247, 0.3);
}

.mock-cards {
  display: flex;
  gap: 8px;
  padding: 10px;
  height: 100%;
  box-sizing: border-box;
}

.mock-card {
  flex: 1;
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 8px 10px;
  background: #1e1e2e;
  align-self: flex-start;
}

.mock-card-name {
  font-size: 11px;
  font-weight: 700;
  color: #cdd6f4;
}

.mock-card-lines {
  font-size: 9px;
  color: #45475a;
  margin-top: 3px;
}
</style>
