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
  // 自動 spawn の警告トースト (issue #120 §7 / #130 / #137)。
  // メイン / サブウィンドウが共通の composable から引くため、SFC ローカルではなくここに置く。
  // 配送ごとのトースト (deliveredSummary) は #137 で廃止した（購読状態はカードのバッジが見せる）。
  eventDelivery: {
    spawnWarningSummary: 'ターミナルが増えています',
    spawnWarningDetail: '{name} の未読 {pending} 件のためにターミナルを追加しました。開いているターミナルが {live} 個になっています。{threshold} 個以上で画面が固まる事象が報告されているため、使っていないタブを閉じることをおすすめします',
  },
  update: {
    title: 'oretachi アップデート',
    available: '新しいバージョン {version} が利用可能です。今すぐ更新しますか？',
    checkFailed: 'アップデートの確認に失敗しました。\n{error}',
    installFailed: 'アップデートのインストールに失敗しました。\n{error}',
  },
  externalLink: {
    title: '外部リンクを開く',
    confirm: '既定のブラウザで次の URL を開きますか？\n\n{url}',
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
