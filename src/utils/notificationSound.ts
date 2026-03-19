import { convertFileSrc } from "@tauri-apps/api/core";
import { appDataDir, join } from "@tauri-apps/api/path";

export const SYSTEM_SOUND_PREFIX = "system:";
export const CUSTOM_SOUND_PREFIX = "custom:";

/**
 * sound文字列からオーディオURLを解決する
 * - "system:<filename>" → C:\Windows\Media\<filename>
 * - "custom:<filename>" → app_data_dir/notification-sounds/<filename>
 */
async function resolveSoundUrl(sound: string): Promise<string | null> {
  if (!sound) return null;

  if (sound.startsWith(SYSTEM_SOUND_PREFIX)) {
    const filename = sound.slice(SYSTEM_SOUND_PREFIX.length);
    // Windows: C:\Windows\Media\<filename>
    const filePath = `C:\\Windows\\Media\\${filename}`;
    return convertFileSrc(filePath);
  }

  if (sound.startsWith(CUSTOM_SOUND_PREFIX)) {
    const filename = sound.slice(CUSTOM_SOUND_PREFIX.length);
    const dir = await appDataDir();
    const filePath = await join(dir, "notification-sounds", filename);
    return convertFileSrc(filePath);
  }

  return null;
}

/**
 * 指定の通知音を指定音量(0-100)で再生する
 */
export async function playNotificationSound(sound: string, volume: number): Promise<void> {
  const url = await resolveSoundUrl(sound);
  if (!url) return;
  const audio = new Audio(url);
  audio.volume = Math.max(0, Math.min(1, volume / 100));
  await audio.play();
}
