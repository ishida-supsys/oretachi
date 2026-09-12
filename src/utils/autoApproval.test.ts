import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-log', () => ({
  debug: vi.fn(() => Promise.resolve()),
  info: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}))

import { invoke } from '@tauri-apps/api/core'
import {
  hasApprovalPrompt,
  detectOretachiToolPrompt,
  isSameApprovalScreen,
  runApprovalLoop,
  APPROVAL_SCAN_LINES,
} from './autoApproval'

/** ダイアログの上下にログ行・空行が並んだ、実機に近い画面を作る */
function screenWithDialog(label: string, above = 12, below = 12): string {
  return [
    ...Array.from({ length: above }, (_, i) => `log ${i}`),
    ccPrompt(label),
    ...Array.from({ length: below }, () => ''),
  ].join('\n')
}

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

  it('detects numbered ► 1. Yes', () => {
    expect(hasApprovalPrompt('   ► 1. Yes')).toBe(true)
  })

  // #252 レビュー指摘 (Critical): `Yes` の後ろを開けるとプラン承認ダイアログが
  // 検出対象になり、AI 判定 (CLI コマンドの危険性しか見ない) が safe を返した瞬間に
  // `1. Yes, and use auto mode` が確定して自動承認のゲート自体が無効化される
  it('does NOT detect the plan approval dialog', () => {
    const plan = [
      ' Claude has written up a plan and is ready to execute. Would you like to proceed?',
      '',
      ' ❯ 1. Yes, and use auto mode',
      '   2. Yes, manually approve edits',
      '   3. Tell Claude what to change',
    ].join('\n')
    expect(hasApprovalPrompt(plan)).toBe(false)
  })

  it('does NOT detect an AskUserQuestion option that merely starts with Yes', () => {
    expect(hasApprovalPrompt(' ❯ 1. Yes, use approach A')).toBe(false)
  })

  it('does NOT detect "Yes, and don\'t ask again" when the cursor sits on it', () => {
    expect(hasApprovalPrompt("   ❯ 2. Yes, and don't ask again for Bash(rm:*) commands")).toBe(false)
  })

  // 入力待ちの自動候補（ゴーストテキスト）。`❯` と中身の間が NBSP になる（#289）
  it('does NOT detect an auto-suggestion that happens to read "Yes"', () => {
    expect(hasApprovalPrompt('❯\u00a0Yes')).toBe(false)
    expect(hasApprovalPrompt('❯\u00a0Do you want to run the tests?')).toBe(false)
  })

  it('still detects a real dialog line, which uses a plain space', () => {
    expect(hasApprovalPrompt('❯ Yes')).toBe(true)
  })

  it('does not match a numbered word merely starting with Yes', () => {
    expect(hasApprovalPrompt(' ❯ 1. Yesterday の集計')).toBe(false)
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
    expect(hasApprovalPrompt('  1. Yes')).toBe(false)
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

  // 比較は末尾 N 行の生比較ではなくプロンプト行アンカーのダイアログ領域。
  // ダイアログの下に 1 行増えるだけで不一致になると #252 と同じストールに戻る
  it('tolerates output appended below the dialog', () => {
    const before = screenWithDialog('Write(a.txt)')
    const after = `${before}\n✳ Thinking…`
    expect(isSameApprovalScreen(before, after)).toBe(true)
  })

  // 出力が 1 行増えるとバッファ末尾からの窓がずれる。アンカー相対で切り出すので
  // ダイアログ領域の中身は変わらない
  it('tolerates the window having scrolled by a line', () => {
    const before = screenWithDialog('Write(a.txt)')
    const after = before.split('\n').slice(1).join('\n')
    expect(isSameApprovalScreen(before, after)).toBe(true)
  })

  // 領域はプロンプト行の上も含める。含めないと「別ファイルに対する同じ形の
  // Write ダイアログ」を同一と誤認して未判定のダイアログへ Enter を送る
  it('detects a header change above the prompt line', () => {
    const before = ccPrompt('Write(a.txt)').replace(
      'Reactアーティファクトのモジュールを操作する',
      'Write(a.txt)'
    )
    const after = ccPrompt('Write(a.txt)').replace(
      'Reactアーティファクトのモジュールを操作する',
      'Write(b.txt)'
    )
    expect(isSameApprovalScreen(before, after)).toBe(false)
  })

  // スクロールバックに残った自動候補の行（`❯<NBSP>Yes`）をアンカーにすると、
  // その領域は判定中に変わらないので**ダイアログが差し替わっても「同じ画面」**になり、
  // 未判定のダイアログへ Enter を送る（#289）
  it('does not anchor the compared region on a stale auto-suggestion line', () => {
    const ghost = '❯\u00a0Yes'
    // ゴースト行とダイアログの間を `TRAIL` より広く空ける。ゴーストにアンカーが
    // 付くと比較領域はこの共通の埋め草だけになり、別物のダイアログが同一と判定される
    const filler = Array.from({ length: 14 }, (_, i) => `log ${i}`)
    const before = [ghost, ...filler, ccPrompt('Write(a.txt)')].join('\n')
    const after = [ghost, ...filler, ccPrompt('Bash(rm -rf /)')].join('\n')
    expect(isSameApprovalScreen(before, after)).toBe(false)
  })

  it('is false when either side has no approval prompt at all', () => {
    const screen = ccPrompt('Write(a.txt)')
    expect(isSameApprovalScreen(screen, 'Running tests...')).toBe(false)
    expect(isSameApprovalScreen('Running tests...', screen)).toBe(false)
  })
})

describe('APPROVAL_SCAN_LINES', () => {
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

  it('returns null for artifact delete (destructive command)', () => {
    // ツール名は自動承認対象だが、command: "delete" は復元できないので即承認しない
    const screen = [
      'plugin:oretachi:oretachi - artifact (MCP)',
      '  command: "delete"',
      '  id: "report-2026-09-10"',
      '',
      ' Do you want to proceed?',
      ' ❯ 1. Yes',
      "   2. Yes, and don't ask again for plugin:oretachi:oretachi - artifact commands in X:\\devel\\worktree\\oretachi-zlvc",
      '   3. No',
    ].join('\n')
    expect(detectOretachiToolPrompt(screen)).toBeNull()
  })

  it('returns null for artifact delete with a quoted param key', () => {
    // params の描画形は CC のバージョン依存なので、キーがクォートされていても拾う
    const screen = [
      'plugin:oretachi:oretachi - artifact (MCP)',
      '  { "command": "delete", "id": "report-2026-09-10" }',
      '',
      ' Do you want to proceed?',
      ' ❯ 1. Yes',
      '   3. No',
    ].join('\n')
    expect(detectOretachiToolPrompt(screen)).toBeNull()
  })

  it('still approves artifact with non-destructive commands', () => {
    for (const command of ['create', 'update', 'rewrite', 'get', 'outline']) {
      const screen = [
        'plugin:oretachi:oretachi - artifact (MCP)',
        `  command: "${command}"`,
        '  id: "report-2026-09-10"',
        '',
        ' Do you want to proceed?',
        ' ❯ 1. Yes',
        '   3. No',
      ].join('\n')
      expect(detectOretachiToolPrompt(screen)).toBe('artifact')
    }
  })

  it('keeps approving artifact_module delete (single module only)', () => {
    const screen = [
      'plugin:oretachi:oretachi - artifact_module (MCP)',
      '  command: "delete"',
      '  module_name: "components/Header"',
      '',
      ' Do you want to proceed?',
      ' ❯ 1. Yes',
      '   3. No',
    ].join('\n')
    expect(detectOretachiToolPrompt(screen)).toBe('artifact_module')
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

/** xterm の Terminal を getRecentLines が触る範囲だけ模したフェイク */
function fakeTerminal(getScreen: () => string) {
  return {
    get buffer() {
      const lines = getScreen().split('\n')
      return {
        active: {
          length: lines.length,
          getLine: (i: number) => ({ translateToString: () => lines[i] }),
        },
      }
    },
  } as any
}

function fakeTermRef(id: number, getScreen: () => string) {
  const writes: string[] = []
  return {
    ref: {
      id,
      getTerminal: () => fakeTerminal(getScreen),
      write: async (d: string) => {
        writes.push(d)
      },
    },
    writes,
  }
}

/** 実機のビューポート(34行)を空行で埋めた状態。#252 の再現形 */
function paddedPrompt(label: string): string {
  return `${ccPrompt(label)}\n${Array.from({ length: 20 }, () => '').join('\n')}`
}

describe('runApprovalLoop', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  // #252 本体の回帰テスト。再チェックの窓が 10 行に戻ると、この画面では
  // `Do you want to` が窓から外れて Enter が送られなくなる
  it('sends Enter when the judged dialog is still on screen', async () => {
    vi.mocked(invoke).mockResolvedValue({ safe: true, command: 'Write(a.txt)' })
    const { ref, writes } = fakeTermRef(1, () => paddedPrompt('Write(a.txt)'))

    const result = await runApprovalLoop([ref], 'wt-1', 'X:/devel/worktree/x')

    expect(result.approved).toBe(true)
    expect(writes).toEqual(['\r'])
  })

  it('does not send Enter when the prompt disappeared during judgment', async () => {
    let screen = paddedPrompt('Write(a.txt)')
    vi.mocked(invoke).mockImplementation(async () => {
      screen = 'done\n\n> '
      return { safe: true, command: 'Write(a.txt)' }
    })
    const { ref, writes } = fakeTermRef(1, () => screen)

    const result = await runApprovalLoop([ref], 'wt-1', 'X:/devel/worktree/x')

    expect(result.approved).toBe(false)
    expect(writes).toEqual([])
  })

  // 判定に 20〜35 秒かかるので、その間に人が手で消して別のダイアログが出ていることがある。
  // `hasApprovalPrompt` は真なので窓を広げただけでは防げない
  it('does not send Enter when a different dialog replaced the judged one', async () => {
    let screen = paddedPrompt('Write(a.txt)')
    vi.mocked(invoke).mockImplementation(async () => {
      screen = paddedPrompt('Bash(rm -rf /)')
      return { safe: true, command: 'Write(a.txt)' }
    })
    const { ref, writes } = fakeTermRef(1, () => screen)

    const result = await runApprovalLoop([ref], 'wt-1', 'X:/devel/worktree/x')

    expect(result.approved).toBe(false)
    expect(writes).toEqual([])
  })

  // skip を `break` にすると、この経路には再試行が無いので tid=2 の本物のダイアログが
  // 一度も判定されないまま取りこぼされる
  it('keeps checking the remaining terminals after a skip', async () => {
    let first = paddedPrompt('Write(a.txt)')
    const second = paddedPrompt('Write(b.txt)')
    vi.mocked(invoke).mockImplementation(async () => {
      first = 'done\n\n> '
      return { safe: true, command: 'Write' }
    })
    const a = fakeTermRef(1, () => first)
    const b = fakeTermRef(2, () => second)

    const result = await runApprovalLoop([a.ref, b.ref], 'wt-1', 'X:/devel/worktree/x')

    expect(a.writes).toEqual([])
    expect(result.approved).toBe(true)
    expect(b.writes).toEqual(['\r'])
  })

  it('does not send Enter when the judgment is unsafe', async () => {
    vi.mocked(invoke).mockResolvedValue({ safe: false, command: 'Bash(rm -rf /)' })
    const { ref, writes } = fakeTermRef(1, () => paddedPrompt('Bash(rm -rf /)'))

    const result = await runApprovalLoop([ref], 'wt-1', 'X:/devel/worktree/x')

    expect(result.approved).toBe(false)
    expect(result.lastCommand).toBe('Bash(rm -rf /)')
    expect(writes).toEqual([])
  })

  // プラン承認ダイアログは検出対象外。AI 判定 (CLI コマンドの危険性しか見ない) を
  // 走らせてはいけない
  it('never judges the plan approval dialog', async () => {
    vi.mocked(invoke).mockResolvedValue({ safe: true, command: 'plan' })
    const plan = [
      ' Claude has written up a plan and is ready to execute. Would you like to proceed?',
      '',
      ' ❯ 1. Yes, and use auto mode',
      '   2. Yes, manually approve edits',
      '   3. Tell Claude what to change',
    ].join('\n')
    const { ref, writes } = fakeTermRef(1, () => plan)

    const result = await runApprovalLoop([ref], 'wt-1', 'X:/devel/worktree/x')

    expect(invoke).not.toHaveBeenCalled()
    expect(result.approved).toBe(false)
    expect(writes).toEqual([])
  })
})
