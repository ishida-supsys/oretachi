# Setup Script & Notification Configuration Guide

## Overview

oretachi provides two features: **automatic setup script execution when adding a worktree**, and **notifications from AI agents to the frontend**.

- **Setup scripts**: Automate common initialization tasks (copying config files, installing dependencies, etc.) that run every time a worktree is added.
- **Notifications**: AI agents can send notifications to oretachi via MCP tool, HTTP API, or CLI, which are then shown as OS notifications or in-app badges.

---

## 1. Configuring the Setup Script

### How to configure

1. Open the oretachi **Settings** screen
2. Find the target repository under the **Repository** section
3. Click the **Select** button in the "Run script" field and choose a `.ps1` (PowerShell) or `.sh` file

The configured script runs automatically every time a worktree is added under that repository.

### When it runs

The script executes in the terminal immediately after a new worktree's terminal starts. For task execution, the AI agent launch waits until the script completes.

### Environment variables

The following environment variables are automatically set when the script runs:

| Variable | Description |
|---|---|
| `ORETACHI_REPO_NAME` | Repository name (as registered in the settings screen) |
| `ORETACHI_WORKTREE_NAME` | Name of the newly added worktree |

### Internal execution commands by shell (reference)

oretachi auto-detects the shell type and invokes the script in the following format:

| Shell | Invocation |
|---|---|
| PowerShell / pwsh | `$env:ORETACHI_REPO_NAME="..."; $env:ORETACHI_WORKTREE_NAME="..."; Set-ExecutionPolicy -Scope Process Bypass; & "<path>"` |
| CMD | `set ORETACHI_REPO_NAME=...&& set ORETACHI_WORKTREE_NAME=...&& call "<path>"` |
| sh / bash / zsh | `ORETACHI_REPO_NAME="..." ORETACHI_WORKTREE_NAME="..." sh "<path>"` |

---

## 2. Setup Script Samples

### PowerShell version

Copies shared config files from the main repository into the worktree.

```powershell
# worktree-setup.ps1
#
# ORETACHI_REPO_NAME    : repository name
# ORETACHI_WORKTREE_NAME: name of this worktree

param()

$ErrorActionPreference = "Stop"

Write-Output "[setup] START: $env:ORETACHI_WORKTREE_NAME"

# Get the main repository root
$gitCommonDir = git rev-parse --git-common-dir 2>&1
if (-not $gitCommonDir -or $gitCommonDir -match "^fatal") {
    Write-Output "[setup] ERROR: git repository not found"
    exit 1
}

if ([System.IO.Path]::IsPathRooted($gitCommonDir)) {
    $repoRoot = Split-Path -Parent $gitCommonDir
} else {
    $repoRoot = git rev-parse --show-toplevel 2>&1
}

$worktreeRoot = git rev-parse --show-toplevel 2>&1
Write-Output "[setup] repoRoot   : $repoRoot"
Write-Output "[setup] worktreeRoot: $worktreeRoot"

# List of files to copy (relative paths from the repository root)
$files = @(
    ".env.local",
    "config/local.json"
)

foreach ($rel in $files) {
    $src = Join-Path $repoRoot $rel
    $dst = Join-Path $worktreeRoot $rel

    if (Test-Path $src) {
        $dstDir = Split-Path -Parent $dst
        if (-not (Test-Path $dstDir)) {
            New-Item -ItemType Directory -Path $dstDir | Out-Null
        }
        Copy-Item -Path $src -Destination $dst -Force
        Write-Output "[setup] Copied: $rel"
    } else {
        Write-Output "[setup] SKIP (not found): $rel"
    }
}

Write-Output "[setup] DONE"
```

### sh version

```sh
#!/bin/sh
# worktree-setup.sh
#
# ORETACHI_REPO_NAME     : repository name
# ORETACHI_WORKTREE_NAME : name of this worktree

set -e

echo "[setup] START: ${ORETACHI_WORKTREE_NAME}"

# Get the main repository root
git_common_dir=$(git rev-parse --git-common-dir 2>&1)
if [ -z "$git_common_dir" ]; then
    echo "[setup] ERROR: git repository not found"
    exit 1
fi

case "$git_common_dir" in
    /*) repo_root=$(dirname "$git_common_dir") ;;
    *)  repo_root=$(git rev-parse --show-toplevel) ;;
esac

worktree_root=$(git rev-parse --show-toplevel)
echo "[setup] repoRoot    : ${repo_root}"
echo "[setup] worktreeRoot: ${worktree_root}"

# List of files to copy (relative paths from the repository root)
files=".env.local config/local.json"

for rel in $files; do
    src="${repo_root}/${rel}"
    dst="${worktree_root}/${rel}"

    if [ -f "$src" ]; then
        dst_dir=$(dirname "$dst")
        mkdir -p "$dst_dir"
        cp "$src" "$dst"
        echo "[setup] Copied: ${rel}"
    else
        echo "[setup] SKIP (not found): ${rel}"
    fi
done

echo "[setup] DONE"
```

---

## 3. Notification Configuration

### Enabling OS notifications

1. Open the oretachi **Settings** screen
2. Turn on **"Enable OS notifications"**
3. If the OS shows a permission dialog, allow it

### How notifications work

There are three ways for AI agents or external scripts to send notifications to oretachi:

#### Method 1: CLI (`oretachi.exe --notify`)

Pass the `--notify` option to the oretachi executable to send a notification to the running instance. This can also be called from the end of a setup script.

```
oretachi.exe --notify <worktree-name>

# Short form
oretachi.exe -n <worktree-name>
```

#### Method 2: HTTP API (`POST /notify`)

Send a POST request to the local HTTP server that oretachi exposes while running.

```sh
curl -s -X POST "http://127.0.0.1:<PORT>/notify" \
  -H "Content-Type: application/json" \
  -d "{\"worktree\": \"${ORETACHI_WORKTREE_NAME}\"}"
```

```powershell
Invoke-RestMethod -Uri "http://127.0.0.1:<PORT>/notify" `
  -Method Post `
  -ContentType "application/json" `
  -Body "{`"worktree`": `"$env:ORETACHI_WORKTREE_NAME`"}"
```

#### Method 3: MCP tool (`notify_worktree`)

Call the `notify_worktree` tool from an MCP client such as Claude Code. Use this when an AI agent wants to send a notification upon completing a task.

```json
{
  "tool": "notify_worktree",
  "parameters": { "worktree_name": "<worktree-name>" }
}
```

---

## 4. Example: Sending a notification after script completion

By calling the CLI at the end of a setup script, you can notify oretachi when initialization is complete.

```powershell
# Append to the end of worktree-setup.ps1

# --- Setup steps (omitted) ---

# Send notification to oretachi
# Set $oretachiBin to the path of the installed executable
$oretachiBin = "$env:LOCALAPPDATA\oretachi\oretachi.exe"
if (Test-Path $oretachiBin) {
    & $oretachiBin --notify $env:ORETACHI_WORKTREE_NAME
    Write-Output "[setup] Notification sent"
} else {
    Write-Output "[setup] oretachi.exe not found, skipping notification"
}
```

```sh
#!/bin/sh
# Append to the end of worktree-setup.sh

# --- Setup steps (omitted) ---

# Send notification to oretachi
ORETACHI_BIN="${HOME}/.local/bin/oretachi"
if [ -x "$ORETACHI_BIN" ]; then
    "$ORETACHI_BIN" --notify "${ORETACHI_WORKTREE_NAME}"
    echo "[setup] Notification sent"
else
    echo "[setup] oretachi not found, skipping notification"
fi
```
