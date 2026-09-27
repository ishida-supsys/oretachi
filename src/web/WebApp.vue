<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useWebRouter } from "./useWebRouter";
import WebWorktreesPage from "./WebWorktreesPage.vue";
import WebArtifactListPage from "./WebArtifactListPage.vue";
import WebArtifactViewPage from "./WebArtifactViewPage.vue";
import { parseWebPath } from "../utils/webRoute";

const { t } = useI18n({ useScope: "global" });
const router = useWebRouter();
const route = router.route;

/**
 * どのページの fetch が 401 を返しても、以後は問答無用でこの画面に切り替える
 * （Cookie が無い/失効しているので、他のページを出しても全滅する）。
 */
const unauthorized = ref(false);
function markUnauthorized() {
  unauthorized.value = true;
}

// v-if/v-else-if で discriminated union を絞り込むための computed。
// `route.value.page === 'list'` を直接テンプレートの条件式にすると
// vue-tsc がネストしたプロパティアクセスまでは絞り込んでくれないため、ここで用意する。
const listRoute = computed(() => (route.value.page === "list" ? route.value : null));
const viewRoute = computed(() => (route.value.page === "view" ? route.value : null));

function openWorktree(id: string) {
  router.push({ page: "list", scope: "worktree", scopeKey: id });
}
function openRepository(key: string) {
  router.push({ page: "list", scope: "repository", scopeKey: key });
}
function backToWorktrees() {
  router.push({ page: "worktrees" });
}
function openArtifact(artifactId: string) {
  const r = route.value;
  if (r.page !== "list") return;
  router.push({ page: "view", scope: r.scope, scopeKey: r.scopeKey, artifactId });
}
function navigateToPath(path: string, mode: "push" | "replace") {
  const target = parseWebPath(path);
  if (mode === "replace") {
    router.replace(target);
  } else {
    router.push(target);
  }
}
</script>

<template>
  <div
    v-if="unauthorized"
    class="flex h-screen flex-col items-center justify-center gap-4 bg-gray-50 px-6 text-center"
  >
    <div class="text-4xl">🔒</div>
    <div class="text-lg font-bold text-gray-800">{{ t("webViewer.unauthorizedTitle") }}</div>
    <p class="max-w-md text-sm leading-relaxed text-gray-500">{{ t("webViewer.unauthorizedBody") }}</p>
  </div>

  <WebWorktreesPage
    v-else-if="route.page === 'worktrees'"
    @unauthorized="markUnauthorized"
    @open-worktree="openWorktree"
    @open-repository="openRepository"
  />

  <WebArtifactListPage
    v-else-if="listRoute"
    :scope="listRoute.scope"
    :scope-key="listRoute.scopeKey"
    @unauthorized="markUnauthorized"
    @back="backToWorktrees"
    @open="openArtifact"
  />

  <WebArtifactViewPage
    v-else-if="viewRoute"
    :key="`${viewRoute.scope}:${viewRoute.scopeKey}`"
    :scope="viewRoute.scope"
    :scope-key="viewRoute.scopeKey"
    :artifact-id="viewRoute.artifactId"
    @unauthorized="markUnauthorized"
    @navigate="navigateToPath"
  />

  <div v-else class="flex h-screen flex-col items-center justify-center gap-2 bg-gray-50 text-center">
    <div class="text-lg font-bold text-gray-800">{{ t("webViewer.notFoundTitle") }}</div>
    <p class="text-sm text-gray-500">{{ t("webViewer.notFoundBody") }}</p>
  </div>
</template>
