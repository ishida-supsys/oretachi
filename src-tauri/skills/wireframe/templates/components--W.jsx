
const { useState, useRef, useEffect, useCallback } = React;

// CUSTOMIZE: エンティティ名 → Catppuccin Mocha 色のマップ
// キーはエンティティ名（W コンポーネントの `e` prop に渡す文字列と一致させること）
// 色は以下のパレットから選ぶ:
//   red    #f38ba8 — ドメインコアエンティティ
//   peach  #fab387 — サービス・ユースケース層
//   blue   #89b4fa — インフラ・アダプター層
//   green  #a6e3a1 — プレゼンテーション・UI
//   yellow #f9e2af — 値オブジェクト・列挙型
//   mauve  #cba6f7 — 共通・ユーティリティ
//   teal   #94e2d5 — イベント・メッセージ
//   sky    #89dceb — 外部クライアント・API
const EC = {
  // ── 例（実際のドメインに合わせて書き換える） ──────────────────
  // User: '#f38ba8', Task: '#f38ba8',           // red   — ドメイン
  // UserService: '#fab387', TaskService: '#fab387', // peach — サービス
  // CalendarAdapter: '#89b4fa',                  // blue  — アダプター
  // Email: '#f9e2af', TaskStatus: '#f9e2af',     // yellow — 値オブジェクト
};

// W: ワイヤーフレームアイテム（エンティティバインディングツールチップ付き）
// position: fixed でツールチップを描画するため overflow: auto による切り抜け問題なし
function W({ e, f, sa, style, children }) {
  const [hov, setHov] = useState(false);
  const [pos, setPos] = useState(null);
  const ref = useRef(null);
  const color = EC[e] || '#cdd6f4';

  const measure = useCallback(() => {
    if (ref.current && e) {
      const r = ref.current.getBoundingClientRect();
      setPos({ x: r.left, y: r.top });
    }
  }, [e]);

  // showAll 切り替え時 & スクロール時に再計測
  useEffect(() => {
    if (!sa || !e) { if (!hov) setPos(null); return; }
    measure();
    const onScroll = () => measure();
    window.addEventListener('scroll', onScroll, true);
    return () => window.removeEventListener('scroll', onScroll, true);
  }, [sa, measure]);

  return (
    <div ref={ref} style={style}
      onMouseEnter={() => { measure(); setHov(true); }}
      onMouseLeave={() => { setHov(false); if (!sa) setPos(null); }}>
      {children}
      {(hov || sa) && e && pos && (
        <div style={{
          position: 'fixed', left: pos.x, top: pos.y - 26,
          background: '#1e1e2e', color, border: `1px solid ${color}55`,
          borderRadius: 4, padding: '2px 8px', fontSize: 10,
          fontFamily: 'monospace', whiteSpace: 'nowrap',
          zIndex: 9999, pointerEvents: 'none',
          boxShadow: '0 2px 8px rgba(0,0,0,0.6)',
        }}>{e}.{f}</div>
      )}
    </div>
  );
}

exports.default = W;
exports.EC = EC;
