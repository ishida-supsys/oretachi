
// 1 案のカード。データは防御的に読む（値の誤りで一覧全体が描画不能にならないように）。
const { useState } = React;

const FONT = 'system-ui, sans-serif';
const MONO = 'ui-monospace, SFMono-Regular, Menlo, monospace';

const SIZE = {
  L: { label: '大', color: '#f38ba8' },
  M: { label: '中', color: '#f9e2af' },
  S: { label: '小', color: '#a6e3a1' },
};
const EFFECT = {
  high: { label: '高', color: '#a6e3a1' },
  mid: { label: '中', color: '#f9e2af' },
  low: { label: '低', color: '#9399b2' },
};
// リスクは高いほど警戒色
const RISK = {
  high: { label: '高', color: '#f38ba8' },
  mid: { label: '中', color: '#f9e2af' },
  low: { label: '低', color: '#a6e3a1' },
};

/** 文字列だけの配列にする（文字列以外の要素は React が描画できず throw するので落とす） */
function strings(v) {
  return Array.isArray(v) ? v.filter(s => typeof s === 'string' && s.trim()).map(s => s.trim()) : [];
}

function Badge({ prefix, table, value }) {
  const t = table[value] || { label: '不明', color: '#6c7086' };
  return (
    <span style={{
      fontSize: 11, fontWeight: 700, color: t.color, background: `${t.color}1a`,
      border: `1px solid ${t.color}55`, borderRadius: 4, padding: '2px 7px', whiteSpace: 'nowrap',
    }}>
      {prefix} {t.label}
    </span>
  );
}

function Section({ title, items, mono }) {
  if (items.length === 0) return null;
  return (
    <div style={{ marginTop: 10 }}>
      <div style={{ fontSize: 11, fontWeight: 700, color: '#9399b2', marginBottom: 3 }}>{title}</div>
      <ul style={{ margin: 0, paddingLeft: 18, fontSize: 12.5, lineHeight: 1.7, fontFamily: mono ? MONO : FONT }}>
        {items.map((s, i) => <li key={i}>{s}</li>)}
      </ul>
    </div>
  );
}

/** `repoUrl` が https のときだけ issue リンクにする（当てずっぽうのリンクは張らない） */
function issueLinks(p, repoUrl) {
  const nums = (Array.isArray(p.relatedIssues) ? p.relatedIssues : []).filter(n => Number.isInteger(n) && n > 0);
  if (nums.length === 0) return null;
  const base = typeof repoUrl === 'string' && /^https:\/\//.test(repoUrl) ? repoUrl.replace(/\/+$/, '') : null;
  return (
    <div style={{ marginTop: 10, fontSize: 12.5 }}>
      <span style={{ fontSize: 11, fontWeight: 700, color: '#9399b2' }}>関連する既存 issue: </span>
      {nums.map(n => base
        ? <a key={n} href={`${base}/issues/${n}`} style={{ color: '#89b4fa', marginRight: 8 }}>#{n}</a>
        : <code key={n} style={{ marginRight: 8 }}>#{n}</code>)}
    </div>
  );
}

function ProposalCard({ p, repoUrl, checked, readOnly, warning, onToggle }) {
  const [open, setOpen] = useState(false);
  const evidence = (Array.isArray(p.evidence) ? p.evidence : [])
    .filter(e => e && typeof e.path === 'string')
    .map(e => `${e.path}${Number.isInteger(e.line) ? `:${e.line}` : ''}${typeof e.note === 'string' && e.note ? ` — ${e.note}` : ''}`);
  const deps = strings(p.dependsOn);

  return (
    <div style={{
      background: '#1e1e2e', border: `1px solid ${checked ? '#89b4fa' : '#313244'}`, borderRadius: 8,
      padding: '14px 16px', fontFamily: FONT, opacity: readOnly && !checked ? 0.55 : 1,
    }}>
      <label style={{ display: 'flex', gap: 10, alignItems: 'flex-start', cursor: readOnly ? 'default' : 'pointer' }}>
        <input
          type="checkbox" checked={!!checked} disabled={readOnly}
          onChange={() => onToggle(p.id)} style={{ marginTop: 4 }} />
        <div style={{ flex: 1 }}>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
            <span style={{ fontSize: 14.5, fontWeight: 700 }}>{String(p.title || '(無題)')}</span>
            <Badge prefix="サイズ" table={SIZE} value={p.size} />
            <Badge prefix="効果" table={EFFECT} value={p.effect} />
            {p.risk != null && <Badge prefix="リスク" table={RISK} value={p.risk} />}
          </div>
          <div style={{ fontSize: 13, lineHeight: 1.7, marginTop: 6, color: '#bac2de' }}>
            {String(p.summary || '')}
          </div>
        </div>
      </label>

      {warning && (
        <div style={{
          marginTop: 8, fontSize: 12, color: '#f9e2af', background: '#f9e2af12',
          border: '1px solid #f9e2af44', borderRadius: 6, padding: '6px 10px',
        }}>
          依存先 {warning.missing.join(', ')} が採用されていません（ブロックはしません）
        </div>
      )}

      <button
        type="button" onClick={() => setOpen(o => !o)}
        style={{
          marginTop: 8, background: 'none', border: 'none', color: '#89b4fa',
          cursor: 'pointer', fontSize: 12, padding: 0, fontFamily: FONT,
        }}>
        {open ? '▼ 詳細を閉じる' : `▶ 詳細（根拠 ${evidence.length} 件）`}
      </button>
      {open && (
        <div>
          <Section title="根拠" items={evidence} mono />
          <Section title="対象範囲" items={strings(p.scope)} />
          <Section title="やらないこと" items={strings(p.outOfScope)} />
          <Section title="完了条件" items={strings(p.doneWhen)} />
          <Section title="推奨分割（sub-issue 候補）" items={strings(p.tasks)} />
          <Section title="依存する案" items={deps} />
          {issueLinks(p, repoUrl)}
        </div>
      )}
    </div>
  );
}

exports.default = ProposalCard;
exports.Badge = Badge;
