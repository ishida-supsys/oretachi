
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

function App() {
  const [pan, setPan] = useState({ x: 20, y: 20 });
  // CUSTOMIZE: 初期ズームをタスク数・キャンバスサイズに合わせて調整（小さいほど広い範囲が見える）
  const [zoom, setZoom] = useState(1);
  const [dragging, setDragging] = useState(false);
  const lastPos = useRef(null);
  const dragMoved = useRef(false); // ドラッグ中に移動が発生したか追跡
  const containerRef = useRef(null);

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
    setZoom(z => Math.min(2.5, Math.max(0.2, z * factor)));
  }, []);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    el.addEventListener('wheel', handleWheel, { passive: false });
    return () => el.removeEventListener('wheel', handleWheel);
  }, [handleWheel]);

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

      {/* タイトル + 進捗サマリ */}
      <div data-ui="1" style={{
        position: 'absolute', top: 14, left: 14,
        background: 'rgba(24,24,37,0.97)', border: '1px solid #313244',
        borderRadius: 8, padding: '10px 16px', zIndex: 10,
      }}>
        {/* CUSTOMIZE: タイトルを親issueに合わせて変更 */}
        <div style={{ fontSize: 15, fontWeight: 700, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif' }}>
          チームワーク計画フロー
        </div>
        <div style={{ fontSize: 11, color: '#6c7086', fontFamily: 'system-ui,sans-serif', marginTop: 2 }}>
          {doneCount} / {TASKS.length} 完了
        </div>
      </div>

      {/* 操作説明 + ビューリセット */}
      <div data-ui="1" style={{
        position: 'absolute', top: 14, right: 14,
        display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 6, zIndex: 10,
      }}>
        <button
          onClick={resetView}
          style={{
            background: '#313244', border: '1px solid #45475a',
            borderRadius: 6, padding: '6px 14px',
            color: '#cdd6f4', fontSize: 12, fontFamily: 'system-ui,sans-serif',
            cursor: 'pointer', fontWeight: 600,
          }}
        >
          ⟲ 表示をリセット
        </button>
        <div style={{
          background: 'rgba(24,24,37,0.93)', border: '1px solid #313244',
          borderRadius: 6, padding: '7px 13px',
          fontSize: 11, color: '#6c7086', fontFamily: 'system-ui,sans-serif',
          lineHeight: '1.75',
        }}>
          <div>Scroll: zoom | Drag: pan</div>
        </div>
      </div>

      {/* 凡例 */}
      <div data-ui="1" style={{
        position: 'absolute', bottom: 14, left: 14,
        background: 'rgba(24,24,37,0.97)', border: '1px solid #313244',
        borderRadius: 8, padding: '9px 14px',
        display: 'flex', flexDirection: 'column', gap: 4, zIndex: 10,
      }}>
        {Object.keys(STATUS_COLORS).map(key => (
          <div key={key} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <div style={{ width: 10, height: 10, borderRadius: 2, background: STATUS_COLORS[key].bg, flexShrink: 0 }} />
            <span style={{ fontSize: 11, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif' }}>{STATUS_LABELS[key]}</span>
          </div>
        ))}
        <div style={{ fontSize: 10, color: '#6c7086', fontFamily: 'system-ui,sans-serif', marginTop: 4, borderTop: '1px solid #313244', paddingTop: 4 }}>
          実線: blocks / 破線: informs
        </div>
      </div>

      {/* 次に着手可能なタスク + 要ユーザー確認タスク + 最近のメッセージ */}
      <div data-ui="1" style={{
        position: 'absolute', bottom: 14, right: 14,
        display: 'flex', gap: 12, zIndex: 10, flexWrap: 'wrap', maxWidth: '75vw',
      }}>
        <div style={{
          background: 'rgba(24,24,37,0.97)', border: '1px solid #313244',
          borderRadius: 8, padding: '9px 14px', minWidth: 220,
        }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: '#a6e3a1', fontFamily: 'system-ui,sans-serif' }}>
            次に着手可能 ({readyTasks.length})
          </div>
          {readyTasks.map(t => (
            <div key={t.id} style={{ fontSize: 11, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif', marginTop: 3 }}>
              #{t.issueNumber} {t.title}
            </div>
          ))}
        </div>
        <div style={{
          background: 'rgba(24,24,37,0.97)', border: '1px solid #313244',
          borderRadius: 8, padding: '9px 14px', minWidth: 220,
        }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: '#fab387', fontFamily: 'system-ui,sans-serif' }}>
            要ユーザー確認 ({confirmTasks.length})
          </div>
          {confirmTasks.map(t => (
            <div key={t.id} style={{ fontSize: 11, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif', marginTop: 3 }}>
              #{t.issueNumber} {t.title}
              {t.confirmationNote && (
                <div style={{ fontSize: 10, color: '#6c7086' }}>{t.confirmationNote}</div>
              )}
            </div>
          ))}
        </div>
        <div style={{
          background: 'rgba(24,24,37,0.97)', border: '1px solid #313244',
          borderRadius: 8, padding: '9px 14px', minWidth: 260, maxHeight: 140, overflow: 'auto',
        }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: '#89b4fa', fontFamily: 'system-ui,sans-serif' }}>
            最近のメッセージ
          </div>
          {(MESSAGES || []).slice(0, 5).map((m, i) => (
            <div key={i} style={{ fontSize: 10, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif', marginTop: 4 }}>
              <span style={{ color: '#6c7086' }}>{m.ts}</span> {m.from}: {m.text}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

exports.default = App;
