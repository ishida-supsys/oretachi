
const { useState } = React;

// CUSTOMIZE: 画面モジュールの require を追加・変更する
// 単一 default export の場合: const XxxScreen = require('./screens/XxxScreen').default;
// 複数画面をまとめた場合:    const { LoginScreen, RegisterScreen } = require('./screens/AuthScreens');
const OverviewScreen = require('./screens/OverviewScreen').default;
// const ExampleScreen = require('./screens/ExampleScreen').default;

// CUSTOMIZE: タブ一覧を定義する
// key: 画面モジュールに対応する一意なキー（kebab-case）
// label: タブに表示する文字列
// 先頭は必ず { key: 'overview', label: 'Overview' } を置く（画面遷移図）
const TABS = [
  { key: 'overview', label: 'Overview' },
  // { key: 'login',      label: 'Login' },
  // { key: 'task-list',  label: 'Task List' },
];

function App() {
  const [tab, setTab] = useState('overview');
  const [showAll, setShowAll] = useState(false);

  return (
    <div style={{
      height: '100vh',
      background: '#11111b',
      fontFamily: 'system-ui, sans-serif',
      display: 'flex',
      flexDirection: 'column',
      boxSizing: 'border-box',
      overflow: 'hidden',
    }}>
      {/* タブバー — フル幅 */}
      <div style={{
        background: '#1e1e2e',
        borderBottom: '1px solid #313244',
        padding: '0 24px',
        display: 'flex',
        alignItems: 'stretch',
        flexShrink: 0,
        gap: 2,
      }}>
        {TABS.map(t => (
          <button key={t.key} onClick={() => setTab(t.key)} style={{
            padding: '12px 20px',
            fontSize: 13, fontWeight: 600,
            border: 'none',
            borderBottom: tab === t.key ? '2px solid #cba6f7' : '2px solid transparent',
            background: 'transparent',
            color: tab === t.key ? '#cba6f7' : '#6c7086',
            cursor: 'pointer',
            whiteSpace: 'nowrap',
          }}>{t.label}</button>
        ))}
        <div style={{ flex: 1 }} />
        <button onClick={() => setShowAll(s => !s)} style={{
          padding: '8px 16px', margin: '6px 0',
          borderRadius: 6, fontSize: 12, fontWeight: 600,
          border: '1px solid ' + (showAll ? '#f9e2af' : '#45475a'),
          background: showAll ? '#f9e2af' : 'transparent',
          color: showAll ? '#1e1e2e' : '#f9e2af',
          cursor: 'pointer',
          alignSelf: 'center',
        }}>パラメータ表示</button>
      </div>

      {/* スクリーン — 残り高さを全て埋める */}
      <div style={{ flex: 1, overflow: 'auto', background: '#fafafa' }}>
        {/* CUSTOMIZE: 各タブキーに対応する画面コンポーネントを条件分岐で描画する
            - OverviewScreen は go のみ（sa 不要）
            - 各画面は sa={showAll} go={setTab} を渡す
            - 複数バリアントがある画面（作成/編集）は画面固有の props も渡す
        */}
        {tab === 'overview'  && <OverviewScreen go={setTab} />}
        {/* {tab === 'login'     && <LoginScreen sa={showAll} go={setTab} />} */}
        {/* {tab === 'task-list' && <ExampleScreen sa={showAll} go={setTab} />} */}
      </div>
    </div>
  );
}

exports.default = App;
