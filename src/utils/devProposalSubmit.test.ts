import { describe, it, expect, beforeEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import vm from 'node:vm'

// dev-proposal スキルのテンプレート (`src-tauri/skills/dev-proposal/templates/lib--submit.jsx`)
// の送信ロジックを検証する。テンプレートは `src/` の外でビルドも type-check も通らないので、
// ここで実際に読み込んで確かめる (notificationReportCommand.test.ts と同じ方式)。

type Call = [string, Record<string, unknown>]
type Prompt = Record<string, unknown> | null

interface SubmitModule {
  CC_INPUT_HEADER: string
  flatten: (s: unknown) => string
  hasInputBox: (tail: unknown) => boolean
  canDeliver: (p: Prompt) => boolean
  deliverBlockedReason: (p: Prompt) => string | null
  buildTriggerText: (meta: unknown, submission: unknown) => string
  deliver: (meta: unknown, submission: unknown) => Promise<{ status: string; reason?: string }>
  submit: (a: {
    meta: unknown
    ids: string[]
    persist: (s: unknown) => Promise<unknown>
    now?: () => string
  }) => Promise<{ ok: boolean; error?: string; delivery?: { status: string } }>
  missingDeps: (
    proposals: unknown,
    selection: unknown
  ) => { id: string; missing: string[] }[]
}

const calls: Call[] = []
// ケースごとに差し替える挙動
let inspectResult: Prompt | Error
let failOn: 'none' | 'text' | 'cr'

function loadModule(): SubmitModule {
  const path = resolve(__dirname, '../../src-tauri/skills/dev-proposal/templates/lib--submit.jsx')
  const code = readFileSync(path, 'utf8')
  const moduleObj = { exports: {} as Record<string, unknown> }
  const require_ = (name: string) => {
    if (name !== 'oretachi') throw new Error(`unexpected require: ${name}`)
    return {
      callTool: async (tool: string, params: Record<string, unknown>) => {
        calls.push([tool, params])
        if (tool === 'oretachi_inspect_prompt') {
          if (inspectResult instanceof Error) throw inspectResult
          return inspectResult
        }
        if (tool === 'oretachi_write_terminal') {
          const isCr = params.text === '\r'
          if ((failOn === 'text' && !isCr) || (failOn === 'cr' && isCr)) throw new Error('write failed')
          return 'written'
        }
        throw new Error(`unexpected tool: ${tool}`)
      },
    }
  }
  vm.runInNewContext(code, {
    require: require_,
    exports: moduleObj.exports,
    module: moduleObj,
    console,
    setTimeout,
    Promise,
    Date,
  })
  return moduleObj.exports as unknown as SubmitModule
}

const S = loadModule()
const META = { sessionId: 7, artifactId: 'dev-proposal-1', projectDir: 'C:\\work\\wt-1' }
const SUBMISSION = { ids: ['p1'], submittedAt: '2026-10-07T10:00:00.000Z' }
const RULE = '────────────────────────────────'
// 実機の入力欄: 上下を罫線で挟まれた箱 + その下のフッタ
const BOX_TAIL = ['● 完了しました', '', RULE, '❯ ', RULE, '  ? for shortcuts'].join('\n')
const READY: Prompt = {
  shape: 'text',
  header: S.CC_INPUT_HEADER,
  pendingInput: '',
  inputSuggestion: '',
  tail: BOX_TAIL,
}

const toolsCalled = () => calls.map(c => c[0])
const writes = () => calls.filter(c => c[0] === 'oretachi_write_terminal')

beforeEach(() => {
  calls.length = 0
  inspectResult = READY
  failOn = 'none'
})

describe('canDeliver (許可リスト)', () => {
  it('CC の入力欄が空で入力待ちなら送ってよい', () => {
    expect(S.canDeliver(READY)).toBe(true)
  })

  it('ゴーストテキスト(inputSuggestion)だけなら送ってよい', () => {
    // Rust は候補の行を `❯ ⟪自動候補…⟫` に書き換える。先頭は `❯` のまま
    const tail = ['', RULE, '❯ ⟪自動候補（ユーザー入力ではない）: 進捗どう？⟫', RULE, '  ? for shortcuts'].join('\n')
    expect(S.canDeliver({ ...READY, inputSuggestion: '進捗どう？', tail })).toBe(true)
  })

  it('箱を同定できなかった縮退画面 (罫線に挟まれていない `❯` 行) は送らない', () => {
    // CC 終了後に starship の `❯` プロンプトが残ったシェル。Rust は header=CC・pendingInput='' で返す
    const starship = ['PS X:\\wt> git status', 'nothing to commit', '', '❯'].join('\n')
    const p = { ...READY, tail: starship }
    expect(S.canDeliver(p)).toBe(false)
    expect(S.deliverBlockedReason(p)).toContain('入力欄を確認できません')
    // 出力行が `> ` で始まるだけの画面
    expect(S.canDeliver({ ...READY, tail: ['> quoted output', 'next line'].join('\n') })).toBe(false)
    // 上だけ / 下だけが罫線
    expect(S.canDeliver({ ...READY, tail: [RULE, '❯'].join('\n') })).toBe(false)
    expect(S.canDeliver({ ...READY, tail: ['❯', RULE].join('\n') })).toBe(false)
    // tail が無い
    expect(S.canDeliver({ ...READY, tail: undefined })).toBe(false)
  })

  it('スクロールバックの古い罫線ではなく、最後の `❯` 行の直上直下を見る', () => {
    const tail = [RULE, '❯ old', RULE, '', 'PS X:\\wt>', '❯'].join('\n')
    expect(S.hasInputBox(tail)).toBe(false)
  })

  it('実機の入力欄 (上罫線の右端にワークツリー名のラベルが入る) を箱として認める', () => {
    // 実機の inspect_prompt の tail（148 桁）。上罫線は `───…─── add-dev-proposal-skill ─`
    const labeled = '─'.repeat(100) + ' add-dev-proposal-skill ─'
    const tail = [
      '✽ Finagling… (12s)',
      '',
      labeled,
      '❯',
      '─'.repeat(148),
      '  ⏵⏵ auto mode on (shift+tab to cycle) · esc to interrupt',
    ].join('\n')
    expect(S.hasInputBox(tail)).toBe(true)
    expect(S.canDeliver({ ...READY, tail })).toBe(true)
  })

  it('ASCII の `---` で始まる diff 行は罫線と読まない', () => {
    expect(S.hasInputBox(['--- a/file.ts', '❯', '+++ b/file.ts'].join('\n'))).toBe(false)
  })

  it('罫線と入力行の間の空行は飛ばして判定する', () => {
    expect(S.hasInputBox([RULE, '', '❯', '', RULE].join('\n'))).toBe(true)
  })

  it.each([
    ['permission', { shape: 'permission', header: 'Do you want to proceed?', pendingInput: '' }],
    ['askUserQuestion', { shape: 'askUserQuestion', header: 'Q', pendingInput: '' }],
    ['unknown', { shape: 'unknown', header: '', pendingInput: '' }],
    ['pager', { shape: 'pager', header: '', pendingInput: '' }],
  ])('%s は送らない', (_name, p) => {
    expect(S.canDeliver(p)).toBe(false)
    expect(S.deliverBlockedReason(p)).toBeTruthy()
  })

  it('シェルのプロンプトへは送らない (本文がコマンドとして実行される)', () => {
    const p = { shape: 'text', header: '[シェルのプロンプト] PS X:\\wt>', pendingInput: '' }
    expect(S.canDeliver(p)).toBe(false)
    expect(S.deliverBlockedReason(p)).toContain('Claude Code')
  })

  it('打ちかけがあれば送らない', () => {
    expect(S.canDeliver({ ...READY, pendingInput: '/co' })).toBe(false)
    expect(S.canDeliver({ ...READY, pendingInput: 'hello' })).toBe(false)
  })

  it('画面が取れなければ送らない', () => {
    expect(S.canDeliver(null)).toBe(false)
    expect(S.deliverBlockedReason(null)).toBeTruthy()
  })
})

describe('deliver', () => {
  it('inspect → 本文 → CR の順で、どちらも submit:false の別呼び出し', async () => {
    const r = await S.deliver(META, SUBMISSION)
    expect(r.status).toBe('sent')
    expect(toolsCalled()).toEqual([
      'oretachi_inspect_prompt',
      'oretachi_write_terminal',
      'oretachi_write_terminal',
    ])
    expect(calls[0][1]).toEqual({ session_id: 7 })
    expect(calls[1][1].submit).toBe(false)
    expect(calls[1][1].session_id).toBe(7)
    expect(calls[2][1]).toEqual({ session_id: 7, text: '\r', submit: false })
  })

  it.each([
    ['ダイアログ', { shape: 'permission', header: 'x', pendingInput: '' }],
    ['unknown', { shape: 'unknown', header: '', pendingInput: '' }],
    ['シェル', { shape: 'text', header: '[シェルのプロンプト] PS>', pendingInput: '' }],
    ['打ちかけ', { ...READY, pendingInput: 'abc' }],
  ])('%s のときは何も書かず notReady', async (_n, p) => {
    inspectResult = p as Prompt
    const r = await S.deliver(META, SUBMISSION)
    expect(r.status).toBe('notReady')
    expect(r.reason).toBeTruthy()
    expect(writes()).toHaveLength(0)
  })

  it('inspect が例外 (session_id 失効) → failed。何も書かない', async () => {
    inspectResult = new Error('session not found')
    expect((await S.deliver(META, SUBMISSION)).status).toBe('failed')
    expect(writes()).toHaveLength(0)
  })

  it('本文の書き込みが例外 → failed (再送してよい)。CR は送らない', async () => {
    failOn = 'text'
    expect((await S.deliver(META, SUBMISSION)).status).toBe('failed')
    expect(writes()).toHaveLength(1)
  })

  it('CR の書き込みが例外 → pastedOnly (本文は届いている)', async () => {
    failOn = 'cr'
    expect((await S.deliver(META, SUBMISSION)).status).toBe('pastedOnly')
    expect(writes()).toHaveLength(2)
  })
})

describe('submit', () => {
  it('persist → inspect → 本文 → CR の順に動く', async () => {
    const order: string[] = []
    const r = await S.submit({
      meta: META,
      ids: ['p1', 'p2'],
      now: () => 'T1',
      persist: async s => {
        order.push('persist')
        // 端末への呼び出しより前に保存される (生成側が読む正が先に在ること)
        expect(calls).toHaveLength(0)
        expect(s).toEqual({ ids: ['p1', 'p2'], submittedAt: 'T1' })
      },
    })
    expect(r.ok).toBe(true)
    expect(r.delivery?.status).toBe('sent')
    expect(order).toEqual(['persist'])
    expect(toolsCalled()).toEqual([
      'oretachi_inspect_prompt',
      'oretachi_write_terminal',
      'oretachi_write_terminal',
    ])
  })

  it('persist が失敗したら inspect も write も呼ばない', async () => {
    const r = await S.submit({
      meta: META,
      ids: ['p1'],
      persist: async () => {
        throw new Error('quota exceeded')
      },
    })
    expect(r.ok).toBe(false)
    expect(r.error).toContain('quota exceeded')
    expect(calls).toHaveLength(0)
  })
})

describe('buildTriggerText', () => {
  it('1 行で、スキル名・artifactId・project_dir・submittedAt を含む', () => {
    const t = S.buildTriggerText(META, SUBMISSION)
    expect(t).not.toMatch(/[\x00-\x1f\x7f]/)
    expect(t).toContain('oretachi:dev-proposal')
    expect(t).toContain('dev-proposal-1')
    expect(t).toContain('C:\\work\\wt-1')
    expect(t).toContain(SUBMISSION.submittedAt)
  })

  it('制御文字や改行を含む値でも 1 行に畳む (ESC を注入させない)', () => {
    const t = S.buildTriggerText(
      { ...META, projectDir: 'C:\\a\nb\x1b[31m' },
      SUBMISSION
    )
    expect(t).not.toMatch(/[\x00-\x1f\x7f]/)
  })

  it('C1 制御文字 (8bit CSI など) も落とす', () => {
    const t = S.buildTriggerText({ ...META, projectDir: 'C:\\a\u009b31mb\u0085c' }, SUBMISSION)
    expect(t).not.toMatch(/[\x80-\x9f]/)
  })
})

describe('missingDeps', () => {
  const PROPOSALS = [
    { id: 'p1', title: 'A' },
    { id: 'p2', title: 'B', dependsOn: ['p1'] },
    { id: 'p3', title: 'C', dependsOn: ['p1', 'p2'] },
  ]

  it('採用した案の依存先が未チェックなら検出する', () => {
    expect(S.missingDeps(PROPOSALS, { p2: true })).toEqual([
      { id: 'p2', title: 'B', missing: ['p1'] },
    ])
    expect(S.missingDeps(PROPOSALS, { p3: true, p1: true })).toEqual([
      { id: 'p3', title: 'C', missing: ['p2'] },
    ])
  })

  it('依存先が揃っていれば空、未採用の案は見ない', () => {
    expect(S.missingDeps(PROPOSALS, { p1: true, p2: true })).toEqual([])
    expect(S.missingDeps(PROPOSALS, {})).toEqual([])
  })
})
