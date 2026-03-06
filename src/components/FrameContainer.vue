<script setup lang="ts">
import { computed } from "vue";
import Splitter from "primevue/splitter";
import SplitterPanel from "primevue/splitterpanel";
import FramePane from "./FramePane.vue";
import type { FrameNode, FrameLeaf } from "../types/frame";

interface SubTerminalEntry {
  id: number;
  title: string;
  sessionId: number;
  snapshot: string;
}

const props = defineProps<{
  node: FrameNode;
  terminalEntries: Map<number, SubTerminalEntry>;
}>();

const emit = defineEmits<{
  switchTerminal: [leafId: string, terminalId: number];
  closeTerminal: [leafId: string, terminalId: number];
  titleChange: [terminalId: number, title: string];
  splitRequest: [leafId: string, direction: "left" | "right" | "top" | "bottom"];
  tabDrop: [sourceLeafId: string, terminalId: number, targetLeafId: string, insertIndex?: number];
  tabEdgeDrop: [sourceLeafId: string, terminalId: number, targetLeafId: string, direction: "left" | "right" | "top" | "bottom"];
  tabReorder: [leafId: string, terminalId: number, insertIndex: number];
  requestAddTerminal: [leafId: string];
  resizeEnd: [nodeId: string, sizes: number[]];
}>();

const splitterKey = computed(() => {
  if (props.node.type === "container") {
    return props.node.children.map((c) => c.id).join("-");
  }
  return props.node.id;
});

function onResizeEnd(event: { sizes: number[] }) {
  if (props.node.type === "container") {
    emit("resizeEnd", props.node.id, event.sizes);
  }
}

</script>

<template>
  <!-- リーフ -->
  <FramePane
    v-if="node.type === 'leaf'"
    :leaf="(node as FrameLeaf)"
    :terminal-entries="terminalEntries"
    @switch-terminal="(leafId, terminalId) => emit('switchTerminal', leafId, terminalId)"
    @close-terminal="(leafId, terminalId) => emit('closeTerminal', leafId, terminalId)"
    @title-change="(terminalId, title) => emit('titleChange', terminalId, title)"
    @split-request="(leafId, direction) => emit('splitRequest', leafId, direction)"
    @tab-drop="(srcLeafId, terminalId, tgtLeafId, insertIndex) => emit('tabDrop', srcLeafId, terminalId, tgtLeafId, insertIndex)"
    @tab-edge-drop="(srcLeafId, terminalId, tgtLeafId, dir) => emit('tabEdgeDrop', srcLeafId, terminalId, tgtLeafId, dir)"
    @tab-reorder="(leafId, terminalId, insertIndex) => emit('tabReorder', leafId, terminalId, insertIndex)"
    @request-add-terminal="(leafId) => emit('requestAddTerminal', leafId)"
  />

  <!-- コンテナ -->
  <Splitter
    v-else-if="node.type === 'container'"
    :key="splitterKey"
    :layout="node.layout"
    :gutter-size="4"
    class="frame-splitter"
    @resizeend="onResizeEnd"
  >
    <SplitterPanel
      v-for="(child, i) in node.children"
      :key="child.id"
      :size="node.sizes[i]"
      :min-size="10"
      class="frame-splitter-panel"
    >
      <!-- 再帰 -->
      <FrameContainer
        :node="child"
        :terminal-entries="terminalEntries"
        @switch-terminal="(leafId, terminalId) => emit('switchTerminal', leafId, terminalId)"
        @close-terminal="(leafId, terminalId) => emit('closeTerminal', leafId, terminalId)"
        @title-change="(terminalId, title) => emit('titleChange', terminalId, title)"
        @split-request="(leafId, direction) => emit('splitRequest', leafId, direction)"
        @tab-drop="(srcLeafId, terminalId, tgtLeafId, insertIndex) => emit('tabDrop', srcLeafId, terminalId, tgtLeafId, insertIndex)"
        @tab-edge-drop="(srcLeafId, terminalId, tgtLeafId, dir) => emit('tabEdgeDrop', srcLeafId, terminalId, tgtLeafId, dir)"
        @tab-reorder="(leafId, terminalId, insertIndex) => emit('tabReorder', leafId, terminalId, insertIndex)"
        @request-add-terminal="(leafId) => emit('requestAddTerminal', leafId)"
        @resize-end="(nodeId, sizes) => emit('resizeEnd', nodeId, sizes)"
      />
    </SplitterPanel>
  </Splitter>
</template>

<style scoped>
.frame-splitter {
  width: 100%;
  height: 100%;
}

.frame-splitter-panel {
  overflow: hidden;
}
</style>

<style>
/* グローバル: Splitter の高さ伝搬と Catppuccin テーマ */
.frame-splitter.p-splitter {
  height: 100%;
  background: transparent;
  border: none;
}

.frame-splitter > .p-splitter-gutter {
  background-color: #313244;
  transition: background-color 0.15s;
}

.frame-splitter > .p-splitter-gutter:hover {
  background-color: #cba6f7;
}

.frame-splitter > .p-splitter-gutter > .p-splitter-gutter-handle {
  background-color: transparent;
}

.frame-splitter > .p-splitterpanel {
  overflow: hidden;
}
</style>
