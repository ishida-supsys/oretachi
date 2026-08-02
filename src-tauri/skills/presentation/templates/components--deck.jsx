
// このファイルはそのまま利用（THEME のみカスタマイズ可）
//
// ── サイズ単位について ──────────────────────────────────────────────
// エントリポイントがスライド枠に CSS 変数 `--u`（スライド幅の 1%）をセットする。
// このファイルのサイズはすべて u(n) = スライド幅の n% で指定しているため、
// 小窓表示でも全画面でもレイアウト比率が変わらない。
//   例: u(4.6) → 幅 960px のスライドで 44px
//
// ── 提供コンポーネント ──────────────────────────────────────────────
//   Slide      16:9 スライドの外枠（パディング + フッタ）
//   SlideTitle 見出し + アクセントライン（sub でサブタイトル）
//   Bullets    箇条書き
//   TwoCol     2カラム（ratio で左右比を指定）
//   Big        数値・キーメッセージの強調
//   Quote      引用
//   CodeBlock  等幅ブロック（シンタックスハイライトなし）
//   Note       発表者メモ・補足（控えめな枠）
//   Center     縦横中央寄せ（タイトル/クロージング用）
//   Footer     フッタ（Slide が自動で描画するので通常は直接使わない）

// CUSTOMIZE: テーマ（配色・フォント）
const THEME = {
  bg: '#ffffff',
  fg: '#1f2430',
  muted: '#6b7280',
  faint: '#aab1bd',
  accent: '#cba6f7',
  accentSoft: '#f4eefd',
  border: '#e6e8ec',
  panel: '#fafbfc',
  font: 'system-ui, -apple-system, "Segoe UI", "Hiragino Sans", "Noto Sans JP", sans-serif',
  mono: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
};

// u(n): スライド幅の n%。--u はエントリポイントがセットする
const u = (n) => `calc(var(--u, 9.6px) * ${n})`;

// ── Footer ───────────────────────────────────────────────────────────
// Props: n (現在ページ), total (総ページ数), label (左下に出す文字列)
function Footer({ n, total, label }) {
  return (
    <div style={{
      borderTop: `1px solid ${THEME.border}`,
      paddingTop: u(1.1),
      marginTop: u(0.6),
      display: 'flex', justifyContent: 'space-between', alignItems: 'center',
      fontFamily: THEME.font, fontSize: u(1.15), color: THEME.faint,
      flexShrink: 0,
    }}>
      <span>{label || ''}</span>
      {total ? <span style={{ fontFamily: THEME.mono }}>{n} / {total}</span> : <span />}
    </div>
  );
}

// ── Slide ────────────────────────────────────────────────────────────
// Props:
//   children (ReactNode) 本文
//   n, total (number)    ページ番号（Footer に渡る）
//   label    (string)    フッタ左のラベル（発表タイトルなど）
//   footer   (bool)      false でフッタを消す（タイトルスライドなど）
//   bg       (string)    背景色の上書き
function Slide({ children, n, total, label, footer = true, bg }) {
  return (
    <div style={{
      width: '100%', height: '100%', boxSizing: 'border-box',
      background: bg || THEME.bg,
      color: THEME.fg, fontFamily: THEME.font,
      padding: `${u(4.4)} ${u(5.4)}`,
      display: 'flex', flexDirection: 'column', gap: u(2.0),
      overflow: 'hidden',
    }}>
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', gap: u(2.0) }}>
        {children}
      </div>
      {footer && <Footer n={n} total={total} label={label} />}
    </div>
  );
}

// ── SlideTitle ───────────────────────────────────────────────────────
// Props: children (見出し), sub (string サブタイトル), align ('left'|'center')
function SlideTitle({ children, sub, align = 'left' }) {
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: u(1.0),
      alignItems: align === 'center' ? 'center' : 'flex-start',
      textAlign: align, flexShrink: 0,
    }}>
      <div style={{ fontSize: u(4.6), fontWeight: 800, lineHeight: 1.2, letterSpacing: '-0.01em' }}>
        {children}
      </div>
      <div style={{ height: u(0.42), width: u(6.4), background: THEME.accent, borderRadius: u(0.3) }} />
      {sub && (
        <div style={{ fontSize: u(1.8), color: THEME.muted, lineHeight: 1.5, marginTop: u(0.3) }}>
          {sub}
        </div>
      )}
    </div>
  );
}

// ── Bullets ──────────────────────────────────────────────────────────
// Props:
//   items (array) 文字列、または { text, sub } オブジェクトの配列
//   dense (bool)  行間を詰める（項目数が多いとき）
function Bullets({ items = [], dense }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: dense ? u(1.1) : u(1.7) }}>
      {items.map((it, i) => {
        const o = typeof it === 'string' ? { text: it } : it;
        return (
          <div key={i} style={{ display: 'flex', gap: u(1.2), alignItems: 'baseline' }}>
            <span style={{ color: THEME.accent, fontWeight: 800, fontSize: u(2.0), lineHeight: 1.4 }}>•</span>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: u(2.2), lineHeight: 1.45 }}>{o.text}</div>
              {o.sub && (
                <div style={{ fontSize: u(1.55), color: THEME.muted, lineHeight: 1.5, marginTop: u(0.35) }}>
                  {o.sub}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── TwoCol ───────────────────────────────────────────────────────────
// Props: left (ReactNode), right (ReactNode), ratio (number 左の比率 0〜1, 既定 0.5)
function TwoCol({ left, right, ratio = 0.5 }) {
  return (
    <div style={{ display: 'flex', gap: u(3.4), flex: 1, minHeight: 0 }}>
      <div style={{ flex: ratio, minWidth: 0, display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
        {left}
      </div>
      <div style={{ flex: 1 - ratio, minWidth: 0, display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
        {right}
      </div>
    </div>
  );
}

// ── Big ──────────────────────────────────────────────────────────────
// Props: value (string 強調する数値/短文), label (string 上のラベル), sub (string 下の補足)
function Big({ value, label, sub }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: u(0.9), alignItems: 'center', textAlign: 'center' }}>
      {label && <div style={{ fontSize: u(1.7), color: THEME.muted, letterSpacing: '0.04em' }}>{label}</div>}
      <div style={{ fontSize: u(9.5), fontWeight: 900, lineHeight: 1.02, color: THEME.fg, letterSpacing: '-0.02em' }}>
        {value}
      </div>
      {sub && <div style={{ fontSize: u(1.9), color: THEME.muted, lineHeight: 1.5 }}>{sub}</div>}
    </div>
  );
}

// ── Quote ────────────────────────────────────────────────────────────
// Props: children (引用本文), cite (string 出典)
function Quote({ children, cite }) {
  return (
    <div style={{
      borderLeft: `${u(0.5)} solid ${THEME.accent}`,
      background: THEME.accentSoft,
      padding: `${u(2.4)} ${u(3.0)}`,
      borderRadius: `0 ${u(0.8)} ${u(0.8)} 0`,
      display: 'flex', flexDirection: 'column', gap: u(1.2),
    }}>
      <div style={{ fontSize: u(2.6), lineHeight: 1.55, fontWeight: 600 }}>{children}</div>
      {cite && <div style={{ fontSize: u(1.5), color: THEME.muted }}>— {cite}</div>}
    </div>
  );
}

// ── CodeBlock ────────────────────────────────────────────────────────
// Props: code (string), caption (string ファイル名など)
// シンタックスハイライトは行わない（サンドボックス内に追加ライブラリを持ち込まない方針）
function CodeBlock({ code, caption }) {
  return (
    <div style={{ border: `1px solid ${THEME.border}`, borderRadius: u(0.8), overflow: 'hidden', background: THEME.panel }}>
      {caption && (
        <div style={{
          padding: `${u(0.7)} ${u(1.4)}`, borderBottom: `1px solid ${THEME.border}`,
          fontSize: u(1.25), color: THEME.muted, fontFamily: THEME.mono,
        }}>{caption}</div>
      )}
      <pre style={{
        margin: 0, padding: u(1.6),
        fontFamily: THEME.mono, fontSize: u(1.5), lineHeight: 1.65,
        color: THEME.fg, whiteSpace: 'pre', overflow: 'auto',
      }}>{code}</pre>
    </div>
  );
}

// ── Note ─────────────────────────────────────────────────────────────
// Props: children (補足テキスト)
function Note({ children }) {
  return (
    <div style={{
      border: `1px dashed ${THEME.border}`, borderRadius: u(0.7),
      padding: `${u(1.1)} ${u(1.6)}`, background: THEME.panel,
      fontSize: u(1.45), color: THEME.muted, lineHeight: 1.55,
    }}>{children}</div>
  );
}

// ── Center ───────────────────────────────────────────────────────────
// Props: children, gap (number u 単位)
function Center({ children, gap = 2.2 }) {
  return (
    <div style={{
      flex: 1, minHeight: 0,
      display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center',
      textAlign: 'center', gap: u(gap),
    }}>{children}</div>
  );
}

exports.THEME = THEME;
exports.u = u;
exports.Slide = Slide;
exports.SlideTitle = SlideTitle;
exports.Bullets = Bullets;
exports.TwoCol = TwoCol;
exports.Big = Big;
exports.Quote = Quote;
exports.CodeBlock = CodeBlock;
exports.Note = Note;
exports.Center = Center;
exports.Footer = Footer;
