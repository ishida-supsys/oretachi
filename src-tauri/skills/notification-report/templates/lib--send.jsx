
// レポートからの送信ロジックと、カードが使う読み取りヘルパー。
//
// 宛先は各通知の発信元ワークツリーで走っている AI 端末で、許可条件は
// **レポートを置いたワークツリーがその宛先ワークツリーを購読していること**（#211）。
// 向きは購読者側が呼び出し元で、宛先側が呼び出し元を購読しているだけでは通らない。
//
// ── 3 つの送信経路 ──────────────────────────────────────────────────────────
//
// 1. **自由入力** — 宛先がダイアログ無しで入力待ち。`sendOne` が本文 → CR を送る
// 2. **ダイアログ（1 択）** — ツール許可 / プラン承認 / y-n / 番号選択。
//    `answerPrompt` が `oretachi_answer_prompt(kind: "select")` を 1 回呼ぶ
// 3. **複数設問の AskUserQuestion** — `answerAll` が
//    `oretachi_answer_prompt(kind: "selectAll")` を 1 回呼び、Rust 側が
//    「1 問選ぶ → 画面が次の設問へ進むのを待つ」を繰り返して最後の Submit まで確定する（#264）
//
// **ダイアログで止まっている宛先へ自由テキストを送ってはいけない。** テキストは
// ダイアログに吸われ、末尾の CR が意図しない選択肢（許可ダイアログの既定は `1. Yes`）の
// 確定として解釈される。これが #215 の出発点。判定は `n.prompt.shape` で行う。
const { callTool } = require('oretachi');

// 候補で言い表せないときの選択肢。選ぶと補足プロンプトが必須になる
const OTHER = 'その他（補足で指示）';

/** 本文を送ってから Enter を送るまでの猶予（ミリ秒）。`sendOne` のコメント参照 */
const SUBMIT_DELAY_MS = 150;

/**
 * キー操作で答える形状。`text` だけが自由入力で、それ以外はダイアログが開いている。
 *
 * `unknown` は**分類できなかった画面**で、キーを送ってはいけない（推測で送ると
 * 別のダイアログの既定選択を確定しうる）。カードは読み取り専用になる。
 */
const DIALOG_SHAPES = ['permission', 'plan', 'askUserQuestion', 'yesno', 'numbered'];

/**
 * 「報告のみ」のイベント種別（#228）。**人の判断を必要としない。**
 *
 * `worktree.created` / `worktree.closed` は `notify_worktree` から発行できず、
 * oretachi 内部の `fire_worktree_created` / `fire_worktree_closed`（`src-tauri/src/lib.rs`）
 * からしか出ない。本文も `format_inbox_line`（`event_db.rs`）が定型文に組み直すので、
 * エージェントが自由文で「判断が欲しい」と書き込む余地が構造的に無い。だから返答 UI を
 * 出さず、読むだけのカードにする。
 *
 * **`completed` / `hook` はここに入れない。** どちらも `notify_worktree` 経由で
 * エージェントが任意の本文を書けるため、「実装は終わったので次の指示が欲しい」の
 * ような返答待ちが混ざりうる。送信 UI を消すと、その返答待ちに気づく導線まで消える。
 *
 * **判定を `kind` に置いているのは、生成側 AI の付け忘れを構造的に防ぐため。**
 * `data/report` に「報告カードです」というフラグを足すと、生成する AI がそれを
 * 落とした瞬間に判断不要の通知が返答待ちとして並ぶ（あるいは逆）。`kind` は
 * `oretachi_poll_inbox` の返り値をそのまま写す既存フィールドなので、ここから
 * 導出すればレポート側で機械的に決まる。
 */
const REPORT_KINDS = ['worktree.created', 'worktree.closed'];

/**
 * このカードが「報告のみ」か（＝返答 UI を出さない）。
 *
 * 報告カードは送信経路を一切持たないので、`subscribed` / `sessionId` / `prompt` を
 * 参照しない。`worktree.closed` は発信元ワークツリーが既に削除済みで、ID 指定の
 * 購読行も fanout 直後に消えている（`lib.rs` の `fire_worktree_closed`）ため、
 * そもそもこの 3 つを埋められない。
 */
function isReportOnly(n) {
  return !!n && REPORT_KINDS.indexOf(n.kind) >= 0;
}

/**
 * Claude Code のダイアログの形状（Rust の `plan_keys` の `is_cc_select()` と対応）。
 *
 * `escapeThenText`（ESC で抜けて本文）と `selectThenText`（`Type something.` を選んで
 * 本文。#265）の**両方**が、Rust 側でこの 3 形状にしか通らない。
 * **UI が Rust に無い経路を出してはいけない** —— 押した瞬間 `unsupported` が返り、
 * そのカードは `readOnly` になって選び直せず死ぬ。
 */
const CC_SELECT_SHAPES = ['permission', 'plan', 'askUserQuestion'];

/**
 * ESC で抜けてから本文を送れる形状。
 *
 * **`escapeHatch` だけで判断してはいけない。** `escapeHatch` は画面末尾に
 * `Esc to cancel` があれば立つので `numbered` / `yesno` でも立ちうるが、Rust 側は
 * `is_cc_select()` の 3 形状しか `escapeThenText` を受け付けない。
 */
const ESCAPABLE_SHAPES = CC_SELECT_SHAPES;

/** 形状ごとの日本語ラベル。カードの見出しに出す */
const SHAPE_LABEL = {
  text: '自由入力',
  permission: 'ツール許可',
  plan: 'プラン承認',
  askUserQuestion: '設問',
  yesno: 'y/n 確認',
  numbered: '番号選択',
  menu: 'ピッカー',
  pager: 'ページャ',
  unknown: '分類不能',
};

/**
 * 改行と制御文字を潰して 1 行に畳む。
 *
 * **改行を残さないのは必須。** `oretachi_write_terminal` は `submit` が true のとき
 * `\n` を `\r` へ正規化するため、複数行のテキストを渡すと行ごとに送信され、
 * 宛先の AI エージェントへプロンプトがばらばらに飛ぶ。
 *
 * ESC を含む C0 制御文字も落とす。本文は通知の中身、補足はユーザー入力なので、
 * ESC がそのまま届くと宛先の TUI へ任意のエスケープシーケンスを流せてしまう
 * （ブラケットペーストで囲んでいないため、囲みからの脱出ではなく直接注入になる）。
 *
 * **`oretachi_answer_prompt` へ渡す本文もここを通す。** Rust 側も制御文字を弾くが、
 * 片方が外れても注入にならないよう二重に置いている。
 */
function flatten(s) {
  return String(s == null ? '' : s)
    .replace(/\r?\n/g, ' / ')
    // eslint-disable-next-line no-control-regex
    .replace(/[\x00-\x1f\x7f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

// ── 本文の読み取り（#264）─────────────────────────────────────────────────────
//
// カードは**生データを一切出さない。** 生の hook JSON が読めないからレポートを
// 作っているので、それをそのまま貼り直したら意味が無い（折りたたみで残すのも
// やらない）。生成側が `paragraphs` / `bullets` / `fields` / `links` へ
// 人が読める形で入れる。材料が足りなければ生成側がターミダルを読み、
// それでも足りなければ該当ワークツリーの issue / git / アーティファクトを見に行く。

/**
 * 段落の配列。**必ず文字列の配列を返す。**
 *
 * 旧形式（`body` の 1 本の文字列）も受ける。`oretachi_poll_inbox` の `body` は
 * **パース済みの JSON オブジェクト**なので、生成側が取り違えるとオブジェクトが入る。
 * React はオブジェクトを子として描画できず throw し、**アーティファクトに
 * エラーバウンダリが無いためレポート全体が描画不能になる**（1 枚のカードの
 * 取り違えで、他の通知への返答窓口まで失われる）。ここはその保険。
 */
function paragraphsOf(n) {
  if (n && Array.isArray(n.paragraphs)) {
    return n.paragraphs.filter(p => typeof p === 'string' && p.trim()).map(p => p.trim());
  }
  const b = n && n.body;
  if (typeof b === 'string' && b.trim()) return [b.trim()];
  if (b == null) return [];
  // ここへ来るのは生成側が `text` ではなく `body`（オブジェクト）を入れた事故。
  // 生 JSON を人へ見せる意味は無いので、事故だと分かる 1 行だけ出す
  return ['（本文を整形できませんでした。ターミナルを開いて確認してください）'];
}

/** 送信テキストへ埋める用の 1 行本文 */
function bodyText(n) {
  const ps = paragraphsOf(n);
  return ps.length > 0 ? ps.join(' / ') : '(本文がありません)';
}

/**
 * 本文中の記法をリッチテキストの断片へ割る（#264）。
 *
 * エージェントは `notify_worktree` の本文に Markdown を書く。素で出すと
 * `**分割して**` のような記号がそのまま並んで読みにくく、`#203` も
 * `https://…` もただの文字列のままでリンクにならない。
 *
 * 返すのは `[{ type, text, href }]`。`type` は `text` / `bold` / `code` / `link`。
 * **描画側で `href` 以外を URL として扱わないこと**（本文はエージェントが書いた
 * 文字列なので、リンク先はここで組み立てたものだけに限る）。
 */
function richSegments(text, repoUrl) {
  const out = [];
  const re = /(\*\*[^*\n]+\*\*|`[^`\n]+`|https?:\/\/[^\s<>"'）)、。]+|#\d+)/g;
  let last = 0;
  let m;
  const src = String(text == null ? '' : text);
  while ((m = re.exec(src)) !== null) {
    if (m.index > last) out.push({ type: 'text', text: src.slice(last, m.index) });
    const t = m[0];
    if (t.slice(0, 2) === '**') {
      out.push({ type: 'bold', text: t.slice(2, -2) });
    } else if (t[0] === '`') {
      out.push({ type: 'code', text: t.slice(1, -1) });
    } else if (t[0] === '#') {
      // リポジトリが分からなければただの文字として出す（当てずっぽうのリンクは張らない）
      if (repoUrl) out.push({ type: 'link', text: t, href: `${repoUrl}/issues/${t.slice(1)}` });
      else out.push({ type: 'code', text: t });
    } else {
      out.push({ type: 'link', text: t, href: t });
    }
    last = m.index + t.length;
  }
  if (last < src.length) out.push({ type: 'text', text: src.slice(last) });
  return out;
}

// ── コマンド実行の選択肢（#288）──────────────────────────────────────────────
//
// `!` 始まりの候補は、宛先の AI への**指示ではなく**、宛先のターミナルで走る
// シェルコマンドとして扱う。Claude Code は入力欄の先頭に `!` が来た状態で CR を
// 受けるとシェルモードとして実行する（実測: `!echo BANGTEST_A` を
// `oretachi_write_terminal(submit: false)` で 1 回書いてから CR を送ると
// `⎿ BANGTEST_A` が返り、シェルとして実行された）。
//
// **前置きを付けてはいけない。** `buildReplyText` の断り書きが先頭に付くと `!` が
// 行頭から外れ、シェルモードに入らないまま「`!` で始まる長文」が 1 回のプロンプトと
// して宛先のエージェントへ飛ぶ。これが #288 の「余計な記述が入って実行されない」。
//
// 同じ理由で**補足プロンプトも付けられない。** 後ろに足した文字はコマンドの一部に
// なる（`components/NotificationCard` が候補側の補足欄を塞いでいる）。
//
// ── 権限ゲートについて（把握した上での設計）────────────────────────────────
//
// **シェルモードで走るコマンドは、宛先エージェントの Bash ツール許可ダイアログを
// 通らない。** 人がターミナルで `!` を打つのと同じ経路だからで、`write_terminal` の
// text は Rust 側でも無加工（`normalize_artifact_tool_params` が出自を前置するのは
// `notify_worktree` の body と `oretachi_add_task` の prompt だけ）。
//
// つまり**ここでの唯一のゲートは、人がカードで中身を読んで押すこと**になる。
// それが成り立つように:
//
//   - 表示と送信は同じ `commandOf` の戻り値を使う（見えている 1 行がそのまま走る）
//   - `flattenCommand` が制御文字を落とす（表示に出ない文字を混ぜられない）
//   - `COMMAND_MAX_LEN` を超える長さは送らせない（読まずに押す形にしない）
//   - `!` の直後がコマンド名らしくない文字列はコマンド扱いしない
//
// これは新しい権限ではない（アーティファクトの JS は元から `oretachi_write_terminal`
// で同じ文字列を書けた）。ただし**人が押す導線としては新しい**ので、候補に何を
// 入れてよいかは SKILL.md の Step 4 で読み取り系に限っている。

/** 候補として見せられるコマンドの長さ上限。これを超えると送信対象から外す */
const COMMAND_MAX_LEN = 300;

/** 前後の空白・制御文字を落として 1 行のコマンドにする。改行は空白へ畳む */
function flattenCommand(s) {
  return String(s == null ? '' : s)
    // eslint-disable-next-line no-control-regex
    .replace(/[\x00-\x1f\x7f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/**
 * コマンド実行の文字列か。`!` の**直後がコマンド名らしい文字**であることまで見る。
 *
 * 先頭 1 文字だけで判定すると、生成側 AI が書いた感嘆符始まりの日本語
 * （`!!至急やって`）や `!` 単独がシェルへ流れる。それらは返答文として
 * 従来どおり前置き付きで送るのが正しい。
 */
function isCommandChoice(s) {
  const t = String(s == null ? '' : s).trim();
  if (t[0] !== '!') return false;
  return /^[A-Za-z0-9._/\\-]/.test(t.slice(1).trim());
}

/**
 * 下書き / 送信記録がコマンド実行なら、宛先へそのまま流す `!` 込みの 1 行を返す。
 * コマンドでなければ null。
 *
 * `その他（補足で指示）` を選んだときは補足欄の中身をコマンド源として見る。
 * 候補ボタンだけを特別扱いすると、補足欄に `!pnpm test` と書いた人が
 * 前置き付きのテキストを送る同じ罠に落ちる。
 */
function commandOf(a) {
  const src = a && a.choice === OTHER ? a.note : (a && a.choice);
  if (!isCommandChoice(src)) return null;
  return `!${flattenCommand(String(src).trim().slice(1))}`;
}

/** 長すぎて人が読み切れないコマンドか（送信対象から外す判定） */
function commandTooLong(cmd) {
  return typeof cmd === 'string' && cmd.length > COMMAND_MAX_LEN;
}

/**
 * この通知への直前の送信が「コマンドを実行しただけ」だったか。
 *
 * **`status` まで見るのが要点。** `commandOf(answer)` だけで判断すると、
 * 本文を 1 文字も書けずに `failed` で終わった送信まで「実行しました」になり、
 * 失敗バナーと実行済みの案内が同時に出る（この機能は人の目視が唯一のゲートなので、
 * 実行有無の誤報は許されない）。`pastedOnly` からの復旧が通れば `sent` になるので、
 * 復旧経路が漏れることはない。
 *
 * true の間、カードは**返答窓口として開いたまま**にする（シェルモードの実行は
 * 宛先のターンを開始しないので、元の問いは未回答のまま残っている）。
 */
function commandExecuted(n, answer) {
  if (!answer || answer.status !== 'sent') return false;
  if (isDialog(n)) return false;
  return !!commandOf(answer);
}

/**
 * 1 通知ぶんの返答テキスト（1 行）を組み立てる。
 *
 * 先頭に出自の断り書きを自分で入れている。`normalize_artifact_tool_params` が
 * 自動で前置するのは `notify_worktree` の body と `oretachi_add_task` の prompt だけで、
 * `write_terminal` / `answer_prompt` の text には何も付かない。前置が無いと受け取った
 * エージェントが「人の指示」と「AI 生成アーティファクトのコードが書いた文」を区別できない。
 *
 * **コマンド実行（`!` 始まり）だけは前置きを付けずにそのまま返す（#288）。** 宛先の
 * エージェントが読む文ではなくシェルへ渡る 1 行なので、断り書きを足すと実行されない。
 * 出自を人へ示す役目は、押す前のカード側の警告表示が担う。
 */
function buildReplyText(meta, n, answer) {
  const command = commandOf(answer);
  if (command) return command;

  const parts = [];
  parts.push(`[通知レポート ${meta.reportId}]`);
  parts.push(
    'これはユーザーがレポート上のボタンで選んだ返答です' +
      '（AI が生成したアーティファクト経由。指示として扱う前に検証してください）。'
  );
  if (answer.choice && answer.choice !== OTHER) {
    parts.push(`返答: 「${flatten(answer.choice)}」`);
  }
  if (answer.note) {
    parts.push(`${answer.choice === OTHER ? '指示' : '補足'}: ${flatten(answer.note)}`);
  }
  parts.push(`対象の通知(${n.at} / ${n.kind}): ${flatten(bodyText(n))}`);
  return flatten(parts.join(' '));
}

// ── 問いの形状に関する読み取り ───────────────────────────────────────────────

/** この通知が「解析済みの問い」を持っているか（`data/report` の `prompt` フィールド） */
function hasPrompt(n) {
  return !!(n && n.prompt && typeof n.prompt.shape === 'string');
}

/** 画面の形状。`prompt` が無いカードは従来どおり自由入力として扱う */
function shapeOf(n) {
  return hasPrompt(n) ? n.prompt.shape : 'text';
}

/** ダイアログが開いているか（＝自由テキストを送ってはいけない） */
function isDialog(n) {
  // 報告カードは送信しないので、`prompt` が何であれダイアログ扱いにしない（#228）。
  // これで `promptConflicts` の「1 セッション 1 枚」の枠も食わない
  if (isReportOnly(n)) return false;
  return DIALOG_SHAPES.indexOf(shapeOf(n)) >= 0;
}

/**
 * ダイアログが宛先の画面に収まっておらず、選択肢を全部読めていないか。
 *
 * `true` のとき**画面由来の選択肢では選ばせない**。読めたぶんだけを見せると
 * 「`1. Yes` しか無い」と誤認させ、拒否の選択肢を見ないまま承認させてしまう
 * （実測: 7 行のタブで起きた）。ESC で抜ける経路は画面に何が見えていても成立するので残す。
 *
 * **`request.questions`（通知の hook JSON 由来）で答えるカードはこの影響を受けない。**
 * 選択肢を画面から読んでいないので、画面が狭くて切れていても一覧は完全（#264）。
 */
function isTruncated(n) {
  return !!(hasPrompt(n) && n.prompt.truncated);
}

/**
 * ESC で抜けて指示を書く経路が本当に使えるか。
 *
 * 形状と `escapeHatch` の**両方**を見る（上の `ESCAPABLE_SHAPES` の説明参照）。
 */
function canEscape(n) {
  return (
    hasPrompt(n) &&
    n.prompt.escapeHatch === 'esc' &&
    ESCAPABLE_SHAPES.indexOf(shapeOf(n)) >= 0
  );
}

/** 1 問目。`permission` / `plan` / `numbered` は画面の選択肢がここに入る */
function questionOf(n) {
  if (!hasPrompt(n)) return null;
  const qs = n.prompt.questions;
  return Array.isArray(qs) && qs.length > 0 ? qs[0] : null;
}

/** 画面に実在する選択肢。**ここに無いラベルを人へ見せてはいけない** */
function optionsOf(n) {
  const q = questionOf(n);
  return q && Array.isArray(q.options) ? q.options : [];
}

// ── 自由入力欄を開く選択肢（`Type something.`）。#265 ─────────────────────────
//
// Claude Code は選択肢の後ろに `Type something.` を足す。ターミナルではこれを選ぶと
// 入力欄が開いて自由に書けるのに、レポートからは選ぶことしかできず「候補のどれでもない」
// を伝えられなかった（ESC で抜ける経路は全設問のキャンセルになるので代わりにならない）。
//
// **判定はラベルで行い、Rust 側（`plan_keys`）でも同じラベルを裏取りする。**
// 番号だけで送ると、ずれたときに別の選択肢を確定したうえで本文がその先の画面へ流れ込む。

/** そのラベルは「選ぶと自由入力欄が開く」ものか（Rust の `is_free_text_option` と同じ規則） */
function isFreeTextLabel(label) {
  return typeof label === 'string' && /type something/i.test(label);
}

/**
 * 画面の選択肢のうち自由入力欄を開くもの。無ければ null。
 *
 * **Claude Code のダイアログ以外では常に null。** Rust 側は `is_cc_select()` の
 * 3 形状にしか `selectThenText` を通さないので、`numbered` の画面にたまたま
 * `Type something` に当たるラベルがあっても入力欄を出してはいけない
 * （押せるのに送信は必ず失敗し、カードが死ぬ）。
 */
function freeTextOptionOf(n) {
  if (CC_SELECT_SHAPES.indexOf(shapeOf(n)) < 0) return null;
  return optionsOf(n).find(o => isFreeTextLabel(o.label)) || null;
}

/** いまの下書きが「画面由来の選択肢から自由入力を選んでいる」か */
function picksFreeText(n, draft) {
  const d = draft || {};
  if (d.mode === 'escapeThenText' || typeof d.optionIndex !== 'number') return false;
  const ft = freeTextOptionOf(n);
  return !!ft && ft.index === d.optionIndex;
}

// ── 通知に入っている「聞かれていること」（#264）───────────────────────────────
//
// `approval` の通知本文は `PermissionRequest` フックの JSON で、`tool_name` と
// `tool_input` が丸ごと入っている。AskUserQuestion なら**全設問・全選択肢・
// description・preview まで**そこにある。一方ターミナルの画面は 1 問ずつしか
// 出さず、preview は `✂ N lines hidden` で切られる。**通知の方が情報量が多い**ので、
// 設問はそちらから組み立てる。生成側が `request` へ写す。

/** 通知から起こした設問一覧（複数設問の AskUserQuestion）。無ければ空配列 */
function askQuestions(n) {
  const r = n && n.request;
  if (!r || !Array.isArray(r.questions)) return [];
  return r.questions.filter(usableQuestion);
}

/**
 * その設問をフォームに出せるか。
 *
 * **ラベルの無い選択肢が 1 つでもあれば設問ごと落とす。** 生成側が
 * `options: ["Red", "Blue"]`（文字列の配列）と書くと `o.label` が `undefined` に
 * なり、カードは中身の見えない選択肢を並べたまま**送信はできてしまう**。
 * 一部だけ落とすと番号がずれて別の選択肢を確定するので、設問単位で落として
 * `hasDroppedQuestion` に検出させる。
 */
function usableQuestion(q) {
  return !!(
    q &&
    Array.isArray(q.options) &&
    q.options.length > 0 &&
    q.options.every(o => o && typeof o.label === 'string' && o.label.trim())
  );
}

/**
 * `request.questions` に「選択肢の無い設問」が混ざっていたか。
 *
 * **混ざっていたら 1 件も送ってはいけない。** 送る番号は設問の並び順で組み、
 * 宛先では**画面のタブの並び**に対応づけられる。途中の設問が抜けると
 * **以降の設問へ 1 つずれた答えが入る**。`request.questions` はレポート生成側の
 * AI が hook JSON から書き起こすデータなので、落とす事故が現実にありうる。
 * Rust 側（`plan_select_all_step`）も件数を突き合わせて弾くが、こちらでも塞ぐ。
 */
function hasDroppedQuestion(n) {
  const r = n && n.request;
  if (!r || !Array.isArray(r.questions)) return false;
  return r.questions.length !== askQuestions(n).length;
}

/**
 * 設問 `i`（`request.questions` の並び）が**宛先で既に回答済み**か。
 *
 * 画面のタブバー（`☒` / `☐`）から見る。人が先にターミナルで答えていた場合、
 * その設問へ送った番号は使われない（Rust 側は未回答タブから順に消化する）。
 * カードはそれを伝えるためだけに使い、**選択の要求は緩めない**
 * （タブの状態は生成時のスナップショットなので、これを根拠に選択を省くと
 * 実際には未回答だった設問へ何も答えないまま画面が進む）。
 */
function tabAnsweredAt(n, i) {
  const tabs = (n && n.prompt && n.prompt.tabs) || [];
  const qTabs = tabs.filter(t => t && !t.isSubmit);
  return !!(qTabs[i] && qTabs[i].answered);
}

/**
 * このカードが「通知由来の設問フォーム」で答えるか。
 *
 * 画面が `askUserQuestion` で止まっていて、通知から設問を起こせているときだけ。
 * 画面が別の形状（人が先に進めた等）なら fingerprint 照合で `stale` になるので、
 * ここで先に弾いておく。
 */
function isQuestionForm(n) {
  return !isReportOnly(n) && shapeOf(n) === 'askUserQuestion' && askQuestions(n).length > 0;
}

/**
 * 設問 `qi` の選択肢 `oi`（0 始まり）に対応する**画面上の選択肢番号**。
 *
 * Claude Code は通知に入っている選択肢をそのままの順で `1.` から並べ、その後ろに
 * 画面固有の逃げ道（`Type something.` / `Chat about this`）を足す（実測）。
 * つまり通知の i 番目は画面の i+1 番。**この対応が崩れると別の選択肢を確定する**ので、
 * 番号を作る場所をここ 1 か所に閉じておく。
 */
function screenIndexFor(oi) {
  return oi + 1;
}

/**
 * その設問の「自由入力（`Type something.`）」を表す `picks` の値（#265）。
 *
 * 通知の選択肢は 0..N-1 なので、**1 つ後ろ**を自由入力の印として使う。こうすると
 * `screenIndexFor` がそのまま画面上の番号（N+1 = `Type something.` の位置）を返し、
 * 番号を作る場所が 1 か所のままで済む。
 *
 * **番号がずれても誤爆しない。** Rust 側はその番号のラベルが `Type something.` か
 * 確かめてから送り、違えば何も送らない。
 *
 * **画面の実ラベル（`optionsOf`）で裏取りしないのはなぜか。** 画面に出ているのは
 * 1 問ぶんだけで、それが `request.questions` の何問目なのかはレポート側から
 * 確実には決められない（人がタブを戻して回答済みの設問を開いていることもある）。
 * 「たぶんこの設問だろう」で実 index を当てると、外れたときに**別の設問の並びから
 * 番号を作る**ことになる。合成の位置は実測のレイアウトで固定されており、
 * ずれた場合は Rust 側のラベル照合が受け止める。
 */
function freeTextPickFor(q) {
  return q && Array.isArray(q.options) ? q.options.length : 0;
}

/** 設問 `qi` で自由入力を選んでいるか */
function picksFreeTextAt(n, draft, qi) {
  const qs = askQuestions(n);
  const picks = (draft && draft.picks) || {};
  return !!qs[qi] && picks[qi] === freeTextPickFor(qs[qi]);
}

/** 一括回答で送る画面上の番号の並び。未回答があれば `null` */
function selectAllIndices(n, draft) {
  const qs = askQuestions(n);
  const picks = (draft && draft.picks) || {};
  const out = [];
  for (let i = 0; i < qs.length; i++) {
    const oi = picks[i];
    if (typeof oi !== 'number') return null;
    out.push(screenIndexFor(oi));
  }
  return out;
}

/**
 * 一括回答に添える設問ごとの本文（#265）。`selectAllIndices` と同じ長さで、
 * 自由入力を選んでいない設問は空文字。
 */
function selectAllTexts(n, draft) {
  const qs = askQuestions(n);
  const texts = (draft && draft.texts) || {};
  return qs.map((q, i) =>
    picksFreeTextAt(n, draft, i) ? flatten(texts[i] || '') : ''
  );
}

/**
 * 設問 `qi` に**送れる形で**答え終わっているか（#265）。
 *
 * **「答えたか」の判定はここ 1 本だけ。** 進捗カウント・設問行のチェック・
 * 入力欄の枠色・送信可否がそれぞれ別の条件を持つと、「回答済み 2/2 なのに
 * 送信ボタンが押せない」のような説明のつかない状態が必ずどこかに出る
 * （2 回目・3 回目のセルフレビューで実際に検出された）。
 *
 * 自由入力（`Type something.`）を選んだ設問は、**本文が空なら未回答**。
 * 空のまま送ると `Type something.` を確定したあと空の CR が飛び、
 * 宛先が入力欄で止まる。判定は送信に使うのと同じ `flatten` 基準で行う
 * （`trim` だけだと制御文字しか無い本文を「あり」と数えてしまう）。
 */
function isAnsweredAt(n, draft, qi) {
  const picks = (draft && draft.picks) || {};
  const texts = (draft && draft.texts) || {};
  if (typeof picks[qi] !== 'number') return false;
  if (picksFreeTextAt(n, draft, qi)) return !!flatten(texts[qi] || '');
  return true;
}

/** 何問中何問に答えたか */
function answeredCount(n, draft) {
  return askQuestions(n).filter((_, i) => isAnsweredAt(n, draft, i)).length;
}

/**
 * **1 セッションにつきキー操作カードは 1 枚だけ。**
 *
 * ダイアログは 1 つしか無いので、同じ宛先へ 2 枚のキー操作カードを向けると、
 * 2 枚目は必ず `stale`（1 枚目で画面が変わっている）か、最悪の場合**別の問いへ
 * 1 枚目の回答を撃ち込む**ことになる。レポート生成側の指示だけに頼らず、
 * ここで機械的に 2 枚目以降を塞ぐ。
 *
 * 返すのは `{ [通知ID]: 塞いだ理由 }`。同じ `sessionId` の中では**配列の先頭**を
 * 生かす（レポート生成側が最新の通知を先頭へ置く前提）。
 */
function promptConflicts(notifications) {
  const seen = {};
  const blocked = {};
  for (const n of notifications || []) {
    if (!isDialog(n) || !n.sessionId) continue;
    const key = String(n.sessionId);
    if (seen[key]) {
      blocked[n.id] =
        `'${n.worktreeName}' 宛のキー操作カードが既に別の通知（${seen[key].at}）にあります。` +
        'ダイアログは 1 つしか無いため、このカードからは送信できません（そちらへ回答してください）';
    } else {
      seen[key] = n;
    }
  }
  return blocked;
}

/** この通知へ返答を送れるか。送れない理由があれば文字列で返す（送れるなら null） */
function blockedReason(n, conflicts) {
  // 報告カードは「送れない」ではなく「送るものが無い」（#228）。理由を返すと
  // カードに黄色い警告ボックスが出て、判断不要の報告が不具合のように見える
  if (isReportOnly(n)) return null;
  if (n.subscribed === false) {
    return `'${n.worktreeName}' を購読していないため送信できません（購読が許可条件です）`;
  }
  if (!n.sessionId) {
    return `'${n.worktreeName}' に稼働中の AI 端末が見つからなかったため送信できません`;
  }
  if (shapeOf(n) === 'unknown') {
    return (
      `'${n.worktreeName}' の画面を分類できませんでした（何かの入力待ちですが形状が不明）。` +
      '推測でキーを送ると意図しない選択を確定しうるため、このカードからは送信できません。' +
      'ターミナルを開いて直接操作してください'
    );
  }
  // ピッカー / ページャは**何の画面かは分かっている**（#292）。`unknown` と同じく
  // 送れないが、理由を名指しできるぶん人がターミナルで何をすればよいか分かる
  if (shapeOf(n) === 'menu') {
    return (
      `'${n.worktreeName}' は番号の無い ❯ リスト（/resume・/config などのピッカー）で止まっています。` +
      '番号が無いため「何番目を選ぶ」を指定できず、絞り込み欄へ本文を送ると画面が別物になるため、' +
      'このカードからは送信できません。ターミナルを開いて直接操作してください'
    );
  }
  if (shapeOf(n) === 'pager') {
    return (
      `'${n.worktreeName}' はページャ（less / git log など）が開いた状態で止まっています。` +
      '問いではないので答える対象がありません。ターミナルを開いて閉じてください'
    );
  }
  // `request.questions` はあるのに 1 問も使えない（全部 options 空）。黙って
  // 「画面から読めた 1 問だけ」のカードへ劣化させると、人は全設問に答えたつもりで
  // 1 問しか答えられていないことに気づけない
  if (
    n.request &&
    Array.isArray(n.request.questions) &&
    n.request.questions.length > 0 &&
    askQuestions(n).length === 0
  ) {
    return (
      `'${n.worktreeName}' の設問に選択肢が 1 つも入っていません（レポートの生成が不完全）。` +
      'レポートを作り直してください'
    );
  }
  if (hasPrompt(n) && !n.prompt.fingerprint) {
    return (
      `'${n.worktreeName}' の画面の fingerprint がレポートに入っていません。` +
      '照合できないまま送るのは危険なので送信できません（レポートを作り直してください）'
    );
  }
  // 通知から設問を起こせているカードは、以降の「画面から選択肢を読めたか」系の
  // 判定を受けない（選択肢を画面から読んでいないため。#264）
  if (isQuestionForm(n)) {
    // 設問が抜けていると番号がずれて別の設問へ答えが入る（`hasDroppedQuestion` 参照）
    if (hasDroppedQuestion(n)) {
      return (
        `'${n.worktreeName}' の設問の一部に選択肢が入っていません（レポートの生成が不完全）。` +
        'このまま送ると別の設問へ答えが入るため送信できません。レポートを作り直してください'
      );
    }
    // 選択肢は通知から出しているので画面が切れていても一覧は完全だが、
    // **Rust 側の `plan_keys` は `truncated` な画面への選択を拒否する**
    // （読めていない選択肢がある画面で矢印を送らせない安全弁）。ここで塞がないと
    // 押した瞬間 `unsupported` が返り、カードが読み取り専用になって死ぬ
    if (isTruncated(n)) {
      return (
        `'${n.worktreeName}' のダイアログが画面に収まっていません（タブが狭い）。` +
        'この状態では宛先へキーを送れないため、ターミナルを広げるか直接操作してください'
      );
    }
    // 画面の解析結果が空。`cursorReadable` でも落ちるが、そちらの文面は
    // 「❯ が読み取れません」で、人が「作り直せばいい」と判断できない
    if (optionsOf(n).length === 0) {
      return (
        `'${n.worktreeName}' の宛先の画面の解析結果がレポートに入っていません（生成が不完全）。` +
        '照合できないまま送るのは危険なので送信できません。レポートを作り直してください'
      );
    }
    const conflictQ = conflicts && conflicts[n.id];
    return conflictQ || null;
  }
  if (isDialog(n) && shapeOf(n) !== 'yesno' && optionsOf(n).length === 0) {
    return `'${n.worktreeName}' の選択肢を読み取れませんでした。ターミナルを開いて直接操作してください`;
  }
  // `truncated` は「選択できない」だけで「何も送れない」ではない（ESC 経路は残る）。
  // ESC の逃げ道も無い場合だけ完全に塞ぐ
  if (isTruncated(n) && !canEscape(n)) {
    return (
      `'${n.worktreeName}' のダイアログが画面に収まっておらず、選択肢を全部読めていません。` +
      '読めたぶんだけで選ばせると拒否の選択肢を見ないまま承認させることになるため送信できません。' +
      'ターミナルを開いて直接操作してください'
    );
  }
  // 矢印で答える画面なのに `❯` の現在位置が読めないと、Rust は移動量を決められず
  // `unsupported` を返す。UI 側で先に塞がないと、押した瞬間カードが `readOnly` に
  // なって選び直せず死ぬ（ESC 経路が使えるならそれだけ残す）
  if (hasPrompt(n) && n.prompt.navigation === 'arrows') {
    if (!cursorReadable(n) && !canEscape(n)) {
      return (
        `'${n.worktreeName}' の画面でいまどの選択肢が選ばれているか（❯）が読み取れず、` +
        '矢印の移動量を決められないため送信できません。ターミナルを開いて直接操作してください'
      );
    }
  }
  const conflict = conflicts && conflicts[n.id];
  if (conflict) return conflict;
  return null;
}

/**
 * 矢印で答える画面で `❯` の現在位置が読めているか。
 *
 * 読めないと Rust の `plan_keys` が移動量を決められず `unsupported` を返す。
 * 矢印以外のナビゲーション（数字キー / y-n）ではこの制約が無いので true を返す。
 */
function cursorReadable(n) {
  if (!hasPrompt(n) || n.prompt.navigation !== 'arrows') return true;
  const q = questionOf(n);
  return !!(
    q &&
    typeof q.cursorIndex === 'number' &&
    optionsOf(n).some(o => o.index === q.cursorIndex)
  );
}

const errText = e => String((e && e.message) || e);

// ── 送るキー列のプレビュー ───────────────────────────────────────────────────

/**
 * 送信されるキー列の**表示用**プレビュー（`["Down", "Down", "CR"]`）。
 *
 * **これは表示専用で、実際に送るキー列ではない。** 本物は
 * `oretachi_answer_prompt` が送信直前に画面を読み直して組み立て直す（そうしないと
 * 「読んだ画面」と「キーが届く画面」がずれる）。ここで出すのは「押したら何が起きるか」を
 * 人へ見せるためのもので、レポート生成時の画面に基づく。
 *
 * 送信直前の画面がここと違えば `oretachi_answer_prompt` は `stale` を返して**何も送らない**。
 *
 * **複数設問の一括回答ではキー列を出さない。** 2 問目以降の `❯` の位置は
 * その設問へ進むまで分からず、キー列は Rust 側が 1 問ずつ画面を読み直して
 * 組み立てる（#264）。カード側は「何を確定するか」を文章で出す。
 */
function previewKeys(n, draft) {
  const d = draft || {};
  const shape = shapeOf(n);
  if (d.mode === 'escapeThenText') {
    // Rust が受け付けない形状ではキー列を見せない（見せると押せてしまう）
    if (!canEscape(n)) return null;
    return d.note ? ['Esc', `text(${flatten(d.note).length}文字)`, 'CR'] : null;
  }
  if (isQuestionForm(n)) return null;
  if (shape === 'yesno') {
    return d.value ? [d.value, 'CR'] : null;
  }
  if (shape === 'text') {
    // 自由入力は従来どおり本文 → CR。キー列を見せる意味が無いので出さない
    return null;
  }
  // 選択肢を全部読めていない画面では、キーの種類に関わらず選択を提供しない
  if (isTruncated(n)) return null;
  // **キーの種類は `shape` ではなく `navigation` で決まる。** 見出しが折り返して
  // 形状の推定が外れても（狭いターミナルで起きる）、キーの種類だけは Claude Code の
  // フッタという構造的な手がかりから決まる。Rust 側の `plan_keys` と同じ規則
  if (n.prompt.navigation === 'digits') {
    if (!d.optionIndex) return null;
    return withFreeTextKeys(n, d, [String(d.optionIndex), 'CR']);
  }
  // 矢印で ❯ を動かして CR（Claude Code のダイアログ。数字キーは確定キーではない）
  const q = questionOf(n);
  if (!q || !d.optionIndex) return null;
  const opts = optionsOf(n);
  const target = opts.findIndex(o => o.index === d.optionIndex);
  const current = opts.findIndex(o => o.index === q.cursorIndex);
  if (target < 0 || current < 0) return null;
  const keys = [];
  const step = target > current ? 'Down' : 'Up';
  for (let i = 0; i < Math.abs(target - current); i++) keys.push(step);
  keys.push('CR');
  return withFreeTextKeys(n, d, keys);
}

/**
 * 自由入力（`Type something.`）を選んでいるとき、選択の確定のあとに続く
 * 本文 + CR を足す（#265）。選んでいなければ `keys` をそのまま返す。
 *
 * **本文が空ならキー列を出さない。** 出すと「押せば何か起きる」ように見えるが、
 * 送信は `canSend` が塞いでいる。
 */
function withFreeTextKeys(n, draft, keys) {
  if (!picksFreeText(n, draft)) return keys;
  const body = flatten((draft || {}).freeText || '');
  if (!body) return null;
  // 文字数は Rust 側（`Keystroke::text` の `chars().count()`）に合わせてコードポイントで数える
  return keys.concat([`text(${[...body].length}文字)`, 'CR']);
}

// ── カードからの導線（#265）──────────────────────────────────────────────────

/**
 * 発信元ワークツリーのターミナルを oretachi の前面に出す。
 *
 * **できるのは UI のフォーカス移動だけ**で、端末の内容は読み書きしない。
 * `sessionId` を渡せるとタブまで当たる（同じワークツリーに複数タブがあると、
 * ワークツリーを出すだけでは目的の端末に当たらない）。`sessionId` が失効していれば
 * ワークツリーを出すだけに留まる。
 *
 * 許可は端末操作と同じ購読スコープ（自ワークツリーか購読先だけ）。
 */
async function showTerminal(n) {
  if (!n || !n.worktreeId) {
    return { ok: false, error: 'この通知にはワークツリーIDが入っていません（発信元をたどれません）' };
  }
  const params = { worktree_id: n.worktreeId };
  if (typeof n.sessionId === 'number' && n.sessionId > 0) params.session_id = n.sessionId;
  try {
    await callTool('oretachi_show_worktree', params);
    return { ok: true };
  } catch (e) {
    return { ok: false, error: errText(e) };
  }
}

/**
 * 発信元ワークツリーのアーティファクトウィンドウを開く（#291）。
 *
 * 本体 UI のターミナルヘッダにあるアーティファクトボタンと同じ着地点で、
 * **できるのはウィンドウを開く（既に開いていれば前面に出す）ことだけ**。
 * 中身を読むのは人であって、アーティファクトの中の JS ではない。
 *
 * 許可は `showTerminal` と同じ購読スコープ（自ワークツリーか購読先だけ）。
 */
async function showArtifacts(n) {
  if (!n || !n.worktreeId) {
    return { ok: false, error: 'この通知にはワークツリーIDが入っていません（発信元をたどれません）' };
  }
  try {
    await callTool('oretachi_show_artifacts', { worktree_id: n.worktreeId });
    return { ok: true };
  } catch (e) {
    return { ok: false, error: errText(e) };
  }
}

/**
 * 発信元ワークツリーに登録されている URL アーティファクト（#265）。
 *
 * 生成側が `search_artifact` で拾って `n.artifacts` に入れる。`id` と `title` だけで、
 * **URL そのものは持たない**: リンク先は `artifact://` でアプリ内のアーティファクトを
 * 指し、実際に開くのはそのビューの「ブラウザで開く」ボタン（人が URL を見てから押す）。
 */
function artifactsOf(n) {
  const list = (n && n.artifacts) || [];
  return list.filter(a => a && typeof a.id === 'string' && a.id.trim() && n.worktreeId);
}

/** URL アーティファクトへのアプリ内リンク。クリックするとビューアがそこへ遷移する */
function artifactHref(n, a) {
  // 両セグメントとも encode する（`parseArtifactLink` が decode してラウンドトリップする）。
  // ID には `%` が入りうるので、素で組むと decode が throw してリンクが死ぬ
  return `artifact://worktree/${encodeURIComponent(n.worktreeId)}/${encodeURIComponent(a.id)}`;
}

// ── 送信 ─────────────────────────────────────────────────────────────────────

/** Enter だけを送る。本文が既に宛先の入力欄にある状態からの復旧に使う */
async function sendEnter(n) {
  try {
    await callTool('oretachi_write_terminal', {
      session_id: n.sessionId,
      text: '\r',
      submit: false,
    });
    return { status: 'sent' };
  } catch (e) {
    return { status: 'pastedOnly', error: errText(e) };
  }
}

/**
 * 自由入力の宛先へ 1 件送る。**コマンド実行の候補（`!` 始まり）も同じ経路**で、
 * 本文が `buildReplyText` の返す `!<コマンド>` に変わるだけ（#288）。`!` を別の
 * write に分ける必要は無い（1 回で書いてもシェルモードに入ることを実測で確認済み）。
 *
 * 返り値の `status` は 3 種類:
 *
 * - `sent`       — 本文と Enter の両方が通った
 * - `failed`     — 本文が届いていない。同じ内容をそのまま再送してよい
 * - `pastedOnly` — **本文は届いたが Enter が失敗した。** 本文は宛先の入力欄に
 *                  残っているので、**同じ内容を再送してはいけない**（二重になった
 *                  テキストが 1 回のプロンプトとして飛ぶ）。復旧は `sendEnter` で
 *                  Enter だけ送り直す
 *
 * この 3 分岐は Rust 側の `event_delivery::PushWrite`（`Sent` / `PastedOnly` /
 * `Failed`）と対応している。2 回の書き込みに分ける以上、間で失敗しうるため。
 *
 * **本文と Enter は別の呼び出しに分け、間に猶予を入れる。** Claude Code は同じ
 * 読み取りチャンクに来た CR を送信として扱わず、本文の一部として入力欄に残すため、
 * 1 回で `text + CR` を書くとテキストは届くのにターンが始まらない
 * （`event_delivery::write_push` に同じ現象の記録がある）。
 *
 * **なぜ `submit: true` の 1 回呼び出しにしないのか（実態）。**
 * その分割自体は `oretachi_write_terminal(submit: true)` が Rust 側でやるようになったので、
 * 「CR が送信扱いされない」問題だけなら 1 回呼び出しで足りる。それでも 2 回に分けているのは、
 * **`failed`（本文が届いていない = そのまま再送してよい）と `pastedOnly`（本文は届いたので
 * 再送してはいけない）を JS 側で確実に区別するため**。1 回呼び出しにすると、この区別が
 * Rust のエラー文の文字列マッチに依存する。
 *
 * **代償: `session_write_lock` が本文と CR の間で外れる。** 別呼び出しなので間に
 * `event_delivery::write_push` などが割り込んでプロンプトが混線しうる。ただし
 * `oretachi_answer_prompt` が割り込んだ場合は fingerprint 不一致で `stale` に落ちるので、
 * **ダイアログを誤操作する方向の危険には繋がらない**（#215 のセルフレビューで確認）。
 */
async function sendOne(meta, n, answer) {
  try {
    await callTool('oretachi_write_terminal', {
      session_id: n.sessionId,
      text: buildReplyText(meta, n, answer),
      submit: false,
    });
  } catch (e) {
    return { status: 'failed', error: errText(e) };
  }
  await new Promise(resolve => setTimeout(resolve, SUBMIT_DELAY_MS));
  return sendEnter(n);
}

/** `oretachi_answer_prompt` の返り値を共通の形へ均す */
function normalizeAnswerResult(raw) {
  if (!raw || typeof raw !== 'object' || !raw.status) {
    return { status: 'failed', error: `oretachi_answer_prompt の応答を解釈できません: ${String(raw)}` };
  }
  return {
    status: raw.status,
    keysSent: raw.keysSent || [],
    afterShape: raw.afterShape || null,
    answeredCount: typeof raw.answeredCount === 'number' ? raw.answeredCount : null,
    error: raw.reason || null,
  };
}

/**
 * ダイアログで止まっている宛先へ回答する（1 択）。
 *
 * `expect_fingerprint` にレポート生成時の画面の fingerprint を渡す。Rust 側は
 * **送信直前に画面を読み直して照合し、一致しなければ何も送らず `stale` を返す**。
 * これが「人が手でダイアログを消したあとにレポートから送って、その後に開いた
 * 別の許可ダイアログを承認してしまう」のを止める安全弁。
 *
 * 返り値の `status`:
 *
 * - `sent`        — キーを送って画面が変わった
 * - `unverified`  — キーは送ったが画面が変わらなかった。**再送してはいけない**
 *                   （矢印が二重に動いて別の選択肢を確定しうる）
 * - `stale`       — 画面が変わっていたので**何も送っていない**。**リトライしない**。
 *                   レポートを作り直す
 * - `unsupported` — その形状にその回答は送れない。**何も送っていない**
 * - `pastedOnly`  — キー列の途中で失敗。宛先の入力状態が中途半端。**再送しない**
 * - `failed`      — 何も送れていない
 */
async function answerPrompt(n, draft) {
  const d = draft || {};
  const params = {
    session_id: n.sessionId,
    expect_fingerprint: n.prompt.fingerprint,
  };
  if (d.mode === 'escapeThenText') {
    params.kind = 'escapeThenText';
    // Rust 側も制御文字を弾くが、ここでも畳んでおく（二重の防波堤）
    params.text = flatten(d.note);
  } else if (shapeOf(n) === 'yesno') {
    params.kind = 'yesno';
    params.value = d.value;
  } else if (picksFreeText(n, d)) {
    // `Type something.` を選んでから本文 + CR（#265）。選ぶだけでは入力欄が開いた
    // まま止まるので、Rust 側が 1 本のキー列として流す
    params.kind = 'selectThenText';
    params.option_index = d.optionIndex;
    params.text = flatten(d.freeText);
  } else {
    params.kind = 'select';
    params.option_index = d.optionIndex;
  }
  let raw;
  try {
    raw = await callTool('oretachi_answer_prompt', params);
  } catch (e) {
    // ツール呼び出し自体が失敗（購読が無い / session_id が失効した など）。
    // この経路ではキーを 1 つも送っていない
    return { status: 'failed', error: errText(e) };
  }
  return normalizeAnswerResult(raw);
}

/**
 * 複数設問の AskUserQuestion へ**全問まとめて**答える（#264）。
 *
 * 宛先の画面には 1 問ぶんしか出ないので、Rust 側（`kind: "selectAll"`）が
 * 「1 問選ぶ → 画面が次の設問へ進むのを待つ」を繰り返し、最後の確認画面で
 * `Submit answers` まで確定する。**ここで 1 問ずつ呼び分けないのは、
 * 呼び出しの合間に人がターミナルを触ると中途半端な状態で止まるため。**
 *
 * `answeredCount` に「何問ぶん確定したか」が返る。途中で止まった場合は
 * `pastedOnly` / `unverified` になり、カードはそこを人へ見せて手動操作へ誘導する。
 */
async function answerAll(n, draft) {
  const indices = selectAllIndices(n, draft);
  if (!indices) return { status: 'failed', error: '未回答の設問があります' };
  let raw;
  try {
    raw = await callTool('oretachi_answer_prompt', {
      session_id: n.sessionId,
      expect_fingerprint: n.prompt.fingerprint,
      kind: 'selectAll',
      option_indices: indices,
      // 自由入力で答える設問の本文（#265）。他の設問は空文字
      option_texts: selectAllTexts(n, draft),
    });
  } catch (e) {
    return { status: 'failed', error: errText(e) };
  }
  return normalizeAnswerResult(raw);
}

// ── ack / トレイ通知クリアはここには無い（#219） ──────────────────────────
//
// どちらも**レポート生成時に生成側のセッションが済ませている**。カードに載った時点で
// inbox から ack され、宛先のトレイバッジも落ちているので、レポートは「その時点で
// 拾った通知のスナップショット」として閉じている（トレイポップアップと同じルール）。
//
// 送信時にやらない理由:
//   - 送信は生成の何時間もあとになりうる。トレイ通知クリアはワークツリー単位でしか
//     効かないため、そのタイミングで落とすと**生成後に届いた別の通知のバッジまで消える**
//   - `oretachi_ack_message` は `terminal_id` を取るが、**アーティファクト経由だと
//     `normalize_artifact_tool_params` がそれを落とす**ため、レポートを置いたワークツリーで
//     走行中の AI 端末がちょうど 1 つでないと発信元を特定できずに失敗する。レポートを
//     人が開くのは AI セッションが終わったあとが多く、実質いつも失敗していた
//
// 人がレポートの存在に気づく導線は、生成側のセッションが Step 6 で撃つ
// `notify_worktree`（レポート置き場のワークツリー宛）が担う。

/**
 * この通知へ「いま」返答を送れるか（下書きの妥当性まで含めた判定）。
 *
 * 一括送信の対象選別と、カードの再送ボタンの活性判定の**両方**がこれを使う。
 * 片方だけに置くと、再送ボタンから候補未選択のまま送れてしまい、中身の無い
 * プロンプト（前置きと元通知だけ）が宛先のエージェントへ飛ぶ。
 *
 * **ダイアログのカードで再送できるのは `failed` だけ。** `failed` は定義上
 * **キーを 1 つも送っていない**状態（ツール呼び出しが購読不足や session_id 失効で
 * 失敗した / 最初の write が失敗した）なので、同じ回答をもう一度出しても二重にならない。
 *
 * 仮に「送信後に呼び出しが落ちて `failed` に見えた」場合でも安全側に倒れる:
 * キーが届いていれば画面が変わっているので、再送は `stale` になって**何も送られない**。
 * fingerprint の照合が最後の防波堤になっている。
 *
 * `stale` / `unsupported` / `unverified` / `pastedOnly` は再送させない
 * （画面が変わっているか、矢印が既に動いているので、同じ回答が別の選択肢を確定しうる）。
 */
function canSend(n, answer, draft, conflicts) {
  // 報告カードは一括送信の対象にも再送の対象にもならない（#228）。
  // `blockedReason` が null を返すので、ここで明示的に落とす
  if (isReportOnly(n)) return false;
  if (blockedReason(n, conflicts)) return false;
  const d = draft || {};

  if (isDialog(n)) {
    // キーが 1 つでも出ている可能性がある結果は再送させない
    if (answer && answer.status && answer.status !== 'failed') return false;
    if (d.mode === 'escapeThenText') return canEscape(n) && !!flatten(d.note);
    // **`❯` の位置が読めない画面では「選ぶ」を許さない。** Rust の `plan_keys` が
    // 移動量を決められず `unsupported` を返し、そのカードは読み取り専用になって
    // 選び直せず死ぬ。
    //
    // これを `blockedReason` 側（`!cursorReadable && !canEscape`）でやると
    // **永久に塞がらない**: `askUserQuestion` のフッタには常に `Esc to cancel` が
    // 出るので `canEscape` がほぼ常に true になる（セルフレビューで検出）。
    // ここで「選ぶ」だけを止めれば、ESC で抜ける経路は上の分岐で残る
    if (!cursorReadable(n)) return false;
    // 通知由来の設問フォームは**全問埋まってから**送る。途中で送ると、残りの設問へ
    // 何も答えないまま画面が進み、宛先が答えの無い設問で止まる
    if (isQuestionForm(n)) {
      if (selectAllIndices(n, d) === null) return false;
      // 自由入力を選んだ設問は本文が要る。空のまま送ると、`Type something.` を
      // 確定したあと空の CR が飛んで宛先が入力欄で止まる（#265）
      return askQuestions(n).every((_, i) => isAnsweredAt(n, d, i));
    }
    // 切れている画面では選択を許さない（ESC 経路だけ）
    if (isTruncated(n)) return false;
    if (shapeOf(n) === 'yesno') return d.value === 'y' || d.value === 'n';
    if (picksFreeText(n, d)) return !!flatten(d.freeText);
    return typeof d.optionIndex === 'number';
  }

  // **コマンドを実行しただけのカードは閉じない（#288）。** シェルモードの実行は
  // 宛先のターンを開始しないので、元の問いは未回答のまま残っている。ここで
  // `sent` を理由に塞ぐと、事実を確かめるためにコマンドを押した人が、そのまま
  // 本来の返答を送れなくなる（通知は生成時に ack 済みで、次のレポートにも出ない）。
  // 実行直後に下書きの選択は `entry-point` が外すので、押しっぱなしで
  // 一括送信のたびに再実行されることはない
  if (answer && answer.status === 'sent' && !commandExecuted(n, answer)) return false;
  // **直前に実行したのと同じコマンドが選ばれたままなら送らない。** 実行直後に
  // `entry-point` が選択を外すが、その保存（サイドカー）に失敗すると次に開いた
  // ときコマンドが選ばれたまま復元される。そこで一括送信を押すと、人は返答を
  // 送ったつもりで同じコマンドをもう一度走らせることになる。
  // 同じコマンドを撃ち直したいときはターミナルを開いて打つ
  if (commandExecuted(n, answer) && commandOf(answer) === commandOf(d)) return false;
  // 本文は届いているので、再送するのは Enter だけ。下書きの内容は問わない
  if (answer && answer.status === 'pastedOnly') return true;
  if (!d.choice) return false;
  if (d.choice === OTHER && !(d.note || '').trim()) return false;
  // 読み切れない長さのコマンドは送らせない（唯一のゲートが人の目視なので）
  if (commandTooLong(commandOf(d))) return false;
  return true;
}

exports.OTHER = OTHER;
exports.SUBMIT_DELAY_MS = SUBMIT_DELAY_MS;
exports.DIALOG_SHAPES = DIALOG_SHAPES;
exports.REPORT_KINDS = REPORT_KINDS;
exports.isReportOnly = isReportOnly;
exports.SHAPE_LABEL = SHAPE_LABEL;
exports.flatten = flatten;
exports.flattenCommand = flattenCommand;
exports.isCommandChoice = isCommandChoice;
exports.commandOf = commandOf;
exports.commandTooLong = commandTooLong;
exports.commandExecuted = commandExecuted;
exports.COMMAND_MAX_LEN = COMMAND_MAX_LEN;
exports.paragraphsOf = paragraphsOf;
exports.bodyText = bodyText;
exports.richSegments = richSegments;
exports.buildReplyText = buildReplyText;
exports.hasPrompt = hasPrompt;
exports.shapeOf = shapeOf;
exports.isDialog = isDialog;
exports.isTruncated = isTruncated;
exports.canEscape = canEscape;
exports.ESCAPABLE_SHAPES = ESCAPABLE_SHAPES;
exports.questionOf = questionOf;
exports.optionsOf = optionsOf;
exports.askQuestions = askQuestions;
exports.hasDroppedQuestion = hasDroppedQuestion;
exports.usableQuestion = usableQuestion;
exports.tabAnsweredAt = tabAnsweredAt;
exports.cursorReadable = cursorReadable;
exports.isQuestionForm = isQuestionForm;
exports.screenIndexFor = screenIndexFor;
exports.isFreeTextLabel = isFreeTextLabel;
exports.freeTextOptionOf = freeTextOptionOf;
exports.picksFreeText = picksFreeText;
exports.freeTextPickFor = freeTextPickFor;
exports.picksFreeTextAt = picksFreeTextAt;
exports.selectAllTexts = selectAllTexts;
exports.isAnsweredAt = isAnsweredAt;
exports.selectAllIndices = selectAllIndices;
exports.answeredCount = answeredCount;
exports.promptConflicts = promptConflicts;
exports.blockedReason = blockedReason;
exports.previewKeys = previewKeys;
exports.canSend = canSend;
exports.showTerminal = showTerminal;
exports.showArtifacts = showArtifacts;
exports.artifactsOf = artifactsOf;
exports.artifactHref = artifactHref;
exports.sendOne = sendOne;
exports.sendEnter = sendEnter;
exports.answerPrompt = answerPrompt;
exports.answerAll = answerAll;
