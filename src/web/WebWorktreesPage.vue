<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { fetchWorktreesIndex, type WorktreesIndex } from "../utils/httpArtifactDataSource";

const { t } = useI18n({ useScope: "global" });

const emit = defineEmits<{
  unauthorized: [];
  openWorktree: [id: string];
  openRepository: [key: string];
}>();

type LoadState = "loading" | "ready" | "error";
const state = ref<LoadState>("loading");
const errorMessage = ref("");
const index = ref<WorktreesIndex>({ worktrees: [], repositories: [] });
const filter = ref("");

async function load() {
  state.value = "loading";
  try {
    index.value = await fetchWorktreesIndex(() => emit("unauthorized"));
    state.value = "ready";
  } catch (e) {
    errorMessage.value = e instanceof Error ? e.message : String(e);
    state.value = "error";
  }
}

onMounted(load);

const groupedByRepository = computed(() => {
  const q = filter.value.trim().toLowerCase();
  const matches = (haystack: string | null | undefined) =>
    !q || (haystack ?? "").toLowerCase().includes(q);

  const groups = new Map<string, { repositoryName: string; worktrees: WorktreesIndex["worktrees"] }>();
  for (const w of index.value.worktrees) {
    if (!(matches(w.name) || matches(w.branchName) || matches(w.description))) continue;
    const key = w.repositoryId || w.repositoryName;
    if (!groups.has(key)) groups.set(key, { repositoryName: w.repositoryName, worktrees: [] });
    groups.get(key)!.worktrees.push(w);
  }
  return [...groups.values()];
});

const filteredRepositories = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return index.value.repositories;
  return index.value.repositories.filter((r) => r.name.toLowerCase().includes(q));
});

function formatTime(epochSeconds: number): string {
  if (!epochSeconds) return t("webViewer.never");
  return new Date(epochSeconds * 1000).toLocaleString();
}
</script>

<template>
  <div class="min-h-screen bg-gray-50">
    <header class="border-b border-gray-200 bg-white px-6 py-3">
      <span class="text-base font-bold text-gray-800">{{ t("webViewer.appTitle") }}</span>
    </header>

    <main class="mx-auto flex max-w-4xl flex-col gap-6 px-6 py-6">
      <div v-if="state === 'loading'" class="text-sm text-gray-400">{{ t("webViewer.loading") }}</div>
      <div v-else-if="state === 'error'" class="text-sm text-red-500">
        {{ t("webViewer.loadFailed", { message: errorMessage }) }}
      </div>
      <template v-else>
        <input
          v-model="filter"
          type="text"
          class="w-full max-w-md rounded-md border border-gray-300 px-3 py-2 text-sm"
          :placeholder="t('webViewer.filterPlaceholder')"
        />

        <section class="flex flex-col gap-3">
          <h2 class="text-sm font-bold text-gray-600">{{ t("webViewer.worktreesHeading") }}</h2>
          <div
            v-for="group in groupedByRepository"
            :key="group.repositoryName"
            class="rounded-lg border border-gray-200 bg-white"
          >
            <div class="border-b border-gray-100 px-4 py-2 text-sm font-bold text-gray-700">
              📁 {{ group.repositoryName }}
            </div>
            <div
              v-for="w in group.worktrees"
              :key="w.id"
              class="flex cursor-pointer flex-wrap items-center gap-4 border-t border-gray-100 px-4 py-2.5 first:border-t-0 hover:bg-gray-50"
              @click="emit('openWorktree', w.id)"
            >
              <span class="w-44 truncate text-sm font-semibold text-gray-800 underline">{{ w.name }}</span>
              <span
                v-if="w.isHome"
                class="rounded border border-blue-300 bg-blue-50 px-1.5 py-0.5 text-[11px] font-bold text-blue-600"
                >{{ t("webViewer.home") }}</span
              >
              <span class="w-48 truncate font-mono text-xs text-gray-500">{{ w.branchName }}</span>
              <span class="flex-1 truncate text-xs text-gray-400">{{ w.description || "—" }}</span>
              <span class="w-16 text-right text-xs" :class="w.artifactCount ? 'text-gray-700' : 'text-gray-300'">
                {{ t("webViewer.itemCount", { count: w.artifactCount }) }}
              </span>
              <span class="w-24 text-right text-xs text-gray-400">{{ formatTime(w.lastUpdatedAt) }}</span>
            </div>
          </div>
        </section>

        <section v-if="filteredRepositories.length > 0" class="flex flex-col gap-3">
          <h2 class="text-sm font-bold text-gray-600">{{ t("webViewer.repositoriesHeading") }}</h2>
          <p class="text-xs text-gray-400">{{ t("webViewer.repositoriesNote") }}</p>
          <div class="rounded-lg border border-gray-200 bg-white">
            <div
              v-for="r in filteredRepositories"
              :key="r.key"
              class="flex cursor-pointer items-center gap-4 border-t border-gray-100 px-4 py-2.5 first:border-t-0 hover:bg-gray-50"
              @click="emit('openRepository', r.key)"
            >
              <span class="w-44 truncate text-sm font-semibold text-gray-800 underline">🗄 {{ r.name }}</span>
              <span class="flex-1 truncate font-mono text-xs text-gray-400">{{ r.id }}</span>
              <span class="w-16 text-right text-xs text-gray-700">
                {{ t("webViewer.itemCount", { count: r.artifactCount }) }}
              </span>
            </div>
          </div>
        </section>
      </template>
    </main>
  </div>
</template>
