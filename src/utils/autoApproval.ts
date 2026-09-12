import type { Terminal } from "@xterm/xterm";
import { invoke } from "@tauri-apps/api/core";
import { logDebug } from "./log";

export interface TerminalForApproval {
  id: number;
  getTerminal(): Terminal | null;
  write(data: string): Promise<void>;
}

export interface ApprovalLoopResult {
  approved: boolean;
  lastCommand: string | undefined;
}

/** AI判定の結果 */
export interface JudgeResult {
  safe: boolean;
  command?: string;
}

/** xterm バッファの末尾 N 行をテキストとして取得（ANSI 除去済み） */
export function getRecentLines(terminal: Terminal, n: number): string {
  const buf = terminal.buffer.active;
  const end = buf.length;
  const start = Math.max(0, end - n);
  const lines: string[] = [];
  for (let i = start; i < end; i++) {
    const line = buf.getLine(i);
    if (line) {
      lines.push(line.translateToString(true));
    }
  }
  return lines.join("\n");
}

/**
 * テキスト内に承認プロンプトが含まれるか判定
 *
 * ## Rust 側の `prompt_parser` との二重管理について（#215 で判断）
 *
 * Rust 側に `prompt_parser::parse_prompt` があり、ダイアログの形状と選択肢を構造化して
 * 取り出せる。それでもここの正規表現を残して**二重管理を受け入れている**。理由:
 *
 * - **役割が違う。** ここは「AI 判定を走らせる価値があるか」を判断する安価な boolean ゲートで、
 *   ポーリング tick ごとに全ターミナル分走る。構造化された選択肢は要らない。
 * - **入力が違う。** ここは手元の xterm.js バッファ (`buffer.active`) をそのまま見る。
 *   Rust 側は出力履歴 64KB（`prompt_parser::REPLAY_BYTES`）を VT エミュレータへ流し直して
 *   画面を再生する。tick ごとに
 *   全端末ぶんそれをやると、IPC 往復と再生コストが tick に乗る。
 * - **落ちたときの向きが違う。** ここは誤検出しても AI 判定という次の関門があり、
 *   検出漏れは「自動承認されない」で済む。`prompt_parser` の誤りは他ワークツリーの
 *   ダイアログへキーを送る話なので、`fingerprint` 照合と `unknown` での送信拒否という
 *   別の安全弁が要る。
 *
 * 寄せるなら「フロントが `oretachi_inspect_prompt` を呼ぶ」形になるが、上のコスト差から
 * 現状維持とする。**ただし承認プロンプトの文言が変わったときは両方直す**（`ccPrompt()` の
 * サンプルと `prompt_parser.rs` のテスト用画面が対応関係にある）。
 */
export function hasApprovalPrompt(content: string): boolean {
  return content
    .split("\n")
    .some((line) => !isAutoSuggestionLine(line) && APPROVAL_PROMPT_LINE.test(line));
}

/**
 * Claude Code が**入力待ちの入力欄に出す自動候補（ゴーストテキスト）**の行か（#289）。
 *
 * 候補は `❯` と中身の間に NBSP (U+00A0) が入る（人が打つと NBSP は上書きされて消える）。
 * Rust 側 `prompt_parser::INPUT_PLACEHOLDER_GAP` と同じ手がかりで、実機で確認済み。
 *
 * **これを除外しないと、候補がたまたま `Yes` だった瞬間に `APPROVAL_PROMPT_LINE` が立つ。**
 * NBSP は `\s` に含まれるので `❯<NBSP>Yes` は素の `❯ Yes` と区別が付かず、
 * ダイアログが出ていない入力待ちの端末へ AI 判定 → Enter を送ることになる。
 *
 * **限界: 見るのは候補の 1 行目だけ。** 候補が入力欄の幅を超えて折り返すと 2 行目以降に
 * NBSP が無いので素通しになる（`getRecentLines` は物理行単位なので連結もされない）。
 * 折り返した後半にたまたま承認プロンプトの形が現れる必要があり、1 行目を塞ぐだけでも
 * 従来より厳しくなるため、ここは 1 行目に絞っている。
 */
export function isAutoSuggestionLine(line: string): boolean {
  return /^[\s│┃|║]*[❯►>]\u00a0/.test(line);
}

/**
 * 承認プロンプト行にマッチする正規表現。
 *
 * `❯ Yes` だけでなく **`❯ 1. Yes`（現在の Claude Code の書式）**にも一致させる。
 * 番号付きに一致しなかった頃は、許可ダイアログで実質 `Do you want to` の 1 行だけが
 * 検出条件になっていて、その行がダイアログ上端寄りにあるため窓が狭いと落ちていた (#252)。
 *
 * **`Yes` の後ろを行末に固定しているのは意図的**。`❯ 1. Yes` にマッチさせるだけの
 * つもりで `Yes` の後を開けると、**プラン承認ダイアログ**の
 *
 * ```
 *  Claude has written up a plan and is ready to execute. Would you like to proceed?
 *  ❯ 1. Yes, and use auto mode
 * ```
 *
 * まで検出対象になる。プラン承認は `NotifyKind::Approval` で届き
 * (`useAppAutoApproval.ts` の `kind === "approval"` を通る) 、`ai_judge.rs` の
 * 判定プロンプトは **CLI コマンドの危険性しか問うていない**ので、読み取り中心のプランなら
 * safe と返る。そこへ Enter を送ると `1. Yes, and use auto mode` が確定して
 * auto mode に入り、以後の許可は Claude Code 側で素通しになる = 自動承認の
 * AI 判定ゲートそのものが無効化される。AskUserQuestion の `❯ 1. Yes, use approach A`
 * も同様。許可ダイアログの選択肢1は必ず素の `Yes` なので、行末固定で十分に拾える。
 */
const APPROVAL_PROMPT_LINE =
  /[❯►]\s*(?:\d+\.\s*)?Yes\s*$|\(Y\/n\)|\[Y\/n\]|Allow\s+\w|Do you want to/i;

/** 承認プロンプトを探すときにバッファ末尾から見る行数 */
export const APPROVAL_SCAN_LINES = 60;

/** ダイアログ領域として承認プロンプト行の上下何行を見るか */
const APPROVAL_REGION_LEAD = 8;
const APPROVAL_REGION_TRAIL = 10;

/**
 * バッファ末尾の窓から「ダイアログ領域」だけを切り出して正規化する。
 *
 * **末尾 N 行をそのまま比較してはいけない。** `getRecentLines` はバッファ末尾からの
 * 相対窓なので、ダイアログの下に 1 行出力が増えるだけで窓の先頭がずれて全体が
 * 不一致になる。それを変化とみなすと、`hasApprovalPrompt` が真なのに Enter を送らない
 * = #252 と同じストールに戻る。
 *
 * そこで承認プロンプト行をアンカーにして、そこから上 `LEAD` 行・下 `TRAIL` 行だけを
 * 比較対象にする。上を含めるのはツール名やファイル名がプロンプト行の上に出るため
 * （含めないと「別ファイルに対する同じ形の Write ダイアログ」を同一と誤認する）。
 * 下を打ち切るのはダイアログより後ろに出た出力を無視するため。
 *
 * アンカーは**最初の**マッチ行にする。許可ダイアログでは `Do you want to` が上端で、
 * ここを起点にすると選択肢まで `TRAIL` 行に収まる。
 *
 * 行末の空白と末尾の空行も落とす（xterm はカーソル位置やパディングで揺れる）。
 */
function extractApprovalRegion(content: string): string | null {
  const lines = content.split("\n");
  // 自動候補の行はアンカーにしない（#289）。`getRecentLines` はスクロールバックごと
  // 見るので、過去フレームに残った `❯<NBSP>Yes` が窓に入るとアンカーがそこへ付く。
  // 凍結した領域は判定中に変わらないので、**ダイアログが差し替わっても「同じ画面」**
  // になり、未判定のダイアログへ Enter を送ることになる（`hasApprovalPrompt` /
  // `detectOretachiToolPrompt` と同じガードをここにも掛ける）。
  const anchor = lines.findIndex(
    (line) => !isAutoSuggestionLine(line) && APPROVAL_PROMPT_LINE.test(line)
  );
  if (anchor === -1) return null;
  const start = Math.max(0, anchor - APPROVAL_REGION_LEAD);
  const end = Math.min(lines.length, anchor + APPROVAL_REGION_TRAIL + 1);
  return lines
    .slice(start, end)
    .map((line) => line.replace(/\s+$/, ""))
    .join("\n")
    .replace(/\n+$/, "");
}

/**
 * AI 判定の前後で同じ承認ダイアログが出続けているかを判定する。
 *
 * 判定には 20〜35 秒かかるため、その間に人が手でダイアログを消し、別のダイアログが
 * 出ている可能性がある。`hasApprovalPrompt` が真でも中身が別物なら、未判定の
 * ダイアログへ Enter を送ることになるので送らない。
 *
 * MCP 側の `expect_fingerprint` (`prompt_parser::fingerprint_of`) と**同じ保証ではない**。
 * あちらはパース済み構造だけをハッシュして `tail` を意図的に外している（スピナーの
 * 1 コマで毎回変わって常に stale になるため）。こちらは画面テキストを見るので、
 * 判定中にターミナルがリサイズされて折り返しが変わると不一致になりうる。
 * 不一致は「Enter を送らない」で終わらせず次のターミナルの判定へ進める。
 */
export function isSameApprovalScreen(before: string, after: string): boolean {
  const a = extractApprovalRegion(before);
  const b = extractApprovalRegion(after);
  return a !== null && a === b;
}

/**
 * 無条件に自動承認する oretachi 自身の MCP ツール名。
 *
 * 除外しているもの (従来どおり AI 判定に委ねる):
 * - oretachi_close_worktree / oretachi_kill_terminal … 破壊的
 * - oretachi_spawn_terminal / oretachi_write_terminal … 任意コマンドを PTY に流し込める
 *   (= 任意コード実行)。無条件承認すると安全ゲートが無効化される
 * - oretachi_answer_prompt … 他ワークツリーの**許可ダイアログを承認しうる** (#215)。
 *   矢印 + CR で `1. Yes` を確定できるので write_terminal と同等に任意コード実行と等価
 * - oretachi_add_task … 任意 prompt からワークツリー作成とエージェント実行を発火する
 * - oretachi_import_worktree … settings を書き換えてワークツリーを登録する
 *
 * artifact_module は artifact より前に置く (正規表現の選択肢で長い方を優先させるため)。
 */
export const ORETACHI_AUTO_APPROVE_TOOLS = [
  "artifact_module",
  "artifact",
  "search_artifact",
  "notify_worktree",
  "oretachi_set_description",
  "oretachi_set_tray_notification",
  "oretachi_get_worktree_status",
  "oretachi_get_app_options",
  "oretachi_show_worktree",
  "oretachi_list_repository",
  "oretachi_list_workgroups",
  "oretachi_list_terminals",
  "oretachi_read_terminal",
  "oretachi_inspect_prompt",
] as const;

/**
 * 無条件承認から外す「破壊的な command」。ツール名は自動承認対象でも、この
 * command が画面に見えている呼び出しだけは AI 判定 / 手動承認へ落とす。
 *
 * `artifact(command: "delete")` はアーティファクト本体・モジュール・ストア
 * (useMemory の中身) をまとめて消し、ゴミ箱もバックアップも無い。UI からの削除は
 * 確認ダイアログ必須 (`ArtifactViewerApp.vue`) なので、MCP 経路だけを素通しにしない。
 * `artifact_module` の delete は 1 モジュール単位なので従来どおり対象外。
 *
 * ダイアログにパラメータが出ていない (折り返しで窓の外へ出た / CC が省略した) 場合は
 * 検出できず従来どおり承認される。**ここはフェイルオープンなので単独の防波堤にしない**
 * (同梱スキルの `allowed-tools` で許可されている場合はそもそもダイアログが出ず、
 * この判定も走らない)。削除の本命のゲートは MCP 側の `confirm_delete` 必須化で、
 * これはその手前の追加の網。
 *
 * キーのクォート有無は CC の描画に依存するので両方許す (`command: "delete"` /
 * `"command": "delete"`)。
 */
const ORETACHI_DESTRUCTIVE_COMMANDS: Record<string, RegExp> = {
  artifact: /["']?command["']?\s*:\s*["']?delete\b/i,
};

/** 承認プロンプト行を探すときに前後何行を対象にするか */
const ORETACHI_PROMPT_WINDOW = 8;

/**
 * 承認プロンプトが oretachi 自身の MCP ツール呼び出しかを判定し、ツール名を返す。
 *
 * Claude Code は plan モードで MCP ツールを一律 ask にするため
 * (readOnlyHint が無い MCP ツールは permissions.allow でも抑止できない)、
 * oretachi 側で自分のツールだけを決め打ちで承認する。
 *
 * 画面上の無関係な位置に "oretachi" の文字列があっても誤爆しないよう、
 * 承認プロンプト行の周辺だけを走査する。
 */
export function detectOretachiToolPrompt(content: string): string | null {
  const lines = content.split("\n");
  // 末尾側の承認プロンプト行を探す
  let promptIndex = -1;
  for (let i = lines.length - 1; i >= 0; i--) {
    // ここは `APPROVAL_PROMPT_LINE` を使わない。MCP ツールの許可ダイアログは必ず
    // `Do you want to` 行を持つので現状で足り、番号付き `❯ 1. Yes` を足すと
    // アンカーが 1 行下がって `ORETACHI_PROMPT_WINDOW` の上方向カバーが 1 行減る
    // （ツール名の行はプロンプト行の上に出る）。
    // 自動候補の行は除外する（`hasApprovalPrompt` と同じ理由。#289）
    if (
      !isAutoSuggestionLine(lines[i]) &&
      /❯\s*Yes|►\s*Yes|Do you want to/i.test(lines[i])
    ) {
      promptIndex = i;
      break;
    }
  }
  if (promptIndex === -1) return null;

  const start = Math.max(0, promptIndex - ORETACHI_PROMPT_WINDOW);
  const end = Math.min(lines.length, promptIndex + ORETACHI_PROMPT_WINDOW + 1);
  const window = lines.slice(start, end).join("\n");

  // Claude Code の MCP ツール表示名は必ず "<サーバ名> - <ツール名>" 形式
  //   プラグイン経由: "plugin:oretachi:oretachi - artifact"
  //   直接登録時:     "oretachi - artifact"
  // ハイフンの前後に空白を必須にしないと、cwd のパス (例:
  // "X:\devel\worktree\oretachi-artifact") が「サーバ名 - ツール名」として
  // 誤マッチする。承認プロンプトの選択肢2行目には必ず cwd が含まれるため、
  // ワークツリー名がツール名と一致するだけで任意コマンドが自動承認されてしまう。
  const toolAlternation = ORETACHI_AUTO_APPROVE_TOOLS.join("|");
  const match = window.match(
    new RegExp(
      `(?:^|[\\s(\\[])(?:plugin:oretachi:oretachi|oretachi)\\s+-\\s+(${toolAlternation})(?![\\w-])`,
      "im"
    )
  );
  if (!match) return null;
  const tool = match[1];
  // 破壊的な command が見えている呼び出しは即承認しない (AI 判定 / 手動へ回す)
  if (ORETACHI_DESTRUCTIVE_COMMANDS[tool]?.test(window)) return null;
  return tool;
}

/** ターミナル内容を解析し自動承認すべきか判定 */
export async function analyzeForApproval(
  worktreeId: string,
  content: string,
  cwd: string = "",
  additionalPrompt?: string,
): Promise<JudgeResult> {
  const promptFound = hasApprovalPrompt(content);

  logDebug(
    `[AutoApproval] analyze start worktreeId=${worktreeId} totalLines=${content.split("\n").length} hasApprovalPrompt=${promptFound}`
  );

  if (!promptFound) {
    logDebug("[AutoApproval] → skip: no approval prompt detected");
    return { safe: false };
  }

  // oretachi 自身の MCP ツールは AI 判定を挟まず即承認する。
  // plan モードの Claude Code は MCP ツールを permissions.allow でも抑止できず
  // 一律 ask にするため、ここで拾わないと毎回手動承認になる。
  const oretachiTool = detectOretachiToolPrompt(content);
  if (oretachiTool) {
    logDebug(`[AutoApproval] → oretachi own MCP tool (${oretachiTool}), auto-approve`);
    return { safe: true, command: `oretachi ${oretachiTool}` };
  }

  // AI 判定: claude --model haiku で安全性を判定
  try {
    const result = await invoke<JudgeResult>("judge_approval", {
      worktreeId,
      content,
      cwd,
      additionalPrompt: additionalPrompt || null,
    });
    logDebug(`[AutoApproval] AI judgment: ${result.safe ? "safe" : "unsafe"} command=${result.command ?? "none"}`);
    return result;
  } catch (e) {
    logDebug(`[AutoApproval] AI judgment failed: ${e}`);
    return { safe: false }; // エラー時は安全側 (承認しない)
  }
}

/** 全ターミナルを走査し最初に承認できたものでEnterを送信する */
export async function runApprovalLoop(
  terminals: TerminalForApproval[],
  worktreeId: string,
  cwd: string,
  additionalPrompt?: string,
): Promise<ApprovalLoopResult> {
  let approved = false;
  let lastCommand: string | undefined;

  for (const termRef of terminals) {
    const terminal = termRef.getTerminal();
    if (!terminal) {
      logDebug(`[AutoApproval] tid=${termRef.id} terminal=null, skip`);
      continue;
    }
    // 事前に末尾60行でプロンプト判定し、無ければAI判定と200行取得をスキップ。
    // (大半の tick は「プロンプト無し」なのでここで早期returnすれば debug log ノイズも減る)
    // 60行はプロンプト直後に追加ログが出るケースに対するマージン。
    const quickContent = getRecentLines(terminal, APPROVAL_SCAN_LINES);
    if (!hasApprovalPrompt(quickContent)) {
      continue;
    }
    const content = getRecentLines(terminal, 200);
    logDebug(`[AutoApproval] tid=${termRef.id} content(last200)=${content.slice(-200)}`);
    const judgeResult = await analyzeForApproval(worktreeId, content, cwd, additionalPrompt);
    if (judgeResult.command) {
      lastCommand = judgeResult.command;
    }
    if (judgeResult.safe) {
      // バッファ再チェック: AI判定完了後、同じ承認プロンプトがまだ出ているか確認。
      // **窓は検出と必ず同じ APPROVAL_SCAN_LINES にする。** 以前は 10 行だけ見ていたため、
      // ダイアログ上端寄りの `Do you want to` 行が窓から外れて偽陰性になり、
      // 安全と判定した許可を捨ててセッションが止まっていた (#252)。
      const freshContent = getRecentLines(terminal, APPROVAL_SCAN_LINES);
      // 送らないと決めた場合は `break` ではなく `continue`。この経路には再試行が無く
      // (`runApprovalLoop` はポーリングではなく notify-worktree イベント駆動)、
      // ここで打ち切ると同じイベントで残りのターミナルの本物のダイアログが
      // 一度も判定されないまま取りこぼされる。
      if (!hasApprovalPrompt(freshContent)) {
        logDebug(`[AutoApproval] tid=${termRef.id} → prompt disappeared, skip Enter`);
        continue;
      }
      if (!isSameApprovalScreen(quickContent, freshContent)) {
        logDebug(`[AutoApproval] tid=${termRef.id} → screen changed since judgment, skip Enter`);
        continue;
      }
      logDebug(`[AutoApproval] tid=${termRef.id} → approved, sending Enter`);
      await termRef.write("\r");
      approved = true;
      break;
    } else {
      logDebug(`[AutoApproval] tid=${termRef.id} → not approved`);
    }
  }

  return { approved, lastCommand };
}

/** 進行中のAI判定をキャンセル */
export async function cancelApproval(worktreeId: string): Promise<void> {
  try {
    await invoke("cancel_approval", { worktreeId });
  } catch (e) {
    logDebug(`[AutoApproval] cancelApproval failed: ${e}`);
  }
}
