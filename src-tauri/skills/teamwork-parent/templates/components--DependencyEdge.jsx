
// このファイルはそのまま利用（カスタマイズ不要）
// BOX_WIDTH / BOX_HEIGHT はノードのサイズ。TaskNode もここから import する
const { STOP_PHASE_COLORS, getStopConditions, isEdgeActive, stopPhase } = require('../lib/stopConditions');

const BOX_WIDTH = 220;
const BOX_HEIGHT = 84;

function getEdgePoints(fromTask, toTask) {
  const fCx = fromTask.x + BOX_WIDTH / 2;
  const tCx = toTask.x + BOX_WIDTH / 2;
  const fCy = fromTask.y + BOX_HEIGHT / 2;
  const tCy = toTask.y + BOX_HEIGHT / 2;
  const dx = tCx - fCx;
  const dy = tCy - fCy;

  let fx, fy, tx, ty;
  if (Math.abs(dx) >= Math.abs(dy)) {
    fx = dx >= 0 ? fromTask.x + BOX_WIDTH + 2 : fromTask.x - 2;
    fy = fCy;
    tx = dx >= 0 ? toTask.x - 2 : toTask.x + BOX_WIDTH + 2;
    ty = tCy;
  } else {
    fx = fCx;
    fy = dy >= 0 ? fromTask.y + BOX_HEIGHT + 2 : fromTask.y - 2;
    tx = tCx;
    ty = dy >= 0 ? toTask.y - 2 : toTask.y + BOX_HEIGHT + 2;
  }
  return { fx, fy, tx, ty };
}

// dep: { from, to, kind, stopConditions } — kind: "blocks"(実線) | "informs"(破線)
// stopConditions は親ワークツリーの停止条件(from → to の遷移で親が止まる)。
// hovered = このエッジの停止条件ポップアップが今出ている(App が hover.key と照合して渡す)。
function DependencyEdge({ idx, dep, fromTask, toTask, hovered, onEnter, onLeave }) {
  if (!fromTask || !toTask) return null;
  const { fx, fy, tx, ty } = getEdgePoints(fromTask, toTask);
  const markerId = 'dep-arr-' + idx;
  const isInforms = dep.kind === 'informs';
  const conditions = getStopConditions(dep);
  const hasStop = conditions.length > 0;
  // 遷移元が done になった時点(= 親が次タスクを起動しようとする時点)だけを橙で強調する。
  // まだ到達していない停止条件は灰、クリア済みは緑の ☑ で示し、線は通常色に戻す。
  const phase = stopPhase(dep, isEdgeActive(fromTask));
  const baseStroke = isInforms ? '#585b70' : '#6c7086';
  const stroke = phase === 'active' ? STOP_PHASE_COLORS.active : baseStroke;
  const badgeColor = phase ? STOP_PHASE_COLORS[phase] : baseStroke;
  const mx = (fx + tx) / 2;
  const my = (fy + ty) / 2;

  const handleEnter = () => {
    if (!onEnter || !hasStop) return;
    onEnter({
      key: 'edge-' + idx,
      title: '#' + fromTask.issueNumber + ' → #' + toTask.issueNumber + ' の遷移',
      conditions,
      cx: mx,
      cy: my + 10,
    });
  };

  return (
    <g>
      <defs>
        <marker id={markerId} markerWidth="16" markerHeight="16" refX="12" refY="8" orient="auto">
          <path d="M0,2 L12,8 L0,14 Z" fill={stroke} />
        </marker>
      </defs>
      <line x1={fx} y1={fy} x2={tx} y2={ty}
        stroke={stroke} strokeWidth={(isInforms ? 1 : 1.5) + (hovered ? 2 : 0)}
        strokeDasharray={isInforms ? '6,4' : undefined}
        markerEnd={'url(#' + markerId + ')'} />
      {hasStop && (
        <g>
          {/* ポップアップ表示中はフェーズ色を変えず、外側に明るいリングを足して対象を示す */}
          {hovered && <circle cx={mx} cy={my} r="12" fill="none" stroke="#cdd6f4" strokeWidth="2" />}
          <circle cx={mx} cy={my} r="9" fill="#1e1e2e" stroke={badgeColor} strokeWidth="1.5" />
          <text x={mx} y={my} textAnchor="middle" dominantBaseline="central"
            fontSize="10" fill={badgeColor}>{phase === 'cleared' ? '☑' : '⏸'}</text>
        </g>
      )}
      {/* 透明の太いヒットライン。SVG層は pointerEvents:'none' のため、ここだけ有効化して
          エッジをホバー可能にする。data-ui は付けない(ドラッグでのパンを妨げないため)。 */}
      <line x1={fx} y1={fy} x2={tx} y2={ty}
        stroke="transparent" strokeWidth={12}
        pointerEvents={hasStop ? 'stroke' : 'none'}
        onMouseEnter={handleEnter} onMouseLeave={onLeave} />
    </g>
  );
}

exports.default = DependencyEdge;
exports.BOX_WIDTH = BOX_WIDTH;
exports.BOX_HEIGHT = BOX_HEIGHT;
exports.getEdgePoints = getEdgePoints;
