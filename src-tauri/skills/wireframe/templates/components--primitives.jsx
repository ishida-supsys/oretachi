
// このファイルはそのまま利用（カスタマイズ不要）
// 各コンポーネントは W でラップされ、エンティティバインディングツールチップが付く
// 共通 Props: e (エンティティ名), f (フィールド/メソッド名), sa (showAll フラグ)

const W = require('./W').default;

// WInput: テキスト入力フィールドのプレースホルダー
// Props: label (string), placeholder (string), e, f, sa
function WInput({ label, placeholder = '────────────────', e, f, sa }) {
  return (
    <W e={e} f={f} sa={sa}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
        {label && <span style={{ fontSize: 13, color: '#666', fontFamily: 'system-ui', fontWeight: 500 }}>{label}</span>}
        <div style={{ border: '1px solid #ccc', borderRadius: 6, padding: '10px 14px', background: 'white', fontSize: 14, color: '#bbb', fontFamily: 'system-ui' }}>
          {placeholder}
        </div>
      </div>
    </W>
  );
}

// WTextarea: 複数行テキスト入力フィールドのプレースホルダー
// Props: label (string), e, f, sa
function WTextarea({ label, e, f, sa }) {
  return (
    <W e={e} f={f} sa={sa}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
        {label && <span style={{ fontSize: 13, color: '#666', fontFamily: 'system-ui', fontWeight: 500 }}>{label}</span>}
        <div style={{ border: '1px solid #ccc', borderRadius: 6, padding: '10px 14px', background: 'white', fontSize: 14, color: '#bbb', fontFamily: 'system-ui', height: 100, lineHeight: 1.7 }}>
          ────────────────<br/>──────────<br/>────────────────────
        </div>
      </div>
    </W>
  );
}

// WBtn: ボタン
// Props: label (string), primary (bool), small (bool), e, f, sa, onClick
function WBtn({ label, primary, small, e, f, sa, onClick }) {
  return (
    <W e={e} f={f} sa={sa}>
      <div onClick={onClick} style={{
        border: primary ? 'none' : '1px solid #ccc', borderRadius: 6,
        padding: small ? '8px 14px' : '10px 20px',
        background: primary ? '#444' : '#f0f0f0',
        color: primary ? 'white' : '#555',
        fontSize: small ? 13 : 14, fontWeight: 600, fontFamily: 'system-ui',
        textAlign: 'center', cursor: 'pointer',
      }}>{label}</div>
    </W>
  );
}

// WSelect: ドロップダウン選択フィールドのプレースホルダー
// Props: label (string), e, f, sa
function WSelect({ label, e, f, sa }) {
  return (
    <W e={e} f={f} sa={sa}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
        {label && <span style={{ fontSize: 13, color: '#666', fontFamily: 'system-ui', fontWeight: 500 }}>{label}</span>}
        <div style={{ border: '1px solid #ccc', borderRadius: 6, padding: '10px 14px', background: 'white', fontSize: 14, color: '#999', fontFamily: 'system-ui', display: 'flex', justifyContent: 'space-between' }}>
          <span>選択してください</span><span>▼</span>
        </div>
      </div>
    </W>
  );
}

// WBadge: インラインバッジ（ステータス表示など）
// Props: label (string), color (hex string, デフォルト '#888'), e, f, sa
function WBadge({ label, color = '#888', e, f, sa }) {
  return (
    <W e={e} f={f} sa={sa} style={{ display: 'inline-block' }}>
      <span style={{ background: `${color}22`, color, border: `1px solid ${color}55`, borderRadius: 4, padding: '3px 8px', fontSize: 11, fontFamily: 'monospace', fontWeight: 700 }}>{label}</span>
    </W>
  );
}

// WText: ラベル + 値のペア表示（詳細画面のメタ情報など）
// Props: label (string), value (string), e, f, sa
function WText({ label, value, e, f, sa }) {
  return (
    <W e={e} f={f} sa={sa}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'center', fontFamily: 'system-ui', fontSize: 14 }}>
        {label && <span style={{ color: '#999', fontSize: 13, minWidth: 72, flexShrink: 0 }}>{label}</span>}
        <span style={{ color: '#333' }}>{value}</span>
      </div>
    </W>
  );
}

exports.WInput = WInput;
exports.WTextarea = WTextarea;
exports.WBtn = WBtn;
exports.WSelect = WSelect;
exports.WBadge = WBadge;
exports.WText = WText;
