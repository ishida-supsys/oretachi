import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ArchiveRow, ArchiveListResult } from "../types/archive";

const PAGE_SIZE = 30;

export const archives = ref<ArchiveRow[]>([]);

export const archiveSearchQuery = ref("");
const currentOffset = ref(0);
const hasMore = ref(true);
const isLoading = ref(false);

let pendingReset = false;

async function loadArchives(reset = false): Promise<void> {
  if (isLoading.value) {
    if (reset) pendingReset = true;
    return;
  }
  if (!reset && !hasMore.value) return;

  isLoading.value = true;
  try {
    if (reset) {
      currentOffset.value = 0;
      archives.value = [];
      hasMore.value = true;
    }
    const result = await invoke<ArchiveListResult>("list_archives", {
      search: archiveSearchQuery.value,
      offset: currentOffset.value,
      limit: PAGE_SIZE,
    });
    archives.value.push(...result.items);
    currentOffset.value += result.items.length;
    hasMore.value = result.has_more;
  } catch (e) {
    console.error("Failed to load archives:", e);
  } finally {
    isLoading.value = false;
  }

  if (pendingReset) {
    pendingReset = false;
    await loadArchives(true);
  }
}

async function loadMore(): Promise<void> {
  await loadArchives(false);
}

async function searchArchives(query: string): Promise<void> {
  archiveSearchQuery.value = query;
  await loadArchives(true);
}

export async function saveArchive(archive: ArchiveRow): Promise<void> {
  await invoke("save_archive", { archive });
}

export async function deleteArchive(id: string): Promise<void> {
  const idx = archives.value.findIndex((a) => a.id === id);
  const removed = idx !== -1 ? archives.value[idx] : undefined;
  if (idx !== -1) archives.value.splice(idx, 1);
  try {
    await invoke("delete_archive", { id });
    if (removed !== undefined) currentOffset.value = Math.max(0, currentOffset.value - 1);
  } catch (e) {
    console.error("Failed to delete archive:", e);
    if (removed !== undefined && idx !== -1) {
      archives.value.splice(idx, 0, removed);
    }
  }
}

export function useArchivePersistence() {
  return {
    archives,
    archiveSearchQuery,
    hasMore,
    isLoading,
    loadArchives,
    loadMore,
    searchArchives,
    saveArchive,
    deleteArchive,
  };
}
