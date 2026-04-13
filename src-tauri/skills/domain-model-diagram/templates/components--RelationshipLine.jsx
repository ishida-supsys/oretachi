
// このファイルはそのまま利用（カスタマイズ不要）
// BOX_WIDTH は EntityBox の幅と一致させること（デフォルト 220）

const BOX_WIDTH = 220;

function getBoxHeight(fields) {
  return 26 + fields.length * 19 + 6;
}

// posOverrides: { [entityId]: { x, y } } — フォーカスビューでの動的座標
// focusedId: ツリーモード時の中心エンティティID。指定時は縦方向エッジに固定
function getEdgePoints(fromE, toE, posOverrides, focusedId) {
  const fo = posOverrides && posOverrides[fromE.id];
  const to_ = posOverrides && posOverrides[toE.id];
  const fX = fo ? fo.x : fromE.x;
  const fY = fo ? fo.y : fromE.y;
  const tX = to_ ? to_.x : toE.x;
  const tY = to_ ? to_.y : toE.y;

  const fH = getBoxHeight(fromE.fields);
  const tH = getBoxHeight(toE.fields);
  const fCx = fX + BOX_WIDTH / 2;
  const tCx = tX + BOX_WIDTH / 2;

  let fx, fy, tx, ty;

  if (focusedId) {
    // ツリーモード: 中心エンティティは常に上段、接続先は下段
    // 中心→子: 中心下辺→子上辺、子→中心: 子上辺→中心下辺
    if (fromE.id === focusedId) {
      fx = fCx; fy = fY + fH + 2;
      tx = tCx; ty = tY - 2;
    } else {
      fx = fCx; fy = fY - 2;
      tx = tCx; ty = tY + tH + 2;
    }
  } else {
    const fCy = fY + fH / 2;
    const tCy = tY + tH / 2;
    const dx = tCx - fCx;
    const dy = tCy - fCy;
    if (Math.abs(dx) >= Math.abs(dy)) {
      fx = dx >= 0 ? fX + BOX_WIDTH + 2 : fX - 2;
      fy = fCy;
      tx = dx >= 0 ? tX - 2 : tX + BOX_WIDTH + 2;
      ty = tCy;
    } else {
      fx = fCx;
      fy = dy >= 0 ? fY + fH + 2 : fY - 2;
      tx = tCx;
      ty = dy >= 0 ? tY - 2 : tY + tH + 2;
    }
  }

  return { fx, fy, tx, ty };
}

// renderMode: "line" | "label"
// showLabel: trueのとき常時ラベル表示 (focusモード用)
// posOverrides: フォーカスビューでの動的座標マップ
// focusedId: ツリーモード時の中心エンティティID
function RelationshipLine({ idx, rel, fromEntity, toEntity, highlighted, renderMode, showLabel, posOverrides, focusedId }) {
  if (!fromEntity || !toEntity) return null;
  const { fx, fy, tx, ty } = getEdgePoints(fromEntity, toEntity, posOverrides, focusedId);
  const mx = (fx + tx) / 2;
  const my = (fy + ty) / 2;

  if (renderMode === 'label') {
    if (!highlighted && !showLabel) return null;
    const text = rel.label + ' (' + rel.cardinality + ')';
    const tw = text.length * 5.8 + 16;
    return (
      <g>
        <rect x={mx - tw / 2} y={my - 17} width={tw} height={20} rx={4}
          fill="#1e1e2e" stroke="#cba6f7" strokeWidth={1} />
        <text x={mx} y={my - 4} textAnchor="middle" fill="#cdd6f4"
          fontSize={10} fontFamily="system-ui,sans-serif" fontWeight={600}>
          {text}
        </text>
      </g>
    );
  }

  // renderMode === 'line'
  const markerId = 'arr-' + idx;
  const stroke = (highlighted || showLabel) ? '#cba6f7' : '#45475a';
  const strokeW = (highlighted || showLabel) ? 2.5 : 1;
  const isDashed = rel.label === 'extends';

  return (
    <g>
      <defs>
        <marker id={markerId} markerWidth="16" markerHeight="16" refX="12" refY="8" orient="auto">
          <path d="M0,2 L12,8 L0,14 Z" fill={stroke} />
        </marker>
      </defs>
      <line x1={fx} y1={fy} x2={tx} y2={ty}
        stroke={stroke} strokeWidth={strokeW}
        strokeDasharray={isDashed ? '6,4' : undefined}
        markerEnd={'url(#' + markerId + ')'} />
    </g>
  );
}

exports.default = RelationshipLine;
exports.BOX_WIDTH = BOX_WIDTH;
exports.getBoxHeight = getBoxHeight;
exports.getEdgePoints = getEdgePoints;
