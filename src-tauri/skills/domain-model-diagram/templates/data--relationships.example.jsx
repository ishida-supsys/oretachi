
// ── data/relationships テンプレート例 ─────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の data/relationships モジュールは
// ドメイン分析に基づいて新規生成する。
//
// フィールド仕様:
//   from        (string) : 関係元エンティティの id
//   to          (string) : 関係先エンティティの id
//   label       (string) : 関係の動詞（例: "contains", "uses", "manages", "extends"）
//                          "extends" のみ破線表示になる
//   cardinality (string) : "1:1" | "1:N" | "N:1" | "N:M"
//
// 方向の意味: from → to の向きで矢印が描画される
// グリッドビューでは線のみ（ラベルなし）
// フォーカスビューでは "label (cardinality)" 形式でラベルが表示される

const RELATIONSHIPS = [
  // OrderService → Order: 生成・管理
  { from: 'OrderService', to: 'Order',           label: 'creates',    cardinality: '1:N' },

  // OrderService → OrderRepository: データアクセス
  { from: 'OrderService', to: 'OrderRepository', label: 'uses',       cardinality: '1:1' },

  // OrderRepository → Order: 永続化
  { from: 'OrderRepository', to: 'Order',        label: 'persists',   cardinality: '1:N' },

  // Order → User: 所有
  { from: 'Order', to: 'User',                   label: 'belongs to', cardinality: 'N:1' },
];

exports.default = RELATIONSHIPS;
