/**
 * Web 閲覧ページのルーティング（純粋関数）。3種類のパスしか無いため vue-router は使わない。
 *
 *   /worktrees
 *   /worktrees/:id/artifacts
 *   /worktrees/:id/artifacts/:artifactId
 *   /repositories/:repoKey/artifacts
 *   /repositories/:repoKey/artifacts/:artifactId
 *
 * `:id` / `:repoKey` / `:artifactId` は 1 セグメントの文字列で、
 * ビルド時は encodeURIComponent、パース時は decodeURIComponent する
 * （リポジトリの `key` は `repo_artifacts_key` の 32桁 hex だが、
 * ワークツリー ID・アーティファクト ID は任意文字列になりうるため）。
 */

export type WebScopeKind = "worktree" | "repository";

export type WebRoute =
  | { page: "worktrees" }
  | { page: "list"; scope: WebScopeKind; scopeKey: string }
  | { page: "view"; scope: WebScopeKind; scopeKey: string; artifactId: string }
  | { page: "notFound" };

function safeDecode(segment: string): string | null {
  try {
    return decodeURIComponent(segment);
  } catch {
    return null;
  }
}

export function parseWebPath(pathname: string): WebRoute {
  const segments = pathname.split("/").filter((s) => s.length > 0);
  if (segments.length === 0) return { page: "notFound" };

  const root = segments[0];
  if (root !== "worktrees" && root !== "repositories") return { page: "notFound" };
  const scope: WebScopeKind = root === "worktrees" ? "worktree" : "repository";

  if (scope === "worktree" && segments.length === 1) {
    return { page: "worktrees" };
  }

  if (segments.length === 3 && segments[2] === "artifacts") {
    const scopeKey = safeDecode(segments[1]);
    if (scopeKey === null || scopeKey === "") return { page: "notFound" };
    return { page: "list", scope, scopeKey };
  }

  if (segments.length === 4 && segments[2] === "artifacts") {
    const scopeKey = safeDecode(segments[1]);
    const artifactId = safeDecode(segments[3]);
    if (scopeKey === null || scopeKey === "" || artifactId === null || artifactId === "") {
      return { page: "notFound" };
    }
    return { page: "view", scope, scopeKey, artifactId };
  }

  return { page: "notFound" };
}

export function buildWebPath(route: WebRoute): string {
  switch (route.page) {
    case "worktrees":
      return "/worktrees";
    case "notFound":
      return "/worktrees";
    case "list": {
      const root = route.scope === "worktree" ? "worktrees" : "repositories";
      return `/${root}/${encodeURIComponent(route.scopeKey)}/artifacts`;
    }
    case "view": {
      const root = route.scope === "worktree" ? "worktrees" : "repositories";
      return `/${root}/${encodeURIComponent(route.scopeKey)}/artifacts/${encodeURIComponent(route.artifactId)}`;
    }
  }
}
