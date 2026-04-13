
// ── screens/OverviewScreen テンプレート例 ────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の OverviewScreen は
// ドメイン分析で特定した画面リスト・遷移グラフに基づいて新規生成する。
//
// ── SMETA 配列フィールド仕様 ─────────────────────────────────────────
//   id    (string) : タブキー。go(s.id) で対応タブに遷移
//   label (string) : スクリーンボックスに表示するラベル（日本語可）
//   x, y  (number) : SVG 内の左上座標（px）
//   w, h  (number) : ボックスの幅・高さ（デフォルト: w=128, h=46 推奨）
//
// ── SVG 座標ガイドライン ─────────────────────────────────────────────
//   SVG サイズ: width="820" height="370"（画面数・遷移数に応じて調整）
//   スクリーンボックス列ピッチ: 約 200px（x + w + gap）
//   スクリーンボックス行ピッチ: 約 113px（y + h + gap）
//   配置方針: 左→右に時系列フロー、認証系は左端、メイン画面は中央以降
//
// ── 遷移線スタイル ────────────────────────────────────────────────────
//   前進遷移（solid）: stroke="#666" strokeWidth={1.5} markerEnd="url(#fw)"
//   戻り遷移（dashed）: stroke="#bbb" strokeWidth={1.5} strokeDasharray="5,3" markerEnd="url(#bk)"
//   直線遷移: <line x1 y1 x2 y2 ...>
//   曲線遷移: <path d="M ... Q ... ..." ...> （戻り線など交差を避ける場合）
//   ラベル: <text x={mid} y={mid-8} fontSize={9} fill="#888" fontFamily="system-ui">ラベル</text>
//
// ── エクスポートパターン ──────────────────────────────────────────────
//   exports.default = OverviewScreen;

const SMETA = [
  // ── 認証フロー（左端） ──────────────────────────────────────────
  { id: 'login',       label: 'ログイン',   x: 50,  y: 155, w: 128, h: 46 },
  { id: 'register',    label: '新規登録',   x: 50,  y: 268, w: 128, h: 46 },

  // ── メインフロー（中央以降） ────────────────────────────────────
  { id: 'task-list',   label: 'タスク一覧', x: 252, y: 155, w: 128, h: 46 },
  { id: 'task-detail', label: 'タスク詳細', x: 452, y: 75,  w: 128, h: 46 },
  { id: 'task-edit',   label: 'タスク編集', x: 650, y: 75,  w: 128, h: 46 },
  { id: 'task-create', label: 'タスク作成', x: 452, y: 258, w: 128, h: 46 },
];

function OverviewScreen({ go }) {
  return (
    <div style={{ padding: '28px 32px', background: '#f8f8f8', minHeight: '100%' }}>
      <div style={{ fontFamily: 'system-ui', fontSize: 16, fontWeight: 700, color: '#333', marginBottom: 4 }}>画面遷移図</div>
      <div style={{ fontFamily: 'system-ui', fontSize: 12, color: '#aaa', marginBottom: 24 }}>スクリーンをクリックしてプレビューへ</div>
      <div style={{ display: 'flex', justifyContent: 'center' }}>
        <svg width="820" height="370" style={{ background: 'white', border: '1px solid #e0e0e0', borderRadius: 8, display: 'block', maxWidth: '100%' }}>
          <defs>
            {/* 前進遷移用マーカー */}
            <marker id="fw" markerWidth="8" markerHeight="8" refX="6" refY="4" orient="auto">
              <path d="M0,1 L6,4 L0,7 Z" fill="#666" />
            </marker>
            {/* 戻り遷移用マーカー */}
            <marker id="bk" markerWidth="8" markerHeight="8" refX="6" refY="4" orient="auto">
              <path d="M0,1 L6,4 L0,7 Z" fill="#bbb" />
            </marker>
          </defs>

          {/* ── 前進遷移（実線） ── */}
          {/* 接続座標: ボックスの右辺中央 = (x+w, y+h/2)、左辺中央 = (x, y+h/2)、下辺中央 = (x+w/2, y+h) */}
          <line x1={178} y1={178} x2={250} y2={178} stroke="#666" strokeWidth={1.5} markerEnd="url(#fw)" />
          <text x={214} y={172} textAnchor="middle" fontSize={9} fill="#888" fontFamily="system-ui">ログイン成功</text>

          <line x1={114} y1={201} x2={114} y2={266} stroke="#666" strokeWidth={1.5} markerEnd="url(#fw)" />
          <text x={126} y={236} fontSize={9} fill="#888" fontFamily="system-ui">新規登録へ</text>

          <line x1={178} y1={291} x2={250} y2={187} stroke="#666" strokeWidth={1.5} markerEnd="url(#fw)" />
          <text x={226} y={248} fontSize={9} fill="#888" fontFamily="system-ui">登録成功</text>

          <line x1={380} y1={166} x2={450} y2={100} stroke="#666" strokeWidth={1.5} markerEnd="url(#fw)" />
          <text x={424} y={127} fontSize={9} fill="#888" fontFamily="system-ui">タスク選択</text>

          <line x1={380} y1={190} x2={450} y2={270} stroke="#666" strokeWidth={1.5} markerEnd="url(#fw)" />
          <text x={426} y={240} fontSize={9} fill="#888" fontFamily="system-ui">新規作成</text>

          <line x1={580} y1={98} x2={648} y2={98} stroke="#666" strokeWidth={1.5} markerEnd="url(#fw)" />
          <text x={614} y={92} textAnchor="middle" fontSize={9} fill="#888" fontFamily="system-ui">編集</text>

          {/* ── 戻り遷移（破線）: 曲線で交差を回避 ── */}
          <path d="M 454,77 Q 380,18 324,153" stroke="#bbb" strokeWidth={1.5} strokeDasharray="5,3" fill="none" markerEnd="url(#bk)" />
          <text x={380} y={33} textAnchor="middle" fontSize={9} fill="#bbb" fontFamily="system-ui">戻る</text>

          <path d="M 652,73 Q 616,36 582,73" stroke="#bbb" strokeWidth={1.5} strokeDasharray="5,3" fill="none" markerEnd="url(#bk)" />
          <text x={617} y={40} textAnchor="middle" fontSize={9} fill="#bbb" fontFamily="system-ui">保存/キャンセル</text>

          <path d="M 454,300 Q 375,338 324,203" stroke="#bbb" strokeWidth={1.5} strokeDasharray="5,3" fill="none" markerEnd="url(#bk)" />
          <text x={374} y={340} textAnchor="middle" fontSize={9} fill="#bbb" fontFamily="system-ui">作成/キャンセル</text>

          {/* ── スクリーンボックス（クリックで遷移） ── */}
          {SMETA.map(s => (
            <g key={s.id} onClick={() => go(s.id)} style={{ cursor: 'pointer' }}>
              <rect x={s.x} y={s.y} width={s.w} height={s.h} rx={6}
                fill="white" stroke="#555" strokeWidth={1.8} />
              <text x={s.x + s.w / 2} y={s.y + s.h / 2 + 5}
                textAnchor="middle" fontSize={12} fontFamily="system-ui" fontWeight={600} fill="#222">
                {s.label}
              </text>
            </g>
          ))}
        </svg>
      </div>
      <div style={{ marginTop: 16, fontSize: 11, color: '#bbb', fontFamily: 'system-ui' }}>
        実線: 画面遷移（前進） / 破線: 戻る操作
      </div>
    </div>
  );
}

exports.default = OverviewScreen;
