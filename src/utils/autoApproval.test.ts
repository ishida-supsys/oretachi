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

import { hasApprovalPrompt, detectOretachiToolPrompt } from './autoApproval'

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
