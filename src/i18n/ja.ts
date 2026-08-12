export default {
  taskStatus: {
    generating: '生成中',
    queued: '待機中',
    executing: '実行中',
    completed: '完了',
    error: 'エラー',
  },
  taskTooltipMore: '...他{count}件',
  common: {
    cancel: 'キャンセル',
    delete: '削除',
    close: '閉じる',
    clear: 'クリア',
    notConfigured: '未設定',
  },
  workgroup: {
    autoName: 'グループ({n})',
  },
  search: {
    noResults: '0件',
    results: '{count}件',
    position: '{current} / {total}',
  },
  notification: {
    title: 'Worktree通知',
    titleApproval: '承認が必要です',
    titleCompleted: 'タスク完了',
  },
  // ワークツリー間イベントの配送トースト (issue #120 §7 / #130)。
  // メイン / サブウィンドウが共通の composable から引くため、SFC ローカルではなくここに置く。
  eventDelivery: {
    // メインは非分離の全ワークツリーぶんを出すので、宛先名が無いとどのタブが
    // 動き出したのか分からない（#125 の文言は {name} を渡しながら使っていなかった）
    deliveredSummary: '{name} へワークツリーイベントを配送しました ({count}件)',
    spawnRejectedSummary: '自動 spawn を見送りました',
    spawnRejectedDetail: '{name} に未読が {pending} 件ありますが、ターミナルが {live} 個開いています（上限 {limit}）。ターミナルを整理するか、手動でワークツリーを開いてください',
  },
  update: {
    title: 'oretachi アップデート',
    available: '新しいバージョン {version} が利用可能です。今すぐ更新しますか？',
    checkFailed: 'アップデートの確認に失敗しました。\n{error}',
    installFailed: 'アップデートのインストールに失敗しました。\n{error}',
  },
  panZoom: {
    zoomIn: '拡大 (+)',
    zoomOut: '縮小 (-)',
    fitToView: '全体を表示 (F)',
    resetZoom: '実際のサイズ (0)',
    fullscreen: '全画面ビューアーで開く',
    pin: 'この図で拡大縮小・移動を有効にする',
    unpin: '拡大縮小・移動を無効にする',
  },
  about: {
    label: 'バージョン情報',
    checkUpdate: 'アップデートを確認',
    checking: '確認中...',
    upToDate: '最新バージョンを使用しています。',
  },
}
