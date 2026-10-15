/**
 * Claude Code を、一時ファイルに書いたプロンプト付きで起動するシェルコマンドを組む純関数。
 *
 * シェル別のクォート（PowerShell の `Get-Content` / POSIX の `$(cat ...)`）をここ 1 か所に持つ。
 * タスク実行（`useTaskExecution.launchAgentInTerminal`）と、インスタンス専用 AI ターミナルの起動の
 * 両方がこれを使う。Rust 側でクォートを組み直さないのは、シェル判定とクォートを二重に持つと
 * 注入の穴になりやすいため。
 */

/**
 * `claude --model <m>` にそのままシェルへ渡るので、安全な文字種だけを受理する（Rust の `validate_model` と同じ）。
 * 先頭が `-` だと別のフラグとして解釈されるので拒否する。
 */
export function isSafeModelName(model: string): boolean {
  return model.length > 0 && model.length <= 100 && /^[A-Za-z0-9._[\]][A-Za-z0-9._\-[\]]*$/.test(model);
}

export interface ClaudeLaunchOptions {
  /** claude に渡すフラグ（例: `--permission-mode plan`）。省略なら素の `claude` */
  flags?: string;
  /** プロンプトを書いた一時ファイルの絶対パス。省略（空）なら、プロンプト無しで起動する */
  tempPath?: string;
  isPowerShell: boolean;
  /**
   * プロンプト（位置引数）の前に `--` を置く。**プロンプトが信頼できない入力（アプリ）のときは必須。**
   * 先頭が `-` の文字列は claude にフラグとして解釈され、Windows の PowerShell 5.1 では `$p` に `"` を
   * 含めるだけで引数が分割されるので、`--dangerously-skip-permissions` のようなフラグを注入できてしまう。
   * `--` 以降はすべて位置引数になるので、分割が起きても注入にはならない（プロンプトの中身が変わるだけ）。
   */
  endOfOptions?: boolean;
}

/** 末尾に CR を含む。一時ファイルは起動後に削除する */
export function buildClaudeLaunchCommand(opts: ClaudeLaunchOptions): string {
  const head = opts.flags ? `claude ${opts.flags}` : "claude";
  const base = opts.endOfOptions && opts.tempPath ? `${head} --` : head;
  const tempPath = opts.tempPath;
  if (!tempPath) return `${base}\r`;
  if (opts.isPowerShell) {
    return `$p = Get-Content "${tempPath}" -Raw -Encoding UTF8; ${base} "$p"; Remove-Item "${tempPath}"\r`;
  }
  return `${base} "$(cat "${tempPath}")"; rm "${tempPath}"\r`;
}

/** インスタンス専用 AI ターミナル用のフラグ。権限モード系は付けない（= ユーザーの既定。アプリからは指定できない） */
export function ownedAiTerminalFlags(model?: string | null): string | undefined {
  if (model == null || model === "") return undefined;
  if (!isSafeModelName(model)) throw new Error(`model に使えない文字が含まれています: ${model}`);
  // 引用符で包む。`[` `]` は bash 系シェルではグロブになり、cwd のファイル名次第で別の文字列になるため
  return `--model "${model}"`;
}
