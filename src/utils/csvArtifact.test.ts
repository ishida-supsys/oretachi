import { describe, it, expect } from 'vitest'

import {
  delimiterFor,
  isTableContentType,
  parseCsvArtifact,
  filterCsvRows,
} from './csvArtifact'

const CSV = 'text/csv'
const TSV = 'text/tab-separated-values'

describe('delimiterFor', () => {
  it('returns comma for text/csv', () => {
    expect(delimiterFor(CSV)).toBe(',')
  })

  it('returns tab for text/tab-separated-values', () => {
    expect(delimiterFor(TSV)).toBe('\t')
  })

  it('falls back to comma for unknown types', () => {
    expect(delimiterFor('text/plain')).toBe(',')
  })
})

describe('isTableContentType', () => {
  it('accepts csv and tsv', () => {
    expect(isTableContentType(CSV)).toBe(true)
    expect(isTableContentType(TSV)).toBe(true)
  })

  it('rejects other types', () => {
    expect(isTableContentType('text/markdown')).toBe(false)
  })
})

describe('parseCsvArtifact', () => {
  it('parses a plain CSV with a header row', () => {
    const t = parseCsvArtifact('a,b\n1,2\n3,4', CSV)
    expect(t.columns).toEqual([
      { field: 'c0', header: 'a' },
      { field: 'c1', header: 'b' },
    ])
    expect(t.rows).toEqual([
      { c0: '1', c1: '2' },
      { c0: '3', c1: '4' },
    ])
    expect(t.errors).toEqual([])
    expect(t.raggedRows).toEqual([])
  })

  it('parses TSV using the tab delimiter', () => {
    const t = parseCsvArtifact('a\tb\n1\t2', TSV)
    expect(t.columns.map((c) => c.header)).toEqual(['a', 'b'])
    expect(t.rows).toEqual([{ c0: '1', c1: '2' }])
  })

  it('does not split TSV content on commas', () => {
    const t = parseCsvArtifact('name\tnote\nWidget\ta, b, c', TSV)
    expect(t.rows).toEqual([{ c0: 'Widget', c1: 'a, b, c' }])
  })

  it('honours quoted cells containing the delimiter', () => {
    const t = parseCsvArtifact('a,b\n"x,y",z', CSV)
    expect(t.rows).toEqual([{ c0: 'x,y', c1: 'z' }])
  })

  it('unescapes doubled quotes', () => {
    const t = parseCsvArtifact('a\n"say ""hi"""', CSV)
    expect(t.rows).toEqual([{ c0: 'say "hi"' }])
  })

  it('keeps newlines embedded in quoted cells', () => {
    const t = parseCsvArtifact('a,b\n"line1\nline2",z', CSV)
    expect(t.rows).toEqual([{ c0: 'line1\nline2', c1: 'z' }])
  })

  it('handles CRLF line endings', () => {
    const t = parseCsvArtifact('a,b\r\n1,2\r\n', CSV)
    expect(t.rows).toEqual([{ c0: '1', c1: '2' }])
  })

  it('strips a leading BOM from the first header', () => {
    const t = parseCsvArtifact('﻿a,b\n1,2', CSV)
    expect(t.columns[0].header).toBe('a')
  })

  it('keeps duplicate header names but gives each column a unique field', () => {
    const t = parseCsvArtifact('id,id\n1,2', CSV)
    expect(t.columns).toEqual([
      { field: 'c0', header: 'id' },
      { field: 'c1', header: 'id' },
    ])
    expect(t.rows).toEqual([{ c0: '1', c1: '2' }])
  })

  it('falls back to a positional label for empty headers', () => {
    const t = parseCsvArtifact('a,,c\n1,2,3', CSV)
    expect(t.columns.map((c) => c.header)).toEqual(['a', 'Column 2', 'c'])
  })

  it('pads short rows and records them as ragged', () => {
    const t = parseCsvArtifact('a,b,c\n1,2', CSV)
    expect(t.rows).toEqual([{ c0: '1', c1: '2', c2: '' }])
    expect(t.raggedRows).toEqual([2])
  })

  it('adds extra columns for long rows instead of truncating', () => {
    const t = parseCsvArtifact('a,b\n1,2,3', CSV)
    expect(t.columns.map((c) => c.header)).toEqual(['a', 'b', 'Column 3'])
    expect(t.rows).toEqual([{ c0: '1', c1: '2', c2: '3' }])
    expect(t.raggedRows).toEqual([2])
  })

  it('reports ragged rows with their record number', () => {
    const t = parseCsvArtifact('a,b\n1,2\n3\n5,6\n7', CSV)
    expect(t.raggedRows).toEqual([3, 5])
  })

  it('keeps blank lines out of the data but still counts them in record numbers', () => {
    // レコード: 1=header, 2=(1,2), 3=空行, 4=(3) ← ragged
    const t = parseCsvArtifact('a,b\n1,2\n\n3\n', CSV)
    expect(t.rows).toEqual([
      { c0: '1', c1: '2' },
      { c0: '3', c1: '' },
    ])
    expect(t.raggedRows).toEqual([4])
  })

  it('keeps rows whose cells are all empty', () => {
    const t = parseCsvArtifact('a,b,c\n1,2,3\n,,\n4,5,6', CSV)
    expect(t.rows).toEqual([
      { c0: '1', c1: '2', c2: '3' },
      { c0: '', c1: '', c2: '' },
      { c0: '4', c1: '5', c2: '6' },
    ])
    expect(t.raggedRows).toEqual([])
  })

  it('keeps quoted empty cells', () => {
    const t = parseCsvArtifact('a,b\n"",""\n1,2', CSV)
    expect(t.rows).toEqual([
      { c0: '', c1: '' },
      { c0: '1', c1: '2' },
    ])
  })

  it('keeps whitespace-only cells', () => {
    const t = parseCsvArtifact('a,b\n  ,  \n1,2', CSV)
    expect(t.rows).toEqual([
      { c0: '  ', c1: '  ' },
      { c0: '1', c1: '2' },
    ])
  })

  it('keeps all-empty rows in TSV too', () => {
    const t = parseCsvArtifact('a\tb\n\t\n1\t2', TSV)
    expect(t.rows).toEqual([
      { c0: '', c1: '' },
      { c0: '1', c1: '2' },
    ])
  })

  it('returns an empty table for empty content', () => {
    expect(parseCsvArtifact('', CSV)).toEqual({
      columns: [],
      rows: [],
      errors: [],
      raggedRows: [],
    })
  })

  it('returns an empty table for whitespace-only content', () => {
    expect(parseCsvArtifact('   \n\n', CSV).columns).toEqual([])
  })

  it('returns columns but no rows when only a header is present', () => {
    const t = parseCsvArtifact('a,b', CSV)
    expect(t.columns.map((c) => c.header)).toEqual(['a', 'b'])
    expect(t.rows).toEqual([])
  })

  it('skips blank lines between records', () => {
    const t = parseCsvArtifact('a,b\n1,2\n\n3,4\n', CSV)
    expect(t.rows).toEqual([
      { c0: '1', c1: '2' },
      { c0: '3', c1: '4' },
    ])
    expect(t.raggedRows).toEqual([])
  })
})

describe('filterCsvRows', () => {
  const columns = [
    { field: 'c0', header: 'region' },
    { field: 'c1', header: 'product' },
  ]
  const rows = [
    { c0: 'APAC', c1: 'Widget' },
    { c0: 'EMEA', c1: 'Gadget' },
  ]

  it('returns every row for an empty query', () => {
    expect(filterCsvRows(rows, columns, '   ')).toBe(rows)
  })

  it('matches case-insensitively across all columns', () => {
    expect(filterCsvRows(rows, columns, 'widget')).toEqual([rows[0]])
    expect(filterCsvRows(rows, columns, 'emea')).toEqual([rows[1]])
  })

  it('matches on partial substrings', () => {
    expect(filterCsvRows(rows, columns, 'gad')).toEqual([rows[1]])
    expect(filterCsvRows(rows, columns, 'get')).toEqual(rows)
  })

  it('returns nothing when no cell matches', () => {
    expect(filterCsvRows(rows, columns, 'zzz')).toEqual([])
  })
})
