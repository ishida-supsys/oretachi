import { ref } from "vue";

export type HomePanelMode = "worktree" | "task" | "archive" | "repository";

/** ホームタブのパネル表示モード（worktree 一覧 / task / archive / repository）。
 *  HomeView がローカルに持っていた状態を、App.vue 等からも参照できるようモジュールシングトンとして共有する。 */
const panelMode = ref<HomePanelMode>("worktree");

export function useHomePanel() {
  return {
    panelMode,
  };
}
