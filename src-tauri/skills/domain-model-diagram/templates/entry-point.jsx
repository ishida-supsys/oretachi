
const { useState, useRef, useEffect, useCallback, useMemo } = React;
const ENTITIES = require('./data/entities').default;
const RELATIONSHIPS = require('./data/relationships').default;
const EntityBox = require('./components/EntityBox').default;
const { CATEGORY_COLORS, LAYER_LABELS } = require('./components/EntityBox');
const RelationshipLine = require('./components/RelationshipLine').default;
const { getBoxHeight, BOX_WIDTH } = require('./components/RelationshipLine');

// CUSTOMIZE: キャンバスサイズをエンティティ数・配置に合わせて調整
// CANVAS_W = 最大エンティティ x + BOX_WIDTH + 40, CANVAS_H = 最大エンティティ y + 最大高さ + 40
const CANVAS_W = 1480;
const CANVAS_H = 1340;

// フォーカスビューのキャンバス水平中心
const FC_X = CANVAS_W / 2;

// エンティティIDからインデックスへのマップをグローバルで生成
const ENTITY_MAP_STATIC = {};
ENTITIES.forEach(e => { ENTITY_MAP_STATIC[e.id] = e; });

// エンティティに接続されている他エンティティIDリストを返す
function getConnectedIds(entityId) {
  const s = new Set();
  RELATIONSHIPS.forEach(r => {
    if (r.from === entityId) s.add(r.to);
    if (r.to === entityId) s.add(r.from);
  });
  return Array.from(s);
}

// フォーカスビュー用ツリー座標計算
// 中心エンティティ → 上段中央、接続エンティティ → 下段に横一列
function computeFocusPositions(centerId, connectedIds) {
  const n = connectedIds.length;
  const positions = {};
  const centerEntity = ENTITY_MAP_STATIC[centerId];
  const centerH = getBoxHeight(centerEntity.fields);
  const TREE_TOP = 60;
  const CHILD_Y = TREE_TOP + centerH + 100;
  const GAP = 40;
  const totalWidth = n * BOX_WIDTH + (n - 1) * GAP;
  const startX = FC_X - totalWidth / 2;

  positions[centerId] = {
    x: FC_X - BOX_WIDTH / 2,
    y: TREE_TOP,
  };

  connectedIds.forEach((id, i) => {
    const entity = ENTITY_MAP_STATIC[id];
    if (!entity) return;
    positions[id] = {
      x: startX + i * (BOX_WIDTH + GAP),
      y: CHILD_Y,
    };
  });

  return positions;
}

function App() {
  // layoutMode: "grid" | "focus"
  const [layoutMode, setLayoutMode] = useState('grid');
  const [focusedId, setFocusedId] = useState(null);
  const [pan, setPan] = useState({ x: 20, y: 20 });
  // CUSTOMIZE: 初期ズームをエンティティ数・キャンバスサイズに合わせて調整（小さいほど広い範囲が見える）
  const [zoom, setZoom] = useState(0.62);
  const [dragging, setDragging] = useState(false);
  const lastPos = useRef(null);
  const dragMoved = useRef(false); // ドラッグ中に移動が発生したか追跡
  const containerRef = useRef(null);

  // フォーカスビューの接続エンティティと座標
  const focusConnected = useMemo(() => {
    if (!focusedId) return [];
    return getConnectedIds(focusedId);
  }, [focusedId]);

  const posOverrides = useMemo(() => {
    if (layoutMode !== 'focus' || !focusedId) return null;
    return computeFocusPositions(focusedId, focusConnected);
  }, [layoutMode, focusedId, focusConnected]);

  // フォーカスビューで表示するエンティティ（中心 + 接続のみ）
  const visibleEntityIds = useMemo(() => {
    if (layoutMode !== 'focus' || !focusedId) return null; // null = 全て表示
    const s = new Set([focusedId, ...focusConnected]);
    return s;
  }, [layoutMode, focusedId, focusConnected]);

  // フォーカスビューで表示するリレーション
  const visibleRelIndices = useMemo(() => {
    if (layoutMode !== 'focus' || !focusedId) return null; // null = 全て表示
    const s = new Set();
    RELATIONSHIPS.forEach((r, i) => {
      if (r.from === focusedId || r.to === focusedId) s.add(i);
    });
    return s;
  }, [layoutMode, focusedId]);

  // マウスイベント
  // data-ui 要素（UI コントロール）上のドラッグは無視、それ以外はどこからでもパン可能
  const handleMouseDown = useCallback(e => {
    if (e.target.closest && e.target.closest('[data-ui]')) return;
    lastPos.current = { x: e.clientX, y: e.clientY };
    dragMoved.current = false;
  }, []);

  // 3px 以上移動した時点でドラッグ開始とみなし、dragging フラグをセット
  const handleMouseMove = useCallback(e => {
    if (!lastPos.current) return;
    const dx = e.clientX - lastPos.current.x;
    const dy = e.clientY - lastPos.current.y;
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) {
      dragMoved.current = true;
      setDragging(true);
    }
    setPan(p => ({ x: p.x + dx, y: p.y + dy }));
    lastPos.current = { x: e.clientX, y: e.clientY };
  }, []);

  const handleMouseUp = useCallback(() => {
    setDragging(false);
    lastPos.current = null;
  }, []);

  const handleWheel = useCallback(e => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.9 : 1.1;
    setZoom(z => Math.min(2.5, Math.max(0.15, z * factor)));
  }, []);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    el.addEventListener('wheel', handleWheel, { passive: false });
    return () => el.removeEventListener('wheel', handleWheel);
  }, [handleWheel]);

  // フォーカスビューに切り替えてビューポートをツリー中央に合わせる
  const enterFocus = useCallback((id) => {
    setFocusedId(id);
    setLayoutMode('focus');
    if (containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect();
      const newZoom = 0.72;
      const centerEntity = ENTITY_MAP_STATIC[id];
      const centerH = getBoxHeight(centerEntity.fields);
      const TREE_TOP = 60;
      const CHILD_Y = TREE_TOP + centerH + 100;
      const treeMidY = (TREE_TOP + CHILD_Y + 89) / 2; // 上段中央〜下段中央の中点
      setZoom(newZoom);
      setPan({
        x: rect.width / 2 - FC_X * newZoom,
        y: rect.height / 2 - treeMidY * newZoom,
      });
    }
  }, []);

  // グリッドビューに戻る
  const backToGrid = useCallback(() => {
    setLayoutMode('grid');
    setFocusedId(null);
    setPan({ x: 20, y: 20 });
    setZoom(0.62); // CUSTOMIZE: 初期ズームに合わせる
  }, []);

  // ドラッグ中（移動あり）ならクリックを無視してフォーカス誤作動を防ぐ
  const handleEntityClick = useCallback(id => {
    if (dragMoved.current) return;
    if (layoutMode === 'focus' && id === focusedId) {
      // フォーカス中のエンティティを再クリック → グリッドビューに戻る
      backToGrid();
    } else {
      enterFocus(id);
    }
  }, [layoutMode, focusedId, enterFocus, backToGrid]);

  const handleBgClick = useCallback(e => {
    // 背景クリックでは何もしない（ドラッグ後の意図しない戻りを防ぐ）
  }, []);

  const svgStyle = {
    position: 'absolute', left: 0, top: 0,
    width: CANVAS_W, height: CANVAS_H,
    pointerEvents: 'none', overflow: 'visible',
  };
  const svgViewBox = '0 0 ' + CANVAS_W + ' ' + CANVAS_H;
  const categories = Object.keys(CATEGORY_COLORS);

  const isFocus = layoutMode === 'focus';

  return (
    <div
      ref={containerRef}
      style={{
        width: '100vw', height: '100vh', overflow: 'hidden',
        background: '#181825',
        cursor: dragging ? 'grabbing' : 'grab',
        position: 'relative', userSelect: 'none',
      }}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
      onClick={handleBgClick}
    >
      {/* ドラッグ中は透明オーバーレイを重ねてエンティティへのホバーを防ぐ */}
      {dragging && (
        <div style={{ position: 'absolute', inset: 0, zIndex: 50, cursor: 'grabbing' }} />
      )}
      <div style={{
        position: 'absolute',
        transform: 'translate(' + pan.x + 'px,' + pan.y + 'px) scale(' + zoom + ')',
        transformOrigin: '0 0',
        width: CANVAS_W, height: CANVAS_H,
      }}>
        {/* Layer 1: 接続線 */}
        <svg style={{ ...svgStyle, zIndex: 0 }} viewBox={svgViewBox}>
          {RELATIONSHIPS.map((rel, i) => {
            if (visibleRelIndices && !visibleRelIndices.has(i)) return null;
            return (
              <RelationshipLine key={i} idx={i} rel={rel} renderMode="line"
                fromEntity={ENTITY_MAP_STATIC[rel.from]}
                toEntity={ENTITY_MAP_STATIC[rel.to]}
                highlighted={false}
                showLabel={false}
                posOverrides={posOverrides}
                focusedId={isFocus ? focusedId : null}
              />
            );
          })}
        </svg>

        {/* Layer 2: エンティティボックス */}
        {ENTITIES.map(entity => {
          const isVisible = !visibleEntityIds || visibleEntityIds.has(entity.id);
          const pos = posOverrides && posOverrides[entity.id];
          const isCenter = isFocus && entity.id === focusedId;
          const isConnected = isFocus && focusConnected.includes(entity.id);
          // フォーカスエンティティはラッパーレベルで高いz-indexを持ち、
          // DOM順に関わらず dimmed エンティティの前面に描画される
          const wrapperZ = isCenter ? 30 : isConnected ? 20 : 1;
          return (
            <div key={entity.id} data-entity="1" style={{ position: 'relative', zIndex: wrapperZ }}>
              <EntityBox
                entity={entity}
                selected={isCenter}
                highlighted={isConnected}
                dimmed={isFocus && !isVisible}
                onClick={handleEntityClick}
                x={pos ? pos.x : undefined}
                y={pos ? pos.y : undefined}
              />
            </div>
          );
        })}

        {/* Layer 3: ラベル（フォーカスビューでは常時表示、グリッドビューでは非表示） */}
        <svg style={{ ...svgStyle, zIndex: 3 }} viewBox={svgViewBox}>
          {RELATIONSHIPS.map((rel, i) => {
            if (visibleRelIndices && !visibleRelIndices.has(i)) return null;
            return (
              <RelationshipLine key={'lbl' + i} idx={i} rel={rel} renderMode="label"
                fromEntity={ENTITY_MAP_STATIC[rel.from]}
                toEntity={ENTITY_MAP_STATIC[rel.to]}
                highlighted={false}
                showLabel={isFocus}
                posOverrides={posOverrides}
                focusedId={isFocus ? focusedId : null}
              />
            );
          })}
        </svg>
      </div>

      {/* タイトル */}
      <div data-ui="1" style={{
        position: 'absolute', top: 14, left: 14,
        background: 'rgba(24,24,37,0.97)', border: '1px solid #313244',
        borderRadius: 8, padding: '10px 16px', zIndex: 10,
      }}>
        {/* CUSTOMIZE: タイトルをダイアグラムの対象に合わせて変更 */}
        <div style={{ fontSize: 15, fontWeight: 700, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif' }}>
          Domain Model Diagram
        </div>
        <div style={{ fontSize: 11, color: '#6c7086', fontFamily: 'system-ui,sans-serif', marginTop: 2 }}>
          {ENTITIES.length} entities · {RELATIONSHIPS.length} relationships
        </div>
      </div>

      {/* フォーカスビュー: 戻るボタン + エンティティ名 */}
      {isFocus && (
        <div data-ui="1" style={{
          position: 'absolute', top: 14, right: 14,
          display: 'flex', flexDirection: 'column', gap: 6, zIndex: 10,
        }}>
          <button
            onClick={backToGrid}
            style={{
              background: '#313244', border: '1px solid #45475a',
              borderRadius: 6, padding: '6px 14px',
              color: '#cdd6f4', fontSize: 12, fontFamily: 'system-ui,sans-serif',
              cursor: 'pointer', fontWeight: 600,
            }}
          >
            ← Back to overview
          </button>
          <div style={{
            background: 'rgba(24,24,37,0.95)', border: '1px solid #45475a',
            borderRadius: 6, padding: '6px 12px',
            fontSize: 11, color: '#6c7086', fontFamily: 'system-ui,sans-serif',
            lineHeight: 1.7,
          }}>
            <div style={{ color: '#cba6f7', fontWeight: 700, fontSize: 12 }}>{focusedId}</div>
            <div>{focusConnected.length} connections</div>
            <div style={{ marginTop: 2, color: '#585b70' }}>Click center: back to grid</div>
            <div style={{ color: '#585b70' }}>Click other: re-focus</div>
          </div>
        </div>
      )}

      {/* グリッドビュー: 操作説明 */}
      {!isFocus && (
        <div data-ui="1" style={{
          position: 'absolute', top: 14, right: 14,
          background: 'rgba(24,24,37,0.93)', border: '1px solid #313244',
          borderRadius: 6, padding: '7px 13px',
          fontSize: 11, color: '#6c7086', fontFamily: 'system-ui,sans-serif',
          lineHeight: '1.75', zIndex: 10,
        }}>
          <div>Scroll: zoom | Drag: pan</div>
          <div>Click entity: focus view + labels</div>
        </div>
      )}

      {/* カテゴリ凡例 */}
      <div data-ui="1" style={{
        position: 'absolute', bottom: 14, left: 14,
        background: 'rgba(24,24,37,0.97)', border: '1px solid #313244',
        borderRadius: 8, padding: '9px 14px',
        display: 'flex', flexWrap: 'wrap', gap: '5px 14px', maxWidth: 640,
        pointerEvents: 'none', zIndex: 10,
      }}>
        {categories.map(cat => (
          <div key={cat} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <div style={{ width: 10, height: 10, borderRadius: 2, background: CATEGORY_COLORS[cat].bg, flexShrink: 0 }} />
            <span style={{ fontSize: 11, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif' }}>{LAYER_LABELS[cat] || cat}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

exports.default = App;
