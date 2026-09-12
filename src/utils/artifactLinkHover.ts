/**
 * sandbox iframe（html / react アーティファクト）から届いたリンクのホバー通知を、
 * 親側の URL ポップアップへ橋渡しする。
 *
 * iframe が送ってくる座標は iframe 自身のビューポート基準なので、iframe の表示位置を
 * 足して親のビューポート座標へ直す必要がある。html / react の両ビューが同じ処理を
 * するためここへ出した（ポップアップ本体は ArtifactLinkHoverPopup.vue）。
 *
 * 渡す href / 座標はどちらもアーティファクトの自己申告なので `selfDeclared: true` を付ける
 * （ポップアップ側がブラウザで開く前に確認ダイアログを挟む根拠になる）。
 *
 * 座標は必ず iframe の矩形内へ丸める。注入した通知スクリプトはアーティファクトの JS と
 * 同じレルムで動くので、座標は「アーティファクトの自己申告」でしかない。ポップアップは
 * position: fixed で body へ出るため、丸めないと本文側が任意の位置＝親アプリの UI の上に
 * 自前のテキストを出せてしまう（表示先を iframe の上に閉じ込める）。
 */

import type { ArtifactLinkHover, ArtifactLinkRect } from "./artifactFrameLink";

/** ArtifactLinkHoverPopup が defineExpose しているうち、ここで使う分だけ */
export interface ArtifactLinkHoverPopupApi {
  showFor(
    rawHref: string,
    rect: ArtifactLinkRect,
    options?: { selfDeclared?: boolean },
  ): void;
  scheduleHide(): void;
  hideNow(): void;
}

export function applyFrameLinkHover(
  hover: ArtifactLinkHover,
  frame: HTMLIFrameElement | null,
  popup: ArtifactLinkHoverPopupApi | null,
): void {
  if (!popup) return;
  // href が null なら「リンクから離れた」。iframe を跨いでマウスが動く間に閉じないよう
  // 即閉じにはしない（ポップアップ自身へ乗れば取り消される）
  if (hover.href === null || !hover.rect) {
    popup.scheduleHide();
    return;
  }
  if (!frame) return;
  const frameRect = frame.getBoundingClientRect();
  // 幅・高さ 0 は iframe が表示されていない（react ビューの Code タブは v-show で
  // display: none）。teleport 先の body にはその display が効かないので、ここで閉じる
  if (frameRect.width <= 0 || frameRect.height <= 0) {
    popup.hideNow();
    return;
  }
  const left = clamp(hover.rect.left, 0, frameRect.width);
  const top = clamp(hover.rect.top, 0, frameRect.height);
  popup.showFor(
    hover.href,
    {
      left: frameRect.left + left,
      top: frameRect.top + top,
      width: clamp(hover.rect.width, 0, frameRect.width - left),
      height: clamp(hover.rect.height, 0, frameRect.height - top),
    },
    // href も座標もアーティファクトの自己申告。ポップアップ側は開く前に確認を挟む
    { selfDeclared: true },
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}
