import { ref } from "vue";

export type HomePanelMode = "worktree" | "task" | "archive" | "subscription";

/** ホームタブのパネル表示モード（worktree 一覧 / task / archive / subscription）。
 *  HomeView がローカルに持っていた状態を、App.vue 等からも参照できるようモジュールシングトンとして共有する。 */
const panelMode = ref<HomePanelMode>("worktree");

export type HomeListMode = "worktree" | "repository";

/** worktree パネル内のカード一覧の中身（ワークツリー ⇄ リポジトリ）。
 *  ワークグループバー先頭の「リポジトリ」チップで切り替える。panelMode が worktree のときのみ意味を持ち、
 *  task/archive へ切り替えてもリセットしない（セッション内でスティッキー）。 */
const listMode = ref<HomeListMode>("worktree");

export function useHomePanel() {
  return {
    panelMode,
    listMode,
  };
}
