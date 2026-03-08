import { describe, it, expect, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-log', () => ({
  debug: vi.fn(),
}))

import { hasApprovalPrompt } from './autoApproval'

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
