import { createApp } from "vue";
import "./styles.css";
import PrimeVue from "primevue/config";
import ToastService from "primevue/toastservice";
import Aura from "@primeuix/themes/aura";
import { attachConsole } from "@tauri-apps/plugin-log";
import { invoke } from "@tauri-apps/api/core";
import { i18n, setLocale } from "./i18n";
import type { AppSettings } from "./types/settings";

const params = new URLSearchParams(window.location.search);
const mode = params.get("mode");

async function mountApp() {
  await attachConsole();

  // ロケール先読み
  try {
    const loaded = await invoke<AppSettings>("get_settings");
    if (loaded.locale) {
      setLocale(loaded.locale as "en" | "ja");
    }
    if (loaded.appearance?.enableAcrylic !== false) {
      document.documentElement.classList.add("transparent-mode");
    }
  } catch {
    // settings 読み込み失敗時はデフォルト (enableAcrylic=true) として透明モードに
    document.documentElement.classList.add("transparent-mode");
  }

  let rootComponent;
  if (mode === "subwindow") {
    rootComponent = (await import("./SubWindowApp.vue")).default;
  } else if (mode === "tray") {
    rootComponent = (await import("./TrayPopupApp.vue")).default;
  } else if (mode === "codereview") {
    rootComponent = (await import("./CodeReviewApp.vue")).default;
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
    .mount("#app");
}

mountApp();
