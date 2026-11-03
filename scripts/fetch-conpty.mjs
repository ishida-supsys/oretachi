/**
 * 新しい ConPTY (conpty.dll / OpenConsole.exe) を NuGet から取得し、
 * `src-tauri/conpty/` へ展開する (#423)。
 *
 * OS 内蔵の ConPTY は、全角文字の右半分を上書きする BS を出力して文字が欠けることがある (#421)。
 * portable-pty は exe の隣の conpty.dll を優先してロードするため、同梱するだけで切り替わる。
 * 展開物は `src-tauri/tauri.windows.conf.json` の bundle.resources で exe の隣へ配置される。
 *
 * 実行タイミング:
 *   - prepare (pnpm install 後)
 *   - beforeBuildCommand 経由 (pnpm tauri build)
 *   - Windows 以外のターゲットでは何もしない
 *
 * 更新手順:
 *   1. CONPTY_VERSION を新しい安定版へ変える
 *   2. NUPKG_SHA256 を差し替える (取得した nupkg の sha256sum)
 *   3. THIRD_PARTY_NOTICES.md のバージョン表記を更新する
 */
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const CONPTY_VERSION = "1.25.260930003";
const NUPKG_SHA256 = "02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee";

const root = fileURLToPath(new URL("..", import.meta.url));
const outDir = path.join(root, "src-tauri", "conpty");

function hostTargetTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = out.match(/^host:\s*(.+)$/m);
  if (!match) {
    throw new Error("Failed to determine host target triple from `rustc -vV`");
  }
  return match[1].trim();
}

const triple = process.env.TAURI_ENV_TARGET_TRIPLE?.trim() || hostTargetTriple();
if (!triple.includes("windows")) {
  console.log(`  skip conpty (not a Windows target: ${triple})`);
  process.exit(0);
}

const archName = triple.split("-")[0];
const arch = { x86_64: "x64", aarch64: "arm64" }[archName];
if (!arch) {
  throw new Error(`Unsupported architecture for bundled ConPTY: ${archName}`);
}

const stamp = `${CONPTY_VERSION} ${arch}`;
const stampPath = path.join(outDir, "VERSION");
if (
  fs.existsSync(stampPath) &&
  fs.readFileSync(stampPath, "utf8").trim() === stamp &&
  fs.existsSync(path.join(outDir, "conpty.dll")) &&
  fs.existsSync(path.join(outDir, "OpenConsole.exe"))
) {
  console.log(`  conpty up to date (${stamp})`);
  process.exit(0);
}

const url = `https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/${CONPTY_VERSION}/microsoft.windows.console.conpty.${CONPTY_VERSION}.nupkg`;
console.log(`  downloading ConPTY ${CONPTY_VERSION} (${arch}) ...`);
// NuGet に届かないとき prepare (pnpm install) が長くハングしないよう、本体の読み込みも含めて打ち切る
const res = await fetch(url, { signal: AbortSignal.timeout(60_000) });
if (!res.ok) {
  throw new Error(`Failed to download ${url}: ${res.status} ${res.statusText}`);
}
const buf = Buffer.from(await res.arrayBuffer());
const sha = crypto.createHash("sha256").update(buf).digest("hex");
if (sha !== NUPKG_SHA256) {
  throw new Error(`ConPTY nupkg SHA256 mismatch: expected ${NUPKG_SHA256}, got ${sha}`);
}

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "oretachi-conpty-"));
try {
  const nupkgPath = path.join(tmp, "conpty.nupkg");
  fs.writeFileSync(nupkgPath, buf);
  const extractDir = path.join(tmp, "x");
  fs.mkdirSync(extractDir);
  // Windows 10 以降同梱の bsdtar は zip を展開できる。
  // PATH 上の tar は Git Bash の GNU tar のことがあり、zip を読めず `X:` をリモートホストと誤解するため、
  // System32 の tar を明示し、cwd からの相対パスで渡す。
  const systemTar = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");
  const tarExe = process.platform === "win32" && fs.existsSync(systemTar) ? systemTar : "tar";
  execFileSync(tarExe, ["-xf", "conpty.nupkg", "-C", "x"], { cwd: tmp, stdio: "inherit" });

  const files = [
    [path.join(extractDir, "runtimes", `win-${arch}`, "native", "conpty.dll"), "conpty.dll"],
    [path.join(extractDir, "build", "native", "runtimes", arch, "OpenConsole.exe"), "OpenConsole.exe"],
  ];
  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(outDir, { recursive: true });
  for (const [src, name] of files) {
    if (!fs.existsSync(src)) {
      throw new Error(`Expected file missing in nupkg: ${src}`);
    }
    fs.copyFileSync(src, path.join(outDir, name));
  }
  fs.writeFileSync(stampPath, stamp + "\n");
} finally {
  fs.rmSync(tmp, { recursive: true, force: true });
}

console.log(`  staged ConPTY: src-tauri/conpty/ (${stamp})`);
