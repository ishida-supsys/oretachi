
// このファイルはそのまま利用（カスタマイズ不要）
// BOX_WIDTH / BOX_HEIGHT は TaskNode のサイズと一致させること

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

// dep: { from, to, kind } — kind: "blocks"(実線) | "informs"(破線)
function DependencyEdge({ idx, dep, fromTask, toTask }) {
  if (!fromTask || !toTask) return null;
  const { fx, fy, tx, ty } = getEdgePoints(fromTask, toTask);
  const markerId = 'dep-arr-' + idx;
  const isInforms = dep.kind === 'informs';
  const stroke = isInforms ? '#585b70' : '#6c7086';

  return (
    <g>
      <defs>
        <marker id={markerId} markerWidth="16" markerHeight="16" refX="12" refY="8" orient="auto">
          <path d="M0,2 L12,8 L0,14 Z" fill={stroke} />
        </marker>
      </defs>
      <line x1={fx} y1={fy} x2={tx} y2={ty}
        stroke={stroke} strokeWidth={isInforms ? 1 : 1.5}
        strokeDasharray={isInforms ? '6,4' : undefined}
        markerEnd={'url(#' + markerId + ')'} />
    </g>
  );
}

exports.default = DependencyEdge;
exports.BOX_WIDTH = BOX_WIDTH;
exports.BOX_HEIGHT = BOX_HEIGHT;
exports.getEdgePoints = getEdgePoints;
