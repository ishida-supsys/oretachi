
// このファイルはそのまま利用（カスタマイズ不要）

function Card({ children }) {
  return (
    <div style={{ border: '1px solid #e0e0e0', borderRadius: 8, padding: '16px 20px', background: 'white', display: 'flex', flexDirection: 'column', gap: 12 }}>
      {children}
    </div>
  );
}

function Divider() {
  return <div style={{ borderTop: '1px solid #ebebeb', margin: '4px 0' }} />;
}

function Row({ children, gap = 12 }) {
  return <div style={{ display: 'flex', gap, alignItems: 'center', flexWrap: 'wrap' }}>{children}</div>;
}

function Spacer({ h = 8 }) {
  return <div style={{ height: h }} />;
}

// Frame: 画面全体のラッパー。ヘッダーバー + コンテンツエリア
// Props:
//   title     (string)    ヘッダーに表示するタイトル
//   back      (bool)      true のとき ← 戻るリンクを表示
//   backLabel (string)    戻るリンクのラベル（デフォルト: '戻る'）
//   onBack    (function)  戻るリンクのクリックハンドラ（通常 () => go('...')）
//   rightEl   (ReactNode) ヘッダー右端に配置する要素（例: 編集ボタン）
//   children  (ReactNode) コンテンツエリアの中身
function Frame({ title, back, backLabel = '戻る', onBack, rightEl, children }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: '100%', background: '#fafafa' }}>
      <div style={{
        background: '#f0f0f0', borderBottom: '1px solid #d8d8d8',
        padding: '16px 28px', display: 'flex', alignItems: 'center',
        justifyContent: 'space-between', flexShrink: 0,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          {back && (
            <span onClick={onBack} style={{ fontSize: 13, color: '#555', cursor: 'pointer', fontFamily: 'system-ui' }}>
              ← {backLabel}
            </span>
          )}
          {title && (
            <span style={{ fontSize: 16, fontWeight: 700, color: '#222', fontFamily: 'system-ui' }}>{title}</span>
          )}
        </div>
        {rightEl}
      </div>
      <div style={{ flex: 1, padding: '24px 28px', display: 'flex', flexDirection: 'column', gap: 16 }}>
        {children}
      </div>
    </div>
  );
}

exports.Card = Card;
exports.Divider = Divider;
exports.Row = Row;
exports.Spacer = Spacer;
exports.Frame = Frame;
