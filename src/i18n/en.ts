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
  // Warning toast for auto spawn (issue #120 §7 / #130 / #137).
  // Shared by the main window and sub-windows through a common composable, so it lives
  // in the global catalog rather than an SFC-local <i18n> block.
  // The per-delivery toast (deliveredSummary) was dropped in #137 — subscription state
  // is now shown persistently by the card badges instead.
  eventDelivery: {
    spawnWarningSummary: 'Many terminals open',
    spawnWarningDetail: 'Added a terminal for {pending} unread message(s) in {name}. {live} terminals are now open. The window has been reported to freeze at {threshold} or more, so closing unused tabs is recommended.',
  },
  update: {
    title: 'oretachi Update',
    available: 'A new version {version} is available. Update now?',
    checkFailed: 'Failed to check for updates.\n{error}',
    installFailed: 'Failed to install the update.\n{error}',
  },
  externalLink: {
    title: 'Open external link',
    confirm: 'Open this URL in your default browser?\n\n{url}',
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
