
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
  ackInbox,
  clearNotifications,
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

function App() {
  // ── サイドカー（artifact_store）に置く状態 ─────────────────────────────
  //
  // 本体 JSON ではなくサイドカーに置くのは、送信後に「返答済み」へ書き換える操作が
  // 本体だと自分自身の locked_while_open に弾かれるため。ビューア由来の書き込みは
  // ロックの対象外なので、サイドカーなら開いたまま更新できる。
  const [answers, setAnswers] = useMemory('answers', {});   // 送信済みの記録
  const [drafts, setDrafts] = useMemory('drafts', {});      // 選択と補足の下書き
  const [ack, setAck] = useMemory('ack', null);             // ack の結果
  const [cleared, setCleared] = useMemory('cleared', null); // トレイ通知クリアの結果

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
    const acked = [];
    // 返答が届いた宛先ワークツリー。ack（inbox）とは別ストアのトレイバッジを
    // 落とすのに使う（#218）
    const sentWorktreeIds = [];
    // この送信ループ後の各カードの status。トレイバッジはワークツリー粒度でしか
    // 落とせないので、**そのワークツリーのカードが全部 sent になったか**をここで見る。
    // `answers` はクロージャに閉じ込まれた送信前の値なので使えない
    const statusById = {};
    for (const x of NOTIFICATIONS) statusById[x.id] = (answers[x.id] || {}).status || null;
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
        statusById[n.id] = result.status;
        if (result.status === 'sent') {
          acked.push(...(n.inboxIds || []));
          if (n.worktreeId) sentWorktreeIds.push(n.worktreeId);
        }
      }
      // 1 件も送れていないときは ack を触らない。触ると直前の
      // 「N 件既読化しました」/「ack 不可」の表示が skipped で消える
      if (acked.length > 0) {
        const outcome = await ackInbox(acked);
        try {
          await setAck({ ...outcome, at: nowLabel() });
        } catch (e) {
          console.warn('ack 結果の保存に失敗しました', e);
        }
      }
      // 返答が届いた宛先のトレイバッジを落とす（#218）。ack は inbox（sqlite）で
      // バッジはフロントの別ストアなので、両方やらないと捌き終わったワークツリーが
      // トレイポップアップの巡回に残り続ける。ack と違い AI セッションの稼働は不要。
      //
      // **落とせるのはワークツリー単位**（`NotificationRegistry` に通知単位の粒度が無い）。
      // なので「そのワークツリーのカードが全部 sent」になった宛先だけに絞る。絞らないと、
      // まだ未返答のカードが残っているワークツリーのバッジまで消えて、人が気づく導線が
      // 失われる（`promptConflicts` で塞がれた 2 枚目など）。
      // レポート生成後に届いた通知はこの粒度では区別できず一緒に落ちるが、それは
      // トレイポップアップを1件送りしても同じ（離脱時にワークツリーごと既読になる）。
      const clearable = sentWorktreeIds.filter(wid => {
        const cards = NOTIFICATIONS.filter(x => x.worktreeId === wid);
        return cards.length > 0 && cards.every(x => statusById[x.id] === 'sent');
      });
      if (clearable.length > 0) {
        const outcome = await clearNotifications(clearable);
        try {
          await setCleared({ ...outcome, at: nowLabel() });
        } catch (e) {
          console.warn('通知クリア結果の保存に失敗しました', e);
        }
      }
    } finally {
      setInflightId(null);
      setBusy(false);
    }
  }, [busy, answers, drafts, setAnswers, setAck, setCleared, conflicts]);

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
          通知レポート — {META.generatedAt}
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
          このレポートは {META.generatedAt} 時点のスナップショットです。開いている間に届いた通知は含まれず、
          次のレポートに回ります。各カードの現況要約も生成時にターミナルを読んだ内容で、以後は更新されません。
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
            <b>何も送らず「画面が変わった」と表示</b>されます（そのときはレポートを作り直してください）。
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

        {/* トレイバッジのクリア結果。失敗してもバッジが残るだけで返答は届いている */}
        {cleared && cleared.ok && cleared.ok.length > 0 && (
          <div style={{ fontSize: 11.5, color: '#a6e3a1' }}>
            {cleared.ok.length} 件のワークツリーのトレイ通知をクリアしました（{cleared.at}）
          </div>
        )}
        {cleared && cleared.failed && Object.keys(cleared.failed).length > 0 && (
          <div style={{
            fontSize: 12, color: '#fab387',
            background: '#fab38714', border: '1px solid #fab38744', borderRadius: 6,
            padding: '9px 12px', lineHeight: 1.8,
          }}>
            <b>トレイ通知のクリアに失敗（{cleared.at}）</b> — 返答は届いていますが、
            トレイバッジが残るため同じワークツリーがトレイポップアップの巡回に出続けます。
            <div style={{ marginTop: 4, color: '#9399b2', fontFamily: MONO, fontSize: 11 }}>
              {Object.entries(cleared.failed).map(([id, err]) => `${id}: ${err}`).join(' / ')}
            </div>
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
