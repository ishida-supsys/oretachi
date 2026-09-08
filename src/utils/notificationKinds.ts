import {
  NOTIFY_KINDS,
  type NotifyKind,
  type NotificationKindSetting,
  type NotificationSoundSettings,
} from "../types/settings";

/** 通知種別ごとの設定（issue #140）の解決とマイグレーション。
 *
 *  `kind` と `event_kind` を統合した結果、通知音と ON/OFF の設定対象が3種別から7種別へ
 *  増えた。`worktree.message` のようにドットを含むキーがあるため、旧来のフラットな
 *  `{ approval, completed, general }` では表現できず `kinds` マップへ移した。
 *
 *  ここは Vue reactivity を使わない純粋関数なので `utils/` に置く（`composables/` は
 *  reactivity を使うもの）。vitest は `environment: 'node'` なのでここだけがテストできる。 */

/** 種別ごとの既定値。
 *
 *  **統合前に通知が出ていなかった種別は既定 OFF にする。** #140 で設定対象が
 *  3 種別から 7 種別へ増えたが、既定を ON にすると「設定を触っていないユーザーの
 *  通知が勝手に増える」ことになる。トグルはあるので、欲しい人が入れれば足りる。
 *
 *  - `hook` は分単位で降ってくるので既定 OFF（統合前も UI 通知を素通ししていた）。
 *  - `worktree.*` は既定 OFF。**この3種別はどれも「自分が起こした操作」**で、
 *    発火元ワークツリーへ届く（ワークツリーを作った / 閉じた / 自分のエージェントが
 *    メッセージを送った）。自分の操作を自分に通知し返しても情報が無く、
 *    `enableOsNotification` を ON にしているユーザーには純粋なノイズになる。
 *    なお受信側にはトーストもバッジも出さない（#137）ので、ここでの `enabled` は
 *    実質「音と OS 通知を出すか」の意味。
 *  - `approval` / `completed` / `general` は統合前の挙動どおり ON
 *    （音は未設定なら鳴らないので、実際に増えるものは無い）。 */
export const DEFAULT_NOTIFICATION_KIND_SETTINGS: Readonly<Record<NotifyKind, NotificationKindSetting>> =
  Object.freeze({
    hook: { enabled: false, sound: null },
    approval: { enabled: true, sound: null },
    completed: { enabled: true, sound: null },
    general: { enabled: true, sound: null },
    "worktree.message": { enabled: false, sound: null },
    "worktree.created": { enabled: false, sound: null },
    "worktree.closed": { enabled: false, sound: null },
  });

/** 統合前のフラットなキーと種別の対応。移行でしか使わない。 */
const LEGACY_FLAT_KEYS = ["approval", "completed", "general"] as const;

/** バッジ（未読カウント）を積む種別か。
 *
 *  `worktree.*` は**送信側からのトースト／バッジを出さない**（#137 の決定）。
 *  他ワークツリーの状態変化をどう扱うかは購読する側が制御すべきもので、
 *  発火側が受信側の画面を動かすのは筋が違う。 */
export function showsBadge(kind: NotifyKind): boolean {
  return !kind.startsWith("worktree.");
}

/** トレイ通知オフ（`tray: false`）のワークツリーでも提示する種別か（#225）。
 *
 *  `tray: false` が載るのは「フック由来 かつ `resolveTrayNotification === false`」のときだけ
 *  （`mcp_server.rs` の `/notify`）。これを kind を問わず落としていたため、
 *  `PermissionRequest` 由来の `approval`（ツール許可 / プラン承認 / AskUserQuestion）も
 *  消えてしまい、**人の入力を待って止まったことが誰にも伝わらなかった**。
 *
 *  通すのは `approval` だけに絞る。teamwork-parent がトレイ通知をオフにする狙いは
 *  `Stop` → `completed` や高頻度な `hook` のノイズを止めることなので、そこは従来どおり
 *  抑制したまま「人待ちだけは通す」形にする。 */
export function passesTrayOff(kind: NotifyKind): boolean {
  return kind === "approval";
}

/** その種別の設定を解決する（未設定なら既定値）。 */
export function resolveKindSetting(
  settings: NotificationSoundSettings | undefined,
  kind: NotifyKind,
): NotificationKindSetting {
  const fallback = DEFAULT_NOTIFICATION_KIND_SETTINGS[kind];
  const configured = settings?.kinds?.[kind];
  if (!configured) return { ...fallback };
  return {
    enabled: configured.enabled,
    sound: configured.sound ?? null,
    os: configured.os,
  };
}

/** その種別で音を鳴らすか。`enabled` が false なら音の設定に関わらず鳴らさない。 */
export function shouldPlaySound(
  settings: NotificationSoundSettings | undefined,
  kind: NotifyKind,
): string | null {
  const s = resolveKindSetting(settings, kind);
  if (!s.enabled) return null;
  return s.sound && s.sound.length > 0 ? s.sound : null;
}

/** その種別で OS 通知を出すか。`os` 未指定なら `enabled` に従う。 */
export function shouldSendOsNotification(
  settings: NotificationSoundSettings | undefined,
  kind: NotifyKind,
): boolean {
  const s = resolveKindSetting(settings, kind);
  if (!s.enabled) return false;
  return s.os ?? true;
}

/** 文字列が既知の種別か（Rust から届いた値の検証用）。 */
export function isNotifyKind(value: string): value is NotifyKind {
  return (NOTIFY_KINDS as readonly string[]).includes(value);
}

/** 旧フラット形式を `kinds` マップへ畳む（冪等）。
 *
 *  戻り値は「設定を書き換えたか」。`loadSettingsOnce` の他のマイグレーションと同じ流儀で、
 *  true のときだけ `save_settings` に乗せる。
 *
 *  **旧キーは畳んだあと削除する。** 残すと、UI で `kinds` を編集したあとに
 *  もう一度この関数を通ったとき（設定ファイルを手で戻した等）に旧値が復活する。 */
export function migrateNotificationSound(sound: NotificationSoundSettings | undefined): boolean {
  if (!sound) return false;
  let changed = false;

  if (!sound.kinds) {
    sound.kinds = {};
    changed = true;
  }
  const kinds = sound.kinds;

  // 旧フラット値の引き継ぎ。値が入っていた種別だけを ON にする（未設定は既定値のまま）。
  for (const key of LEGACY_FLAT_KEYS) {
    if (!(key in sound)) continue;
    const legacy = sound[key];
    if (!kinds[key]) {
      kinds[key] = { enabled: true, sound: legacy ?? null };
    }
    delete sound[key];
    changed = true;
  }

  // 未設定の種別に既定値を入れておく。UI がすべての行を描けるようにするため。
  for (const kind of NOTIFY_KINDS) {
    if (!kinds[kind]) {
      kinds[kind] = { ...DEFAULT_NOTIFICATION_KIND_SETTINGS[kind] };
      changed = true;
    }
  }

  return changed;
}
