import { createApp } from "vue";
import "../styles.css";
import PrimeVue from "primevue/config";
import ToastService from "primevue/toastservice";
import Tooltip from "primevue/tooltip";
import Aura from "@primeuix/themes/aura";
import { i18n, setLocale } from "../i18n";
import WebApp from "./WebApp.vue";

// アーティファクト Web 閲覧のエントリ (#339)。Tauri ランタイムが無いブラウザで直接開かれるため、
// `@tauri-apps/api` 系はこのファイル自身は元より、依存グラフのどこからも import しないこと。
// （`src/main.ts` はここを経由せず、Tauri 版アプリのエントリのまま残す）

if (navigator.language?.toLowerCase().startsWith("ja")) {
  setLocale("ja");
}

createApp(WebApp)
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
