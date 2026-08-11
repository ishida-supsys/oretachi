import { platform } from "@tauri-apps/plugin-os";

export const isMac = platform() === "macos";
/** パス比較の大文字小文字を無視すべきか等、Windows 固有の分岐に使う */
export const isWindows = platform() === "windows";
