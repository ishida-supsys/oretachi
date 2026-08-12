export default {
  taskStatus: {
    generating: 'Generating',
    queued: 'Queued',
    executing: 'Running',
    completed: 'Done',
    error: 'Error',
  },
  taskTooltipMore: '...and {count} more',
  common: {
    cancel: 'Cancel',
    delete: 'Delete',
    close: 'Close',
    clear: 'Clear',
    notConfigured: 'Not configured',
  },
  workgroup: {
    autoName: 'Group ({n})',
  },
  search: {
    noResults: '0 results',
    results: '{count} results',
    position: '{current} / {total}',
  },
  notification: {
    title: 'Worktree Notification',
    titleApproval: 'Approval Required',
    titleCompleted: 'Task Completed',
  },
  // Toast for declined auto spawn (issue #120 §7 / #130 / #137).
  // Shared by the main window and sub-windows through a common composable, so it lives
  // in the global catalog rather than an SFC-local <i18n> block.
  // The per-delivery toast (deliveredSummary) was dropped in #137 — subscription state
  // is now shown persistently by the card badges instead.
  eventDelivery: {
    spawnRejectedSummary: 'Auto spawn declined',
    spawnRejectedDetail: '{name} has {pending} unread message(s) but {live} terminals are open (limit {limit}). Close some terminals or open the worktree manually.',
  },
  update: {
    title: 'oretachi Update',
    available: 'A new version {version} is available. Update now?',
    checkFailed: 'Failed to check for updates.\n{error}',
    installFailed: 'Failed to install the update.\n{error}',
  },
  panZoom: {
    zoomIn: 'Zoom in (+)',
    zoomOut: 'Zoom out (-)',
    fitToView: 'Fit to view (F)',
    resetZoom: 'Actual size (0)',
    fullscreen: 'Open in fullscreen viewer',
    pin: 'Enable zoom and pan on this diagram',
    unpin: 'Disable zoom and pan',
  },
  about: {
    label: 'About',
    checkUpdate: 'Check for updates',
    checking: 'Checking...',
    upToDate: 'You are using the latest version.',
  },
}
