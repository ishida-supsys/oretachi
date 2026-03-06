<div align="center">
  <img src="src-tauri/icons/128x128.png" alt="oretachi icon" width="128" />

  # oretachi

  **Multi-session terminal manager for developers working with Git worktrees and AI coding agents**

  [日本語版 README はこちら](README_ja.md)
</div>

---

## Features

- **Multi-terminal management** — PTY-based terminal emulator (xterm.js + WebGL) with tabs and split panes
- **Git Worktree management** — Create/remove worktrees, manage multiple repositories, Git LFS support
- **AI Auto-approval** — Detects approval prompts in terminal output and uses AI (Claude) to judge safety, then auto-approves
- **Sub-windows** — Detach any worktree into an independent window; window state is saved and restored on next launch
- **Notification system** — Send notifications to worktrees via MCP, REST API, or CLI; tray popup shows unread count
- **Built-in MCP Server** — Streamable HTTP MCP protocol server, usable directly from AI coding agents
- **Hotkeys** — `Alt`+key to instantly focus any worktree; fully customizable key bindings
- **IDE integration** — Auto-detect and open worktrees in Cursor, VS Code, or Antigravity

## Tech Stack

| Layer | Technology |
|---|---|
| Desktop framework | Tauri 2 |
| Backend | Rust, portable-pty, axum, rmcp |
| Frontend | Vue 3, TypeScript, Vite |
| Terminal | xterm.js (WebGL renderer) |
| UI | Tailwind CSS 4, PrimeVue 4 |

## Prerequisites

- [Node.js](https://nodejs.org/) (LTS recommended)
- [pnpm](https://pnpm.io/)
- [Rust toolchain](https://rustup.rs/)

## Installation

```bash
# Clone the repository
git clone https://github.com/YOUR_USERNAME/oretachi.git
cd oretachi

# Install frontend dependencies
pnpm install

# Build the app
pnpm tauri build
```

The built installer will be placed in `src-tauri/target/release/bundle/`.

## Development

```bash
pnpm tauri dev
```

Logs are written to the platform-specific app log directory.

## License

MIT — see [LICENSE](LICENSE) for details.
