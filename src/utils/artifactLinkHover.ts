/**
 * sandbox iframe（html / react アーティファクト）から届いたリンクのホバー通知を、
 * 親側の URL ポップアップへ橋渡しする。
 *
 * iframe が送ってくる座標は iframe 自身のビューポート基準なので、iframe の表示位置を
 * 足して親のビューポート座標へ直す必要がある。html / react の両ビューが同じ処理を
 * するためここへ出した（ポップアップ本体は ArtifactLinkHoverPopup.vue）。
 */

import type { ArtifactLinkHover, ArtifactLinkRect } from "./artifactFrameLink";

/** ArtifactLinkHoverPopup が defineExpose しているうち、ここで使う分だけ */
export interface ArtifactLinkHoverPopupApi {
  showFor(rawHref: string, rect: ArtifactLinkRect): void;
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
  popup.showFor(hover.href, {
    left: frameRect.left + hover.rect.left,
    top: frameRect.top + hover.rect.top,
    width: hover.rect.width,
    height: hover.rect.height,
  });
}
