/**
 * 通知サイドカー (oretachi-notify) をビルドし、Tauri の externalBin が要求する
 * `src-tauri/binaries/oretachi-notify-<target-triple>[.exe]` へステージングする。
 *
 * 実行タイミング:
 *   - prepare (pnpm install 後) : クリーンクローン直後にサイドカーを用意
 *   - beforeBuildCommand 経由 (pnpm tauri build) : リリースビルド時に最新版を反映
 *   - サイドカーのソースを変更したら手動で `pnpm build:sidecar`
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const srcTauri = path.join(root, "src-tauri");

// host target triple を rustc から取得 (例: x86_64-pc-windows-msvc)
function hostTargetTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = out.match(/^host:\s*(.+)$/m);
  if (!match) {
    throw new Error("Failed to determine host target triple from `rustc -vV`");
  }
  return match[1].trim();
}

// ビルド対象の target triple を解決する。
// `tauri build [--target <triple>]` 実行時、Tauri は beforeBuildCommand に
// TAURI_ENV_TARGET_TRIPLE（CLI がビルド中の triple）を渡す。これを最優先で尊重し、
// クロスコンパイル時にも要求される triple のサイドカーを生成する。
// 未設定時（例: pnpm install の prepare フック）は host triple にフォールバックする。
const envTriple = process.env.TAURI_ENV_TARGET_TRIPLE?.trim();
const triple = envTriple || hostTargetTriple();
const isWindows = triple.includes("windows");
const exeExt = isWindows ? ".exe" : "";

console.log(`  building oretachi-notify sidecar for ${triple} ...`);
execFileSync(
  "cargo",
  ["build", "--release", "-p", "oretachi-notify", "--target", triple],
  { cwd: srcTauri, stdio: "inherit" }
);

const builtPath = path.join(srcTauri, "target", triple, "release", `oretachi-notify${exeExt}`);
if (!fs.existsSync(builtPath)) {
  throw new Error(`Built sidecar not found at ${builtPath}`);
}

const binariesDir = path.join(srcTauri, "binaries");
fs.mkdirSync(binariesDir, { recursive: true });
const destPath = path.join(binariesDir, `oretachi-notify-${triple}${exeExt}`);
fs.copyFileSync(builtPath, destPath);

console.log(`  staged sidecar: src-tauri/binaries/oretachi-notify-${triple}${exeExt}`);
