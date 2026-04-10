/**
 * pnpm install 後に自動実行 (prepare フック) される vendor コピースクリプト。
 * React 18 UMD ビルドを public/vendor/ に配置する。
 * public/vendor/ は .gitignore で除外済み。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const destDir = path.join(root, "public", "vendor");

fs.mkdirSync(destDir, { recursive: true });

const files = [
  ["react/umd/react.production.min.js", "react.production.min.js"],
  ["react-dom/umd/react-dom.production.min.js", "react-dom.production.min.js"],
  ["@tailwindcss/browser/dist/index.global.js", "tailwindcss-browser.js"],
];

for (const [src, dest] of files) {
  const srcPath = path.join(root, "node_modules", src);
  const destPath = path.join(destDir, dest);
  fs.copyFileSync(srcPath, destPath);
  console.log(`  copied: node_modules/${src} → public/vendor/${dest}`);
}
