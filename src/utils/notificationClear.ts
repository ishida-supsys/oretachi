/** 条件付きクリアの条件（#354）。Rust 側 `notification_matches` と同じ規則。 */
export interface ClearExpectation {
  expectedCount?: number;
  expectedKind?: string;
}

/**
 * 通知エントリが条件に一致するときだけ true。
 * 条件が無ければ常に true（従来どおり無条件クリア）。条件があるのにエントリが無ければ false。
 * 指定された項目だけ比較する。
 */
export function shouldClearNotification(
  entry: { count: number; kind: string } | undefined,
  expected: ClearExpectation,
): boolean {
  const { expectedCount, expectedKind } = expected;
  if (expectedCount === undefined && expectedKind === undefined) return true;
  if (!entry) return false;
  if (expectedCount !== undefined && entry.count !== expectedCount) return false;
  if (expectedKind !== undefined && entry.kind !== expectedKind) return false;
  return true;
}
