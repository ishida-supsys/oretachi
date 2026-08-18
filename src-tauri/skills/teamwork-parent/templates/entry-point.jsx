
const { useState, useRef, useEffect, useCallback } = React;
const TASKS = require('./data/flow').default;
const { DEPENDENCIES, MESSAGES } = require('./data/flow');
const TaskNode = require('./components/TaskNode').default;
const { STATUS_COLORS, STATUS_LABELS } = require('./components/TaskNode');
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
  const lastPos = useRef(null);
  const dragMoved = useRef(false); // ドラッグ中に移動が発生したか追跡
  const containerRef = useRef(null);

  const togglePanel = useCallback(key => {
    setOpenPanel(p => (p === key ? null : key));
  }, []);

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

  const doneCount = TASKS.filter(t => t.status === 'done').length;
  const confirmTasks = TASKS.filter(t => t.requiresUserConfirmation);
  const readyTasks = TASKS.filter(t => {
    if (t.status !== 'not_started') return false;
    const blockedBy = DEPENDENCIES.filter(d => d.to === t.id && d.kind === 'blocks').map(d => d.from);
    return blockedBy.every(id => TASK_MAP[id] && TASK_MAP[id].status === 'done');
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
              fromTask={TASK_MAP[dep.from]} toTask={TASK_MAP[dep.to]} />
          ))}
        </svg>

        {/* Layer 2: タスクノード */}
        {TASKS.map(task => (
          <TaskNode key={task.id} task={task} x={task.x} y={task.y} />
        ))}
      </div>

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
              <div style={{ fontSize: 11, fontWeight: 700, color: '#fab387', fontFamily: FONT }}>
                要ユーザー確認 ({confirmTasks.length})
              </div>
              {confirmTasks.map(t => (
                <div key={t.id} style={{ fontSize: 11, color: '#cdd6f4', fontFamily: FONT, marginTop: 3 }}>
                  #{t.issueNumber} {t.title}
                  {t.confirmationNote && (
                    <div style={{ fontSize: 10, color: '#6c7086' }}>{t.confirmationNote}</div>
                  )}
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
          badge={'▸' + readyTasks.length + ' ⏸' + confirmTasks.length}
          title="次に着手可能 / 要ユーザー確認 / 最近のメッセージ"
          active={openPanel === 'status'}
          onClick={() => togglePanel('status')}
        />
      </div>
    </div>
  );
}

exports.default = App;
