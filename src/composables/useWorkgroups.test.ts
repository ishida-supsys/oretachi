import { describe, it, expect, vi, beforeEach } from "vitest";

// `useWorkgroups` は `useSettings` 経由で Tauri のプラグイン群に触るため、モジュール読み込みの
// 時点で落ちる。テストで意味を持つのは `invoke` だけなので、それ以外は最小のスタブにする。
//
// `save_settings` に渡るのは reactive proxy の**参照そのもの**（クローンされない）なので、
// 呼び出し時点でスナップショットを取らないと「そのとき保存された値」を検証できない
// （後から読むと常に現在値になり、保存が起きていなくてもテストが通ってしまう）。
const savedSnapshots: AppSettings[] = [];
const invokeMock = vi.fn((cmd: string, args?: unknown) => {
  if (cmd === "save_settings") {
    const { settings } = args as { settings: AppSettings };
    savedSnapshots.push(JSON.parse(JSON.stringify(settings)) as AppSettings);
  }
  return Promise.resolve(undefined);
});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: [string, unknown?]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => "windows" }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(),
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  debug: vi.fn(),
  info: vi.fn(),
  warn: vi.fn(),
  error: vi.fn(),
}));
vi.mock("../i18n", () => ({ setLocale: vi.fn(), i18n: { global: { t: () => "" } } }));

import { useWorkgroups } from "./useWorkgroups";
import { useSettings } from "./useSettings";
import type { AppSettings } from "../types/settings";

const { updateWorkgroup, deleteWorkgroupRecord } = useWorkgroups();
const { settings } = useSettings();

/** 直近の `save_settings` で実際にディスクへ渡った内容 */
function lastSaved(): AppSettings | undefined {
  return savedSnapshots[savedSnapshots.length - 1];
}

beforeEach(() => {
  // mockReset は vi.fn(impl) の impl を復元するため、実装（スナップショット記録）は残る
  invokeMock.mockReset();
  savedSnapshots.length = 0;
  settings.value = {
    repositories: [],
    worktreeBaseDir: "",
    worktrees: [],
    workgroups: [{ id: "wg-1" }, { id: "wg-2" }],
    terminal: { fontSize: 14 },
  } as unknown as AppSettings;
});

// 編集ダイアログの「保存」は明示的な確定操作なので、デバウンスを待たずに書き込む（#261）。
// デバウンス中にアプリが落ちる / 強制終了されると「設定したのに保存されていない」になる。
describe("useWorkgroups.updateWorkgroup", () => {
  it("パッチをその場で settings へ反映し、デバウンスを待たずに保存する", () => {
    updateWorkgroup("wg-1", { autoReturnHomeAfterTask: true });

    expect(savedSnapshots).toHaveLength(1);
    expect(settings.value.workgroups?.[0].autoReturnHomeAfterTask).toBe(true);
    expect(lastSaved()?.workgroups?.[0].autoReturnHomeAfterTask).toBe(true);
  });

  it("false へ戻したときも改めて保存する（未設定に落とさない）", () => {
    updateWorkgroup("wg-1", { autoReturnHomeAfterTask: true });
    updateWorkgroup("wg-1", { autoReturnHomeAfterTask: false });

    expect(savedSnapshots).toHaveLength(2);
    expect(savedSnapshots[0].workgroups?.[0].autoReturnHomeAfterTask).toBe(true);
    expect(lastSaved()?.workgroups?.[0].autoReturnHomeAfterTask).toBe(false);
  });

  it("対象は id で選ぶ（他グループを巻き込まない）", () => {
    updateWorkgroup("wg-2", { autoReturnHomeAfterTask: true });

    expect(lastSaved()?.workgroups?.[0].autoReturnHomeAfterTask).toBeUndefined();
    expect(lastSaved()?.workgroups?.[1].autoReturnHomeAfterTask).toBe(true);
  });

  it("存在しない id では保存しない", () => {
    updateWorkgroup("wg-missing", { autoReturnHomeAfterTask: true });

    expect(savedSnapshots).toHaveLength(0);
  });
});

// 削除も確認ダイアログを経た確定操作なので同じ扱いにする
describe("useWorkgroups.deleteWorkgroupRecord", () => {
  it("削除結果を即座に保存する", () => {
    deleteWorkgroupRecord("wg-1");

    expect(savedSnapshots).toHaveLength(1);
    expect(lastSaved()?.workgroups?.map((g) => g.id)).toEqual(["wg-2"]);
  });
});
