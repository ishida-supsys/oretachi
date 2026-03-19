import { invoke } from "@tauri-apps/api/core";

export const SYSTEM_SOUND_PREFIX = "system:";
export const CUSTOM_SOUND_PREFIX = "custom:";

/**
 * 指定の通知音を指定音量(0-100)で再生する。
 * Rustコマンド経由でファイルをbase64読み込みし、Blob URLを生成して再生する。
 */
export async function playNotificationSound(sound: string, volume: number): Promise<void> {
  if (!sound) return;

  const base64 = await invoke<string>("read_audio_file", { sound });
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }

  const ext = sound.split(".").pop()?.toLowerCase() ?? "wav";
  const mimeType = ext === "mp3" ? "audio/mpeg" : ext === "ogg" ? "audio/ogg" : "audio/wav";
  const blob = new Blob([bytes], { type: mimeType });
  const url = URL.createObjectURL(blob);

  const audio = new Audio(url);
  audio.volume = Math.max(0, Math.min(1, volume / 100));
  try {
    await audio.play();
  } finally {
    // 再生完了後にBlobURLを解放
    audio.addEventListener("ended", () => URL.revokeObjectURL(url), { once: true });
  }
}
