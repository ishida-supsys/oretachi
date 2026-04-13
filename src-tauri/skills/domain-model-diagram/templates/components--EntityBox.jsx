
// CUSTOMIZE: カテゴリ名とカラーをドメインに合わせて定義
// キー名はエンティティの category フィールドに対応する
// bg: Catppuccin Mocha パレットから選択（red:#f38ba8 peach:#fab387 blue:#89b4fa green:#a6e3a1
//     yellow:#f9e2af mauve:#cba6f7 teal:#94e2d5 sky:#89dceb）
// text: 常に #1e1e2e（暗背景上でのコントラスト確保）
const CATEGORY_COLORS = {
  domain:         { bg: '#f38ba8', text: '#1e1e2e' }, // red   — ドメイン層
  usecase:        { bg: '#fab387', text: '#1e1e2e' }, // peach — ユースケース層
  infrastructure: { bg: '#89b4fa', text: '#1e1e2e' }, // blue  — インフラ層
  presentation:   { bg: '#a6e3a1', text: '#1e1e2e' }, // green — プレゼンテーション層
};

// CUSTOMIZE: 凡例に表示する人間向けラベル（日本語可）
// キーは CATEGORY_COLORS のキーと一致させる
const LAYER_LABELS = {
  domain:         'Domain ドメイン層',
  usecase:        'Use Case ユースケース層',
  infrastructure: 'Infrastructure インフラ層',
  presentation:   'Presentation プレゼンテーション層',
};

function EntityBox({ entity, selected, highlighted, dimmed, onClick, x, y }) {
  const color = CATEGORY_COLORS[entity.category] || { bg: '#cdd6f4', text: '#1e1e2e' };
  const active = selected || highlighted;
  const posX = x !== undefined ? x : entity.x;
  const posY = y !== undefined ? y : entity.y;

  return (
    <div
      onClick={() => onClick(entity.id)}
      style={{
        position: 'absolute',
        left: posX,
        top: posY,
        width: 220,
        background: '#1e1e2e',
        border: `2px solid ${active ? color.bg : '#313244'}`,
        borderRadius: 7,
        overflow: 'hidden',
        cursor: 'pointer',
        boxShadow: selected
          ? `0 0 16px ${color.bg}88`
          : highlighted
          ? `0 0 8px ${color.bg}55`
          : '0 2px 6px rgba(0,0,0,0.5)',
        transition: 'left 0.4s ease, top 0.4s ease, opacity 0.35s ease, border-color 0.12s, box-shadow 0.12s',
        zIndex: selected ? 20 : highlighted ? 10 : 1,
        userSelect: 'none',
        opacity: dimmed ? 0.3 : 1,
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
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
      }}>
        {entity.name}
      </div>
      <div style={{ padding: '3px 0 4px' }}>
        {entity.fields.map((field, i) => (
          <div key={i} style={{
            padding: '2px 9px',
            fontSize: 10,
            fontFamily: 'monospace',
            color: '#a6adc8',
            borderTop: i > 0 ? '1px solid #2a2a3e' : undefined,
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}>
            {field}
          </div>
        ))}
      </div>
    </div>
  );
}

exports.default = EntityBox;
exports.CATEGORY_COLORS = CATEGORY_COLORS;
exports.LAYER_LABELS = LAYER_LABELS;
