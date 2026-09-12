import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import vm from 'node:vm'

// 通知レポートスキルのテンプレート (`src-tauri/skills/notification-report/templates/lib--send.jsx`)
// のうち、**コマンド実行の候補 (`!` 始まり)** に関する部分を検証する (#288)。
//
// このテンプレートは `src/` の外にあり、ビルドもtype-checkも通らない
// (アーティファクトのビューアが実行時に babel で変換して動かす)。境界条件
// (`!` だけ / 感嘆符始まりの日本語 / `その他` 経由 / 実行後の再返答) は
// 目視でしか検出できなかったので、ここで実際に読み込んで確かめる。

type Call = [string, Record<string, unknown>]

interface SendModule {
  OTHER: string
  COMMAND_MAX_LEN: number
  isCommandChoice: (s: unknown) => boolean
  commandOf: (a: unknown) => string | null
  commandTooLong: (s: unknown) => boolean
  commandExecuted: (n: unknown, answer: unknown) => boolean
  buildReplyText: (meta: unknown, n: unknown, answer: unknown) => string
  canSend: (n: unknown, answer: unknown, draft: unknown, conflicts: unknown) => boolean
  sendOne: (meta: unknown, n: unknown, answer: unknown) => Promise<{ status: string }>
}

const calls: Call[] = []

function loadSendModule(): SendModule {
  const path = resolve(
    __dirname,
    '../../src-tauri/skills/notification-report/templates/lib--send.jsx'
  )
  // `lib--send.jsx` は拡張子こそ .jsx だが JSX を含まない純粋なロジックなので、
  // 変換せずそのまま評価できる（ここが壊れたらテストが構文エラーで落ちる）
  const code = readFileSync(path, 'utf8')
  const moduleObj = { exports: {} as Record<string, unknown> }
  const require_ = (name: string) => {
    if (name === 'oretachi') {
      return {
        callTool: async (tool: string, params: Record<string, unknown>) => {
          calls.push([tool, params])
          return 'written'
        },
      }
    }
    throw new Error(`unexpected require: ${name}`)
  }
  vm.runInNewContext(code, {
    require: require_,
    exports: moduleObj.exports,
    module: moduleObj,
    console,
    setTimeout,
    Promise,
  })
  return moduleObj.exports as unknown as SendModule
}

const S = loadSendModule()
const META = { reportId: 'notif-report-1' }
const NOTIFICATION = { at: '14:05', kind: 'worktree.message', paragraphs: ['差分が 900 行です'] }
// `shape` が `text`（ダイアログ無しで入力待ち）のカード
const TEXT_CARD = { id: 'a', kind: 'worktree.message', sessionId: 1, subscribed: true, prompt: null }

describe('commandOf', () => {
  it('`!` 始まりの候補をコマンドとして取り出す', () => {
    expect(S.commandOf({ choice: '!git diff --stat' })).toBe('!git diff --stat')
    expect(S.commandOf({ choice: '!./scripts/check.sh' })).toBe('!./scripts/check.sh')
  })

  it('改行と余分な空白を 1 行へ畳む', () => {
    expect(S.commandOf({ choice: '  !git   diff \n --stat  ' })).toBe('!git diff --stat')
    expect(S.commandOf({ choice: '! git diff' })).toBe('!git diff')
  })

  it('コマンドでない候補は null', () => {
    expect(S.commandOf({ choice: '分割して' })).toBeNull()
    expect(S.commandOf({ choice: '!' })).toBeNull()
    expect(S.commandOf({ choice: '!   ' })).toBeNull()
  })

  it('感嘆符始まりの日本語はコマンド扱いしない', () => {
    expect(S.commandOf({ choice: '!!至急やって' })).toBeNull()
    expect(S.commandOf({ choice: '!至急やって' })).toBeNull()
  })

  it('`その他` を選んだときは補足欄をコマンド源として見る', () => {
    expect(S.commandOf({ choice: S.OTHER, note: '!pnpm run type-check' })).toBe('!pnpm run type-check')
    expect(S.commandOf({ choice: S.OTHER, note: 'こうして' })).toBeNull()
  })

  it('コマンドの候補では残っている補足を無視する', () => {
    expect(S.commandOf({ choice: '!git status', note: 'よろしく' })).toBe('!git status')
  })
})

describe('buildReplyText', () => {
  it('コマンドは前置き無しでそのまま送る', () => {
    // 前置きが付くと `!` が行頭から外れ、宛先がシェルモードに入らない（#288 の原因）
    expect(S.buildReplyText(META, NOTIFICATION, { choice: '!git diff --stat' })).toBe('!git diff --stat')
    expect(S.buildReplyText(META, NOTIFICATION, { choice: S.OTHER, note: '!gh pr checks' })).toBe('!gh pr checks')
  })

  it('通常の返答には出自の断り書きが付いたまま', () => {
    const text = S.buildReplyText(META, NOTIFICATION, { choice: '分割して' })
    expect(text.startsWith('[通知レポート notif-report-1]')).toBe(true)
    expect(text).toContain('返答: 「分割して」')
  })

  it('感嘆符始まりの日本語は通常の返答として送る', () => {
    expect(S.buildReplyText(META, NOTIFICATION, { choice: '!!至急やって' }).startsWith('[通知レポート'))
      .toBe(true)
  })
})

describe('canSend', () => {
  it('コマンドの候補を送れる', () => {
    expect(S.canSend(TEXT_CARD, null, { choice: '!git diff --stat' }, {})).toBe(true)
  })

  it('読み切れない長さのコマンドは送らせない', () => {
    const long = '!' + 'a'.repeat(S.COMMAND_MAX_LEN)
    expect(S.commandTooLong(long)).toBe(true)
    expect(S.canSend(TEXT_CARD, null, { choice: long }, {})).toBe(false)
  })

  it('通常の返答を送り終えたカードは閉じる', () => {
    expect(S.canSend(TEXT_CARD, { choice: '分割して', status: 'sent' }, { choice: '1 本でよい' }, {}))
      .toBe(false)
  })

  it('コマンドを実行しただけのカードは返答窓口として開いたまま', () => {
    // シェルモードの実行は宛先のターンを開始しないので、元の問いは未回答のまま
    expect(S.canSend(TEXT_CARD, { choice: '!git diff --stat', status: 'sent' }, { choice: '分割して' }, {}))
      .toBe(true)
    // 実行直後は選択が外れるので、一括送信で同じコマンドが再実行されることはない
    expect(S.canSend(TEXT_CARD, { choice: '!git diff --stat', status: 'sent' }, { choice: null }, {}))
      .toBe(false)
  })

  it('直前に実行したのと同じコマンドは送り直せない', () => {
    // 下書きの保存に失敗して選択が残ったまま復元されても、二重実行にならない
    expect(S.canSend(
      TEXT_CARD,
      { choice: '!git diff --stat', status: 'sent' },
      { choice: '!git diff --stat' },
      {}
    )).toBe(false)
    // 別のコマンドなら送れる
    expect(S.canSend(
      TEXT_CARD,
      { choice: '!git diff --stat', status: 'sent' },
      { choice: '!git status' },
      {}
    )).toBe(true)
  })
})

describe('commandExecuted', () => {
  it('コマンドの送信が通ったときだけ true', () => {
    expect(S.commandExecuted(TEXT_CARD, { choice: '!git diff --stat', status: 'sent' })).toBe(true)
  })

  it('送信に失敗したコマンドを「実行した」と誤報しない', () => {
    // `failed` は本文を 1 文字も書けていない状態。実行済みの案内を出すと事実と逆になる
    expect(S.commandExecuted(TEXT_CARD, { choice: '!git diff --stat', status: 'failed' })).toBe(false)
    expect(S.commandExecuted(TEXT_CARD, { choice: '!git diff --stat', status: 'pastedOnly' })).toBe(false)
  })

  it('通常の返答・未送信では false', () => {
    expect(S.commandExecuted(TEXT_CARD, { choice: '分割して', status: 'sent' })).toBe(false)
    expect(S.commandExecuted(TEXT_CARD, null)).toBe(false)
  })

  it('ダイアログのカードでは false', () => {
    const dialogCard = { ...TEXT_CARD, prompt: { shape: 'permission' } }
    expect(S.commandExecuted(dialogCard, { choice: '!git diff --stat', status: 'sent' })).toBe(false)
  })
})

describe('sendOne', () => {
  it('コマンド本体と CR を別々に書き込む', async () => {
    calls.length = 0
    const result = await S.sendOne(META, { ...TEXT_CARD, ...NOTIFICATION }, { choice: '!git diff --stat' })
    expect(calls[0]).toEqual([
      'oretachi_write_terminal',
      { session_id: 1, text: '!git diff --stat', submit: false },
    ])
    expect(calls[1]).toEqual([
      'oretachi_write_terminal',
      { session_id: 1, text: '\r', submit: false },
    ])
    expect(result.status).toBe('sent')
  })
})
