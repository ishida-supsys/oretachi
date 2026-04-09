<script setup lang="ts">
import { ref, computed, watch, nextTick } from "vue";
import { useI18n } from "vue-i18n";
import { fuzzyFilter, type FuzzyResult } from "../../utils/fuzzyMatch";

const { t } = useI18n();

const props = defineProps<{
  visible: boolean;
  repoPath: string;
  files: string[];
  isLoading: boolean;
}>();

const emit = defineEmits<{
  close: [];
  "select-file": [filePath: string];
}>();

const DISPLAY_LIMIT = 50;

const query = ref("");
const selectedIndex = ref(0);
const inputRef = ref<HTMLInputElement | null>(null);
const listRef = ref<HTMLUListElement | null>(null);

const filteredFiles = computed<FuzzyResult[]>(() =>
  fuzzyFilter(query.value, props.files, DISPLAY_LIMIT),
);

watch(
  () => props.visible,
  async (val) => {
    if (val) {
      query.value = "";
      selectedIndex.value = 0;
      await nextTick();
      inputRef.value?.focus();
    }
  },
);

watch(query, () => {
  selectedIndex.value = 0;
});

watch(selectedIndex, async (idx) => {
  await nextTick();
  const el = listRef.value?.children[idx] as HTMLElement | undefined;
  el?.scrollIntoView({ block: "nearest" });
});

function selectFile(path: string) {
  emit("select-file", path);
}

function onKeydown(e: KeyboardEvent) {
  const len = filteredFiles.value.length;
  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    emit("close");
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    selectedIndex.value = len > 0 ? (selectedIndex.value + 1) % len : 0;
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    selectedIndex.value = len > 0 ? (selectedIndex.value - 1 + len) % len : 0;
  } else if (e.key === "Enter") {
    e.preventDefault();
    const file = filteredFiles.value[selectedIndex.value];
    if (file) {
      selectFile(file.path);
    }
  }
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="visible"
      class="quick-open-overlay"
      @click.self="emit('close')"
    >
      <div class="quick-open-palette">
        <div class="quick-open-input-wrap">
          <i class="pi pi-search quick-open-icon" />
          <input
            ref="inputRef"
            v-model="query"
            class="quick-open-input"
            :placeholder="t('placeholder')"
            autocomplete="off"
            spellcheck="false"
            @keydown="onKeydown"
          />
          <span v-if="isLoading" class="quick-open-loading">
            <i class="pi pi-spin pi-spinner" />
          </span>
        </div>
        <ul
          v-if="filteredFiles.length > 0"
          ref="listRef"
          class="quick-open-list"
        >
          <li
            v-for="(file, i) in filteredFiles"
            :key="file.path"
            class="quick-open-item"
            :class="{ selected: i === selectedIndex }"
            @click="selectFile(file.path)"
            @mouseenter="selectedIndex = i"
          >
            <span class="quick-open-name">{{ file.name }}</span>
            <span v-if="file.dir" class="quick-open-dir">{{ file.dir }}</span>
          </li>
        </ul>
        <div
          v-else-if="!isLoading && query.length > 0"
          class="quick-open-empty"
        >
          {{ t("noResults") }}
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.quick-open-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding-top: 15vh;
  z-index: 200;
}

.quick-open-palette {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 8px;
  width: 520px;
  max-width: 90vw;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.quick-open-input-wrap {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: 1px solid #313244;
}

.quick-open-icon {
  color: #6c7086;
  font-size: 14px;
  flex-shrink: 0;
}

.quick-open-input {
  flex: 1;
  background: transparent;
  border: none;
  outline: none;
  color: #cdd6f4;
  font-size: 14px;
  font-family: inherit;
}

.quick-open-input::placeholder {
  color: #6c7086;
}

.quick-open-loading {
  color: #6c7086;
  font-size: 13px;
}

.quick-open-list {
  list-style: none;
  margin: 0;
  padding: 4px 0;
  max-height: 320px;
  overflow-y: auto;
}

.quick-open-item {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 6px 14px;
  cursor: pointer;
  border-radius: 0;
}

.quick-open-item.selected,
.quick-open-item:hover {
  background: #313244;
}

.quick-open-name {
  font-size: 13px;
  color: #cdd6f4;
  flex-shrink: 0;
}

.quick-open-dir {
  font-size: 11px;
  color: #6c7086;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  direction: rtl;
  text-align: left;
}

.quick-open-empty {
  padding: 16px 14px;
  font-size: 13px;
  color: #6c7086;
  text-align: center;
}
</style>

<i18n lang="json">
{
  "en": {
    "placeholder": "Search files by name...",
    "noResults": "No matching files"
  },
  "ja": {
    "placeholder": "ファイル名で検索...",
    "noResults": "一致するファイルがありません"
  }
}
</i18n>
