import { defineConfig, loadEnv } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import VueI18nPlugin from "@intlify/unplugin-vue-i18n/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// 別ワークツリーで dev ビルドを同時に立ち上げると 1420 が衝突して起動できないため、
// env で退避できるようにする（tauri 側は `tauri dev --config` で devUrl を合わせる）。
// HMR が devPort + 1 を使うので、**複数インスタンスを立てるときは 2 以上離すこと**
// （strictPort: true なので衝突すると起動に失敗する）。
//
// vite は `.env` を `process.env` へは載せないので、Rust 側（dotenvy）と同じように
// `.env` で指定できるよう `loadEnv` も見る。シェルの環境変数を優先する。
// 空文字は未設定と同じ扱い（`.env` には既定値のドキュメントとして空値を置いてある）。
function resolveDevPort(mode: string): number {
  // @ts-expect-error process is a nodejs global
  const fromShell = process.env.ORETACHI_DEV_PORT;
  // @ts-expect-error process is a nodejs global
  const fromFile = loadEnv(mode, process.cwd(), "").ORETACHI_DEV_PORT;
  return Number(fromShell || fromFile) || 1420;
}

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => {
  const devPort = resolveDevPort(mode);
  return {
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
  };
});
