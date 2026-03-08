<div align="center">
  <img src="src-tauri/icons/128x128.png" alt="oretachi icon" width="128" />

  # oretachi

  **Multi-session terminal manager for developers working with Git worktrees and AI coding agents**

  <img width="1915" height="908" alt="Image" src="https://github.com/user-attachments/assets/13f3152b-f087-4dff-82c3-7898398e74b1" />

  [日本語版 README はこちら](README_ja.md)
</div>

---

## Features

- **Multi-terminal management** — PTY-based terminal emulator with tabs and split panes (horizontal/vertical resize), in-terminal search
- **Git Worktree management** — Create/remove worktrees, manage multiple repositories
- **Task execution** — Auto-generate worktrees from Issue/PR URLs or free-text descriptions and run tasks in parallel with AI agents
- **AI Auto-approval** — Detects approval prompts in terminal output and uses AI to judge safety, then auto-approves
- **AI agent support** — Auto-detect Claude Code, Gemini CLI, Codex CLI, and Cline CLI; identify AI sessions via process tree analysis
- **Code Reviewer** — Built-in diff viewer with file tree and commit history display
- **Sub-windows** — Detach any worktree into an independent window
- **Notification system** — Send notifications to worktrees via MCP or REST API; tray popup shows unread count
- **Built-in MCP Server** — Streamable HTTP MCP protocol server, provides repository and worktree information, usable directly from AI agents
- **Hotkeys** — `Alt`+key to instantly focus any worktree, auto-assignment on worktree creation, fully customizable key bindings
- **IDE integration** — Auto-detect Cursor, VS Code, and Antigravity; built-in CodeReviewer also available

### Task Execution

Submit a task such as an Issue/PR URL or a specific fix description, and it will automatically create a worktree and start working on it.

https://github.com/user-attachments/assets/be44a731-a25e-41e6-ac03-502bf7a651eb

### Tray Popup

Review worktrees that received notifications one by one via the tray popup.

https://github.com/user-attachments/assets/23a0e4b5-6586-41ed-8d30-af0e4b641a36

### Code Reviewer

Select "CodeReviewer" as the IDE option to launch the built-in viewer for code review.

<img width="1200" height="834" alt="Image" src="https://github.com/user-attachments/assets/b6c53230-6675-4ff4-ba6d-8f78a85fbcc9" />

## Installation

Download the installer from the Releases tab.

To build manually:

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
