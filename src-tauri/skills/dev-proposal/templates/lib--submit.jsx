
// 開発企画の送信ロジック。JSX を含まない純 JS（vitest が vm でそのまま評価する）。
//
// 送信ボタンは 2 段で動く:
//   1. 採用 id を `submission`（artifact_store）へ保存する
//   2. 生成元セッション（`META.sessionId`）の入力欄へ「起動の合図」を 1 行打ち込む
//
// **採用内容の正は 1 のストアで、2 の文面は合図でしかない。** 生成側は Step 5 で
// `artifact_store(read)` から `submission` を読む。`write_terminal` の text には出自の
// 前置きが付かない（`notify_worktree` / `add_task` と違う）ので、文面に採用内容を
// 載せて経路にすると「AI 生成アーティファクトが書いた文」を指示として扱わせることになる。
const { callTool } = require('oretachi');

/** 本文を送ってから Enter を送るまでの猶予（ミリ秒）。notification-report の `sendOne` と同じ理由 */
const SUBMIT_DELAY_MS = 150;

/** `oretachi_inspect_prompt` が Claude Code の入力欄に付ける `header`（Rust の `FreeInputKind::as_header`） */
const CC_INPUT_HEADER = '[Claude Code の入力欄]';

const errText = e => String((e && e.message) || e);

/**
 * 改行と制御文字を潰して 1 行に畳む（notification-report `lib/send` の `flatten` と同じ実装）。
 *
 * 改行を残すと、複数行のテキストが行ごとに送信されて宛先へプロンプトがばらばらに飛ぶ。
 * ESC を含む C0 制御文字も落とす（宛先の TUI へエスケープシーケンスを注入させない）。
 */
function flatten(s) {
  return String(s == null ? '' : s)
    .replace(/\r?\n/g, ' / ')
    // C1（U+0080-009F）も落とす。U+009B は 8bit CSI として解釈する端末がありうる
    // eslint-disable-next-line no-control-regex
    .replace(/[\x00-\x1f\x7f-\x9f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/** 罫線だけの行か（Rust の `is_rule_line` と同じ文字集合） */
const RULE_LINE = /^[─━═\-╭╮╰╯┌┐└┘]+$/;
/**
 * 右端にラベルが埋め込まれた罫線（実機: `────…──── add-dev-proposal-skill ─`）。
 * 入力欄の上罫線にはワークツリー名などが入る。**ラベル付きは罫線だけの行ではない**ので
 * 別に受ける。ASCII の `-` は含めない（`--- a/file` のような diff 行を罫線と読まないため）。
 */
const LABELED_RULE_LINE = /^[─━═]{3,}\s.*$/;
const isRuleLine = l => RULE_LINE.test(l) || LABELED_RULE_LINE.test(l);

/**
 * `tail`（画面末尾 12 行）に、**上下を罫線で挟まれた入力欄の箱**があるか。
 *
 * 最後の `❯` / `>` 始まりの行を入力欄の候補とし、その直上と直下（空行を除く）の
 * 行がどちらも罫線行であることを要求する。**`header` と `pendingInput` だけでは足りない**:
 * Rust は箱として同定できなかった `❯` / `>` 始まりの行（starship のプロンプト、`> quoted`
 * 形の出力行、再描画途中の画面）も `header=[Claude Code の入力欄]`・`pendingInput=''` の
 * `text` として返す（`prompt_parser.rs` の `looks_like_free_input` の縮退経路）。そこでの
 * 空は「未入力」ではなく「未読」で、CC が終了してシェルが残った端末が通ってしまう。
 * 縮退経路の行は罫線に挟まれていないので、この条件で弾ける。
 *
 * 自動候補の行は Rust が `❯ ⟪自動候補…⟫` に書き換えるが、先頭は `❯` のままなので判定は変わらない。
 */
function hasInputBox(tail) {
  if (typeof tail !== 'string') return false;
  const lines = tail.split('\n').map(l => l.trim());
  let i = lines.length - 1;
  while (i >= 0 && !/^[❯>]/.test(lines[i])) i--;
  if (i < 0) return false;
  const isRule = isRuleLine;
  let up = i - 1;
  while (up >= 0 && lines[up] === '') up--;
  let down = i + 1;
  while (down < lines.length && lines[down] === '') down++;
  return up >= 0 && down < lines.length && isRule(lines[up]) && isRule(lines[down]);
}

/**
 * いま宛先へ 1 行を打ち込んでよい画面か（**許可リスト方式**）。
 *
 * 次の 4 つがすべて成り立つときだけ true:
 *   - `shape === 'text'`            ダイアログが開いていない（開いていると文字が選択に吸われ、
 *                                    末尾の CR が既定の選択肢の確定になる。`unknown` / `pager` も除く）
 *   - `header` が CC の入力欄        `text` はシェルのプロンプトも含む。CC が終了して PowerShell が
 *                                    残った端末へ流すと、本文がコマンドとして実行される
 *   - `pendingInput === ''`         打ちかけとの連結を防ぐ。`/` `@` で始まる打ちかけは補完ポップアップが
 *                                    開いており、CR が補完の確定に食われる
 *   - `hasInputBox(tail)`           **空の `pendingInput` は「未入力」とは限らず「未読」でもある**
 *                                    （箱を同定できなかった縮退経路）。罫線で挟まれた箱が見えている
 *                                    ときだけ「未入力」と信じる
 *
 * `inputSuggestion`（ゴーストテキスト）は**判定に使わない**。ユーザー入力ではなく、候補が出ている
 * ときは `pendingInput` が空なので送ってよい（#289）。
 */
function canDeliver(prompt) {
  return (
    !!prompt &&
    prompt.shape === 'text' &&
    prompt.header === CC_INPUT_HEADER &&
    prompt.pendingInput === '' &&
    hasInputBox(prompt.tail)
  );
}

/** `canDeliver` が false のときに画面へ出す理由。true なら null */
function deliverBlockedReason(prompt) {
  if (canDeliver(prompt)) return null;
  if (!prompt) return 'セッションの画面を取得できませんでした';
  if (prompt.shape !== 'text') {
    return 'セッションがダイアログまたは分類不能の画面で止まっています。先にそちらを片付けてください';
  }
  if (prompt.header !== CC_INPUT_HEADER) {
    return 'セッションの入力欄が Claude Code ではありません（終了している可能性があります）';
  }
  if (prompt.pendingInput !== '') {
    return 'セッションの入力欄に打ちかけのテキストがあります。消してから再送してください';
  }
  return 'セッションの入力欄を確認できませんでした（Claude Code が終了している、または画面の再描画中の可能性があります）';
}

/**
 * 生成元セッションへ打ち込む 1 行。**これ単体で処理できる**ようにする
 * （押下時点で /clear や compact 済みでスキル本文がコンテキストに無いことがある）。
 */
function buildTriggerText(meta, submission) {
  return flatten(
    '[dev-proposal] oretachi:dev-proposal スキルを読み込み、' +
      `artifact_store(command:read, id:${meta.artifactId}, project_dir:${meta.projectDir}) の ` +
      `submission (submittedAt=${submission.submittedAt}) を読んで Step 5 を進めてください。` +
      'これはユーザーがアーティファクトの送信ボタンで確定した合図です。'
  );
}

/** 2 回に分けて書く。`failed`（本文未到達 = 再送可）と `pastedOnly`（再送不可）を JS 側で区別するため */
async function writeTrigger(meta, text) {
  try {
    await callTool('oretachi_write_terminal', { session_id: meta.sessionId, text, submit: false });
  } catch (e) {
    return { status: 'failed', error: errText(e) };
  }
  await new Promise(resolve => setTimeout(resolve, SUBMIT_DELAY_MS));
  try {
    await callTool('oretachi_write_terminal', { session_id: meta.sessionId, text: '\r', submit: false });
  } catch (e) {
    return { status: 'pastedOnly', error: errText(e) };
  }
  return { status: 'sent' };
}

/**
 * 合図を送る。返り値の `status`:
 *   - `sent`       本文と Enter の両方が通った
 *   - `notReady`   宛先が入力待ちでない。**何も送っていない**ので再送してよい（`reason` に理由）
 *   - `failed`     本文が届いていない（session_id 失効 / 購読不足など）。再送してよい
 *   - `pastedOnly` 本文は入力欄にあるが Enter が失敗。**再送してはいけない**（二重になる）
 */
async function deliver(meta, submission) {
  let prompt;
  try {
    prompt = await callTool('oretachi_inspect_prompt', { session_id: meta.sessionId });
  } catch (e) {
    return { status: 'failed', error: errText(e) };
  }
  if (!canDeliver(prompt)) return { status: 'notReady', reason: deliverBlockedReason(prompt) };
  return writeTrigger(meta, buildTriggerText(meta, submission));
}

/**
 * 送信ボタンの本体。`persist(submission)` は entry-point が `useMemory` のセッターを包んで渡す。
 * **保存に失敗したら端末へは送らない**（生成側が読む正が無いまま合図だけ飛ぶのを防ぐ）。
 */
async function submit({ meta, ids, persist, now }) {
  const submission = { ids: ids.slice(), submittedAt: (now || (() => new Date().toISOString()))() };
  try {
    await persist(submission);
  } catch (e) {
    return { ok: false, error: `採用内容を保存できませんでした: ${errText(e)}` };
  }
  return { ok: true, submission, delivery: await deliver(meta, submission) };
}

/** 依存先が未チェックの案 `[{ id, title, missing: [id…] }]`（ブロックはせず警告用） */
function missingDeps(proposals, selection) {
  const sel = selection || {};
  const out = [];
  for (const p of proposals || []) {
    if (!sel[p.id] || !Array.isArray(p.dependsOn)) continue;
    const missing = p.dependsOn.filter(d => typeof d === 'string' && !sel[d]);
    if (missing.length > 0) out.push({ id: p.id, title: p.title, missing });
  }
  return out;
}

exports.SUBMIT_DELAY_MS = SUBMIT_DELAY_MS;
exports.CC_INPUT_HEADER = CC_INPUT_HEADER;
exports.flatten = flatten;
exports.hasInputBox = hasInputBox;
exports.canDeliver = canDeliver;
exports.deliverBlockedReason = deliverBlockedReason;
exports.buildTriggerText = buildTriggerText;
exports.deliver = deliver;
exports.submit = submit;
exports.missingDeps = missingDeps;
