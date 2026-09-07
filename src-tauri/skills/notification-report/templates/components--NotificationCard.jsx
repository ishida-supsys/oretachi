
// 通知 1 件のカード。カスタマイズ不要（そのまま利用）。
const { OTHER } = require('../lib/send');

// 返答状態ごとのアクセント色（Catppuccin Mocha）
const ACCENT = { pending: '#f9e2af', sent: '#a6e3a1', failed: '#f38ba8' };
const MARK = { pending: '●', sent: '✓', failed: '⚠' };
const STATUS_LABEL = { pending: '未返答', sent: '返答済み', failed: '送信失敗' };

// 通知種別（notify_worktree の kind）
const KIND_COLOR = {
  general: '#6c7086',
  approval: '#fab387',
  completed: '#a6e3a1',
  hook: '#cba6f7',
};

// ワークツリーの現況。レポート生成時に read_terminal を読んで要約した値が入る
const PHASE_COLOR = {
  '設計中': '#89b4fa',
  '実装中': '#fab387',
  '実装完了': '#a6e3a1',
  'レビュー対応中': '#cba6f7',
  '停止条件待ち': '#f9e2af',
  '不明': '#6c7086',
};

const FONT = 'system-ui, sans-serif';
const MONO = 'ui-monospace, SFMono-Regular, Menlo, monospace';

function Badge({ label, color, title }) {
  return (
    <span title={title} style={{
      background: `${color}22`, color, border: `1px solid ${color}55`,
      borderRadius: 4, padding: '2px 8px',
      fontSize: 10.5, fontFamily: MONO, fontWeight: 700, whiteSpace: 'nowrap',
    }}>{label}</span>
  );
}

// 候補ボタン。単一選択トグル。`other` は「どれでもない」パターンで破線にして区別する
function ChoiceChip({ label, selected, disabled, other, onClick }) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      style={{
        border: (other ? '1px dashed ' : '1px solid ') + (selected ? '#89b4fa' : '#45475a'),
        borderRadius: 999,
        padding: '6px 14px',
        background: selected ? '#89b4fa' : 'transparent',
        color: disabled ? '#45475a' : (selected ? '#181825' : (other ? '#9399b2' : '#cdd6f4')),
        fontSize: 12, fontWeight: 600, fontFamily: FONT,
        cursor: disabled ? 'default' : 'pointer', whiteSpace: 'nowrap',
      }}
    >{label}</button>
  );
}

// ワークツリーの identity: description（無ければターミナルから推定したミッション）と現況。
// 通知本文だけでは「どのワークツリーが何をしていて、いまどの段階か」が分からない
function WorktreeIdentity({ n }) {
  const phaseColor = PHASE_COLOR[n.phase] || PHASE_COLOR['不明'];
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ fontSize: 12, fontFamily: FONT, lineHeight: 1.6 }}>
        {n.desc ? (
          <span style={{ color: '#bac2de' }}>{n.desc}</span>
        ) : (
          <span style={{ color: '#7f849c' }}>
            <span style={{
              border: '1px dashed #45475a', borderRadius: 4, padding: '1px 6px',
              fontSize: 10, marginRight: 6,
            }}>description 未設定</span>
            {n.descFallback || 'ミッション不明（ターミナルからも読み取れませんでした）'}
          </span>
        )}
      </div>
      {n.phase && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          <Badge label={n.phase} color={phaseColor} title="ターミナル読み取りから要約した現況" />
          {n.phaseSummary && (
            <span style={{ fontSize: 11.5, color: '#9399b2', fontFamily: FONT, lineHeight: 1.6 }}>
              {n.phaseSummary}
            </span>
          )}
          {n.readAt && (
            <span style={{ fontSize: 10, color: '#585b70', fontFamily: MONO }}>{n.readAt} 時点</span>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Props:
 *   n        通知データ（data/report の 1 要素）
 *   answer   送信済みの記録（サイドカー）。未送信なら null
 *   draft    { choice, note } — 入力中の下書き（サイドカー）
 *   blocked  送信できない理由（文字列）。null なら送れる
 *   inflight この 1 件を送信中か
 *   busy     一括送信の実行中か（入力を止める）
 *   onPick / onNote / onRetry
 */
function NotificationCard({ n, answer, draft, blocked, inflight, busy, onPick, onNote, onRetry }) {
  const d = draft || {};
  const status = answer ? answer.status : 'pending';
  const accent = ACCENT[status] || ACCENT.pending;
  const kindColor = KIND_COLOR[n.kind] || KIND_COLOR.general;
  // 「その他」を選んだのに補足が空 → 送信対象にできない
  const otherNeedsNote = d.choice === OTHER && !(d.note || '').trim();
  // 送信済み（成功）は読み取り専用にする。失敗は選び直して再送できる
  const readOnly = status === 'sent';
  const locked = readOnly || busy || !!blocked;

  return (
    <div style={{
      border: '1px solid #313244', borderLeft: `4px solid ${accent}`,
      borderRadius: 8, background: readOnly ? '#16161f' : '#181825',
      opacity: readOnly ? 0.8 : 1,
      padding: '14px 18px', display: 'flex', flexDirection: 'column', gap: 10,
    }}>
      {/* 見出し: 状態 / 発信元ワークツリー / issue / 時刻 / 種別 */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
        <span style={{ color: accent, fontSize: 13 }}>{MARK[status]}</span>
        <span style={{ fontSize: 13, fontWeight: 700, color: '#cdd6f4', fontFamily: FONT }}>
          {n.worktreeName}
        </span>
        {n.issueRef && (
          <span style={{ fontSize: 12, color: '#9399b2', fontFamily: MONO }}>{n.issueRef}</span>
        )}
        <span style={{ fontSize: 11, color: '#585b70', fontFamily: MONO }}>{n.at}</span>
        <Badge label={n.kind} color={kindColor} />
        <div style={{ flex: 1 }} />
        {answer && (
          <Badge
            label={status === 'sent' ? `${STATUS_LABEL.sent} ${answer.at || ''}`.trim() : STATUS_LABEL.failed}
            color={accent} />
        )}
        {inflight && <Badge label="送信中…" color="#89b4fa" />}
      </div>

      <WorktreeIdentity n={n} />

      <div style={{ borderTop: '1px solid #262637' }} />

      {/* 通知本文（人の判断に必要十分な全文） */}
      <div style={{ fontSize: 13, color: '#cdd6f4', fontFamily: FONT, lineHeight: 1.8, whiteSpace: 'pre-wrap' }}>
        {n.body}
      </div>

      {/* 子ワークツリーのアーティファクトへの artifact:// リンク */}
      {n.link && (
        <a href={n.link} style={{
          display: 'inline-flex', alignItems: 'center', gap: 6, alignSelf: 'flex-start',
          fontSize: 12, fontFamily: FONT, color: '#89b4fa', textDecoration: 'none',
        }}>
          <span>🔗</span>
          <span style={{ textDecoration: 'underline' }}>{n.linkLabel || 'アーティファクトを開く'}</span>
        </a>
      )}

      {/* 送信できない理由（購読なし / 端末不在） */}
      {blocked && (
        <div style={{
          fontSize: 12, fontFamily: FONT, color: '#f9e2af',
          background: '#f9e2af14', border: '1px solid #f9e2af44', borderRadius: 6,
          padding: '8px 12px', lineHeight: 1.7,
        }}>{blocked}</div>
      )}

      {/* 送信失敗のエラー */}
      {status === 'failed' && answer.error && (
        <div style={{
          fontSize: 12, fontFamily: FONT, color: '#f38ba8',
          background: '#f38ba814', border: '1px solid #f38ba844', borderRadius: 6,
          padding: '8px 12px', lineHeight: 1.7, whiteSpace: 'pre-wrap',
        }}>{answer.error}</div>
      )}

      <div style={{ borderTop: '1px solid #262637' }} />

      {readOnly ? (
        /* 返答済み: 送信内容だけを残す（ボタン・入力欄は出さない） */
        <div style={{ fontSize: 12, fontFamily: FONT, color: '#bac2de', lineHeight: 1.8 }}>
          <span style={{ color: '#6c7086' }}>送信内容: </span>
          <b>{answer.choice === OTHER ? '（補足で直接指示）' : answer.choice}</b>
          {answer.note ? <span style={{ color: '#9399b2' }}> / {answer.note}</span> : null}
        </div>
      ) : (
        <React.Fragment>
          {/* 候補ボタン。末尾の「その他」は常に出す固定の選択肢 */}
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            {(n.choices || []).map(c => (
              <ChoiceChip key={c} label={c} disabled={locked}
                selected={d.choice === c}
                onClick={() => onPick(d.choice === c ? null : c)} />
            ))}
            <ChoiceChip label={OTHER} other disabled={locked}
              selected={d.choice === OTHER}
              onClick={() => onPick(d.choice === OTHER ? null : OTHER)} />
          </div>

          {/* 補足プロンプト。「その他」選択時は必須 */}
          <textarea
            value={d.note || ''}
            disabled={locked}
            onChange={e => onNote(e.target.value)}
            rows={2}
            placeholder={d.choice === OTHER
              ? '補足プロンプト（必須） — 「その他」を選んだので、ここに直接指示を書く'
              : '補足プロンプト（任意） — 候補で足りないときはここに書く'}
            style={{
              width: '100%', boxSizing: 'border-box', resize: 'vertical',
              border: '1px solid ' + (otherNeedsNote ? '#f38ba8' : '#45475a'),
              borderRadius: 6, background: otherNeedsNote ? '#f38ba80f' : '#11111b',
              padding: '9px 12px',
              fontSize: 13, fontFamily: FONT, color: '#cdd6f4', lineHeight: 1.7,
            }}
          />

          {otherNeedsNote && (
            <div style={{ fontSize: 11, color: '#f38ba8', fontFamily: FONT }}>
              補足プロンプトが空のため、この 1 件は送信対象に入りません
            </div>
          )}

          {status === 'failed' && !blocked && (
            <div style={{ display: 'flex', gap: 8 }}>
              <button type="button" disabled={busy} onClick={onRetry} style={{
                border: '1px solid #45475a', borderRadius: 6, padding: '7px 14px',
                background: 'transparent', color: busy ? '#45475a' : '#cdd6f4',
                fontSize: 12, fontWeight: 600, fontFamily: FONT,
                cursor: busy ? 'default' : 'pointer',
              }}>この 1 件を再送</button>
            </div>
          )}
        </React.Fragment>
      )}
    </div>
  );
}

exports.default = NotificationCard;
exports.ACCENT = ACCENT;
exports.PHASE_COLOR = PHASE_COLOR;
exports.Badge = Badge;
