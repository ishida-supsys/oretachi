<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import MacTrafficLights from "./MacTrafficLights.vue";
import ArtifactUrlHoverMenu from "./ArtifactUrlHoverMenu.vue";
import ArtifactIcon from "./ArtifactIcon.vue";
import { isMac } from "../composables/usePlatform";
import type { UrlArtifactEntry } from "../types/artifact";
import { subscriptionCounts } from "../composables/useEventSubscriptions";
import { EMPTY_SUBSCRIPTION_COUNTS } from "../utils/subscriptionCounts";

const { t } = useI18n();

const props = defineProps<{
  /** 購読バッジの件数を引くのに使う（#137）。ホーム / リポジトリ擬似ワークツリーや、
   *  まだ ID を渡していない呼び出し元では undefined になりうる。 */
  worktreeId?: string;
  worktreeName: string;
  branchName: string;
  hotkeyChar?: string;
  artifactCount?: number;
  /** 登録済み URL アーティファクト（アイコン隣のドロップダウン用） */
  artifactUrls?: UrlArtifactEntry[];
  autoApproval: boolean;
  aiJudging: boolean;
  isWindowFocused: boolean;
  showWindowControls?: boolean;
  taskTooltip?: string;
  /** ホームワークツリー: ブランチ表示の代わりにパスを出す */
  isHome?: boolean;
  /** リポジトリ擬似ワークツリー: ホームと同じくブランチの代わりにパスを出す */
  isRepository?: boolean;
  /** ホーム / リポジトリのときに branchName の代わりに表示するパス */
  homePath?: string;
}>();

defineEmits<{
  "open-in-ide": [];
  "open-artifacts": [];
  "cancel-ai-judging": [];
  "click-auto-approval": [];
  /** 購読バッジのクリック。開き先のダイアログはウィンドウごとに用意する
   *  （メインは App.vue、サブは SubWindowApp.vue が自前で出す） */
  "open-subscriptions": [worktreeId: string];
}>();

/** 購読バッジの件数（#137）。カードと同じくモジュールシングルトンから直接引く。
 *  **サブウィンドウでは `initSubscriptionsScoped` を呼んでいないと常に 0 件になる**
 *  （軽量版の `initTerminalUnread` は購読一覧を読まない）。 */
const subCounts = computed(() =>
  props.worktreeId
    ? subscriptionCounts.value.get(props.worktreeId) ?? EMPTY_SUBSCRIPTION_COUNTS
    : EMPTY_SUBSCRIPTION_COUNTS,
);
const hasSubscriptions = computed(
  () => subCounts.value.outgoing > 0 || subCounts.value.incoming > 0,
);

function onHeaderDrag(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('button')) return;
  getCurrentWindow().startDragging();
}

async function minimize() {
  await getCurrentWindow().minimize();
}

async function toggleMaximize() {
  await getCurrentWindow().toggleMaximize();
}

async function closeWindow() {
  await getCurrentWindow().close();
}
</script>

<template>
  <div
    class="flex items-center justify-between border-b shrink-0 pr-2 py-1 transition-colors duration-200"
    :class="[
      props.isWindowFocused ? 'border-[#cba6f7]/50' : 'opacity-80 border-[#313244]',
      isMac && props.showWindowControls ? '' : 'pl-4'
    ]"
    @mousedown.left="props.showWindowControls ? onHeaderDrag($event) : undefined"
  >
    <div class="flex items-center gap-2">
      <!-- Mac: トラフィックライト -->
      <MacTrafficLights
        v-if="isMac && props.showWindowControls"
        :is-window-focused="props.isWindowFocused"
        @close="closeWindow"
        @minimize="minimize"
        @maximize="toggleMaximize"
      />
      <span
        v-if="props.isRepository"
        class="pi pi-folder shrink-0"
        style="font-size: 12px; color: #fab387"
      />
      <span
        class="text-sm font-semibold transition-colors duration-200"
        :class="props.isWindowFocused ? 'text-[#cba6f7]' : 'text-[#6c7086]'"
      >
        {{ props.worktreeName }}
      </span>
      <span
        v-if="props.isHome"
        class="text-[10px] px-1.5 py-0.5 rounded font-mono font-bold"
        style="background: rgba(203,166,247,0.15); color: #cba6f7; border: 1px solid rgba(203,166,247,0.3)"
      >HOME</span>
      <span
        v-if="props.isHome || props.isRepository"
        class="flex items-center gap-1 text-xs font-mono text-[#9399b2]"
      >
        <span class="pi pi-map-marker" style="font-size: 10px" />
        {{ props.homePath }}
      </span>
      <span
        v-else
        v-tooltip.bottom="props.taskTooltip ? { value: props.taskTooltip, escape: false, showDelay: 300, class: 'task-tooltip-sm' } : undefined"
        class="flex items-center gap-1 text-xs font-mono text-[#9399b2]"
        :class="{ 'cursor-help': props.taskTooltip }"
      >
        <span class="pi pi-code-branch" style="font-size: 10px" />
        {{ props.branchName }}
      </span>
      <span
        v-if="props.hotkeyChar"
        class="text-[10px] px-1.5 py-0.5 rounded font-mono font-medium"
        style="background: rgba(203,166,247,0.15); color: #cba6f7; border: 1px solid rgba(203,166,247,0.3)"
      >Alt+{{ props.hotkeyChar.toUpperCase() }}</span>
      <button
        v-if="props.autoApproval"
        class="text-[10px] px-1.5 py-0.5 rounded font-medium cursor-pointer border-none"
        style="background: rgba(166, 227, 161, 0.15); color: #a6e3a1; border: 1px solid rgba(166, 227, 161, 0.3)"
        :title="t('editAutoApprovalPrompt')"
        @click="$emit('click-auto-approval')"
      >{{ t('autoApprovalBadge') }}</button>
      <button
        v-if="props.aiJudging"
        class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-semibold cursor-pointer border-none"
        style="background: #f9e2af; color: #1e1e2e"
        @click="$emit('cancel-ai-judging')"
      >
        <span class="pi pi-spin pi-spinner" style="font-size: 9px" />
        {{ t('aiJudgingBadge') }}
      </button>
      <ArtifactUrlHoverMenu
        v-if="props.artifactCount && props.artifactCount > 0"
        :urls="props.artifactUrls ?? []"
      >
        <button
          class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-medium cursor-pointer border-none"
          style="background: rgba(137, 180, 250, 0.15); color: #89b4fa; border: 1px solid rgba(137, 180, 250, 0.3)"
          :title="t('openArtifacts')"
          @click="$emit('open-artifacts')"
        >
          <ArtifactIcon :has-url="(props.artifactUrls?.length ?? 0) > 0" style="font-size: 9px" />
          {{ props.artifactCount }}
        </button>
      </ArtifactUrlHoverMenu>
      <!-- 購読バッジ（#137）。カードと同じ意味づけ:
           pi-share-alt = このワークツリーが張っている購読 / pi-bolt = このワークツリーへ届く購読。
           0 件の側は枠ごと省く -->
      <button
        v-if="hasSubscriptions && props.worktreeId"
        class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-medium cursor-pointer border-none"
        style="background: rgba(148, 226, 213, 0.15); color: #94e2d5; border: 1px solid rgba(148, 226, 213, 0.3)"
        :title="t('subscriptionsTooltip', { out: subCounts.outgoing, in: subCounts.incoming })"
        @click="$emit('open-subscriptions', props.worktreeId)"
      >
        <span v-if="subCounts.outgoing > 0" class="flex items-center gap-0.5">
          <i class="pi pi-share-alt" style="font-size: 9px" />{{ subCounts.outgoing }}
        </span>
        <span
          v-if="subCounts.outgoing > 0 && subCounts.incoming > 0"
          class="self-stretch w-px"
          style="background: rgba(148, 226, 213, 0.35)"
          aria-hidden="true"
        />
        <span v-if="subCounts.incoming > 0" class="flex items-center gap-0.5">
          <i class="pi pi-bolt" style="font-size: 9px" />{{ subCounts.incoming }}
        </span>
      </button>
    </div>
    <!-- 右端のボタン列。5つとも hdr-btn で同一寸法・同一アイコンサイズに揃える
         （個別に w-*/h-*/font-size を書くと必ずどれかがずれるため、寸法はここに集約する） -->
    <div class="flex items-center gap-1">
      <button
        class="hdr-btn bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4]"
        :title="t('openInIde')"
        @click="$emit('open-in-ide')"
      >
        <span class="pi pi-code" />
      </button>
      <!-- ウィンドウ操作との間隔は下の spacer が持つので、ここでは mr-* を付けない -->
      <ArtifactUrlHoverMenu :urls="props.artifactUrls ?? []">
        <button
          class="hdr-btn bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4]"
          :title="t('openArtifacts')"
          @click="$emit('open-artifacts')"
        >
          <ArtifactIcon :has-url="(props.artifactUrls?.length ?? 0) > 0" />
        </button>
      </ArtifactUrlHoverMenu>
      <template v-if="props.showWindowControls && !isMac">
        <!-- アプリ機能ボタンとウィンドウ操作の区切り（誤クリック防止の間隔） -->
        <span class="w-2 shrink-0" aria-hidden="true" />
        <button
          class="hdr-btn hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4]"
          :title="t('minimize')"
          @click="minimize"
        >
          <span class="pi pi-minus" />
        </button>
        <button
          class="hdr-btn hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4]"
          :title="t('maximize')"
          @click="toggleMaximize"
        >
          <span class="pi pi-stop" />
        </button>
        <button
          class="hdr-btn hover:bg-[#c0392b] hover:text-white text-[#6c7086]"
          :title="t('close')"
          @click="closeWindow"
        >
          <span class="pi pi-times" />
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
/**
 * ヘッダ右端のボタン共通スタイル。
 * 寸法は rem 基準に統一する。webview の setZoom は px/rem を同率で拡大するのでズームでは崩れないが、
 * 単位を混ぜると（w-7 = rem と inline の px）差分を見落としやすいため rem に寄せている。
 */
.hdr-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 1.75rem;
  height: 1.75rem;
  border-radius: 0.25rem;
  transition: background-color 0.15s, color 0.15s;
}

/* アイコンの実寸も揃える。pi の既定 font-size (1rem) と line-height を上書きして中央に置く。
   ArtifactIcon はルートが .pi ではなく .artifact-icon で、中の実アイコンが font-size: 1em で
   呼び出し元に追従する作りなので、こちらにも同じサイズを渡す。 */
.hdr-btn .pi,
.hdr-btn .artifact-icon {
  font-size: 0.875rem;
  line-height: 1;
}
</style>

<i18n lang="json">
{
  "en": {
    "autoApprovalBadge": "Auto approval",
    "aiJudgingBadge": "AI judging",
    "openInIde": "Open in IDE",
    "openArtifacts": "Artifacts",
    "subscriptionsTooltip": "Subscriptions: {out} outgoing / {in} incoming",
    "minimize": "Minimize",
    "maximize": "Maximize",
    "close": "Close",
    "editAutoApprovalPrompt": "Edit additional prompt"
  },
  "ja": {
    "autoApprovalBadge": "自動承認",
    "aiJudgingBadge": "AI判定中",
    "openInIde": "IDE で開く",
    "openArtifacts": "アーティファクト",
    "subscriptionsTooltip": "購読: このワークツリーから {out} 件 / このワークツリーへ {in} 件",
    "minimize": "最小化",
    "maximize": "最大化",
    "close": "閉じる",
    "editAutoApprovalPrompt": "追加プロンプトを編集"
  }
}
</i18n>
