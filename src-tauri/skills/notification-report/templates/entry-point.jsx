
const { useState, useCallback, useMemo } = React;
const { useMemory } = require('oretachi');

const { META, NOTIFICATIONS } = require('./data/report');
const NotificationCard = require('./components/NotificationCard').default;
const { Badge } = require('./components/NotificationCard');
const {
  blockedReason,
  canSend,
  sendOne,
  sendEnter,
  answerPrompt,
  isDialog,
  promptConflicts,
} = require('./lib/send');

const FONT = 'system-ui, sans-serif';
const MONO = 'ui-monospace, SFMono-Regular, Menlo, monospace';

function nowLabel() {
  const d = new Date();
  const p = v => String(v).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

// 基準時刻の表示。生成したセッションには時刻が渡っていないため、整形は
// ここ（ブラウザ側）でやる。`generatedAtMs` は載せた通知のうち最大の
// `createdAt`（epoch ms）で、`oretachi_poll_inbox` の返り値からそのまま取れる。
// `generatedAt`（文字列）が入っているレポートは旧形式なのでそちらを使う。
function generatedLabel(meta) {
  // Number() を通すのは、生成側が epoch ms を文字列で入れても 1970 年や
  // 「(時刻不明)」へ黙って落ちないようにするため。`0` / 負値 / NaN は
  // 「入っていない」と同じ扱いにする（1970-01-01 を出すより無害）
  const ms = Number(meta.generatedAtMs);
  if (Number.isFinite(ms) && ms > 0) {
    const d = new Date(ms);
    const p = v => String(v).padStart(2, '0');
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }
  return meta.generatedAt || '(時刻不明)';
}

function App() {
  // ── サイドカー（artifact_store）に置く状態 ─────────────────────────────
  //
  // 本体 JSON ではなくサイドカーに置くのは、送信後に「返答済み」へ書き換える操作が
  // 本体だと自分自身の locked_while_open に弾かれるため。ビューア由来の書き込みは
  // ロックの対象外なので、サイドカーなら開いたまま更新できる。
  //
  // ack（inbox）とトレイ通知のクリアはここには無い。**レポート生成時に生成側の
  // セッションが済ませている**ので、このレポートは「その時点で拾った通知」だけを
  // 抱えた閉じたスナップショットになっている（#219）。
  const [answers, setAnswers] = useMemory('answers', {});   // 送信済みの記録
  const [drafts, setDrafts] = useMemory('drafts', {});      // 選択と補足の下書き

  // ── ローカル state（永続不要な進行状況） ──────────────────────────────
  const [busy, setBusy] = useState(false);
  const [inflightId, setInflightId] = useState(null);

  const setDraft = useCallback((id, patch) => {
    setDrafts(prev => ({ ...prev, [id]: { ...(prev[id] || {}), ...patch } }))
      .catch(() => {});
  }, [setDrafts]);

  // **同じ宛先へキー操作カードを 2 枚向けない**（#215）。ダイアログは 1 つしか無いので、
  // 2 枚目は必ず stale になるか、最悪の場合 1 枚目の回答を別の問いへ撃ち込む。
  // レポート生成側の指示だけに頼らず、ここで機械的に 2 枚目以降を塞ぐ
  const conflicts = useMemo(() => promptConflicts(NOTIFICATIONS), []);
  const blockedFor = useCallback(n => blockedReason(n, conflicts), [conflicts]);

  // 送信対象の判定は `lib/send` の canSend に寄せてある。一括送信の選別・
  // 再送ボタンの活性・送信ループのガードが同じ判定を使うようにするため
  const sendable = NOTIFICATIONS.filter(n => canSend(n, answers[n.id], drafts[n.id], conflicts));

  const pending = NOTIFICATIONS.filter(n => {
    const rec = answers[n.id];
    return !rec || rec.status !== 'sent';
  });

  // 1 件ずつ順に送る。宛先ごとに別々のツール呼び出しになる（宛先の AI 端末が
  // 別々なのでまとめられない）。記録は 1 件ごとにサイドカーへ書くので、途中で
  // 閉じても「どこまで届いたか」は残る。
  //
  // **同じ session_id へ複数向くことがあっても直列。** ダイアログ経路は 1 件ごとに
  // Rust 側で fingerprint 照合が走るので、後続は自動的に stale になる（撃ち込まれない）。
  const send = useCallback(async (targets) => {
    if (busy || targets.length === 0) return;
    setBusy(true);
    try {
      for (const n of targets) {
        const prev = answers[n.id];
        const d = drafts[n.id] || {};
        // 一括送信のボタンは sendable で絞ってあるが、カードの再送ボタンは
        // 1 件を直接渡してくる。ここで弾かないと、候補を解除したまま再送して
        // 中身の無いプロンプトを送れてしまう
        if (!canSend(n, prev, d, conflicts)) continue;
        setInflightId(n.id);

        let result;
        let rec;
        if (isDialog(n)) {
          // ダイアログ経路。**キー列は Rust 側が送信直前の画面から組み立て直す。**
          // ここで組み立てて渡すと「読んだ画面」と「キーが届く画面」がずれる
          result = await answerPrompt(n, d);
          rec = {
            mode: d.mode || 'select',
            optionIndex: typeof d.optionIndex === 'number' ? d.optionIndex : null,
            value: d.value || null,
            note: (d.note || '').trim(),
            status: result.status,
            keysSent: result.keysSent || [],
            afterShape: result.afterShape || null,
            at: nowLabel(),
          };
        } else {
          // 自由入力経路。本文だけ届いている状態からの復旧は Enter の送り直し。
          // 同じ本文をもう一度書くと二重になったテキストが 1 回のプロンプトとして飛ぶ
          const resume = prev && prev.status === 'pastedOnly';
          result = resume ? await sendEnter(n) : await sendOne(META, n, d);
          rec = resume
            ? { ...prev, status: result.status, at: nowLabel() }
            : { choice: d.choice, note: (d.note || '').trim(), status: result.status, at: nowLabel() };
        }
        if (result.error) rec.error = result.error;
        else delete rec.error;

        // **1 件ごとに待って保存する。** サイドカーの保存は 400ms の debounce +
        // IPC 往復なので、N 件送ると N×(400ms + 往復) が上乗せされる。それでも
        // 待つのは、ここで落ちても「どこまで届いたか」を残すため。特に
        // `pastedOnly` / `unverified` は記録が無いまま閉じると、次に開いた人が
        // 同じ回答を送って二重に入力してしまう。速度より取り違えの防止を採る
        try {
          await setAnswers(p => ({ ...p, [n.id]: rec }));
        } catch (e) {
          // サイドカーへ書けなくても送信自体は済んでいる。表示だけが古くなる
          console.warn('返答状態の保存に失敗しました', e);
        }
      }
    } finally {
      setInflightId(null);
      setBusy(false);
    }
  }, [busy, answers, drafts, setAnswers, conflicts]);

  const sentCount = NOTIFICATIONS.filter(n => (answers[n.id] || {}).status === 'sent').length;
  const failedCount = NOTIFICATIONS.filter(n => {
    const s = (answers[n.id] || {}).status;
    return s === 'failed' || s === 'stale' || s === 'unsupported';
  }).length;
  const dialogCount = NOTIFICATIONS.filter(isDialog).length;

  return (
    <div style={{
      minHeight: '100vh', background: '#11111b', color: '#cdd6f4', fontFamily: FONT,
      display: 'flex', flexDirection: 'column',
    }}>
      {/* ヘッダー */}
      <div style={{
        position: 'sticky', top: 0, zIndex: 10,
        background: '#1e1e2e', borderBottom: '1px solid #313244',
        padding: '14px 24px', display: 'flex', alignItems: 'center', gap: 14, flexWrap: 'wrap',
      }}>
        <span style={{ fontSize: 15, fontWeight: 700 }}>
          {/* CUSTOMIZE: タイトルは data/report の META に合わせる */}
          通知レポート — {generatedLabel(META)}
        </span>
        <span style={{ fontSize: 12, color: '#9399b2', fontWeight: 600 }}>
          未返答 {pending.length} / 全 {NOTIFICATIONS.length} 件
        </span>
        {dialogCount > 0 && <Badge label={`ダイアログ待ち ${dialogCount}`} color="#f38ba8" />}
        {sentCount > 0 && <Badge label={`返答済み ${sentCount}`} color="#a6e3a1" />}
        {failedCount > 0 && <Badge label={`未送信 ${failedCount}`} color="#f38ba8" />}
        <div style={{ flex: 1 }} />
        <button
          type="button"
          disabled={busy || sendable.length === 0}
          onClick={() => send(sendable)}
          style={{
            border: 'none', borderRadius: 6, padding: '9px 18px',
            background: (busy || sendable.length === 0) ? '#313244' : '#89b4fa',
            color: (busy || sendable.length === 0) ? '#6c7086' : '#11111b',
            fontSize: 13, fontWeight: 700, fontFamily: FONT,
            cursor: (busy || sendable.length === 0) ? 'default' : 'pointer',
          }}
        >
          {busy ? '送信中…' : `選択した ${sendable.length} 件へ送信`}
        </button>
      </div>

      <div style={{ padding: '20px 24px', display: 'flex', flexDirection: 'column', gap: 14 }}>
        {/* スナップショットであることの明示。開いている間に届いた通知は次のレポートへ回る */}
        <div style={{
          fontSize: 11.5, color: '#cba6f7',
          background: '#cba6f712', border: '1px solid #cba6f744', borderRadius: 6,
          padding: '9px 12px', lineHeight: 1.7,
        }}>
          このレポートは {generatedLabel(META)} 時点のスナップショットです。ここに載っている通知は
          <b>生成時に既読化（ack）とトレイ通知のクリアを済ませる運用</b>で、成功していれば以後のレポートには
          出てきません（このレポートが返答窓口になります）。既読化に失敗したぶんは次のレポートに再掲されます
          — 生成したセッションの報告を確認してください。生成後に届いた通知は含まれず、次のレポートに回ります。
          各カードの現況要約も生成時にターミナルを読んだ内容で、以後は更新されません。
        </div>

        {/* ダイアログ操作の注意。何が送られるのかを人が理解した上で押させる */}
        {dialogCount > 0 && (
          <div style={{
            fontSize: 11.5, color: '#f38ba8',
            background: '#f38ba812', border: '1px solid #f38ba844', borderRadius: 6,
            padding: '9px 12px', lineHeight: 1.7,
          }}>
            <b>ダイアログで止まっている宛先が {dialogCount} 件あります。</b>
            これらのカードは自由テキストではなく<b>キー操作</b>で回答します（テキストを送るとダイアログに吸われ、
            末尾の Enter が意図しない選択肢の確定として解釈されるため）。選択肢は<b>宛先の画面に実在するものだけ</b>を
            出しており、送信前にキー列をプレビューできます。送信直前に画面が変わっていた場合は
            <b>何も送らず「画面が変わった」と表示</b>されます（そのときは AI にレポートの作り直しを頼んでください。
            既読化済みの通知も拾い直せるようになっています）。
          </div>
        )}

        {NOTIFICATIONS.length === 0 && (
          <div style={{ fontSize: 13, color: '#6c7086', padding: '24px 0', textAlign: 'center' }}>
            返答が必要な通知はありませんでした。
          </div>
        )}

        {NOTIFICATIONS.map(n => (
          <NotificationCard
            key={n.id}
            n={n}
            answer={answers[n.id] || null}
            draft={drafts[n.id]}
            blocked={blockedFor(n)}
            inflight={inflightId === n.id}
            busy={busy}
            canSend={canSend(n, answers[n.id], drafts[n.id], conflicts)}
            onPick={c => setDraft(n.id, { choice: c })}
            onNote={v => setDraft(n.id, { note: v })}
            onDraft={patch => setDraft(n.id, patch)}
            onRetry={() => send([n])}
          />
        ))}

        {/* 宛先の内訳。session_id は生成時に埋め込んだ値なので、失効したら作り直す */}
        <div style={{
          fontSize: 11, color: '#6c7086', fontFamily: FONT,
          borderTop: '1px dashed #313244', paddingTop: 12, lineHeight: 1.9,
        }}>
          <div>
            返答は各ワークツリーの AI 端末へ直接送られます
            （レポートの置き場所 <b>{META.callerWorktree}</b> がその宛先を購読していることが許可条件）。
          </div>
          <div style={{ fontFamily: MONO, fontSize: 10.5, color: '#585b70' }}>
            {NOTIFICATIONS.map(n => `${n.worktreeName}:session ${n.sessionId || '—'}`).join('  /  ')}
          </div>
          <div>
            session_id は生成時に埋め込んだ値です。アプリ再起動やタブ再作成で失効するので、
            送信がエラーになったら AI にレポートの作り直しを頼んでください（既読化済みの通知も拾い直せます）。
          </div>
        </div>
      </div>
    </div>
  );
}

exports.default = App;
