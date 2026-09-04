import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { error } from "@tauri-apps/plugin-log";
import { platform } from "@tauri-apps/plugin-os";
import type {
  AppSettings,
  HotkeyBinding,
  HotkeySettings,
  NotificationHookEntry,
  Workgroup,
  WorktreeEntry,
} from "../types/settings";
import { setLocale } from "../i18n";
import { setVerboseLogging } from "../utils/log";
import { isHomeWorktree, makeHomeWorktreeEntry } from "../utils/homeWorktree";
import {
  isRepositoryWorktree,
  makeRepositoryWorktreeEntry,
  makeRepositoryWorktreeId,
} from "../utils/repositoryWorktree";
import { initialTrayNotification } from "../utils/trayNotification";

/**
 * 擬似ワークツリー（ホーム / リポジトリ）を新規生成するときの `trayNotification` 初期値を焼き込む。
 *
 * これらは `workgroupId` を持たずに作られ、後段の `migrateWorkgroups` が先頭グループへ
 * 割り当てる。`useWorkgroups.groupOf` の「未設定なら先頭グループ」規則に従うと、
 * 生成時点で参照すべきグループは先頭グループになるので、その resolver を渡す。
 *
 * 通常ワークツリーの作成経路（App.vue / useTaskExecution）と同じく、グループ側が
 * 未設定なら何も書かない（未設定のまま = 実効値 true）。
 */
function bakeTrayNotification(entry: WorktreeEntry, loaded: AppSettings): WorktreeEntry {
  const initial = initialTrayNotification(entry, () => loaded.workgroups?.[0]);
  if (initial !== undefined) entry.trayNotification = initial;
  return entry;
}

const isMac = platform() === "macos";

const defaultHotkeys = () => ({
  globalTrayPopup: isMac ? { meta: true, shift: true, key: "o" } : { ctrl: true, shift: true, key: "o" },
  terminalNext: { ctrl: true, key: "Tab" },
  terminalPrev: { ctrl: true, shift: true, key: "Tab" },
  terminalAdd: isMac ? { meta: true, key: "t" } : { ctrl: true, key: "t" },
  terminalClose: isMac ? { meta: true, key: "w" } : { ctrl: true, key: "w" },
  trayNext: isMac ? { meta: true, key: "n" } : { ctrl: true, key: "n" },
  homeTab: { alt: true, key: "0" },
  addTask: isMac ? { meta: true, shift: true, key: "n" } : { ctrl: true, shift: true, key: "n" },
  workgroupNext: { ctrl: true, key: "PageDown" },
  workgroupPrev: { ctrl: true, key: "PageUp" },
});

function migrateHotkeys(hotkeys: HotkeySettings): boolean {
  let changed = false;
  const isOldDefault = (b: HotkeyBinding, key: string, shift = false) =>
    !!b.ctrl && !b.alt && !b.meta && !!b.shift === shift && b.key === key;

  // focusMainWindow (Alt+M) → homeTab (Alt+0)
  const isOldFocusMainWindow = (b: HotkeyBinding) =>
    !b.ctrl && !b.meta && !b.shift && !!b.alt && b.key === "m";
  if (isOldFocusMainWindow(hotkeys.homeTab)) {
    hotkeys.homeTab = { alt: true, key: "0" };
    changed = true;
  }

  // 全プラットフォーム: Ctrl+Q → Ctrl+W / ⌘+W
  if (isOldDefault(hotkeys.terminalClose, "q")) {
    hotkeys.terminalClose = isMac ? { meta: true, key: "w" } : { ctrl: true, key: "w" };
    changed = true;
  }
  // macOS: Ctrl → ⌘ 変換 (Tab 切替は除外)
  if (isMac) {
    if (isOldDefault(hotkeys.terminalAdd, "t")) {
      hotkeys.terminalAdd = { meta: true, key: "t" };
      changed = true;
    }
    if (isOldDefault(hotkeys.trayNext, "n")) {
      hotkeys.trayNext = { meta: true, key: "n" };
      changed = true;
    }
    if (isOldDefault(hotkeys.globalTrayPopup, "o", true)) {
      hotkeys.globalTrayPopup = { meta: true, shift: true, key: "o" };
      changed = true;
    }
  }
  return changed;
}

/**
 * ワークグループ移行 (冪等):
 * 1. グループが無ければデフォルトグループを1件作成し、旧グローバル設定を引き継ぐ
 * 2. workgroupId 未設定 or 不明なグループを指すワークツリーを先頭グループへ割り当て
 * 3. activeWorkgroupId を補完
 * 変更があれば true を返す。
 */
function migrateWorkgroups(loaded: AppSettings): boolean {
  let changed = false;

  if (!loaded.workgroups || loaded.workgroups.length === 0) {
    const defaultGroup: Workgroup = {
      id: `wg-default-${Date.now()}`,
      autoAssignHotkey: loaded.autoAssignHotkey ?? false,
      taskAddAgent: loaded.aiAgent?.taskAddAgent,
      claudeCodeMode: "plan",
    };
    loaded.workgroups = [defaultGroup];
    changed = true;
  }

  const groupIds = new Set(loaded.workgroups.map((g) => g.id));
  const firstId = loaded.workgroups[0].id;
  for (const wt of loaded.worktrees) {
    if (!wt.workgroupId || !groupIds.has(wt.workgroupId)) {
      wt.workgroupId = firstId;
      changed = true;
    }
  }

  if (!loaded.activeWorkgroupId || !groupIds.has(loaded.activeWorkgroupId)) {
    loaded.activeWorkgroupId = firstId;
    changed = true;
  }

  return changed;
}

/**
 * trayNotification 移行 (#171、一度きり):
 *
 * 旧仕様では `trayNotification` 未設定のワークツリーは所属ワークグループの既定値へ
 * フォールバックしていた。新仕様ではフォールバックしない（未設定 = 実効値 true）ため、
 * 移行しないとグループを OFF にしていたユーザーのワークツリーが一斉に通知オンへ戻る。
 * ここで**当時の実効値**を個別値として焼き直し、アップデート前後で実効値を保つ。
 *
 * **冪等ではなく一度きり**なので `trayNotificationMigrated` フラグで実行済みを覚える。
 * 毎回走らせると、`oretachi_set_tray_notification` の `enabled` 省略呼び出し
 * （= キー削除で「未設定に戻す」）のたびに次回起動でグループ既定値が無言で再適用され、
 * 未設定へ戻せなくなる。フラグは Rust 側 `AppSettings` にも持たせること
 * （serde が未知フィールドを落とすため、フロントだけに足すと保存時に消えて毎回走る）。
 *
 * `migrateWorkgroups` の後に呼ぶこと（workgroupId の補完が済んでいる前提）。
 * 変更があれば true を返す。
 */
export function migrateTrayNotification(loaded: AppSettings): boolean {
  if (loaded.trayNotificationMigrated) return false;
  loaded.trayNotificationMigrated = true;

  const groups = loaded.workgroups ?? [];
  const groupOf = (wt: Pick<WorktreeEntry, "workgroupId">) =>
    groups.find((g) => g.id === wt.workgroupId) ?? groups[0];

  for (const wt of loaded.worktrees) {
    // 既に個別値があるものは触らない（null は Rust 由来の「未設定」なので対象）
    if (wt.trayNotification === true || wt.trayNotification === false) continue;
    const inherited = initialTrayNotification(wt, groupOf);
    if (inherited !== undefined) wt.trayNotification = inherited;
  }
  // フラグ自体の永続化が必要なので、値を書かなくても必ず true を返す
  return true;
}

/**
 * ホームワークツリー移行 (冪等):
 * 1. worktreeBaseDir が未設定なら何もしない（ホームは作らない）
 * 2. isHome エントリが無ければ配列先頭に挿入する
 * 3. 既存エントリの path が worktreeBaseDir とズレていれば追従させる
 * 変更があれば true を返す。migrateWorkgroups より前に呼ぶこと（workgroupId の補完を任せるため）。
 */
export function migrateHomeWorktree(loaded: AppSettings): boolean {
  const baseDir = loaded.worktreeBaseDir?.trim();
  if (!baseDir) return false;

  const existing = loaded.worktrees.find(isHomeWorktree);
  if (!existing) {
    loaded.worktrees.unshift(bakeTrayNotification(makeHomeWorktreeEntry(baseDir), loaded));
    return true;
  }
  if (existing.path !== baseDir) {
    existing.path = baseDir;
    return true;
  }
  return false;
}

/**
 * リポジトリ擬似ワークツリー移行 (冪等):
 * 1. settings.repositories の各リポジトリに対応する isRepository エントリを、ホームの直後に挿入する
 * 2. 既存エントリの name / repositoryName / path をリポジトリ側へ追従させる
 * 3. 対応するリポジトリが無くなった isRepository エントリを除去する (prune)
 *
 * 除去した擬似ワークツリー ID (pruned) と、settings に変更を加えたか (changed) を返す。
 * migrateWorkgroups より前に呼ぶこと（workgroupId の補完を任せるため）。
 */
export function migrateRepositoryWorktrees(
  loaded: AppSettings,
): { changed: boolean; pruned: string[] } {
  const repositories = loaded.repositories ?? [];
  const wanted = new Map(repositories.map((r) => [makeRepositoryWorktreeId(r.id), r]));
  let changed = false;

  // 1-2. 既存エントリの追従 + 消えたリポジトリの prune
  //
  // 判定はフラグではなく ID で行う。isRepository を知らない旧バージョンで settings.json を
  // 保存すると serde が未知フィールドを落とすため、フラグだけを見ると擬似エントリが
  // 通常ワークツリーとして残り続けて二度と回収できなくなる。ID は決定論的なので復元できる。
  const pruned: string[] = [];
  const kept = loaded.worktrees.filter((entry) => {
    const repo = wanted.get(entry.id);
    if (!repo) {
      if (!isRepositoryWorktree(entry)) return true;
      pruned.push(entry.id);
      return false;
    }
    // 擬似ワークツリーとしての不変条件も併せて強制する（手編集・旧バージョン保存への防御）
    if (
      entry.isRepository !== true ||
      entry.name !== repo.name ||
      entry.repositoryId !== repo.id ||
      entry.repositoryName !== repo.name ||
      entry.path !== repo.path ||
      entry.branchName !== ""
    ) {
      entry.isRepository = true;
      entry.name = repo.name;
      entry.repositoryId = repo.id;
      entry.repositoryName = repo.name;
      entry.path = repo.path;
      entry.branchName = "";
      changed = true;
    }
    return true;
  });

  if (pruned.length > 0) {
    loaded.worktrees = kept;
    changed = true;
    if (loaded.detachedWorktreeIds) {
      const before = loaded.detachedWorktreeIds.length;
      loaded.detachedWorktreeIds = loaded.detachedWorktreeIds.filter((id) => !pruned.includes(id));
      if (loaded.detachedWorktreeIds.length !== before) changed = true;
    }
  }

  // 3. 未登録のリポジトリ分を挿入する。ホームの直後（= 擬似エントリ群の先頭側）に置く
  const existingIds = new Set(loaded.worktrees.map((w) => w.id));
  const missing = repositories.filter((r) => !existingIds.has(makeRepositoryWorktreeId(r.id)));
  if (missing.length > 0) {
    const insertAt = loaded.worktrees.findIndex(isHomeWorktree) + 1;
    loaded.worktrees.splice(
      insertAt,
      0,
      ...missing.map((r) => bakeTrayNotification(makeRepositoryWorktreeEntry(r), loaded)),
    );
    changed = true;
  }

  return { changed, pruned };
}

/** prune された擬似ワークツリーの残骸（セッションファイル・アーティファクト）を掃除する */
export function cleanupPrunedWorktrees(worktreeIds: string[]): void {
  for (const worktreeId of worktreeIds) {
    void invoke("delete_terminal_session", { worktreeId }).catch(() => {
      /* 存在しない場合は無視 */
    });
    void invoke("delete_artifacts", { worktreeId }).catch(() => {
      /* 存在しない場合は無視 */
    });
  }
}

/**
 * 現在の settings.value に対してリポジトリ擬似ワークツリーを再同期する。
 * リポジトリの追加/削除直後に呼ぶ（自ウィンドウ発の settings-changed は無視されるため自動追随しない）。
 * 呼び出し側で syncWorktreesFromSettings() + scheduleSave() をセットで実行すること。
 */
export function syncRepositoryWorktrees(): string[] {
  const { pruned } = migrateRepositoryWorktrees(settings.value);
  cleanupPrunedWorktrees(pruned);
  return pruned;
}

/**
 * ホームの .claude/ (プラグイン設定 + 同梱スキル) を用意する。
 * overwrite=false なので既存のスキルファイルは温存され、ユーザーの編集を壊さない。
 * 失敗してもアプリの動作は続行させる（ログのみ）。
 */
export async function setupHomeClaudeDir(homePath: string, overwrite = false): Promise<void> {
  if (!homePath.trim()) return;
  try {
    await invoke("setup_home_claude_dir", { homePath, overwrite });
  } catch (e) {
    console.error("ホームの .claude/ 準備に失敗:", e);
  }
}

/**
 * 任意のディレクトリの .claude/settings.local.json に oretachi プラグイン設定を書く。
 * write_plugin_config は既存 JSON をマージする冪等実装なので、何度呼んでも安全。
 * ワークツリー追加時だけでなく、リポジトリ登録時や「再適用」操作からも使う。
 * 失敗は呼び出し側に投げる（明示操作ではエラーを見せたいため）。
 */
export async function applyPluginConfig(
  path: string,
  name: string,
  hooks: NotificationHookEntry[] = [],
): Promise<void> {
  await invoke("write_claude_plugin_config", {
    worktreePath: path,
    worktreeName: name,
    hooks,
  });
}

const settings = ref<AppSettings>({
  repositories: [],
  worktreeBaseDir: "",
  worktrees: [],
  terminal: { fontSize: 14 },
  hotkeys: defaultHotkeys(),
  alwaysOnTop: false,
});

// Debug Mode (settings.debugMode) を webview 側の詳細ログ override に連動させる。
// 起動時のロード・設定タブでのトグル変更の双方でこの watch が発火する。
// (dev ビルドでは log.ts 側で常に verbose 扱いのため override の値に関わらず詳細ログは出る)
watch(
  () => settings.value.debugMode,
  (enabled) => setVerboseLogging(enabled === true),
  { immediate: true },
);

let saveTimer: ReturnType<typeof setTimeout> | null = null;

async function loadSettingsOnce() {
  const loaded = await invoke<AppSettings>("get_settings");
  // 古い設定ファイルに hotkeys がない場合のデフォルト補完
  if (!loaded.hotkeys) {
    loaded.hotkeys = defaultHotkeys();
  } else {
    const def = defaultHotkeys();
    loaded.hotkeys = { ...def, ...loaded.hotkeys };
    // 旧フォーマット移行: globalTrayPopup が文字列だった場合はデフォルトに置換
    if (typeof loaded.hotkeys.globalTrayPopup === "string") {
      loaded.hotkeys.globalTrayPopup = def.globalTrayPopup;
    }
  }
  if (loaded.alwaysOnTop === undefined) {
    loaded.alwaysOnTop = false;
  }
  // ホットキー・ホーム・リポジトリ・ワークグループ マイグレーション (冪等)
  // ホーム/リポジトリはワークグループより先に入れる（workgroupId の補完を migrateWorkgroups に任せるため）
  const hotkeyChanged = migrateHotkeys(loaded.hotkeys);
  const homeChanged = migrateHomeWorktree(loaded);
  const { changed: repositoryChanged, pruned: prunedRepoIds } = migrateRepositoryWorktrees(loaded);
  const workgroupChanged = migrateWorkgroups(loaded);
  // ワークグループ確定後に実行する（workgroupId の補完結果を使うため）
  const trayChanged = migrateTrayNotification(loaded);
  if (hotkeyChanged || homeChanged || repositoryChanged || workgroupChanged || trayChanged) {
    try {
      await invoke("save_settings", { settings: loaded });
    } catch (e) {
      console.error("マイグレーション保存に失敗:", e);
    }
  }
  // ホームを新規作成 or パス変更したときだけ .claude/ を用意する（待たない）
  if (homeChanged) {
    void setupHomeClaudeDir(loaded.worktreeBaseDir);
  }
  // 他ウィンドウ発の削除や手編集された settings.json で消えたリポジトリの残骸を掃除する
  cleanupPrunedWorktrees(prunedRepoIds);
  if (loaded.locale) {
    setLocale(loaded.locale as "en" | "ja");
  }
  settings.value = loaded;
}

// 再入防止 (coalescing): 実行中に再度呼ばれたら 1 回だけ追加実行する。
// settings-changed の連続発火で loadSettings が並行起動し、get_settings の
// 戻り順前後で古い内容が新しい内容を上書きする競合を構造的に排除する。
let loadInflight: Promise<void> | null = null;
let loadRerun = false;

async function loadSettings(): Promise<void> {
  if (loadInflight) {
    loadRerun = true;
    return loadInflight;
  }
  loadInflight = (async () => {
    do {
      loadRerun = false;
      try {
        await loadSettingsOnce();
      } catch (e) {
        // get_settings 失敗時も rerun を取りこぼさず、呼び出し側に reject を
        // 伝播させない（リスナーの後続処理が止まらないようにする）。
        console.error("設定の読み込みに失敗:", e);
      }
    } while (loadRerun);
  })().finally(() => {
    loadInflight = null;
  });
  return loadInflight;
}

function scheduleSave() {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await invoke("save_settings", { settings: settings.value });
      await emit("settings-changed", { source: getCurrentWindow().label });
    } catch (e) {
      console.error("設定の保存に失敗:", e);
    }
  }, 500);
}

async function flushSave() {
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  try {
    await invoke("save_settings", { settings: settings.value });
    await emit("settings-changed", { source: getCurrentWindow().label });
  } catch (e) {
    error(`設定の保存に失敗: ${e}`);
  }
}

export function useSettings() {
  return { settings, loadSettings, scheduleSave, flushSave };
}
