
// CUSTOMIZE: 状態カラーはそのまま利用可。凡例ラベルのみ日本語化などお好みで調整可
// キー名は task の status フィールドに対応する
const { STOP_PHASE_COLORS, getStopConditions, stopStats, isTaskActive, stopPhase } = require('../lib/stopConditions');
const { BOX_WIDTH, BOX_HEIGHT } = require('./DependencyEdge');

const STATUS_COLORS = {
  not_started:      { bg: '#585b70', text: '#1e1e2e' }, // surface — 未着手
  in_progress:      { bg: '#89b4fa', text: '#1e1e2e' }, // blue    — 進行中
  blocked:          { bg: '#f38ba8', text: '#1e1e2e' }, // red     — ブロック中
  waiting_approval: { bg: '#fab387', text: '#1e1e2e' }, // peach   — 承認待ち
  done:             { bg: '#a6e3a1', text: '#1e1e2e' }, // green   — 完了
};

const STATUS_LABELS = {
  not_started: '未着手',
  in_progress: '進行中',
  blocked: 'ブロック中',
  waiting_approval: '承認待ち',
  done: '完了',
};

// hovered = このノードの停止条件ポップアップが今出ている(App が hover.key と照合して渡す)。
// フェーズ色は意味を持つので変えず、明るいリングで対象だけを示す。
function TaskNode({ task, x, y, hovered, onEnter, onLeave }) {
  const color = STATUS_COLORS[task.status] || { bg: '#cdd6f4', text: '#1e1e2e' };
  const conditions = getStopConditions(task);
  const stats = stopStats(task);
  const phase = stopPhase(task, isTaskActive(task));
  const stopColor = phase ? STOP_PHASE_COLORS[phase] : null;
  const stopIcon = phase === 'cleared' ? '☑' : '⏸';

  const handleEnter = () => {
    if (!onEnter || conditions.length === 0) return;
    onEnter({
      key: task.id,
      title: '#' + task.issueNumber + ' ' + task.title,
      conditions,
      cx: x + BOX_WIDTH / 2,
      cy: y + BOX_HEIGHT,   // ノードの下端から吹き出す
    });
  };

  // ホバーポップアップが使えない環境(キーボード/タッチ)向けのフォールバック
  const nativeTitle = conditions.length === 0 ? undefined
    : '停止条件 ' + stats.done + '/' + stats.total + ' クリア\n'
      + conditions.map(sc => (sc.checked ? '☑ ' : '☐ ') + sc.text).join('\n');

  return (
    <div
      title={nativeTitle}
      onMouseEnter={handleEnter}
      onMouseLeave={onLeave}
      style={{
        position: 'absolute',
        left: x,
        top: y,
        width: BOX_WIDTH,
        background: '#1e1e2e',
        // 未クリアの停止条件がある間は二重枠で強調する(色はフェーズ色)。
        // 全クリア済み(cleared)は通常枠に戻す。
        border: phase && phase !== 'cleared' ? '2px double ' + stopColor : '1px solid #313244',
        borderRadius: 7,
        overflow: 'hidden',
        boxShadow: hovered
          ? '0 0 0 2px #cdd6f4, 0 4px 12px rgba(0,0,0,0.6)'
          : '0 2px 6px rgba(0,0,0,0.5)',
        userSelect: 'none',
      }}
    >
      <div style={{
        background: color.bg,
        color: color.text,
        padding: '5px 10px',
        fontSize: 11,
        fontWeight: 700,
        fontFamily: 'system-ui, sans-serif',
        letterSpacing: '0.01em',
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        gap: 6,
      }}>
        <span style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 }}>
          #{task.issueNumber}
        </span>
        <span style={{ display: 'flex', alignItems: 'center', gap: 5, flexShrink: 0 }}>
          {stats.total > 0 && (
            <span style={{
              fontSize: 9, fontWeight: 700, lineHeight: 1.6,
              background: '#1e1e2e',
              color: stopColor,
              borderRadius: 3, padding: '0 5px', whiteSpace: 'nowrap',
            }}>
              {stopIcon} {stats.done}/{stats.total}
            </span>
          )}
          <span style={{ fontSize: 9, fontWeight: 600 }}>{STATUS_LABELS[task.status] || task.status}</span>
        </span>
      </div>
      <div style={{ padding: '6px 10px' }}>
        <div style={{
          fontSize: 12, color: '#cdd6f4', fontFamily: 'system-ui,sans-serif',
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {task.title}
        </div>
        {task.branch && (
          <div style={{ fontSize: 10, fontFamily: 'monospace', color: '#a6adc8', marginTop: 3 }}>
            {task.branch}
          </div>
        )}
        {stats.total > 0 && (
          <div style={{
            fontSize: 10, marginTop: 4, fontWeight: 600,
            color: stopColor,
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
          }}>
            {stopIcon} 停止条件 {stats.done}/{stats.total} クリア
          </div>
        )}
      </div>
    </div>
  );
}

exports.default = TaskNode;
exports.STATUS_COLORS = STATUS_COLORS;
exports.STATUS_LABELS = STATUS_LABELS;
