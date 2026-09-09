import { describe, it, expect, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-log', () => ({
  debug: vi.fn(() => Promise.resolve()),
  info: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}))

import {
  hasApprovalPrompt,
  detectOretachiToolPrompt,
  isSameApprovalScreen,
  APPROVAL_SCAN_LINES,
} from './autoApproval'

describe('hasApprovalPrompt', () => {
  it('detects ❯ Yes', () => {
    expect(hasApprovalPrompt('❯ Yes')).toBe(true)
  })

  it('detects ► Yes', () => {
    expect(hasApprovalPrompt('► Yes')).toBe(true)
  })

  it('detects (Y/n)', () => {
    expect(hasApprovalPrompt('Continue? (Y/n)')).toBe(true)
  })

  it('detects [Y/n]', () => {
    expect(hasApprovalPrompt('Proceed? [Y/n]')).toBe(true)
  })

  it('detects Allow word', () => {
    expect(hasApprovalPrompt('Allow read')).toBe(true)
  })

  it('detects Do you want to', () => {
    expect(hasApprovalPrompt('Do you want to continue?')).toBe(true)
  })

  it('returns false for normal output', () => {
    expect(hasApprovalPrompt('Running tests...')).toBe(false)
  })

  it('returns false for empty string', () => {
    expect(hasApprovalPrompt('')).toBe(false)
  })

  it('is case-insensitive for Do you want to', () => {
    expect(hasApprovalPrompt('do you want to proceed?')).toBe(true)
  })

  it('detects in multiline content', () => {
    const content = 'line1\nline2\n❯ Yes\nline4'
    expect(hasApprovalPrompt(content)).toBe(true)
  })

  it('returns false for multi-line non-approval content', () => {
    const content = 'Compiling...\nDone.\nSuccess!'
    expect(hasApprovalPrompt(content)).toBe(false)
  })

  // #252: Claude Code の現在の書式は `❯ 1. Yes`。番号付きに一致しないと、許可ダイアログの
  // 検出が実質 `Do you want to` の 1 行だけに依存する
  it('detects numbered ❯ 1. Yes', () => {
    expect(hasApprovalPrompt(' ❯ 1. Yes')).toBe(true)
  })

  it('detects numbered ► 2. Yes', () => {
    expect(hasApprovalPrompt('   ► 2. Yes, and switch to acceptEdits')).toBe(true)
  })

  it('detects the numbered dialog without relying on the "Do you want to" line', () => {
    const content = [
      ' ❯ 1. Yes',
      "   2. Yes, and don't ask again",
      '   3. No',
      '',
      ' Esc to cancel · Tab to amend',
    ].join('\n')
    expect(hasApprovalPrompt(content)).toBe(true)
  })

  it('does not match a bare numbered list', () => {
    expect(hasApprovalPrompt('  1. Yesterday の集計')).toBe(false)
  })
})

describe('isSameApprovalScreen', () => {
  it('treats an untouched dialog as the same screen', () => {
    const screen = ccPrompt('Write(verify-225-3.txt)')
    expect(isSameApprovalScreen(screen, screen)).toBe(true)
  })

  it('ignores trailing whitespace and trailing blank lines', () => {
    const screen = ccPrompt('Write(verify-225-3.txt)')
    expect(isSameApprovalScreen(screen, `${screen}   \n\n\n`)).toBe(true)
    expect(isSameApprovalScreen(screen, screen.replace(' ❯ 1. Yes', ' ❯ 1. Yes    '))).toBe(true)
  })

  // 判定に 20〜35 秒かかるので、その間に人が手でダイアログを消し別のダイアログが
  // 出ていることがある。未判定のダイアログへ Enter を送らないための照合
  it('detects a different dialog appearing in place of the judged one', () => {
    const before = ccPrompt('Write(verify-225-3.txt)')
    const after = ccPrompt('Bash(rm -rf /)')
    expect(hasApprovalPrompt(after)).toBe(true)
    expect(isSameApprovalScreen(before, after)).toBe(false)
  })

  it('detects the selection cursor having moved', () => {
    const before = ccPrompt('Write(a.txt)')
    const after = before.replace(' ❯ 1. Yes', '   1. Yes').replace('   3. No', ' ❯ 3. No')
    expect(isSameApprovalScreen(before, after)).toBe(false)
  })
})

describe('APPROVAL_SCAN_LINES', () => {
  // #252 の本体: 検出 60 行 / 再チェック 10 行の非対称が偽陰性を生んでいた。
  // 両方がこの定数を使うことで窓が揺れない
  it('is wide enough to hold the dialog plus viewport padding', () => {
    expect(APPROVAL_SCAN_LINES).toBe(60)
  })

  it('keeps the "Do you want to" line inside the window even with viewport padding', () => {
    // 実機のビューポート (34 行) を空行で埋めた末尾から数えると
    // `Do you want to` は 6 行目より上に押し上げられる
    const padding = Array.from({ length: 20 }, () => '').join('\n')
    const screen = `${ccPrompt('Write(verify-225-3.txt)')}\n${padding}`
    const lines = screen.split('\n')
    const window = lines.slice(Math.max(0, lines.length - APPROVAL_SCAN_LINES)).join('\n')
    const narrowWindow = lines.slice(Math.max(0, lines.length - 10)).join('\n')
    expect(hasApprovalPrompt(window)).toBe(true)
    expect(narrowWindow).not.toContain('Do you want to')
  })
})

/** 実際に Claude Code が出す承認プロンプトを模したテキストを作る */
function ccPrompt(label: string, cwd = 'X:\\devel\\worktree\\oretachi-zlvc'): string {
  return [
    'Reactアーティファクトのモジュールを操作する',
    '',
    ' Do you want to proceed?',
    ' ❯ 1. Yes',
    `   2. Yes, and don't ask again for ${label} commands in ${cwd}`,
    '   3. No',
    '',
    ' Esc to cancel · Tab to amend',
  ].join('\n')
}

describe('detectOretachiToolPrompt', () => {
  it('detects plugin-scoped oretachi tool', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - artifact_module')))
      .toBe('artifact_module')
  })

  it('detects artifact without matching artifact_module first', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - artifact')))
      .toBe('artifact')
  })

  it('detects directly-registered oretachi server', () => {
    expect(detectOretachiToolPrompt(ccPrompt('oretachi - oretachi_read_terminal')))
      .toBe('oretachi_read_terminal')
  })

  it('detects read-only list_workgroups', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - oretachi_list_workgroups')))
      .toBe('oretachi_list_workgroups')
  })

  it('returns null for destructive close_worktree', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - oretachi_close_worktree')))
      .toBeNull()
  })

  it('returns null for destructive kill_terminal', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - oretachi_kill_terminal')))
      .toBeNull()
  })

  it('returns null for arbitrary-code-execution tools', () => {
    for (const tool of ['oretachi_spawn_terminal', 'oretachi_write_terminal', 'oretachi_add_task']) {
      expect(detectOretachiToolPrompt(ccPrompt(`plugin:oretachi:oretachi - ${tool}`)))
        .toBeNull()
    }
  })

  // #215: answer_prompt は矢印 + CR で他ワークツリーの許可ダイアログの `1. Yes` を
  // 確定できる。無条件承認すると「承認ダイアログを自動承認するツールが自動承認される」
  // という穴になり、write_terminal と同じく任意コード実行と等価になる
  it('returns null for oretachi_answer_prompt', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - oretachi_answer_prompt')))
      .toBeNull()
  })

  // 解析するだけの read-only ツールは read_terminal と同じ扱いで自動承認する
  it('detects read-only inspect_prompt', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - oretachi_inspect_prompt')))
      .toBe('oretachi_inspect_prompt')
  })

  it('does not match a worktree path that ends with a tool name', () => {
    // 選択肢2行目の cwd は必ずウィンドウ内に入る。
    // ワークツリー名が oretachi-artifact だと任意コマンドが自動承認されうる
    const content = ccPrompt('Bash(rm:*)', 'X:\\devel\\worktree\\oretachi-artifact')
    expect(detectOretachiToolPrompt(content)).toBeNull()
  })

  it('does not match a tool name embedded in a shell command path', () => {
    const content = [
      ' Bash(rm -rf X:/devel/worktree/oretachi-artifact_module/dist)',
      ' Do you want to proceed?',
      ' ❯ 1. Yes',
      '   3. No',
    ].join('\n')
    expect(detectOretachiToolPrompt(content)).toBeNull()
  })

  it('does not match a tool name with a trailing suffix', () => {
    expect(detectOretachiToolPrompt(ccPrompt('plugin:oretachi:oretachi - artifacts')))
      .toBeNull()
  })

  it('returns null for another MCP server', () => {
    expect(detectOretachiToolPrompt(ccPrompt('obsidian-mcp-tools - get_vault_file')))
      .toBeNull()
  })

  it('returns null when there is no approval prompt at all', () => {
    expect(detectOretachiToolPrompt('plugin:oretachi:oretachi - artifact finished')).toBeNull()
  })

  it('returns null when the oretachi mention is far from the prompt', () => {
    const noise = Array.from({ length: 30 }, (_, i) => `log line ${i}`).join('\n')
    const content = [
      'plugin:oretachi:oretachi - artifact created earlier',
      noise,
      ' Do you want to proceed?',
      ' ❯ 1. Yes',
      "   2. Yes, and don't ask again for Bash(rm:*) commands",
      '   3. No',
    ].join('\n')
    expect(detectOretachiToolPrompt(content)).toBeNull()
  })

  it('uses the last approval prompt in the buffer', () => {
    const content = [
      ccPrompt('plugin:oretachi:oretachi - oretachi_close_worktree'),
      Array.from({ length: 30 }, (_, i) => `log ${i}`).join('\n'),
      ccPrompt('plugin:oretachi:oretachi - search_artifact'),
    ].join('\n')
    expect(detectOretachiToolPrompt(content)).toBe('search_artifact')
  })
})
