<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import DataTable from "primevue/datatable";
import Column from "primevue/column";
import ArtifactCodeView from "./ArtifactCodeView.vue";
import { parseCsvArtifact, filterCsvRows, TSV_CONTENT_TYPE } from "../../utils/csvArtifact";

const props = defineProps<{
  content: string;
  contentType: string;
}>();

const { t } = useI18n();

/** 警告バナーに並べる最大件数。全件並べると数千行の壊れた CSV でバナーが UI を埋め尽くす */
const MAX_ISSUES_SHOWN = 5;
const SEARCH_DEBOUNCE_MS = 150;
/**
 * 仮想スクロールの行高。VirtualScroller は spacer 高と translate をこの値で計算するため、
 * 実際の行高と一致していないとスクロール位置がずれる。
 * 下の .csv-table :deep(td) の height と必ず揃えること。
 */
const ROW_HEIGHT = 32;

type Mode = "table" | "raw";
const mode = ref<Mode>("table");

// searchInput が入力の即時反映、search がデバウンス後の確定値。
// filteredRows は全行フルスキャンなので打鍵ごとには走らせない
const searchInput = ref("");
const search = ref("");
let searchTimer: ReturnType<typeof setTimeout> | null = null;
watch(searchInput, (v) => {
  if (searchTimer !== null) clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    search.value = v;
    searchTimer = null;
  }, SEARCH_DEBOUNCE_MS);
});
onUnmounted(() => {
  if (searchTimer !== null) clearTimeout(searchTimer);
});

const table = computed(() => parseCsvArtifact(props.content, props.contentType));
const filteredRows = computed(() =>
  filterCsvRows(table.value.rows, table.value.columns, search.value)
);

const delimiterLabel = computed(() => (props.contentType === TSV_CONTENT_TYPE ? "\\t" : ","));

const hasIssues = computed(
  () => table.value.errors.length > 0 || table.value.raggedRows.length > 0
);
const shownErrors = computed(() => table.value.errors.slice(0, MAX_ISSUES_SHOWN));
const hiddenErrorCount = computed(() =>
  Math.max(0, table.value.errors.length - MAX_ISSUES_SHOWN)
);

const raggedLabel = computed(() => {
  const rows = table.value.raggedRows;
  if (rows.length === 0) return "";
  const head = rows.slice(0, MAX_ISSUES_SHOWN).join(", ");
  return rows.length > MAX_ISSUES_SHOWN ? `${head}, …` : head;
});
</script>

<template>
  <div class="table-view">
    <div class="table-toolbar">
      <button :class="{ active: mode === 'table' }" @click="mode = 'table'">
        <span class="pi pi-table" />
        Table
      </button>
      <button :class="{ active: mode === 'raw' }" @click="mode = 'raw'">
        <span class="pi pi-align-left" />
        Raw
      </button>

      <template v-if="mode === 'table'">
        <div class="search-box">
          <span class="pi pi-search" />
          <input v-model="searchInput" type="text" :placeholder="t('searchPlaceholder')" />
        </div>
      </template>

      <div class="spacer" />

      <span class="row-count">
        {{ t("rowCount", { shown: filteredRows.length, total: table.rows.length }) }}
      </span>
      <span class="delimiter">delimiter: "{{ delimiterLabel }}"</span>
    </div>

    <div v-if="hasIssues" class="table-warning">
      <span class="pi pi-exclamation-triangle" />
      <div class="warning-lines">
        <div v-for="(e, i) in shownErrors" :key="`e${i}`">{{ e }}</div>
        <div v-if="hiddenErrorCount > 0">{{ t("moreErrors", { n: hiddenErrorCount }) }}</div>
        <div v-if="table.raggedRows.length > 0">
          {{ t("raggedRows", { n: table.raggedRows.length, records: raggedLabel }) }}
        </div>
      </div>
    </div>

    <div v-if="mode === 'table'" class="table-area">
      <div v-if="table.columns.length === 0" class="empty-table">
        {{ t("emptyTable") }}
      </div>
      <div v-else-if="filteredRows.length === 0" class="empty-table">
        {{ search.trim() === "" ? t("noDataRows") : t("noMatches") }}
      </div>
      <DataTable
        v-else
        :value="filteredRows"
        sort-mode="single"
        scrollable
        scroll-height="flex"
        size="small"
        :virtual-scroller-options="{ itemSize: ROW_HEIGHT }"
        class="csv-table"
      >
        <Column
          v-for="col in table.columns"
          :key="col.field"
          :field="col.field"
          :header="col.header"
          sortable
        />
      </DataTable>
    </div>

    <div v-else class="raw-area">
      <ArtifactCodeView :content="content" language="plaintext" />
    </div>
  </div>
</template>

<style scoped>
.table-view {
  height: 100%;
  width: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.table-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  background: #181825;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}

.table-toolbar button {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 12px;
  border: 1px solid #313244;
  border-radius: 4px;
  background: transparent;
  color: #6c7086;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

.table-toolbar button:hover {
  background: #313244;
  color: #cdd6f4;
}

.table-toolbar button.active {
  background: #313244;
  color: #cdd6f4;
  border-color: #45475a;
}

.table-toolbar button .pi {
  font-size: 11px;
}

.search-box {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  border: 1px solid #313244;
  border-radius: 4px;
  background: #1e1e2e;
  margin-left: 4px;
}

.search-box .pi {
  font-size: 11px;
  color: #6c7086;
}

.search-box input {
  border: none;
  outline: none;
  background: transparent;
  color: #cdd6f4;
  font-size: 12px;
  width: 200px;
}

.search-box input::placeholder {
  color: #45475a;
}

.spacer {
  flex: 1;
}

.row-count {
  font-size: 12px;
  color: #6c7086;
}

.delimiter {
  font-size: 11px;
  color: #45475a;
  font-family: monospace;
}

.table-warning {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 8px 12px 0;
  padding: 8px 12px;
  font-size: 12px;
  color: #f9e2af;
  background: rgba(249, 226, 175, 0.08);
  border: 1px solid rgba(249, 226, 175, 0.3);
  border-radius: 6px;
  flex-shrink: 0;
}

.warning-lines {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.table-area,
.raw-area {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.table-area {
  padding: 8px 12px 12px;
}

.empty-table {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #6c7086;
  font-size: 13px;
}

/* PrimeVue DataTable を Catppuccin Mocha に寄せる */
.csv-table {
  flex: 1;
  min-height: 0;
  font-size: 12.5px;
}

.csv-table :deep(.p-datatable-table-container) {
  border: 1px solid #313244;
  border-radius: 6px;
}

.csv-table :deep(.p-datatable-thead > tr > th) {
  background: #181825;
  color: #cdd6f4;
  border-color: #313244;
  font-weight: 600;
  white-space: nowrap;
}

.csv-table :deep(.p-datatable-tbody > tr) {
  background: #1e1e2e;
  color: #cdd6f4;
}

.csv-table :deep(.p-datatable-tbody > tr:nth-child(even)) {
  background: #1c1c2b;
}

/* 行高は ROW_HEIGHT (32px) に固定する。テーマのパディングやフォントサイズ由来の
   端数で itemSize とずれると仮想スクロールの位置計算が狂うため */
.csv-table :deep(.p-datatable-tbody > tr > td) {
  border-color: #262636;
  white-space: nowrap;
  box-sizing: border-box;
  height: 32px;
  padding-top: 0;
  padding-bottom: 0;
  line-height: 1.4;
}

.csv-table :deep(.p-datatable-tbody > tr:hover) {
  background: #313244;
}

.csv-table :deep(.p-sortable-column:hover) {
  background: #313244;
  color: #cdd6f4;
}

.csv-table :deep(.p-sortable-column-icon) {
  color: #6c7086;
}
</style>

<i18n lang="json">
{
  "en": {
    "searchPlaceholder": "Search all columns…",
    "rowCount": "{shown} / {total} rows",
    "emptyTable": "No data",
    "noDataRows": "Header only — no data rows",
    "noMatches": "No rows match the search",
    "moreErrors": "…and {n} more error(s)",
    "raggedRows": "{n} record(s) have a different column count than the header (record: {records}). Missing cells are shown as blank."
  },
  "ja": {
    "searchPlaceholder": "全カラムを検索…",
    "rowCount": "{shown} / {total} 行",
    "emptyTable": "データがありません",
    "noDataRows": "ヘッダのみでデータ行がありません",
    "noMatches": "検索に一致する行がありません",
    "moreErrors": "…他 {n} 件のエラー",
    "raggedRows": "{n} 件のレコードがヘッダと列数が一致しません (レコード: {records})。不足分は空欄として表示します。"
  }
}
</i18n>
