
// ── data/entities テンプレート例 ──────────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の data/entities モジュールは
// ドメイン分析に基づいて新規生成する。
//
// フィールド仕様:
//   id       (string) : エンティティの一意な識別子。リレーション定義でも使う。
//   name     (string) : 表示名。括弧付きの補足可（例: "EventBus (singleton)"）
//   category (string) : CATEGORY_COLORS のキーに一致する文字列
//   x, y     (number) : グリッド配置の座標（px）
//   fields   (string[]): フィールド/プロパティ一覧。1エントリ = 1行表示
//
// 座標ガイドライン:
//   列幅: 280px（BOX_WIDTH 220 + 間隔 60）
//   列の開始 x: 40, 320, 600, 880, 1160, ...
//   行の高さ見積もり: 26 + fieldCount * 19 + 50 px
//   ※ カテゴリを空間的にまとめると見やすい

const ENTITIES = [
  // ── ドメイン層 ────────────────────────────────────────────────
  { id: 'User', name: 'User', category: 'domain', x: 320, y: 40,
    fields: ['id: string', 'name: string', 'email: string', 'createdAt: Date'] },

  { id: 'Order', name: 'Order', category: 'domain', x: 600, y: 40,
    fields: ['id: string', 'userId: string', 'status: OrderStatus', 'items: OrderItem[]', 'total: number'] },

  // ── ユースケース層 ───────────────────────────────────────────
  { id: 'OrderService', name: 'OrderService', category: 'usecase', x: 40, y: 40,
    fields: ['placeOrder(userId, items): Order', 'cancelOrder(orderId): void', 'getOrderHistory(userId): Order[]'] },

  // ── インフラ層 ────────────────────────────────────────────────
  { id: 'OrderRepository', name: 'OrderRepository', category: 'infrastructure', x: 880, y: 40,
    fields: ['save(order: Order): void', 'findById(id): Order | null', 'findByUserId(userId): Order[]'] },
];

exports.default = ENTITIES;
