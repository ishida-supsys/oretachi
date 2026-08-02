
const { useState, useEffect, useRef, useCallback } = React;

const { THEME } = require('./components/deck');

// CUSTOMIZE: スライドモジュールの require を追加・変更する
// 単一 default export の場合: const TitleSlide = require('./slides/TitleSlide').default;
// 複数スライドをまとめた場合:  const { IntroSlide, AgendaSlide } = require('./slides/IntroSlides');
const TitleSlide = require('./slides/TitleSlide').default;
// const AgendaSlide = require('./slides/AgendaSlide').default;

// CUSTOMIZE: 発表全体のタイトル（フッタ左に表示される）
const DECK_LABEL = 'プレゼンテーションタイトル';

// CUSTOMIZE: スライド一覧を定義する
// key:   スライドモジュールに対応する一意なキー（kebab-case）
// label: サムネイル一覧に表示する文字列
// kind:  スライド種別（title|bullets|two-col|diagram|quote|code|closing）— サムネイルのバッジ表示用
const SLIDES = [
  { key: 'title', label: 'タイトル', kind: 'title' },
  // { key: 'agenda', label: 'アジェンダ', kind: 'bullets' },
];

// スライド枠のサイズを親要素にフィットさせつつ 16:9 を維持する。
// 併せて CSS 変数 --u（スライド幅の 1%）をセットし、components/deck の u(n) が追従する。
function useStageBox(ref) {
  const [box, setBox] = useState({ w: 0, h: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => {
      const cw = el.clientWidth;
      const ch = el.clientHeight;
      if (cw <= 0 || ch <= 0) return;
      const w = Math.min(cw, (ch * 16) / 9);
      setBox({ w, h: (w * 9) / 16 });
    };
    update();
    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', update);
      return () => window.removeEventListener('resize', update);
    }
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return box;
}

function ThumbCard({ slide, n, active, onClick }) {
  return (
    <div onClick={onClick} style={{
      border: active ? `2px solid ${THEME.accent}` : '1px solid #313244',
      borderRadius: 6, overflow: 'hidden', cursor: 'pointer',
      background: '#1e1e2e',
    }}>
      <div style={{
        aspectRatio: '16 / 9', padding: '10px 12px', boxSizing: 'border-box',
        display: 'flex', flexDirection: 'column', gap: 5, background: '#fff',
      }}>
        <div style={{
          fontSize: 10, fontWeight: 800, color: '#3b4252',
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>{slide.label}</div>
        <div style={{ height: 2, width: 22, background: THEME.accent, borderRadius: 1 }} />
        <div style={{ display: 'flex', flexDirection: 'column', gap: 3, marginTop: 2 }}>
          <div style={{ height: 3, width: '82%', background: '#eceef1', borderRadius: 2 }} />
          <div style={{ height: 3, width: '68%', background: '#eceef1', borderRadius: 2 }} />
          <div style={{ height: 3, width: '74%', background: '#eceef1', borderRadius: 2 }} />
        </div>
      </div>
      <div style={{
        display: 'flex', justifyContent: 'space-between',
        padding: '4px 8px', fontSize: 9, fontFamily: 'monospace',
        color: active ? '#cdd6f4' : '#6c7086',
        borderTop: '1px solid #313244',
      }}>
        <span>{n}</span><span>{slide.kind}</span>
      </div>
    </div>
  );
}

function Bar({ children }) {
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 10,
      padding: '8px 14px', background: '#1e1e2e',
      borderTop: '1px solid #313244', flexShrink: 0,
    }}>{children}</div>
  );
}

function BarBtn({ children, onClick, title }) {
  return (
    <button onClick={onClick} title={title} style={{
      border: '1px solid #45475a', borderRadius: 6,
      background: 'transparent', color: '#cdd6f4',
      padding: '5px 12px', fontSize: 13, cursor: 'pointer',
      fontFamily: 'system-ui, sans-serif',
    }}>{children}</button>
  );
}

function App() {
  const [i, setI] = useState(0);
  const [thumbs, setThumbs] = useState(false);
  const [maximized, setMaximized] = useState(false);
  const rootRef = useRef(null);
  const stageRef = useRef(null);
  const box = useStageBox(stageRef);

  const total = SLIDES.length;
  const go = useCallback((n) => {
    setI(Math.max(0, Math.min(total - 1, n)));
    setThumbs(false);
  }, [total]);

  useEffect(() => {
    const onKey = (ev) => {
      if (ev.key === 'ArrowRight' || ev.key === 'PageDown' || ev.key === ' ') {
        ev.preventDefault(); go(i + 1);
      } else if (ev.key === 'ArrowLeft' || ev.key === 'PageUp') {
        ev.preventDefault(); go(i - 1);
      } else if (ev.key === 'Home') {
        ev.preventDefault(); go(0);
      } else if (ev.key === 'End') {
        ev.preventDefault(); go(total - 1);
      } else if (ev.key === 'Escape') {
        // ネイティブ全画面の Esc はブラウザが処理する。ここは擬似全画面用
        if (thumbs) setThumbs(false);
        else if (maximized && !document.fullscreenElement) setMaximized(false);
      } else if (ev.key === 'g' || ev.key === 'G') {
        setThumbs((s) => !s);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [i, go, total, thumbs, maximized]);

  // ネイティブ全画面を Esc やブラウザ UI から抜けた場合も maximized を追従させる。
  // これが無いと state が true のまま残り、「終了」ボタンが全画面に再突入してしまう。
  useEffect(() => {
    const onChange = () => {
      if (!document.fullscreenElement) setMaximized(false);
    };
    document.addEventListener('fullscreenchange', onChange);
    return () => document.removeEventListener('fullscreenchange', onChange);
  }, []);

  // 全画面。sandbox iframe で requestFullscreen が拒否される環境では
  // 操作バーを畳んでスライドだけを表示する擬似全画面にフォールバックする。
  const toggleFullscreen = () => {
    const el = rootRef.current;
    if (document.fullscreenElement) {
      if (document.exitFullscreen) document.exitFullscreen();
      setMaximized(false);
      return;
    }
    // 擬似全画面中（ネイティブ全画面ではない）なら、まず擬似全画面を解除する
    if (maximized) {
      setMaximized(false);
      return;
    }
    const req = el && el.requestFullscreen;
    if (req) {
      req.call(el).then(() => setMaximized(true)).catch(() => setMaximized(true));
    } else {
      setMaximized(true);
    }
  };

  // CUSTOMIZE: 各スライドキーに対応するコンポーネントを条件分岐で描画する
  //   共通 props: n（現在ページ = i + 1）, total, label（DECK_LABEL）
  const renderSlide = (key) => {
    const p = { n: i + 1, total, label: DECK_LABEL };
    if (key === 'title') return <TitleSlide {...p} />;
    // if (key === 'agenda') return <AgendaSlide {...p} />;
    return null;
  };

  const current = SLIDES[i];

  return (
    <div ref={rootRef} style={{
      height: '100vh', width: '100vw', boxSizing: 'border-box',
      background: '#11111b', display: 'flex', flexDirection: 'column',
      overflow: 'hidden', fontFamily: 'system-ui, sans-serif',
    }}>
      {/* ステージ */}
      <div ref={stageRef} style={{
        flex: 1, minHeight: 0, display: 'flex',
        alignItems: 'center', justifyContent: 'center',
        padding: maximized ? 0 : 16, boxSizing: 'border-box',
        position: 'relative',
      }}>
        {box.w > 0 && (
          <div style={{
            width: box.w, height: box.h,
            '--u': `${box.w / 100}px`,
            background: '#fff',
            boxShadow: maximized ? 'none' : '0 6px 28px rgba(0,0,0,0.45)',
            overflow: 'hidden',
          }}>
            {current && renderSlide(current.key)}
          </div>
        )}

        {/* サムネイル一覧オーバーレイ */}
        {thumbs && (
          <div onClick={() => setThumbs(false)} style={{
            position: 'absolute', inset: 0, background: 'rgba(17,17,27,0.94)',
            padding: 24, boxSizing: 'border-box', overflow: 'auto', zIndex: 10,
          }}>
            <div onClick={(ev) => ev.stopPropagation()} style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fill, minmax(150px, 1fr))',
              gap: 14,
            }}>
              {SLIDES.map((s, n) => (
                <ThumbCard key={s.key} slide={s} n={n + 1} active={n === i} onClick={() => go(n)} />
              ))}
            </div>
          </div>
        )}
      </div>

      {/* 進捗バー + 操作バー */}
      {!maximized && (
        <>
          <div style={{ height: 3, background: '#313244', flexShrink: 0 }}>
            <div style={{
              width: `${total > 0 ? ((i + 1) / total) * 100 : 0}%`,
              height: '100%', background: THEME.accent,
              transition: 'width 0.18s ease',
            }} />
          </div>
          <Bar>
            <BarBtn onClick={() => go(i - 1)} title="前のスライド (←)">◀</BarBtn>
            <BarBtn onClick={() => go(i + 1)} title="次のスライド (→ / Space)">▶</BarBtn>
            <span style={{ fontSize: 12.5, color: '#cdd6f4', fontFamily: 'monospace' }}>
              {i + 1} / {total}
            </span>
            <div style={{ flex: 1 }} />
            <span style={{ fontSize: 12, color: '#6c7086' }}>{current ? current.label : ''}</span>
            <div style={{ flex: 1 }} />
            <BarBtn onClick={() => setThumbs((s) => !s)} title="サムネイル一覧 (G)">⊞ サムネイル</BarBtn>
            <BarBtn onClick={toggleFullscreen} title="全画面">⛶ 全画面</BarBtn>
          </Bar>
        </>
      )}

      {/* 擬似全画面時の復帰ボタン */}
      {maximized && (
        <div style={{ position: 'fixed', right: 12, bottom: 12, zIndex: 20, opacity: 0.55 }}>
          <BarBtn onClick={toggleFullscreen} title="全画面を終了">⛶ 終了</BarBtn>
        </div>
      )}
    </div>
  );
}

exports.default = App;
