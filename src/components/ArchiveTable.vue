<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { ArchiveRow } from "../types/archive";

const { t } = useI18n();

defineProps<{
  items: ArchiveRow[];
}>();

const emit = defineEmits<{
  delete: [id: string];
}>();

function formatDate(ts: number): string {
  return new Date(ts).toLocaleString();
}
</script>

<template>
  <div class="archive-table-wrapper">
    <table v-if="items.length > 0" class="archive-table">
      <thead>
        <tr>
          <th>{{ t('colName') }}</th>
          <th>{{ t('colBranch') }}</th>
          <th>{{ t('colRepository') }}</th>
          <th>{{ t('colArchivedAt') }}</th>
          <th class="col-actions"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="item in items" :key="item.id">
          <td class="cell-name">{{ item.name }}</td>
          <td class="cell-branch">
            <span class="branch-badge">{{ item.branch_name }}</span>
          </td>
          <td class="cell-repo">{{ item.repository_name }}</td>
          <td class="cell-date">{{ formatDate(item.archived_at) }}</td>
          <td class="cell-actions">
            <button
              class="btn-delete"
              :title="t('deleteTitle')"
              @click="emit('delete', item.id)"
            >
              <span class="pi pi-trash" />
            </button>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.archive-table-wrapper {
  width: 100%;
  overflow-x: auto;
}

.archive-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}

.archive-table thead tr {
  background: #181825;
  border-bottom: 1px solid #313244;
}

.archive-table th {
  padding: 8px 12px;
  text-align: left;
  font-size: 11px;
  font-weight: 600;
  color: #6c7086;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  white-space: nowrap;
}

.archive-table tbody tr {
  border-bottom: 1px solid #1e1e2e;
  transition: background 0.1s;
}

.archive-table tbody tr:hover {
  background: #181825;
}

.archive-table td {
  padding: 9px 12px;
  color: #cdd6f4;
  vertical-align: middle;
}

.cell-name {
  font-weight: 500;
}

.branch-badge {
  font-family: monospace;
  font-size: 12px;
  background: #313244;
  padding: 2px 8px;
  border-radius: 4px;
  color: #cdd6f4;
}

.cell-repo {
  color: #a6adc8;
}

.cell-date {
  color: #6c7086;
  font-size: 12px;
  white-space: nowrap;
}

.col-actions,
.cell-actions {
  width: 40px;
  text-align: center;
}

.btn-delete {
  background: none;
  border: none;
  padding: 4px 6px;
  border-radius: 4px;
  color: #6c7086;
  cursor: pointer;
  font-size: 13px;
  transition: color 0.15s, background 0.15s;
}

.btn-delete:hover {
  color: #f38ba8;
  background: #313244;
}
</style>

<i18n lang="json">
{
  "en": {
    "colName": "Name",
    "colBranch": "Branch",
    "colRepository": "Repository",
    "colArchivedAt": "Archived At",
    "deleteTitle": "Delete archive"
  },
  "ja": {
    "colName": "名前",
    "colBranch": "ブランチ",
    "colRepository": "リポジトリ",
    "colArchivedAt": "アーカイブ日時",
    "deleteTitle": "アーカイブを削除"
  }
}
</i18n>
