/**
 * React アーティファクトの iframe に埋め込むベンダースクリプト（React / ReactDOM /
 * Babel standalone / Tailwind browser）のロード。
 *
 * ビューア表示とエクスポート（zip に入れる `view.html`）の両方から使うため、
 * コンポーネントではなくここに置く。Promise をキャッシュしているので、
 * 同時に呼ばれても実体のフェッチは 1 回だけ。
 */

export type VendorScripts = {
  react: string;
  reactDom: string;
  babel: string;
  tailwind: string;
};

// 失敗時は null にリセットしてリトライ可能にする
let vendorPromise: Promise<VendorScripts> | null = null;

async function fetchText(url: string): Promise<string> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`Failed to load ${url}: ${r.status} ${r.statusText}`);
  return r.text();
}

export function loadVendors(): Promise<VendorScripts> {
  if (!vendorPromise) {
    vendorPromise = Promise.all([
      fetchText("/vendor/react.production.min.js"),
      fetchText("/vendor/react-dom.production.min.js"),
      import("@babel/standalone/babel.min.js?raw").then((m) => m.default),
      fetchText("/vendor/tailwindcss-browser.js"),
    ])
      .then(([react, reactDom, babel, tailwind]) => ({ react, reactDom, babel, tailwind }))
      .catch((e) => {
        vendorPromise = null;
        throw e;
      });
  }
  return vendorPromise;
}
