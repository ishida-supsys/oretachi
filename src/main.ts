import { createApp } from "vue";
import "./styles.css";
import PrimeVue from "primevue/config";
import ToastService from "primevue/toastservice";
import Tooltip from "primevue/tooltip";
import Aura from "@primeuix/themes/aura";
import { attachConsole } from "@tauri-apps/plugin-log";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { platform } from "@tauri-apps/plugin-os";
import { i18n, setLocale } from "./i18n";
import { uiZoomFactor } from "./utils/uiScale";
import type { AppSettings } from "./types/settings";

const params = new URLSearchParams(window.location.search);
const mode = params.get("mode");

let appliedZoom = 1.0;
async function applyUiScale(settings: AppSettings) {
  const zoom = uiZoomFactor(settings);
  if (zoom === appliedZoom) return;
  try {
    await getCurrentWebview().setZoom(zoom);
    appliedZoom = zoom;
  } catch (e) {
    console.warn("Failed to set webview zoom:", e);
  }
}

async function mountApp() {
  document.addEventListener("contextmenu", (e) => e.preventDefault());

  // Rust→webview ログ転送リスナーは UI スレッドコストになるため本番では張らない (issue #59)
  if (import.meta.env.DEV) {
    await attachConsole();
  }

  // ロケール先読み
  try {
    const loaded = await invoke<AppSettings>("get_settings");
    if (loaded.locale) {
      setLocale(loaded.locale as "en" | "ja");
    }
    if (loaded.appearance?.enableAcrylic !== false && platform() !== "macos") {
      document.documentElement.classList.add("transparent-mode");
    }
    await applyUiScale(loaded);
  } catch {
    // settings 読み込み失敗時はデフォルト (enableAcrylic=true) として透明モードに
    if (platform() !== "macos") {
      document.documentElement.classList.add("transparent-mode");
    }
  }

  // UIスケール変更を全ウィンドウモード共通で追従 (emit はグローバルブロードキャスト)
  listen("settings-changed", async () => {
    try {
      const s = await invoke<AppSettings>("get_settings");
      await applyUiScale(s);
    } catch {
      // 取得失敗時は現状のズームを維持
    }
  });

  let rootComponent;
  if (mode === "subwindow") {
    rootComponent = (await import("./SubWindowApp.vue")).default;
  } else if (mode === "tray") {
    rootComponent = (await import("./TrayPopupApp.vue")).default;
  } else if (mode === "codereview") {
    rootComponent = (await import("./CodeReviewApp.vue")).default;
  } else if (mode === "artifact") {
    rootComponent = (await import("./ArtifactViewerApp.vue")).default;
  } else {
    rootComponent = (await import("./App.vue")).default;
  }

  createApp(rootComponent)
    .use(PrimeVue, {
      theme: {
        preset: Aura,
        options: {
          cssLayer: {
            name: "primevue",
            order: "tailwind-base, primevue, tailwind-utilities",
          },
        },
      },
    })
    .use(ToastService)
    .use(i18n)
    .directive("tooltip", Tooltip)
    .mount("#app");
}

mountApp();
