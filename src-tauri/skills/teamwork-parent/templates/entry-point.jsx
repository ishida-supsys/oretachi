
const { useState, useRef, useEffect, useCallback } = React;
const TASKS = require('./data/flow').default;
const { DEPENDENCIES, MESSAGES } = require('./data/flow');
const TaskNode = require('./components/TaskNode').default;
const { STATUS_COLORS, STATUS_LABELS } = require('./components/TaskNode');
const { STOP_PHASE_COLORS, getStopConditions, stopStats } = require('./lib/stopConditions');
const DependencyEdge = require('./components/DependencyEdge').default;
const { BOX_WIDTH, BOX_HEIGHT } = require('./components/DependencyEdge');

// CUSTOMIZE: キャンバスサイズをタスク数・配置に合わせて調整
// CANVAS_W = 最大タスク x + BOX_WIDTH + 40, CANVAS_H = 最大タスク y + BOX_HEIGHT + 40
const CANVAS_W = 1200;
const CANVAS_H = 480;

const TASK_MAP = {};
TASKS.forEach(t => { TASK_MAP[t.id] = t; });

const FONT = 'system-ui,sans-serif';
const CARD = {
  background: 'rgba(24,24,37,0.97)',
  border: '1px solid #313244',
  borderRadius: 8,
};

// 四隅の畳んだ欄。クリックで対応するパネルを開閉する。
// フロー図を覆わないよう、既定ではこのチップだけが見えている状態。
function Chip({ label, badge, active, onClick, title }) {
  return (
    <button
      data-ui="1"
      title={title}
      onClick={onClick}
      style={{
        ...CARD,
        background: active ? '#313244' : CARD.background,
        borderColor: active ? '#45475a' : '#313244',
        padding: '5px 10px',
        display: 'flex', alignItems: 'center', gap: 6,
        color: '#cdd6f4',
        fontSize: 11, fontWeight: 600, fontFamily: FONT,
        cursor: 'pointer', whiteSpace: 'nowrap', lineHeight: 1.4,
      }}
    >
      <span>{label}</span>
      {badge ? <span style={{ color: '#6c7086', fontWeight: 500 }}>{badge}</span> : null}
    </button>
  );
}

// ポップアップの最大幅。ビューポートへのクランプ計算にも使う
const POPUP_MAX_W = 380;

// 停止条件のホバーポップアップ。pan/zoom 変換の外側・最前面に1枚だけ描く。
// 位置はキャンバス座標(hover.cx / hover.cy)を画面座標へ変換して求めるため、
// zoom を上げても文字サイズは変わらず、他ノードにクリップされない。
function StopPopup({ hover, pan, zoom }) {
  if (!hover) return null;
  const total = hover.conditions.length;
  const done = hover.conditions.filter(sc => sc.checked).length;
  const cleared = done === total;
  const accent = cleared ? STOP_PHASE_COLORS.cleared : STOP_PHASE_COLORS.active;

  // コンテナは 100vw/100vh かつ overflow:hidden。はみ出すと読めなくなるのでクランプする
  const vw = typeof window === 'undefined' ? 1280 : window.innerWidth;
  const vh = typeof window === 'undefined' ? 800 : window.innerHeight;
  const half = POPUP_MAX_W / 2;
  const rawLeft = hover.cx * zoom + pan.x;
  const left = Math.min(Math.max(rawLeft, half + 8), Math.max(half + 8, vw - half - 8));
  const rawTop = hover.cy * zoom + pan.y;
  const top = Math.max(rawTop, 8);
  // 内容の高さを概算し、下端に収まらないなら上向きに出す
  const flipUp = top + 46 + total * 20 + 16 > vh;

  return (
    <div style={{
      ...CARD,
      position: 'absolute',
      left, top,
      transform: flipUp ? 'translate(-50%, calc(-100% - 8px))' : 'translate(-50%, 8px)',
      zIndex: 60, // dragging オーバーレイ(50)より上
      pointerEvents: 'none',
      padding: '8px 12px',
      minWidth: 220, maxWidth: POPUP_MAX_W,
      boxShadow: '0 4px 14px rgba(0,0,0,0.6)',
      borderColor: accent,
    }}>
      <div style={{ fontSize: 11, fontWeight: 700, color: accent, fontFamily: FONT }}>
        {cleared ? '☑' : '⏸'} 停止条件 {done}/{total} クリア
      </div>
      <div style={{ fontSize: 10, color: '#a6adc8', fontFamily: FONT, marginTop: 2 }}>
        {hover.title}
      </div>
      <div style={{ marginTop: 6, display: 'flex', flexDirection: 'column', gap: 3 }}>
        {hover.conditions.map(sc => (
          <div key={sc.id} style={{
            fontSize: 11, fontFamily: FONT, lineHeight: 1.5,
            color: sc.checked ? '#6c7086' : '#cdd6f4',
            textDecoration: sc.checked ? 'line-through' : 'none',
          }}>
            {sc.checked ? '☑' : '☐'} {sc.text}
            {sc.checked && sc.checkedAt && (
              <span style={{ color: '#585b70', marginLeft: 6 }}>{sc.checkedAt}</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

// チップから展開されるカード本体。
function PanelBody({ children, style }) {
  return (
    <div data-ui="1" style={{ ...CARD, padding: '9px 14px', ...style }}>
      {children}
    </div>
  );
}

function App() {
  const [pan, setPan] = useState({ x: 20, y: 20 });
  // CUSTOMIZE: 初期ズームをタスク数・キャンバスサイズに合わせて調整（小さいほど広い範囲が見える）
  const [zoom, setZoom] = useState(1);
  const [dragging, setDragging] = useState(false);
  // 開いている欄は常に1つだけ。null = 全部畳んだ状態（既定）
  const [openPanel, setOpenPanel] = useState(null); // null | 'title' | 'help' | 'legend' | 'status'
  // 停止条件ポップアップのホバー対象(ノード or エッジ)。同時に1つだけ
  const [hover, setHover] = useState(null);
  const lastPos = useRef(null);
  const dragMoved = useRef(false); // ドラッグ中に移動が発生したか追跡
  const containerRef = useRef(null);

  const togglePanel = useCallback(key => {
    setOpenPanel(p => (p === key ? null : key));
  }, []);

  const handleStopEnter = useCallback(payload => setHover(payload), []);
  const handleStopLeave = useCallback(() => setHover(null), []);

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
      setHover(null); // ドラッグ中はポップアップを出さない
    }
    setPan(p => ({ x: p.x + dx, y: p.y + dy }));
    lastPos.current = { x: e.clientX, y: e.clientY };
  }, []);

  const handleMouseUp = useCallback(() => {
    // キャンバスを（パンせずに）クリックしたら開いている欄を閉じる
    if (lastPos.current && !dragMoved.current) setOpenPanel(null);
    setDragging(false);
    lastPos.current = null;
  }, []);

  const handleWheel = useCallback(e => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.9 : 1.1;
    setZoom(z => Math.min(2.5, Math.max(0.2, z * factor)));
  }, []);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    el.addEventListener('wheel', handleWheel, { passive: false });
    return () => el.removeEventListener('wheel', handleWheel);
  }, [handleWheel]);

  // Esc でも開いている欄を閉じられる
  useEffect(() => {
    const onKeyDown = e => { if (e.key === 'Escape') setOpenPanel(null); };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const resetView = useCallback(() => {
    setPan({ x: 20, y: 20 });
    setZoom(1); // CUSTOMIZE: 初期ズームに合わせる
  }, []);

  const svgStyle = {
    position: 'absolute', left: 0, top: 0,
    width: CANVAS_W, height: CANVAS_H,
    pointerEvents: 'none', overflow: 'visible',
  };
  const svgViewBox = '0 0 ' + CANVAS_W + ' ' + CANVAS_H;

  // ポップアップが指している対象(ノード or エッジ)。ドラッグ中はポップアップを出さないので null
  const hoverKey = dragging || !hover ? null : hover.key;

  const doneCount = TASKS.filter(t => t.status === 'done').length;
  // 未クリアの停止条件を持つタスク/エッジ(= hasOpenStop)。まだ到達していない pending も含む
  // 「これから人の判定が必要になる箇所」の一覧なので、フェーズでは絞らない。
  const confirmTasks = TASKS.filter(t => stopStats(t).hasOpenStop);
  const confirmEdges = DEPENDENCIES
    .map((d, i) => ({ dep: d, idx: i }))
    .filter(({ dep }) => stopStats(dep).hasOpenStop);
  const confirmCount = confirmTasks.length + confirmEdges.length;
  // 着手可能 = (1) blocks 依存元がすべて done、かつ (2) そのタスクへ入るエッジの停止条件が
  // すべてクリア済み。(2) は kind を問わない — 親の停止条件が未クリアのまま次タスクを
  // 起動してはいけないため、informs エッジに付いた停止条件も起動を止める。
  const readyTasks = TASKS.filter(t => {
    if (t.status !== 'not_started') return false;
    const incoming = DEPENDENCIES.filter(d => d.to === t.id);
    const blocksDone = incoming
      .filter(d => d.kind === 'blocks')
      .every(d => TASK_MAP[d.from] && TASK_MAP[d.from].status === 'done');
    const stopsCleared = incoming.every(d => !stopStats(d).hasOpenStop);
    return blocksDone && stopsCleared;
  });

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
    >
      {/* ドラッグ中は透明オーバーレイを重ねてノードへのホバーを防ぐ */}
      {dragging && (
        <div style={{ position: 'absolute', inset: 0, zIndex: 50, cursor: 'grabbing' }} />
      )}
      <div style={{
        position: 'absolute',
        transform: 'translate(' + pan.x + 'px,' + pan.y + 'px) scale(' + zoom + ')',
        transformOrigin: '0 0',
        width: CANVAS_W, height: CANVAS_H,
      }}>
        {/* Layer 1: 依存線 */}
        <svg style={{ ...svgStyle, zIndex: 0 }} viewBox={svgViewBox}>
          {DEPENDENCIES.map((dep, i) => (
            <DependencyEdge key={i} idx={i} dep={dep}
              fromTask={TASK_MAP[dep.from]} toTask={TASK_MAP[dep.to]}
              hovered={hoverKey === 'edge-' + i}
              onEnter={handleStopEnter} onLeave={handleStopLeave} />
          ))}
        </svg>

        {/* Layer 2: タスクノード */}
        {TASKS.map(task => (
          <TaskNode key={task.id} task={task} x={task.x} y={task.y}
            hovered={hoverKey === task.id}
            onEnter={handleStopEnter} onLeave={handleStopLeave} />
        ))}
      </div>

      {/* 停止条件ポップアップ: pan/zoom 変換の外側・最前面に1枚だけ。ドラッグ中は出さない */}
      <StopPopup hover={dragging ? null : hover} pan={pan} zoom={zoom} />

      {/* 左上: 進捗チップ（展開でタイトル表示） */}
      <div style={{
        position: 'absolute', top: 14, left: 14, zIndex: 10,
        display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: 6,
      }}>
        <Chip
          label={'ⓘ ' + doneCount + ' / ' + TASKS.length + ' 完了'}
          active={openPanel === 'title'}
          title="タイトルを表示"
          onClick={() => togglePanel('title')}
        />
        {openPanel === 'title' && (
          <PanelBody style={{ padding: '10px 16px' }}>
            {/* CUSTOMIZE: タイトルを親issueに合わせて変更 */}
            <div style={{ fontSize: 15, fontWeight: 700, color: '#cdd6f4', fontFamily: FONT }}>
              チームワーク計画フロー
            </div>
          </PanelBody>
        )}
      </div>

      {/* 右上: ビューリセット + 操作説明チップ */}
      <div style={{
        position: 'absolute', top: 14, right: 14, zIndex: 10,
        display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 6,
      }}>
        <div style={{ display: 'flex', gap: 6 }}>
          <Chip label="⟲" title="表示をリセット" onClick={resetView} />
          <Chip
            label="?"
            title="操作説明"
            active={openPanel === 'help'}
            onClick={() => togglePanel('help')}
          />
        </div>
        {openPanel === 'help' && (
          <PanelBody style={{
            padding: '7px 13px',
            fontSize: 11, color: '#6c7086', fontFamily: FONT, lineHeight: '1.75',
          }}>
            <div>Scroll: zoom | Drag: pan</div>
            <div>Esc / 背景クリック: 欄を閉じる</div>
            <div>⏸ 付きのノード/線にホバー: 停止条件を表示(対象を白枠で強調)</div>
          </PanelBody>
        )}
      </div>

      {/* 左下: 凡例チップ（展開は上方向） */}
      <div style={{
        position: 'absolute', bottom: 14, left: 14, zIndex: 10,
        display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: 6,
      }}>
        {openPanel === 'legend' && (
          <PanelBody style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {Object.keys(STATUS_COLORS).map(key => (
              <div key={key} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <div style={{ width: 10, height: 10, borderRadius: 2, background: STATUS_COLORS[key].bg, flexShrink: 0 }} />
                <span style={{ fontSize: 11, color: '#cdd6f4', fontFamily: FONT }}>{STATUS_LABELS[key]}</span>
              </div>
            ))}
            <div style={{ fontSize: 10, color: '#6c7086', fontFamily: FONT, marginTop: 4, borderTop: '1px solid #313244', paddingTop: 4 }}>
              実線: blocks / 破線: informs
            </div>
            <div style={{ fontSize: 10, color: '#6c7086', fontFamily: FONT }}>
              停止条件 — ⏸灰: 未実行 ／ ⏸橙: 実行中(要判定) ／ ☑緑: 実行済み
            </div>
          </PanelBody>
        )}
        <Chip label="凡例" active={openPanel === 'legend'} onClick={() => togglePanel('legend')} />
      </div>

      {/* 右下: 状況チップ（次に着手可能 + 要ユーザー確認 + 最近のメッセージ。展開は上方向） */}
      <div style={{
        position: 'absolute', bottom: 14, right: 14, zIndex: 10,
        display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 6,
      }}>
        {openPanel === 'status' && (
          <div style={{
            display: 'flex', gap: 12, flexWrap: 'wrap', justifyContent: 'flex-end',
            maxWidth: '60vw', maxHeight: '55vh', overflowY: 'auto',
          }}>
            <PanelBody style={{ minWidth: 220 }}>
              <div style={{ fontSize: 11, fontWeight: 700, color: '#a6e3a1', fontFamily: FONT }}>
                次に着手可能 ({readyTasks.length})
              </div>
              {readyTasks.map(t => (
                <div key={t.id} style={{ fontSize: 11, color: '#cdd6f4', fontFamily: FONT, marginTop: 3 }}>
                  #{t.issueNumber} {t.title}
                </div>
              ))}
            </PanelBody>
            <PanelBody style={{ minWidth: 220 }}>
              <div style={{ fontSize: 11, fontWeight: 700, color: STOP_PHASE_COLORS.active, fontFamily: FONT }}>
                未クリアの停止条件 ({confirmCount})
              </div>
              {confirmTasks.map(t => (
                <div key={t.id} style={{ fontSize: 11, color: '#cdd6f4', fontFamily: FONT, marginTop: 3 }}>
                  #{t.issueNumber} {t.title}
                  {getStopConditions(t).filter(sc => !sc.checked).map(sc => (
                    <div key={sc.id} style={{ fontSize: 10, color: '#6c7086' }}>☐ {sc.text}</div>
                  ))}
                </div>
              ))}
              {confirmEdges.map(({ dep, idx }) => (
                <div key={'edge-' + idx} style={{ fontSize: 11, color: '#cdd6f4', fontFamily: FONT, marginTop: 3 }}>
                  #{(TASK_MAP[dep.from] || {}).issueNumber} → #{(TASK_MAP[dep.to] || {}).issueNumber}
                  <span style={{ color: '#6c7086' }}> (親の停止条件)</span>
                  {getStopConditions(dep).filter(sc => !sc.checked).map(sc => (
                    <div key={sc.id} style={{ fontSize: 10, color: '#6c7086' }}>☐ {sc.text}</div>
                  ))}
                </div>
              ))}
            </PanelBody>
            <PanelBody style={{ minWidth: 260, maxHeight: 140, overflow: 'auto' }}>
              <div style={{ fontSize: 11, fontWeight: 700, color: '#89b4fa', fontFamily: FONT }}>
                最近のメッセージ
              </div>
              {(MESSAGES || []).slice(0, 5).map((m, i) => (
                <div key={i} style={{ fontSize: 10, color: '#cdd6f4', fontFamily: FONT, marginTop: 4 }}>
                  <span style={{ color: '#6c7086' }}>{m.ts}</span> {m.from}: {m.text}
                </div>
              ))}
            </PanelBody>
          </div>
        )}
        <Chip
          label="状況"
          badge={'▸' + readyTasks.length + ' ⏸' + confirmCount}
          title={'次に着手可能 / 未クリアの停止条件(子' + confirmTasks.length + ' + 親' + confirmEdges.length + ') / 最近のメッセージ'}
          active={openPanel === 'status'}
          onClick={() => togglePanel('status')}
        />
      </div>
    </div>
  );
}

exports.default = App;
