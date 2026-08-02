// CSV/TSV アーティファクトのパース。
// Vue reactivity を使わない純粋関数なので utils/ に置く (reactArtifactSrcdoc.ts と同方針)。
import Papa from "papaparse";

export type CsvColumn = {
  /** DataTable の field キー。ヘッダ名の重複・空欄でも壊れないよう c0, c1, ... を採番する */
  field: string;
  /** 表示用のヘッダ文字列 */
  header: string;
};

export type CsvTable = {
  columns: CsvColumn[];
  rows: Record<string, string>[];
  /** papaparse 由来のエラーメッセージ (レコード番号付き) */
  errors: string[];
  /** 列数がヘッダと一致しなかったレコードの番号 (ヘッダを1とする1始まりの通し番号) */
  raggedRows: number[];
};

export const TSV_CONTENT_TYPE = "text/tab-separated-values";
export const CSV_CONTENT_TYPE = "text/csv";

/** content_type が CSV/TSV ビューアの対象かどうか */
export function isTableContentType(contentType: string): boolean {
  return contentType === CSV_CONTENT_TYPE || contentType === TSV_CONTENT_TYPE;
}

/** content_type に対応する区切り文字 */
export function delimiterFor(contentType: string): string {
  return contentType === TSV_CONTENT_TYPE ? "\t" : ",";
}

function emptyTable(): CsvTable {
  return { columns: [], rows: [], errors: [], raggedRows: [] };
}

/** papaparse が空行を表す形 (フィールドが1つだけの空文字) かどうか */
function isBlankRecord(record: string[]): boolean {
  return record.length === 1 && record[0] === "";
}

/**
 * CSV/TSV テキストをテーブル構造に変換する。
 * 1行目をヘッダとして扱い、以降をデータ行とする。
 * 列数が不揃いでも描画を止めず、不足は空文字で補完し raggedRows にレコード番号を残す。
 *
 * skipEmptyLines は papaparse に任せず自前で行う。"greedy" は `,,` や `"",""` のような
 * 「全セルが空の実データ行」まで落としてしまい、行が無警告で消えるため。
 * また papaparse がフィルタしても `error.row` の採番は元のレコード列に対して行われるので、
 * 自前でフィルタして元のレコード番号を保持し、errors と raggedRows の採番基準を揃える。
 */
export function parseCsvArtifact(content: string, contentType: string): CsvTable {
  // BOM を除去 (papaparse はヘッダ名に BOM を残すことがある)
  const text = content.replace(/^﻿/, "");
  if (text.trim() === "") return emptyTable();

  const result = Papa.parse<string[]>(text, {
    header: false,
    skipEmptyLines: false,
    delimiter: delimiterFor(contentType),
  });

  // 空行だけを除き、元のレコード番号 (0始まり) を持ち回る
  const kept: { cells: string[]; index: number }[] = [];
  result.data.forEach((record, index) => {
    if (!Array.isArray(record) || isBlankRecord(record)) return;
    kept.push({ cells: record, index });
  });
  if (kept.length === 0) return emptyTable();

  const errors = result.errors.map((e) =>
    typeof e.row === "number" ? `record ${e.row + 1}: ${e.message}` : e.message
  );

  const headerRow = kept[0].cells;
  const dataRows = kept.slice(1);

  // ヘッダより列が多いデータ行があっても切り捨てない
  const columnCount = dataRows.reduce((max, r) => Math.max(max, r.cells.length), headerRow.length);

  const columns: CsvColumn[] = [];
  for (let i = 0; i < columnCount; i++) {
    const raw = (headerRow[i] ?? "").trim();
    columns.push({ field: `c${i}`, header: raw === "" ? `Column ${i + 1}` : raw });
  }

  const raggedRows: number[] = [];
  const rows = dataRows.map(({ cells, index }) => {
    if (cells.length !== headerRow.length) {
      raggedRows.push(index + 1);
    }
    const row: Record<string, string> = {};
    for (let c = 0; c < columnCount; c++) {
      row[`c${c}`] = cells[c] ?? "";
    }
    return row;
  });

  return { columns, rows, errors, raggedRows };
}

/** 全カラム横断の大文字小文字無視・部分一致フィルタ */
export function filterCsvRows(
  rows: Record<string, string>[],
  columns: CsvColumn[],
  query: string
): Record<string, string>[] {
  const q = query.trim().toLowerCase();
  if (q === "") return rows;
  return rows.filter((row) =>
    columns.some((col) => (row[col.field] ?? "").toLowerCase().includes(q))
  );
}
