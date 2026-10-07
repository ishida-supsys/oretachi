
const { useState, useCallback, useMemo } = React;
const { useMemory } = require('oretachi');

const { META, PROPOSALS } = require('./data/proposals');
const ProposalCard = require('./components/ProposalCard').default;
const { submit, deliver, missingDeps } = require('./lib/submit');

const FONT = 'system-ui, sans-serif';

const SIZE_ORDER = { S: 0, M: 1, L: 2 };
const EFFECT_ORDER = { high: 0, mid: 1, low: 2 };
const RISK_ORDER = { low: 0, mid: 1, high: 2 };
// 不明な値は最後尾へ。`risk` は任意項目なので、無ければ「中」として扱う（有利にも不利にもしない）
const rank = (table, v) => (v in table ? table[v] : 99);
const riskRank = v => (v == null ? RISK_ORDER.mid : rank(RISK_ORDER, v));

// 推奨度: サイズが小さく・効果が大きく・リスクが低いほど良い。
// 3 つの順位（0 が最良）の合計が小さいほど上。同点は効果の高い方、次にサイズの小さい方を先にする
const cost = p => rank(SIZE_ORDER, p.size) + rank(EFFECT_ORDER, p.effect) + riskRank(p.risk);

const SORTS = {
  effect: (a, b) => rank(EFFECT_ORDER, a.effect) - rank(EFFECT_ORDER, b.effect) || rank(SIZE_ORDER, a.size) - rank(SIZE_ORDER, b.size),
  sizeDesc: (a, b) => rank(SIZE_ORDER, b.size) - rank(SIZE_ORDER, a.size) || rank(EFFECT_ORDER, a.effect) - rank(EFFECT_ORDER, b.effect),
  recommend: (a, b) => cost(a) - cost(b) || rank(EFFECT_ORDER, a.effect) - rank(EFFECT_ORDER, b.effect) || rank(SIZE_ORDER, a.size) - rank(SIZE_ORDER, b.size),
};

const DELIVERY_LABEL = {
  sent: ['#a6e3a1', 'セッションへ合図を送りました。セッション側で issue 化の確認が始まります'],
  notReady: ['#f9e2af', '合図を送れませんでした（セッションが入力待ちではありません）'],
  failed: ['#f38ba8', '合図を送れませんでした（セッションが見つかりません）'],
  pastedOnly: ['#f38ba8', '合図の本文は入力欄に届きましたが Enter が失敗しました。セッションで Enter を押してください（再送はしないこと）'],
};

function stamp() {
  const d = new Date();
  const p = v => String(v).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

function App() {
  // サイドカー（artifact_store）に置く。本体 JSON は locked_while_open で送信後に書き換えられない。
  // `submission` のキーがそのまま生成側の `artifact_store(read)` の `data.submission` になる
  const [selection, setSelection] = useMemory('selection', {});
  const [submission, setSubmission] = useMemory('submission', null);
  const [delivery, setDelivery] = useMemory('delivery', null);

  const [sizeFilter, setSizeFilter] = useState('all');
  const [effectFilter, setEffectFilter] = useState('all');
  const [sortKey, setSortKey] = useState('sizeDesc');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);

  const proposals = useMemo(() => (Array.isArray(PROPOSALS) ? PROPOSALS.filter(p => p && typeof p.id === 'string') : []), []);
  const submitted = !!submission;

  const visible = useMemo(() => {
    const list = proposals.filter(p =>
      (sizeFilter === 'all' || p.size === sizeFilter) && (effectFilter === 'all' || p.effect === effectFilter));
    // 既定はサイズの大→小（同サイズなら効果の高い順）。`sort` は安定なので同順位の並びは元の順を保つ
    return list.slice().sort(SORTS[sortKey] || SORTS.effect);
  }, [proposals, sizeFilter, effectFilter, sortKey]);

  const chosen = proposals.filter(p => selection[p.id]);
  const warnings = useMemo(() => {
    const m = {};
    for (const w of missingDeps(proposals, selection)) m[w.id] = w;
    return m;
  }, [proposals, selection]);

  const toggle = useCallback(id => {
    setSelection(prev => ({ ...prev, [id]: !prev[id] }))
      .catch(e => console.warn('選択状態の保存に失敗しました', e));
  }, [setSelection]);

  const recordDelivery = useCallback(async d => {
    const rec = { status: d.status, at: stamp(), reason: d.reason || null, error: d.error || null };
    try { await setDelivery(rec); } catch (e) { console.warn('配送結果の保存に失敗しました', e); }
  }, [setDelivery]);

  const onSubmit = useCallback(async () => {
    if (busy || chosen.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      const r = await submit({ meta: META, ids: chosen.map(p => p.id), persist: s => setSubmission(s) });
      if (!r.ok) { setError(r.error); return; }
      await recordDelivery(r.delivery);
    } finally {
      setBusy(false);
    }
  }, [busy, chosen, setSubmission, recordDelivery]);

  // 再送は端末への送信だけをやり直す（submission は保存済み）。
  // `pastedOnly` は本文が入力欄に残っているので再送させない
  const onResend = useCallback(async () => {
    if (busy || !submission) return;
    setBusy(true);
    try { await recordDelivery(await deliver(META, submission)); } finally { setBusy(false); }
  }, [busy, submission, recordDelivery]);

  const onCancel = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await setSubmission(null);
      await setDelivery(null);
    } catch (e) {
      setError(`送信の取り消しを保存できませんでした: ${String((e && e.message) || e)}`);
    } finally {
      setBusy(false);
    }
  }, [busy, setSubmission, setDelivery]);

  const dl = delivery && DELIVERY_LABEL[delivery.status];
  const canResend = submitted && delivery && (delivery.status === 'failed' || delivery.status === 'notReady');
  const btn = (enabled, primary) => ({
    border: 'none', borderRadius: 6, padding: '9px 18px', fontSize: 13, fontWeight: 700, fontFamily: FONT,
    background: enabled ? (primary ? '#89b4fa' : '#45475a') : '#313244',
    color: enabled ? (primary ? '#11111b' : '#cdd6f4') : '#6c7086',
    cursor: enabled ? 'pointer' : 'default',
  });
  const sel = { background: '#313244', color: '#cdd6f4', border: 'none', borderRadius: 4, padding: '4px 8px', fontFamily: FONT, fontSize: 12 };

  return (
    <div style={{ minHeight: '100vh', background: '#11111b', color: '#cdd6f4', fontFamily: FONT }}>
      <div style={{
        position: 'sticky', top: 0, zIndex: 10, background: '#1e1e2e', borderBottom: '1px solid #313244',
        padding: '14px 24px', display: 'flex', alignItems: 'center', gap: 14, flexWrap: 'wrap',
      }}>
        <span style={{ fontSize: 15, fontWeight: 700 }}>開発企画 — {String(META.theme || '')}</span>
        <span style={{ fontSize: 12, color: '#9399b2' }}>
          {String(META.generatedAt || '')} / {proposals.length} 件 / 採用 {chosen.length} 件
        </span>
        <label style={{ fontSize: 12, color: '#9399b2' }}>サイズ{' '}
          <select value={sizeFilter} onChange={e => setSizeFilter(e.target.value)} style={sel}>
            <option value="all">すべて</option><option value="L">大</option><option value="M">中</option><option value="S">小</option>
          </select>
        </label>
        <label style={{ fontSize: 12, color: '#9399b2' }}>効果{' '}
          <select value={effectFilter} onChange={e => setEffectFilter(e.target.value)} style={sel}>
            <option value="all">すべて</option><option value="high">高</option><option value="mid">中</option><option value="low">低</option>
          </select>
        </label>
        <label style={{ fontSize: 12, color: '#9399b2' }}>並べ替え{' '}
          <select value={sortKey} onChange={e => setSortKey(e.target.value)} style={sel}>
            <option value="effect">効果 高→低</option>
            <option value="sizeDesc">サイズ 大→小</option>
            <option value="recommend">推奨度順（小・高効果・低リスク）</option>
          </select>
        </label>
        <div style={{ flex: 1 }} />
        {submitted
          ? <button type="button" disabled={busy} onClick={onCancel} style={btn(!busy, false)}>送信を取り消す</button>
          : <button type="button" disabled={busy || chosen.length === 0} onClick={onSubmit} style={btn(!busy && chosen.length > 0, true)}>
            {busy ? '送信中…' : `採用した ${chosen.length} 件を送信`}
          </button>}
      </div>

      <div style={{ padding: '20px 24px', display: 'flex', flexDirection: 'column', gap: 12 }}>
        {error && (
          <div style={{ fontSize: 12.5, color: '#f38ba8', background: '#f38ba812', border: '1px solid #f38ba844', borderRadius: 6, padding: '9px 12px' }}>
            {error}
          </div>
        )}
        {submitted && (
          <div style={{
            fontSize: 12.5, lineHeight: 1.7, color: dl ? dl[0] : '#9399b2',
            background: '#181825', border: '1px solid #313244', borderRadius: 6, padding: '9px 12px',
          }}>
            <b>送信済み</b>（{String(submission.submittedAt)}）— 採用 {submission.ids.length} 件。
            {dl && <> {dl[1]}{delivery.at ? `（${delivery.at}）` : ''}。</>}
            {!delivery && <> 配送結果は不明です（保存に失敗したか、送信直後にビューアを閉じた可能性があります）。</>}
            {delivery && delivery.reason && <> {String(delivery.reason)}。</>}
            {delivery && delivery.error && <> エラー: {String(delivery.error)}</>}
            {canResend && (
              <button type="button" disabled={busy} onClick={onResend} style={{ ...btn(!busy, true), marginLeft: 10, padding: '4px 12px' }}>
                再送
              </button>
            )}
            {(!delivery || delivery.status === 'failed') && (
              <div style={{ marginTop: 4, color: '#9399b2' }}>
                届かないときは、生成元のセッションに「dev-proposal の送信を確認して」と入力してください。
              </div>
            )}
          </div>
        )}
        {proposals.length === 0 && (
          <div style={{ fontSize: 13, color: '#6c7086', padding: '24px 0', textAlign: 'center' }}>企画がありません。</div>
        )}
        {visible.map(p => (
          <ProposalCard
            key={p.id} p={p} repoUrl={META.repoUrl}
            checked={!!selection[p.id] || (submitted && submission.ids.indexOf(p.id) >= 0)}
            readOnly={submitted} warning={warnings[p.id]} onToggle={toggle} />
        ))}
      </div>
    </div>
  );
}

exports.default = App;
