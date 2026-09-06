import { ref } from "vue";

/**
 * ワークツリーカードの D&D 中に、どのワークツリーを掴んでいるかを共有するモジュールシングルトン。
 *
 * カード一覧(HomeView)とワークグループバー(WorkgroupBar)は親子ではなく別枝にいるため、
 * props リレーではなくここで横断的に持つ。dragover イベントでは dataTransfer の中身を
 * 読めない(types しか見えない)ので、ドロップ先ハイライトの判定にもこの状態を使う。
 */
const draggingWorktreeId = ref<string | null>(null);

export function useWorktreeDrag() {
  return { draggingWorktreeId };
}
