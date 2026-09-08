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
    .some((line) =>
      /❯\s*Yes|►\s*Yes|\(Y\/n\)|\[Y\/n\]|Allow\s+\w|Do you want to/i.test(line)
    );
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
    if (/❯\s*Yes|►\s*Yes|Do you want to/i.test(lines[i])) {
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
  return match ? match[1] : null;
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
    const quickContent = getRecentLines(terminal, 60);
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
      // バッファ再チェック: AI判定完了後、承認プロンプトがまだあるか確認
      const freshContent = getRecentLines(terminal, 10);
      if (!hasApprovalPrompt(freshContent)) {
        logDebug(`[AutoApproval] tid=${termRef.id} → prompt disappeared, skip Enter`);
        break;
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
