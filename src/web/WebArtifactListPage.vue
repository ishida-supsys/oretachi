<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { ArtifactMeta } from "../types/artifact";
import { filterArtifacts, sortArtifacts } from "../utils/artifactList";
import { fetchArtifactList, fetchWorktreesIndex } from "../utils/httpArtifactDataSource";
import type { WebScopeKind } from "../utils/webRoute";

const { t } = useI18n({ useScope: "global" });

const props = defineProps<{
  scope: WebScopeKind;
  scopeKey: string;
}>();

const emit = defineEmits<{
  unauthorized: [];
  back: [];
  open: [artifactId: string];
}>();

type LoadState = "loading" | "ready" | "notFound" | "error";
const state = ref<LoadState>("loading");
const errorMessage = ref("");
const artifacts = ref<ArtifactMeta[]>([]);
const scopeName = ref("");
const scopeSubtitle = ref("");
const searchQuery = ref("");
const typeFilter = ref("");

const TYPE_COLORS: Record<string, string> = {
  "text/markdown": "#89b4fa",
  "application/vnd.ant.react": "#cba6f7",
  "application/vnd.ant.mermaid": "#a6e3a1",
  "text/html": "#fab387",
  "image/svg+xml": "#f9e2af",
  "text/csv": "#94e2d5",
  "text/tab-separated-values": "#94e2d5",
  "application/vnd.ant.code": "#bac2de",
  "text/uri-list": "#89dceb",
};

function typeColor(contentType: string): string {
  return TYPE_COLORS[contentType] ?? "#bac2de";
}

const availableTypes = computed(() => [...new Set(artifacts.value.map((a) => a.content_type))].sort());

const visibleArtifacts = computed(() => {
  const sorted = sortArtifacts(artifacts.value, () => false);
  const filtered = filterArtifacts(sorted, searchQuery.value);
  return typeFilter.value ? filtered.filter((a) => a.content_type === typeFilter.value) : filtered;
});

function formatTime(epochSeconds: number): string {
  if (!epochSeconds) return t("webViewer.never");
  return new Date(epochSeconds * 1000).toLocaleString();
}

async function load() {
  state.value = "loading";
  try {
    const kind = props.scope === "worktree" ? "worktree" : "repository";
    const index = await fetchWorktreesIndex(() => emit("unauthorized"));
    if (kind === "worktree") {
      const w = index.worktrees.find((e) => e.id === props.scopeKey);
      if (!w) {
        state.value = "notFound";
        return;
      }
      scopeName.value = w.name;
      scopeSubtitle.value = w.branchName;
    } else {
      const r = index.repositories.find((e) => e.key === props.scopeKey);
      if (!r) {
        state.value = "notFound";
        return;
      }
      scopeName.value = r.name;
      scopeSubtitle.value = r.id;
    }
    artifacts.value = await fetchArtifactList(kind, props.scopeKey, () => emit("unauthorized"));
    state.value = "ready";
  } catch (e) {
    errorMessage.value = e instanceof Error ? e.message : String(e);
    state.value = "error";
  }
}

onMounted(load);
watch(() => [props.scope, props.scopeKey], load);
</script>

<template>
  <div class="min-h-screen bg-gray-50">
    <header class="flex items-center gap-3 border-b border-gray-200 bg-white px-6 py-3">
      <span class="cursor-pointer text-sm text-gray-500 hover:underline" @click="emit('back')">
        ← {{ t("webViewer.backToWorktrees") }}
      </span>
      <span class="text-base font-bold text-gray-800">{{ scopeName || props.scopeKey }}</span>
    </header>

    <main class="mx-auto flex max-w-4xl flex-col gap-4 px-6 py-6">
      <div v-if="state === 'loading'" class="text-sm text-gray-400">{{ t("webViewer.loading") }}</div>
      <div v-else-if="state === 'notFound'" class="text-sm text-gray-400">{{ t("webViewer.scopeNotFoundBody") }}</div>
      <div v-else-if="state === 'error'" class="text-sm text-red-500">
        {{ t("webViewer.loadFailed", { message: errorMessage }) }}
      </div>
      <template v-else>
        <div class="rounded-lg border border-gray-200 bg-white px-4 py-3 text-xs text-gray-500">
          <span v-if="props.scope === 'worktree'" class="font-mono">{{ scopeSubtitle }}</span>
          <template v-else>
            <span class="font-mono">{{ scopeSubtitle }}</span>
            <span class="ml-3 text-amber-600">⚠ {{ t("webViewer.repositoryScopeNote") }}</span>
          </template>
        </div>

        <div class="flex flex-wrap gap-3">
          <input
            v-model="searchQuery"
            type="text"
            class="w-72 rounded-md border border-gray-300 px-3 py-2 text-sm"
            :placeholder="t('webViewer.searchPlaceholder')"
          />
          <select v-model="typeFilter" class="rounded-md border border-gray-300 px-3 py-2 text-sm">
            <option value="">{{ t("webViewer.typeAll") }}</option>
            <option v-for="ct in availableTypes" :key="ct" :value="ct">{{ ct }}</option>
          </select>
        </div>

        <div v-if="visibleArtifacts.length === 0" class="text-sm text-gray-400">{{ t("webViewer.empty") }}</div>
        <div
          v-for="a in visibleArtifacts"
          :key="a.id"
          class="cursor-pointer rounded-lg border border-gray-200 bg-white px-4 py-3 hover:bg-gray-50"
          @click="emit('open', a.id)"
        >
          <div class="flex items-center gap-3">
            <span class="flex-1 truncate text-sm font-semibold text-gray-800 underline">{{ a.title }}</span>
            <span
              class="rounded border px-1.5 py-0.5 font-mono text-[11px] font-bold"
              :style="{ color: typeColor(a.content_type), borderColor: typeColor(a.content_type) + '55', background: typeColor(a.content_type) + '22' }"
            >
              {{ a.content_type }}
            </span>
          </div>
          <div class="mt-1 flex items-center gap-4 text-xs text-gray-400">
            <span class="font-mono">{{ a.id }}</span>
            <span>{{ formatTime(a.updated_at) }}</span>
          </div>
        </div>
      </template>
    </main>
  </div>
</template>
