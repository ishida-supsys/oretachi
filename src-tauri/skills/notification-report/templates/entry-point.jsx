
const { useState, useCallback } = React;
const { useMemory } = require('oretachi');

const { META, NOTIFICATIONS } = require('./data/report');
const NotificationCard = require('./components/NotificationCard').default;
const { Badge } = require('./components/NotificationCard');
const { OTHER, blockedReason, sendOne, ackInbox } = require('./lib/send');

const FONT = 'system-ui, sans-serif';
const MONO = 'ui-monospace, SFMono-Regular, Menlo, monospace';

function nowLabel() {
  const d = new Date();
  const p = v => String(v).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

function App() {
  // ── サイドカー（artifact_store）に置く状態 ─────────────────────────────
  //
  // 本体 JSON ではなくサイドカーに置くのは、送信後に「返答済み」へ書き換える操作が
  // 本体だと自分自身の locked_while_open に弾かれるため。ビューア由来の書き込みは
  // ロックの対象外なので、サイドカーなら開いたまま更新できる。
  const [answers, setAnswers] = useMemory('answers', {});   // 送信済みの記録
  const [drafts, setDrafts] = useMemory('drafts', {});      // 選択と補足の下書き
  const [ack, setAck] = useMemory('ack', null);             // ack の結果

  // ── ローカル state（永続不要な進行状況） ──────────────────────────────
  const [busy, setBusy] = useState(false);
  const [inflightId, setInflightId] = useState(null);

  const setDraft = useCallback((id, patch) => {
    setDrafts(prev => ({ ...prev, [id]: { ...(prev[id] || {}), ...patch } }))
      .catch(() => {});
  }, [setDrafts]);

  // 送信対象の判定: 未送信または送信失敗で、候補が選ばれていて、
  // 「その他」なら補足が入っていて、購読と端末の条件も満たしているもの
  const sendable = NOTIFICATIONS.filter(n => {
    const rec = answers[n.id];
    if (rec && rec.status === 'sent') return false;
    if (blockedReason(n)) return false;
    const d = drafts[n.id] || {};
    if (!d.choice) return false;
    if (d.choice === OTHER && !(d.note || '').trim()) return false;
    return true;
  });

  const pending = NOTIFICATIONS.filter(n => {
    const rec = answers[n.id];
    return !rec || rec.status !== 'sent';
  });

  // 1 件ずつ順に送る。宛先ごとに write_terminal を呼ぶ（宛先の AI 端末が別々なので
  // まとめられない）。記録は 1 件ごとにサイドカーへ書くので、途中で閉じても
  // 「どこまで届いたか」は残る
  const send = useCallback(async (targets) => {
    if (busy || targets.length === 0) return;
    setBusy(true);
    const acked = [];
    try {
      for (const n of targets) {
        setInflightId(n.id);
        const d = drafts[n.id] || {};
        const result = await sendOne(META, n, d);
        const rec = {
          choice: d.choice,
          note: (d.note || '').trim(),
          status: result.status,
          at: nowLabel(),
        };
        if (result.error) rec.error = result.error;
        try {
          await setAnswers(prev => ({ ...prev, [n.id]: rec }));
        } catch (e) {
          // サイドカーへ書けなくても送信自体は済んでいる。表示だけが古くなる
          console.warn('返答状態の保存に失敗しました', e);
        }
        if (result.status === 'sent') acked.push(...(n.inboxIds || []));
      }
      const outcome = await ackInbox(acked);
      try {
        await setAck({ ...outcome, at: nowLabel() });
      } catch (e) {
        console.warn('ack 結果の保存に失敗しました', e);
      }
    } finally {
      setInflightId(null);
      setBusy(false);
    }
  }, [busy, drafts, setAnswers, setAck]);

  const sentCount = NOTIFICATIONS.filter(n => (answers[n.id] || {}).status === 'sent').length;
  const failedCount = NOTIFICATIONS.filter(n => (answers[n.id] || {}).status === 'failed').length;

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
          通知レポート — {META.generatedAt}
        </span>
        <span style={{ fontSize: 12, color: '#9399b2', fontWeight: 600 }}>
          未返答 {pending.length} / 全 {NOTIFICATIONS.length} 件
        </span>
        {sentCount > 0 && <Badge label={`返答済み ${sentCount}`} color="#a6e3a1" />}
        {failedCount > 0 && <Badge label={`失敗 ${failedCount}`} color="#f38ba8" />}
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
          このレポートは {META.generatedAt} 時点のスナップショットです。開いている間に届いた通知は含まれず、
          次のレポートに回ります。各カードの現況要約も生成時にターミナルを読んだ内容で、以後は更新されません。
        </div>

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
            blocked={blockedReason(n)}
            inflight={inflightId === n.id}
            busy={busy}
            onPick={c => setDraft(n.id, { choice: c })}
            onNote={v => setDraft(n.id, { note: v })}
            onRetry={() => send([n])}
          />
        ))}

        {/* ack の結果。走行中の AI 端末がちょうど 1 つでないと失敗する */}
        {ack && ack.state === 'failed' && (
          <div style={{
            fontSize: 12, color: '#fab387',
            background: '#fab38714', border: '1px solid #fab38744', borderRadius: 6,
            padding: '9px 12px', lineHeight: 1.8,
          }}>
            <b>ack 不可（{ack.at}）</b> — 返答は届いていますが、通知の既読化に失敗しました。
            未 ack のまま残るため、次のセッション開始時に同じ通知が再掲されます。
            <div style={{ marginTop: 4, color: '#9399b2', fontFamily: MONO, fontSize: 11 }}>{ack.error}</div>
          </div>
        )}
        {ack && ack.state === 'ok' && (
          <div style={{ fontSize: 11.5, color: '#a6e3a1' }}>
            {ack.count} 件の通知を既読化しました（{ack.at}）
          </div>
        )}

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
            送信がエラーになったらレポートを作り直してください。
          </div>
        </div>
      </div>
    </div>
  );
}

exports.default = App;
