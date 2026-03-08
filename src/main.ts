import { createApp } from "vue";
import "./styles.css";
import PrimeVue from "primevue/config";
import ToastService from "primevue/toastservice";
import Aura from "@primeuix/themes/aura";
import { attachConsole } from "@tauri-apps/plugin-log";

const params = new URLSearchParams(window.location.search);
const mode = params.get("mode");

async function mountApp() {
  await attachConsole();

  let rootComponent;
  if (mode === "subwindow") {
    rootComponent = (await import("./SubWindowApp.vue")).default;
  } else if (mode === "tray") {
    rootComponent = (await import("./TrayPopupApp.vue")).default;
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
    .mount("#app");
}

mountApp();
