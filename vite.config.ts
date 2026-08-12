import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import VueI18nPlugin from "@intlify/unplugin-vue-i18n/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// 別ワークツリーで dev ビルドを同時に立ち上げると 1420 が衝突して起動できないため、
// env で退避できるようにする（tauri 側は `tauri dev --config` で devUrl を合わせる）。
// HMR が devPort + 1 を使うので、**複数インスタンスを立てるときは 2 以上離すこと**
// （strictPort: true なので衝突すると起動に失敗する）。
// @ts-expect-error process is a nodejs global
const devPort = Number(process.env.ORETACHI_DEV_PORT) || 1420;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    tailwindcss(),
    vue(),
    VueI18nPlugin({ defaultSFCLang: 'json' }),
  ],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: devPort,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          // 既定 (devPort=1420) では従来どおり 1421。devPort をずらしたときも
          // HMR ポートが元の dev ポートと衝突しないよう追従させる。
          port: devPort + 1,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
