export interface FuzzyResult {
  path: string;
  name: string;
  dir: string;
  score: number;
}

const SEPARATORS = new Set(["/", ".", "-", "_"]);

/**
 * パターンを候補文字列にファジーマッチし、スコアを返す。
 * マッチしない場合は null を返す。
 */
export function fuzzyScore(pattern: string, candidate: string): number | null {
  if (pattern.length === 0) return 0;

  const p = pattern.toLowerCase();
  const c = candidate.toLowerCase();

  const basenameStart = c.lastIndexOf("/") + 1;

  let score = 0;
  let ci = 0;
  let pi = 0;
  let consecutive = 0;

  while (pi < p.length && ci < c.length) {
    if (p[pi] === c[ci]) {
      // ベースネーム先頭マッチにボーナス
      if (ci === basenameStart) score += 10;
      // セパレータ直後マッチにボーナス
      if (ci > 0 && SEPARATORS.has(c[ci - 1])) score += 6;
      // 先頭マッチボーナス
      if (ci === 0) score += 8;
      // 連続マッチボーナス
      consecutive++;
      score += 1 + consecutive;
      pi++;
    } else {
      consecutive = 0;
    }
    ci++;
  }

  // パターン全体がマッチしなければ null
  if (pi < p.length) return null;

  // 候補が短いほど高スコア（より正確なマッチ）
  score += Math.max(0, 100 - c.length);

  return score;
}

/**
 * ファイルパスのリストをファジーフィルタリングし、スコア順で上位 limit 件を返す。
 * pattern が空の場合は先頭 limit 件をそのまま返す。
 */
export function fuzzyFilter(pattern: string, files: string[], limit: number): FuzzyResult[] {
  if (pattern.length === 0) {
    return files
      .slice(0, limit)
      .map((path) => {
        const slashIdx = path.lastIndexOf("/");
        return {
          path,
          name: path.slice(slashIdx + 1),
          dir: slashIdx >= 0 ? path.slice(0, slashIdx) : "",
          score: 0,
        };
      });
  }

  const results: FuzzyResult[] = [];
  for (const path of files) {
    const score = fuzzyScore(pattern, path);
    if (score !== null) {
      const slashIdx = path.lastIndexOf("/");
      results.push({
        path,
        name: path.slice(slashIdx + 1),
        dir: slashIdx >= 0 ? path.slice(0, slashIdx) : "",
        score,
      });
    }
  }

  results.sort((a, b) => b.score - a.score);
  return results.slice(0, limit);
}
