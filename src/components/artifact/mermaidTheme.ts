import type { MermaidConfig } from "mermaid";
import DOMPurify from "dompurify";

/**
 * mermaid が生成した SVG を表示前にサニタイズする。
 *
 * dominant-baseline は svg プロファイルの許可属性に含まれないが、これが落ちると
 * sequence 図などのテキストの縦位置がずれるため明示的に許可する。
 * (ラベルは htmlLabels: false により <text> で描かれるので foreignObject は不要)
 */
export function sanitizeMermaidSvg(svg: string): string {
  return DOMPurify.sanitize(svg, {
    USE_PROFILES: { svg: true, svgFilters: true },
    ADD_ATTR: ["dominant-baseline"],
  });
}

// Catppuccin Mocha
const base = "#1e1e2e";
const mantle = "#181825";
const surface0 = "#313244";
const surface1 = "#45475a";
const surface2 = "#585b70";
const overlay0 = "#6c7086";
const overlay2 = "#9399b2";
const text = "#cdd6f4";
const subtext0 = "#a6adc8";
const rosewater = "#f5e0dc";
const mauve = "#cba6f7";
const blue = "#89b4fa";
const teal = "#94e2d5";
const green = "#a6e3a1";
const yellow = "#f9e2af";
const peach = "#fab387";
const red = "#f38ba8";

/**
 * useMaxWidth を切ると mermaid が svg に px 幅/高さを出力する。
 * デフォルト (true) では `width: 100%` + `max-width` になり、パンズームキャンバス側で
 * サイズを測れず、かつビューポート幅に押し込められて文字が小さくなる。
 */
const noMaxWidth = { useMaxWidth: false } as const;

/**
 * mermaid のダークテーマ設定。
 *
 * 方針: ノードやセクションの「塗りは暗色 / 文字は明色 / 枠線はアクセント色」で統一する。
 * mermaid のデフォルトは primaryColor から派生色とテキスト色を自動計算するため、
 * 塗りに明色を指定するとテキストとのコントラストが崩れやすい。図種ごとの変数まで
 * 明示的に指定して、flowchart 以外 (sequence / gantt / pie など) も読める色にする。
 */
export const mermaidConfig: MermaidConfig = {
  startOnLoad: false,
  theme: "dark",
  darkMode: true,
  // htmlLabels を切って全ラベルを SVG の <text> で描かせる。
  // 既定 (true) では foreignObject + HTML でラベルを描くが、表示側の DOMPurify は
  // svg プロファイルしか許可しないため foreignObject が丸ごと除去され、
  // flowchart などのノードラベルが消える。
  htmlLabels: false,
  flowchart: noMaxWidth,
  sequence: noMaxWidth,
  gantt: noMaxWidth,
  journey: noMaxWidth,
  timeline: noMaxWidth,
  class: noMaxWidth,
  state: noMaxWidth,
  er: noMaxWidth,
  pie: noMaxWidth,
  requirement: noMaxWidth,
  architecture: noMaxWidth,
  mindmap: noMaxWidth,
  kanban: noMaxWidth,
  gitGraph: noMaxWidth,
  c4: noMaxWidth,
  sankey: noMaxWidth,
  packet: noMaxWidth,
  block: noMaxWidth,
  radar: noMaxWidth,
  fontFamily:
    '"Segoe UI", system-ui, -apple-system, "Helvetica Neue", "Noto Sans JP", sans-serif',
  fontSize: 15,
  themeVariables: {
    darkMode: true,
    background: base,
    mainBkg: surface0,

    // ノード (primary/secondary/tertiary)
    primaryColor: surface0,
    primaryBorderColor: mauve,
    primaryTextColor: text,
    secondaryColor: surface1,
    secondaryBorderColor: blue,
    secondaryTextColor: text,
    tertiaryColor: surface2,
    tertiaryBorderColor: teal,
    tertiaryTextColor: text,

    // 汎用テキスト
    textColor: text,
    nodeTextColor: text,
    labelTextColor: text,
    titleColor: rosewater,
    lineColor: overlay2,
    arrowheadColor: overlay2,
    nodeBorder: mauve,

    // エッジラベル: 背景を base で塗りつぶして線に埋もれるのを防ぐ
    edgeLabelBackground: base,
    labelBackground: base,
    labelBoxBkgColor: surface0,
    labelBoxBorderColor: mauve,

    // サブグラフ / クラスタ
    clusterBkg: mantle,
    clusterBorder: surface2,

    // note (flowchart / sequence 共通)
    noteBkgColor: surface1,
    noteTextColor: text,
    noteBorderColor: yellow,

    // sequenceDiagram
    actorBkg: surface0,
    actorBorder: mauve,
    actorTextColor: text,
    actorLineColor: overlay0,
    signalColor: text,
    signalTextColor: text,
    loopTextColor: text,
    activationBkgColor: surface1,
    activationBorderColor: mauve,
    sequenceNumberColor: base,

    // stateDiagram
    labelColor: text,
    altBackground: mantle,
    transitionColor: overlay2,
    transitionLabelColor: text,
    stateBkg: surface0,
    stateLabelColor: text,
    compositeBackground: mantle,
    compositeTitleBackground: surface0,
    compositeBorder: surface2,
    innerEndBackground: surface2,
    specialStateColor: text,

    // classDiagram
    classText: text,

    // gantt
    sectionBkgColor: surface0,
    sectionBkgColor2: surface1,
    altSectionBkgColor: mantle,
    taskBkgColor: surface1,
    taskBorderColor: blue,
    taskTextColor: text,
    taskTextLightColor: text,
    // gantt はセクションの奇偶で taskTextDarkColor / taskTextLightColor を使い分ける。
    // ここではバーの塗りが常に暗色なので dark 側も明色にしておく
    taskTextDarkColor: text,
    taskTextOutsideColor: text,
    activeTaskBkgColor: surface2,
    activeTaskBorderColor: teal,
    doneTaskBkgColor: surface0,
    doneTaskBorderColor: green,
    // crit も塗りは暗色に統一し、赤枠で区別する (明色の塗りだと明色テキストが読めない)
    critBkgColor: surface0,
    critBorderColor: red,
    todayLineColor: peach,
    gridColor: surface1,

    // pie
    pieTitleTextColor: rosewater,
    pieSectionTextColor: base,
    pieLegendTextColor: text,
    pieStrokeColor: base,
    pieOuterStrokeColor: surface2,
    pie1: mauve,
    pie2: blue,
    pie3: teal,
    pie4: green,
    pie5: yellow,
    pie6: peach,
    pie7: red,
    pie8: subtext0,

    // journey / quadrant などのフォールバック
    fillType0: surface0,
    fillType1: surface1,
    fillType2: surface2,

    // エラー表示
    errorBkgColor: mantle,
    errorTextColor: red,
  },
};
