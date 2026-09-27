<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import ArtifactViewerApp from "../ArtifactViewerApp.vue";
import {
  createHttpArtifactViewerContext,
  type HttpArtifactViewerContext,
} from "../utils/httpArtifactDataSource";
import { buildWebPath, type WebScopeKind } from "../utils/webRoute";

const { t } = useI18n({ useScope: "global" });

const props = defineProps<{
  scope: WebScopeKind;
  scopeKey: string;
  artifactId?: string;
}>();

const emit = defineEmits<{
  unauthorized: [];
  /**
   * ArtifactViewerApp 内の選択・遷移を Web のパスへ反映する。
   * mode は ArtifactViewerApp の selected イベントをそのまま引き継ぐ
   * （ヘッダーの ◀▶ は常に "replace" で emit するため、ここで握りつぶすと
   * ブラウザ履歴に不要なエントリが積まれてしまう）。
   */
  navigate: [path: string, mode: "push" | "replace"];
}>();

type LoadState = "loading" | "ready" | "notFound" | "error";

const state = ref<LoadState>("loading");
const errorMessage = ref("");
const ctx = ref<HttpArtifactViewerContext | null>(null);

async function load() {
  state.value = "loading";
  ctx.value = null;
  try {
    const result = await createHttpArtifactViewerContext(
      props.scope === "worktree" ? "worktree" : "repository",
      props.scopeKey,
      {
        onUnauthorized: () => emit("unauthorized"),
        // 別スコープへのクロスウィンドウ遷移は常に新規ナビゲーション扱い（push）
        navigateTo: (path) => emit("navigate", path, "push"),
      },
    );
    if (!result) {
      state.value = "notFound";
      return;
    }
    ctx.value = result;
    state.value = "ready";
  } catch (e) {
    errorMessage.value = e instanceof Error ? e.message : String(e);
    state.value = "error";
  }
}

onMounted(load);
// scope/scopeKey が変わったとき（一覧経由ではなく直接 URL を書き換えた場合）は作り直す。
// ArtifactViewerApp ごと再マウントされるよう :key を親側で振っているので、通常はこちらではなく
// key の変化で対応されるが、念のため合わせておく。
watch(() => [props.scope, props.scopeKey], load);

// ブラウザの戻る/進むで同一スコープ内の artifactId だけが変わったとき、
// ArtifactViewerApp を作り直さずに内部の navigateWithin("replace") へ中継する。
watch(
  () => props.artifactId,
  (id) => {
    if (id) ctx.value?.notifyExternalNavigate(id);
  },
);

function onSelected(id: string, mode: "push" | "replace") {
  emit(
    "navigate",
    buildWebPath({ page: "view", scope: props.scope, scopeKey: props.scopeKey, artifactId: id }),
    mode,
  );
}
</script>

<template>
  <!-- ArtifactViewerApp は Tauri 版では「ウィンドウ全体」を前提にしたレイアウト
       (height: 100vh) を持つため、Web 版でもこのページだけは共通ヘッダーを被せず
       そのまま全画面で使う -->
  <div
    v-if="state !== 'ready'"
    class="flex h-screen items-center justify-center text-sm"
    :class="state === 'error' ? 'text-red-400' : 'text-gray-400'"
    style="background: #1e1e2e"
  >
    <span v-if="state === 'loading'">{{ t("webViewer.loading") }}</span>
    <span v-else-if="state === 'notFound'">{{ t("webViewer.scopeNotFoundBody") }}</span>
    <span v-else>{{ t("webViewer.loadFailed", { message: errorMessage }) }}</span>
  </div>
  <ArtifactViewerApp
    v-else-if="ctx"
    :data-source="ctx.dataSource"
    :host="ctx.host"
    :initial-artifact-id="props.artifactId ?? ''"
    @selected="onSelected"
  />
</template>
