import { describe, it, expect } from "vitest";
import { NOTIFY_KINDS, HOOK_NOTIFY_KINDS, type NotificationSoundSettings } from "../types/settings";
import {
  migrateNotificationSound,
  resolveKindSetting,
  shouldPlaySound,
  shouldSendOsNotification,
  showsBadge,
  isNotifyKind,
} from "./notificationKinds";

describe("NOTIFY_KINDS", () => {
  /** Rust 側 `event_db::NotifyKind::ALL` と同じ並び・同じ文字列であること。
   *  向こうにも同じ配列を固定する `test_notify_kind_all_seven_values_pinned` があるので、
   *  片方だけ変えると必ずどちらかが落ちる。 */
  it("は Rust の NotifyKind::ALL と同じ7値を同じ順で持つ", () => {
    expect([...NOTIFY_KINDS]).toEqual([
      "hook",
      "approval",
      "completed",
      "general",
      "worktree.message",
      "worktree.created",
      "worktree.closed",
    ]);
  });

  it("フックが名乗れるのはトースト種別の4値だけ", () => {
    expect([...HOOK_NOTIFY_KINDS]).toEqual(["hook", "approval", "completed", "general"]);
    for (const k of HOOK_NOTIFY_KINDS) {
      expect(NOTIFY_KINDS).toContain(k);
    }
  });

  it("isNotifyKind は未知の値を弾く", () => {
    expect(isNotifyKind("worktree.message")).toBe(true);
    expect(isNotifyKind("bogus")).toBe(false);
    expect(isNotifyKind("Hook")).toBe(false);
  });
});

describe("showsBadge", () => {
  /** `worktree.*` は送信側からトースト／バッジを出さない（#137 / #140）。
   *  他ワークツリーの状態変化は購読バッジが常時見せている。 */
  it("worktree.* はバッジを積まない", () => {
    expect(showsBadge("hook")).toBe(true);
    expect(showsBadge("approval")).toBe(true);
    expect(showsBadge("worktree.message")).toBe(false);
    expect(showsBadge("worktree.created")).toBe(false);
    expect(showsBadge("worktree.closed")).toBe(false);
  });
});

describe("migrateNotificationSound", () => {
  it("旧フラット形式を kinds マップへ畳み、旧キーを消す", () => {
    const sound = {
      volume: 50,
      approval: "system:a.wav",
      completed: null,
      general: "custom:b.mp3",
    } as NotificationSoundSettings;

    expect(migrateNotificationSound(sound)).toBe(true);
    expect(sound.kinds?.approval).toEqual({ enabled: true, sound: "system:a.wav" });
    expect(sound.kinds?.general).toEqual({ enabled: true, sound: "custom:b.mp3" });
    expect(sound.kinds?.completed).toEqual({ enabled: true, sound: null });
    expect("approval" in sound).toBe(false);
    expect("completed" in sound).toBe(false);
    expect("general" in sound).toBe(false);
    // 音量は触らない
    expect(sound.volume).toBe(50);
  });

  /** 統合前に通知が出ていなかった種別を既定 ON にすると、設定を触っていない
   *  ユーザーの通知が勝手に増える。特に `worktree.*` はどれも「自分が起こした操作」で、
   *  発火元へ届くのでノイズにしかならない。 */
  it("統合で増えた種別は既定 OFF、既存3種別は ON", () => {
    const sound = { volume: 80 } as NotificationSoundSettings;
    migrateNotificationSound(sound);
    for (const kind of NOTIFY_KINDS) {
      expect(sound.kinds?.[kind]).toBeDefined();
    }
    for (const kind of ["hook", "worktree.message", "worktree.created", "worktree.closed"] as const) {
      expect(sound.kinds?.[kind]?.enabled, `${kind} は既定 OFF`).toBe(false);
    }
    for (const kind of ["approval", "completed", "general"] as const) {
      expect(sound.kinds?.[kind]?.enabled, `${kind} は従来どおり ON`).toBe(true);
    }
  });

  it("冪等（2回目は何も変えない）", () => {
    const sound = { volume: 80, approval: "system:a.wav" } as NotificationSoundSettings;
    expect(migrateNotificationSound(sound)).toBe(true);
    const snapshot = JSON.stringify(sound);
    expect(migrateNotificationSound(sound)).toBe(false);
    expect(JSON.stringify(sound)).toBe(snapshot);
  });

  it("ユーザーが編集済みの kinds を旧値で上書きしない", () => {
    const sound = {
      volume: 80,
      approval: "system:old.wav",
      kinds: { approval: { enabled: false, sound: "system:new.wav" } },
    } as NotificationSoundSettings;
    migrateNotificationSound(sound);
    expect(sound.kinds?.approval).toEqual({ enabled: false, sound: "system:new.wav" });
    expect("approval" in sound).toBe(false);
  });

  it("設定そのものが無ければ何もしない", () => {
    expect(migrateNotificationSound(undefined)).toBe(false);
  });
});

describe("resolveKindSetting / shouldPlaySound / shouldSendOsNotification", () => {
  it("未設定なら既定値へ倒れる", () => {
    expect(resolveKindSetting(undefined, "approval")).toEqual({ enabled: true, sound: null });
    expect(resolveKindSetting(undefined, "hook")).toEqual({ enabled: false, sound: null });
  });

  it("enabled が false なら音も OS 通知も出さない", () => {
    const sound = {
      volume: 80,
      kinds: { approval: { enabled: false, sound: "system:a.wav", os: true } },
    } as NotificationSoundSettings;
    expect(shouldPlaySound(sound, "approval")).toBeNull();
    expect(shouldSendOsNotification(sound, "approval")).toBe(false);
  });

  it("音が未設定でも OS 通知は出る（別の軸）", () => {
    const sound = {
      volume: 80,
      kinds: { completed: { enabled: true, sound: null } },
    } as NotificationSoundSettings;
    expect(shouldPlaySound(sound, "completed")).toBeNull();
    expect(shouldSendOsNotification(sound, "completed")).toBe(true);
  });

  it("os を明示 false にすると音だけ鳴る", () => {
    const sound = {
      volume: 80,
      kinds: { "worktree.closed": { enabled: true, sound: "system:a.wav", os: false } },
    } as NotificationSoundSettings;
    expect(shouldPlaySound(sound, "worktree.closed")).toBe("system:a.wav");
    expect(shouldSendOsNotification(sound, "worktree.closed")).toBe(false);
  });

  /** 既定 OFF の種別は、設定を触っていない限り音も OS 通知も出さない。 */
  it("既定 OFF の種別は何も出さない", () => {
    for (const kind of ["hook", "worktree.created", "worktree.closed"] as const) {
      expect(shouldPlaySound(undefined, kind)).toBeNull();
      expect(shouldSendOsNotification(undefined, kind)).toBe(false);
    }
  });
});
