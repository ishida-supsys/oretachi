
// ── screens/<ScreenName> テンプレート例 ──────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の画面モジュールは
// ドメイン分析に基づいて新規生成する。
//
// ── インポートパターン ─────────────────────────────────────────────────
//   const W = require('../components/W').default;
//   const { WInput, WTextarea, WBtn, WSelect, WBadge, WText } = require('../components/primitives');
//   const { Frame, Card, Row, Divider, Spacer } = require('../components/layout');
//
// ── Props パターン ────────────────────────────────────────────────────
//   sa  (bool)     : showAll フラグ。W / プリミティブコンポーネント全てに渡す
//   go  (function) : go('tab-key') でタブ切り替え（画面遷移）
//   + 画面固有の props（例: isCreate, userId など）
//
// ── エクスポートパターン ──────────────────────────────────────────────
//   単一画面: exports.default = XxxScreen;
//   複数画面: exports.XxxScreen = XxxScreen; exports.YyyScreen = YyyScreen;
//
// ── エンティティバインディング ────────────────────────────────────────
//   W / プリミティブコンポーネントの e, f props にエンティティ情報を渡す:
//     e: エンティティ名（例: "Task", "User", "TaskService"）
//     f: フィールド名またはメソッドシグネチャ（例: "title", "createTask(title, dueDate)"）
//   バインディングなしの要素は e, f を省略してよい（ツールチップなし）

const W = require('../components/W').default;
const { WInput, WTextarea, WBtn, WSelect, WBadge, WText } = require('../components/primitives');
const { Frame, Card, Row, Divider, Spacer } = require('../components/layout');

// ══════════════════════════════════════════════════════════════════════
// パターン 1: list 型画面
// 用途: エンティティ一覧表示。行クリックで詳細、ヘッダーに新規作成ボタン
// ══════════════════════════════════════════════════════════════════════

// サンプルデータ（2〜3件あればワイヤーフレームとして十分）
const ITEMS = [
  { title: 'サンプルアイテム 1', status: 'TODO',      sc: '#f9e2af', meta: '2026/5/10' },
  { title: 'サンプルアイテム 2', status: 'COMPLETED', sc: '#a6e3a1', meta: '2026/4/30' },
];

function ListScreen({ sa, go }) {
  return (
    <Frame
      title="一覧"
      // rightEl: ヘッダー右端に配置する要素（ログインユーザー情報など）
      rightEl={
        <W e="User" f="name" sa={sa} style={{ fontSize: 13, color: '#444', fontFamily: 'system-ui' }}>
          山田 太郎
        </W>
      }
    >
      {/* 新規作成ボタン: 右揃え */}
      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        <WBtn label="＋ 新規作成" primary e="ItemService" f="create()" sa={sa} onClick={() => go('item-create')} />
      </div>

      {/* 一覧: 各行を Card でラップ */}
      {ITEMS.map((item, i) => (
        <Card key={i}>
          {/* タイトル: クリックで詳細へ */}
          <W e="Item" f="title" sa={sa}>
            <span onClick={() => go('item-detail')} style={{ fontSize: 14, fontWeight: 600, color: '#222', fontFamily: 'system-ui', cursor: 'pointer', textDecoration: 'underline' }}>
              {item.title}
            </span>
          </W>
          {/* メタ情報: Row で横並び */}
          <Row gap={12}>
            <WBadge label={item.status} color={item.sc} e="Item" f="status" sa={sa} />
            <W e="Item" f="dueDate" sa={sa}>
              <span style={{ fontSize: 12, color: '#888', fontFamily: 'system-ui' }}>期日: {item.meta}</span>
            </W>
          </Row>
        </Card>
      ))}
    </Frame>
  );
}

// ══════════════════════════════════════════════════════════════════════
// パターン 2: detail 型画面
// 用途: エンティティの詳細表示。ヘッダー右に編集ボタン、下部にアクション群
// ══════════════════════════════════════════════════════════════════════

function DetailScreen({ sa, go }) {
  return (
    <Frame
      title="詳細"
      back backLabel="一覧へ" onBack={() => go('item-list')}
      rightEl={
        <WBtn label="編集" small e="ItemService" f="update(id, ...)" sa={sa} onClick={() => go('item-edit')} />
      }
    >
      {/* メインコンテンツ Card */}
      <Card>
        <W e="Item" f="title" sa={sa}>
          <span style={{ fontSize: 16, fontWeight: 700, color: '#111', fontFamily: 'system-ui' }}>サンプルアイテム 1</span>
        </W>
        <W e="Item" f="description" sa={sa}>
          <span style={{ fontSize: 13, color: '#666', fontFamily: 'system-ui', lineHeight: 1.7 }}>
            詳細な説明文がここに入ります。
          </span>
        </W>
      </Card>

      {/* メタ情報 Card: WText でラベル＋値の行を並べる */}
      <Card>
        <WText label="状態" value="TODO" e="Item" f="status" sa={sa} />
        <Divider />
        <WText label="期日" value="2026/5/10" e="Item" f="dueDate" sa={sa} />
        <Divider />
        <WText label="担当者" value="山田 太郎" e="Item" f="assigneeId → User.name" sa={sa} />
      </Card>

      {/* アクションボタン Row */}
      <Row gap={12}>
        <WBtn label="完了にする" primary e="ItemService" f="complete(id)" sa={sa} />
        <WBtn label="キャンセル" e="ItemService" f="cancel(id)" sa={sa} onClick={() => go('item-list')} />
      </Row>
    </Frame>
  );
}

// ══════════════════════════════════════════════════════════════════════
// パターン 3: form 型画面（作成 / 編集の共用）
// 用途: エンティティの作成・編集フォーム。isCreate prop で分岐
// ══════════════════════════════════════════════════════════════════════

function FormScreen({ sa, go, isCreate }) {
  return (
    <Frame
      title={isCreate ? '新規作成' : '編集'}
      back backLabel="キャンセル"
      onBack={() => go(isCreate ? 'item-list' : 'item-detail')}
    >
      {/* フォームコンテナ: maxWidth で横幅を制限して中央配置 */}
      <div style={{ maxWidth: 640, width: '100%', display: 'flex', flexDirection: 'column', gap: 20 }}>
        <WInput label="タイトル *" placeholder="タイトルを入力" e="Item" f="title" sa={sa} />
        <WTextarea label="説明" e="Item" f="description" sa={sa} />
        <WInput label="期日" placeholder="2026/05/10" e="Item" f="dueDate" sa={sa} />
        <WSelect label="担当者" e="Item" f="assigneeId" sa={sa} />

        {/* 作成時のみ表示するフィールド */}
        {isCreate && (
          <W e="ExternalService" f="registerEvent(item)" sa={sa}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '10px 14px', border: '1px solid #ddd', borderRadius: 6, background: 'white', cursor: 'pointer' }}>
              <span style={{ fontSize: 14, color: '#888', fontFamily: 'system-ui' }}>☐</span>
              <span style={{ fontSize: 13, color: '#666', fontFamily: 'system-ui' }}>外部サービスに登録する</span>
            </div>
          </W>
        )}

        {/* 送信 / キャンセルボタン */}
        <Row gap={12}>
          <WBtn
            label={isCreate ? '作成する' : '保存する'}
            primary
            e="ItemService"
            f={isCreate ? 'create(title, ...)' : 'update(id, ...)'}
            sa={sa}
            onClick={() => go(isCreate ? 'item-list' : 'item-detail')}
          />
          <WBtn
            label="キャンセル"
            e="ItemService" f="(cancel)"
            sa={sa}
            onClick={() => go(isCreate ? 'item-list' : 'item-detail')}
          />
        </Row>
      </div>
    </Frame>
  );
}

// ══════════════════════════════════════════════════════════════════════
// パターン 4: auth 型画面
// 用途: ログイン / 新規登録フォーム。maxWidth 480 で中央配置
// ══════════════════════════════════════════════════════════════════════

function AuthScreen({ sa, go }) {
  return (
    <Frame title="アプリ名">
      {/* 中央配置フォームコンテナ */}
      <div style={{ maxWidth: 480, width: '100%', margin: '0 auto', padding: '32px 0', display: 'flex', flexDirection: 'column', gap: 18 }}>
        {/* アプリロゴ代替 */}
        <div style={{ textAlign: 'center', fontFamily: 'system-ui', fontSize: 28, fontWeight: 800, color: '#333', marginBottom: 8 }}>□</div>

        <WInput label="メールアドレス" placeholder="user@example.com" e="Email" f="value" sa={sa} />
        <WInput label="パスワード" placeholder="••••••••" />
        <WBtn label="ログイン" primary e="AuthService" f="authenticate(email, password)" sa={sa} onClick={() => go('main')} />

        {/* 画面切り替えリンク */}
        <div style={{ textAlign: 'center' }}>
          <span onClick={() => go('register')} style={{ fontSize: 13, color: '#777', fontFamily: 'system-ui', textDecoration: 'underline', cursor: 'pointer' }}>
            新規登録はこちら
          </span>
        </div>
      </div>
    </Frame>
  );
}

// エクスポート（単一画面の場合）
exports.default = ListScreen;

// エクスポート（複数画面をまとめる場合）
// exports.ListScreen = ListScreen;
// exports.DetailScreen = DetailScreen;
// exports.FormScreen = FormScreen;
// exports.AuthScreen = AuthScreen;
