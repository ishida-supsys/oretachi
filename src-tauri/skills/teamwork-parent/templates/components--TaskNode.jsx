
// CUSTOMIZE: 状態カラーはそのまま利用可。凡例ラベルのみ日本語化などお好みで調整可
// キー名は task の status フィールドに対応する
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

function TaskNode({ task, x, y }) {
  const color = STATUS_COLORS[task.status] || { bg: '#cdd6f4', text: '#1e1e2e' };
  const needsConfirmation = !!task.requiresUserConfirmation;

  return (
    <div
      title={needsConfirmation ? (task.confirmationNote || 'ユーザー確認が必要です') : undefined}
      style={{
        position: 'absolute',
        left: x,
        top: y,
        width: 220,
        background: '#1e1e2e',
        border: needsConfirmation ? `2px double #fab387` : '1px solid #313244',
        borderRadius: 7,
        overflow: 'hidden',
        boxShadow: '0 2px 6px rgba(0,0,0,0.5)',
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
      }}>
        <span style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
          #{task.issueNumber}
        </span>
        <span style={{ fontSize: 9, fontWeight: 600 }}>{STATUS_LABELS[task.status] || task.status}</span>
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
        {needsConfirmation && (
          <div style={{ fontSize: 10, color: '#fab387', marginTop: 4, fontWeight: 600 }}>
            ⏸ 要ユーザー確認
          </div>
        )}
      </div>
    </div>
  );
}

exports.default = TaskNode;
exports.STATUS_COLORS = STATUS_COLORS;
exports.STATUS_LABELS = STATUS_LABELS;
