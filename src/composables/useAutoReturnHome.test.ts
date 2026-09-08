import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ref, nextTick } from "vue";
import { useAutoReturnHome, AUTO_RETURN_HOME_DELAY_MS } from "./useAutoReturnHome";
import type { AppSettings } from "../types/settings";

const WT_ID = "wt-1";

interface Harness {
  goHome: ReturnType<typeof vi.fn>;
  focused: ReturnType<typeof ref<boolean>>;
  schedule: (createdWorktreeId?: string | null, fallbackGroupId?: string) => void;
  cancel: () => void;
  setFocus: (focused: boolean) => Promise<void>;
  /** ユーザーが自分で別のタブへ移った状態にする */
  leaveTab: () => void;
}

function setup(options?: {
  focused?: boolean;
  detached?: boolean;
  /** worktree の所属グループ（未指定なら settings に workgroupId を書かない） */
  worktreeGroupId?: string;
  /** グループごとの autoReturnHomeAfterTask。既定は wg-1 が有効 */
  groups?: { id: string; autoReturnHomeAfterTask?: boolean }[];
  /** goHome が実際に遷移したか（設定画面表示中は false 相当） */
  goHomeAccepted?: boolean;
  /** 今アクティブなタブのワークツリーID（既定は対象ワークツリー） */
  activeWorktreeId?: string;
}): Harness {
  const focused = ref(options?.focused ?? false);
  const activeWorktreeId = ref<string | null>(options?.activeWorktreeId ?? WT_ID);
  const goHome = vi.fn(() => options?.goHomeAccepted ?? true);
  const settings = ref({
    worktrees: [{ id: WT_ID, workgroupId: options?.worktreeGroupId }],
    workgroups: options?.groups ?? [{ id: "wg-1", autoReturnHomeAfterTask: true }],
  } as unknown as AppSettings);

  const ctl = useAutoReturnHome({
    settings,
    // useWorkgroups と同じフォールバック（未設定/不明なら先頭グループ）
    resolvedGroupId: (id) => {
      const list = settings.value.workgroups ?? [];
      if (id && list.some((g) => g.id === id)) return id;
      return list[0]?.id ?? "";
    },
    isWindowFocused: focused,
    isDetached: () => options?.detached ?? false,
    isActiveWorktree: (id) => activeWorktreeId.value === id,
    goHome,
  });

  return {
    goHome,
    focused,
    schedule: (createdWorktreeId = WT_ID, fallbackGroupId) =>
      ctl.schedule(createdWorktreeId, fallbackGroupId),
    cancel: ctl.cancel,
    setFocus: async (value) => {
      focused.value = value;
      await nextTick();
    },
    leaveTab: () => {
      activeWorktreeId.value = "wt-other";
    },
  };
}

describe("useAutoReturnHome", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("非フォーカスで完了したら遅延後にホームへ戻る", () => {
    const h = setup({ focused: false });
    h.schedule();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS - 1);
    expect(h.goHome).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(h.goHome).toHaveBeenCalledTimes(1);
  });

  it("完了時にフォーカスしていても、後でフォーカスが外れたら戻る (#224)", async () => {
    const h = setup({ focused: true });
    h.schedule();
    // フォーカス中はいくら待っても戻らない
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();

    await h.setFocus(false);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS);
    expect(h.goHome).toHaveBeenCalledTimes(1);
  });

  it("カウントダウン中にフォーカスすると中断し、再度外れたら戻る", async () => {
    const h = setup({ focused: false });
    h.schedule();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS - 1);

    await h.setFocus(true);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();

    // 再度離席したらカウントダウンは頭からやり直す
    await h.setFocus(false);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS - 1);
    expect(h.goHome).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(h.goHome).toHaveBeenCalledTimes(1);
  });

  it("復帰後はフォーカス監視も外れ、二度は戻らない", async () => {
    const h = setup({ focused: false });
    h.schedule();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS);
    expect(h.goHome).toHaveBeenCalledTimes(1);

    await h.setFocus(true);
    await h.setFocus(false);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).toHaveBeenCalledTimes(1);
  });

  it("cancel すると予約もフォーカス監視も破棄される", async () => {
    const h = setup({ focused: true });
    h.schedule();
    h.cancel();
    await h.setFocus(false);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();
  });

  it("ワークツリーを生成しなかったタスクでは予約しない", () => {
    const h = setup({ focused: false });
    h.schedule(null);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();
  });

  it("設定が無効なグループでは予約しない", () => {
    const h = setup({ focused: false, groups: [{ id: "wg-1", autoReturnHomeAfterTask: false }] });
    h.schedule();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();
  });

  it("サブウィンドウへ分離済みのワークツリーでは予約しない", () => {
    const h = setup({ focused: false, detached: true });
    h.schedule();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();
  });

  it("ユーザーが自分で別タブへ移っていたら予約を捨てる", async () => {
    const h = setup({ focused: true });
    h.schedule();
    h.leaveTab();
    await h.setFocus(false);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();

    // 予約は破棄済みなので、そのタブへ戻ってきても復活しない
    await h.setFocus(true);
    await h.setFocus(false);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();
  });

  it("カウントダウン中に別タブへ移ったら発火しない", () => {
    const h = setup({ focused: false });
    h.schedule();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS - 1);
    h.leaveTab();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS * 10);
    expect(h.goHome).not.toHaveBeenCalled();
  });

  it("goHome が見送られたら予約を残し、次の離席で再試行する", async () => {
    const h = setup({ focused: false, goHomeAccepted: false });
    h.schedule();
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS);
    expect(h.goHome).toHaveBeenCalledTimes(1);

    await h.setFocus(true);
    await h.setFocus(false);
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS);
    expect(h.goHome).toHaveBeenCalledTimes(2);
  });

  it("判定は生成されたワークツリーの実際の所属グループで行う", () => {
    // 追加時点の見込みは wg-1（無効）だが、実際の所属は wg-2（有効）
    const h = setup({
      focused: false,
      worktreeGroupId: "wg-2",
      groups: [
        { id: "wg-1", autoReturnHomeAfterTask: false },
        { id: "wg-2", autoReturnHomeAfterTask: true },
      ],
    });
    h.schedule(WT_ID, "wg-1");
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS);
    expect(h.goHome).toHaveBeenCalledTimes(1);
  });

  it("生成されたワークツリーが settings に無ければ見込みグループで判定する", () => {
    const h = setup({
      focused: false,
      activeWorktreeId: "wt-unknown",
      groups: [
        { id: "wg-1", autoReturnHomeAfterTask: false },
        { id: "wg-2", autoReturnHomeAfterTask: true },
      ],
    });
    h.schedule("wt-unknown", "wg-2");
    vi.advanceTimersByTime(AUTO_RETURN_HOME_DELAY_MS);
    expect(h.goHome).toHaveBeenCalledTimes(1);
  });
});
