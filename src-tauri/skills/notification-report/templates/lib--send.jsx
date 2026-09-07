
// レポートからの送信ロジック。
//
// 宛先は各通知の発信元ワークツリーで走っている AI 端末で、許可条件は
// **レポートを置いたワークツリーがその宛先ワークツリーを購読していること**（#211）。
// 向きは購読者側が呼び出し元で、宛先側が呼び出し元を購読しているだけでは通らない。
const { callTool } = require('oretachi');

// 候補で言い表せないときの選択肢。選ぶと補足プロンプトが必須になる
const OTHER = 'その他（補足で指示）';

/** 本文を送ってから Enter を送るまでの猶予（ミリ秒）。`sendOne` のコメント参照 */
const SUBMIT_DELAY_MS = 150;

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
 */
function flatten(s) {
  return String(s == null ? '' : s)
    .replace(/\r?\n/g, ' / ')
    // eslint-disable-next-line no-control-regex
    .replace(/[\x00-\x1f\x7f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/**
 * 1 通知ぶんの返答テキスト（1 行）を組み立てる。
 *
 * 先頭に出自の断り書きを自分で入れている。`normalize_artifact_tool_params` が
 * 自動で前置するのは `notify_worktree` の body と `oretachi_add_task` の prompt だけで、
 * `write_terminal` の text には何も付かない。前置が無いと受け取ったエージェントが
 * 「人の指示」と「AI 生成アーティファクトのコードが書いた文」を区別できない。
 */
function buildReplyText(meta, n, answer) {
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
  parts.push(`対象の通知(${n.at} / ${n.kind}): ${flatten(n.body)}`);
  return flatten(parts.join(' '));
}

/** この通知へ返答を送れるか。送れない理由があれば文字列で返す（送れるなら null） */
function blockedReason(n) {
  if (n.subscribed === false) {
    return `'${n.worktreeName}' を購読していないため送信できません（購読が許可条件です）`;
  }
  if (!n.sessionId) {
    return `'${n.worktreeName}' に稼働中の AI 端末が見つからなかったため送信できません`;
  }
  return null;
}

/**
 * 1 件送る。成功なら `{ status: 'sent' }`、失敗なら `{ status: 'failed', error }`。
 *
 * **本文と Enter は別の呼び出しに分け、間に猶予を入れる。** Claude Code は同じ
 * 読み取りチャンクに来た CR を送信として扱わず、本文の一部として入力欄に残すため、
 * 1 回で `text + CR` を書くとテキストは届くのにターンが始まらない
 * （`event_delivery::write_push` に同じ現象の記録がある）。
 *
 * `submit: false` で本文だけを書き、独立した呼び出しで CR だけを送る形にしている。
 * `submit: true` 側も同じ分割をするよう直したが、こちらで分けておけば古い oretachi
 * でも正しく送信でき、ツールの submit 実装に依存しない。
 */
async function sendOne(meta, n, answer) {
  const blocked = blockedReason(n);
  if (blocked) return { status: 'failed', error: blocked };
  try {
    await callTool('oretachi_write_terminal', {
      session_id: n.sessionId,
      text: buildReplyText(meta, n, answer),
      submit: false,
    });
    await new Promise(resolve => setTimeout(resolve, SUBMIT_DELAY_MS));
    await callTool('oretachi_write_terminal', {
      session_id: n.sessionId,
      text: '\r',
      submit: false,
    });
    return { status: 'sent' };
  } catch (e) {
    return { status: 'failed', error: String((e && e.message) || e) };
  }
}

/**
 * 返答した通知を既読化する。
 *
 * `oretachi_ack_message` は `terminal_id` を取らないので、レポートを置いた
 * ワークツリーで**走行中の AI 端末がちょうど 1 つ**でないと失敗する。
 * AI セッション終了後にユーザーがレポートを触る場合は失敗するので、
 * 失敗は許容して表示だけ出す（返答自体は届いている）。
 */
async function ackInbox(inboxIds) {
  const ids = (inboxIds || []).filter(Boolean);
  if (ids.length === 0) return { state: 'skipped' };
  try {
    await callTool('oretachi_ack_message', { ids });
    return { state: 'ok', count: ids.length };
  } catch (e) {
    return { state: 'failed', error: String((e && e.message) || e) };
  }
}

exports.OTHER = OTHER;
exports.SUBMIT_DELAY_MS = SUBMIT_DELAY_MS;
exports.flatten = flatten;
exports.buildReplyText = buildReplyText;
exports.blockedReason = blockedReason;
exports.sendOne = sendOne;
exports.ackInbox = ackInbox;
