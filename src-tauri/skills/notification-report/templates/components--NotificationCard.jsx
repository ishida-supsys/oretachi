
// 通知 1 件のカード。カスタマイズ不要（そのまま利用）。
//
// 形状（`n.prompt.shape`）ごとにレンダリングが変わる（#215）。**選択肢は画面に実在する
// ラベルだけを出す。** レポートを生成した AI が通知本文から創作した候補文を、宛先の画面に
// 実在しない選択肢として見せると人の判断を誤らせる。
const {
  OTHER,
  SHAPE_LABEL,
  shapeOf,
  isDialog,
  isReportOnly,
  bodyText,
  questionOf,
  optionsOf,
  previewKeys,
  isTruncated,
  canEscape,
} = require('../lib/send');

// 返答状態ごとのアクセント色（Catppuccin Mocha）。
// `pastedOnly` は「本文は届いたが Enter だけ失敗」— 失敗と同じ扱いにすると
// 人が同じ内容を送り直してしまうので、専用の色と文言で区別する
const ACCENT = {
  pending: '#f9e2af',
  sent: '#a6e3a1',
  failed: '#f38ba8',
  pastedOnly: '#fab387',
  // 画面が変わったので何も送っていない。赤系にしてリトライ導線を出さない（#215）
  stale: '#f38ba8',
  unsupported: '#f38ba8',
  // キーは送ったが画面が変わらなかった。届いたか分からない
  unverified: '#fab387',
};
const MARK = {
  pending: '●',
  sent: '✓',
  failed: '⚠',
  pastedOnly: '⏸',
  stale: '⚠',
  unsupported: '⚠',
  unverified: '?',
};
const STATUS_LABEL = {
  pending: '未返答',
  sent: '返答済み',
  failed: '送信失敗',
  pastedOnly: 'Enter 未送信',
  stale: '未送信（画面が変わった）',
  unsupported: '未送信（この形状には送れない）',
  unverified: '送信したが未確認',
};

// 通知種別（notify_worktree の kind）。
// `worktree.created` / `worktree.closed` は oretachi が自動発行する報告のみの種別で、
// 返答 UI を持たない（#228）。他と混ざらないよう寒色寄りの落ち着いた色にする
const KIND_COLOR = {
  general: '#6c7086',
  approval: '#fab387',
  completed: '#a6e3a1',
  hook: '#cba6f7',
  'worktree.message': '#89b4fa',
  'worktree.created': '#94e2d5',
  'worktree.closed': '#7f849c',
};

// 報告カードの種別ラベル。生の kind よりも「何が起きたか」が読み取れる
const REPORT_KIND_LABEL = {
  'worktree.created': 'ワークツリー作成',
  'worktree.closed': 'ワークツリークローズ',
};

// 問いの形状。ダイアログ系は目立たせる（何を操作するのか分かるように）
const SHAPE_COLOR = {
  text: '#6c7086',
  permission: '#f38ba8',
  plan: '#cba6f7',
  askUserQuestion: '#89b4fa',
  yesno: '#f9e2af',
  numbered: '#94e2d5',
  unknown: '#6c7086',
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

// 承認対象（`Bash(...)` / ツール名 / cwd / 設問文）。**全文を出す。**
// 何を承認するのか分からないまま `Yes` を押させてはいけない
function PromptContext({ prompt }) {
  const shape = prompt.shape;
  const body = prompt.context || prompt.header;
  if (!body) return null;
  return (
    <div style={{
      border: `1px solid ${(SHAPE_COLOR[shape] || '#45475a')}44`,
      background: `${(SHAPE_COLOR[shape] || '#45475a')}0d`,
      borderRadius: 6, padding: '10px 12px',
      display: 'flex', flexDirection: 'column', gap: 6,
    }}>
      <div style={{ fontSize: 10.5, color: '#7f849c', fontFamily: MONO, fontWeight: 700 }}>
        宛先の画面に出ている内容（全文）
      </div>
      <div style={{
        fontSize: 12, fontFamily: MONO, color: '#cdd6f4',
        lineHeight: 1.7, whiteSpace: 'pre-wrap', wordBreak: 'break-all',
      }}>{body}</div>
      {prompt.header && prompt.context && (
        <div style={{ fontSize: 12.5, fontFamily: FONT, color: '#f9e2af', fontWeight: 700 }}>
          {prompt.header}
        </div>
      )}
    </div>
  );
}

// 送信されるキー列のプレビュー。**何が起きるか見えない状態でダイアログを操作させない**
function KeyPreview({ keys }) {
  if (!keys || keys.length === 0) return null;
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
      <span style={{ fontSize: 10.5, color: '#7f849c', fontFamily: MONO }}>送信されるキー:</span>
      {keys.map((k, i) => (
        <React.Fragment key={i}>
          {i > 0 && <span style={{ color: '#45475a', fontSize: 11 }}>→</span>}
          <span style={{
            border: '1px solid #45475a', borderRadius: 4, padding: '1px 7px',
            fontSize: 11, fontFamily: MONO, color: '#94e2d5', background: '#11111b',
          }}>{k}</span>
        </React.Fragment>
      ))}
    </div>
  );
}

// 画面に実在する選択肢のラジオ。**既定選択は無し**（`Yes` をプリセットしない）
function OptionRadios({ n, draft, disabled, onPickOption }) {
  const q = questionOf(n);
  const opts = optionsOf(n);
  const d = draft || {};
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      {opts.map(o => {
        const selected = d.optionIndex === o.index;
        const isCursor = q && q.cursorIndex === o.index;
        return (
          <button
            key={o.index}
            type="button"
            disabled={disabled}
            onClick={() => onPickOption(selected ? null : o.index)}
            style={{
              display: 'flex', alignItems: 'flex-start', gap: 10, textAlign: 'left',
              border: '1px solid ' + (selected ? '#89b4fa' : '#313244'),
              background: selected ? '#89b4fa14' : 'transparent',
              borderRadius: 6, padding: '8px 12px',
              cursor: disabled ? 'default' : 'pointer',
              fontFamily: FONT, lineHeight: 1.6,
            }}
          >
            <span style={{
              color: selected ? '#89b4fa' : '#585b70', fontSize: 13, flexShrink: 0,
            }}>{selected ? '◉' : '○'}</span>
            <span style={{
              fontSize: 11, fontFamily: MONO, color: '#6c7086', flexShrink: 0, paddingTop: 1,
            }}>{o.index}.</span>
            <span style={{
              fontSize: 12.5, color: disabled ? '#585b70' : '#cdd6f4',
              wordBreak: 'break-word', flex: 1,
            }}>{o.label}</span>
            {isCursor && (
              <span
                title="いま宛先の画面でこの選択肢に ❯ が当たっています（ここからの移動量でキー列が決まります）"
                style={{ fontSize: 10, fontFamily: MONO, color: '#585b70', flexShrink: 0 }}
              >❯ 現在位置</span>
            )}
          </button>
        );
      })}
    </div>
  );
}

/**
 * 報告カード（#228）。人の判断を必要としない購読イベントを「読むだけ」で出す。
 *
 * **送信 UI を一切持たない。** 選択肢・補足欄・再送ボタン・キー列プレビューを
 * 出さないのは、これらが「押さないと片付かない」という圧を作るため。判断が不要な
 * イベントに返答欄を出すと、人は全カードを捌こうとして無い判断を探すことになる。
 *
 * 参照するフィールドは `kind` / `worktreeName` / `branchName` / `at` / `body` /
 * `link` だけ。`sessionId` / `subscribed` / `prompt` / `desc` / `phase` は
 * **報告カードでは収集していない**ので触らない（`worktree.closed` は発信元が
 * 既に削除済みで、`get_worktree_status` も `read_terminal` も引けない）。
 */
function ReportCard({ n }) {
  const accent = KIND_COLOR[n.kind] || '#7f849c';
  return (
    <div style={{
      border: '1px solid #262637', borderLeft: `4px solid ${accent}`,
      borderRadius: 8, background: '#16161f',
      padding: '11px 16px', display: 'flex', flexDirection: 'column', gap: 8,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
        <span style={{ color: accent, fontSize: 12 }}>ℹ</span>
        <span style={{ fontSize: 12.5, fontWeight: 700, color: '#bac2de', fontFamily: FONT }}>
          {n.worktreeName || '(名前不明)'}
        </span>
        {n.branchName && (
          <span style={{ fontSize: 11, color: '#7f849c', fontFamily: MONO }}>{n.branchName}</span>
        )}
        {n.issueRef && (
          <span style={{ fontSize: 11.5, color: '#7f849c', fontFamily: MONO }}>{n.issueRef}</span>
        )}
        <span style={{ fontSize: 10.5, color: '#585b70', fontFamily: MONO }}>{n.at}</span>
        <Badge label={REPORT_KIND_LABEL[n.kind] || n.kind} color={accent} />
        <div style={{ flex: 1 }} />
        <Badge
          label="報告のみ"
          color="#6c7086"
          title="人の判断を必要としないイベントなので、返答欄はありません（読むだけで完結します）" />
      </div>
      <div style={{
        fontSize: 12.5, color: '#a6adc8', fontFamily: FONT,
        lineHeight: 1.7, whiteSpace: 'pre-wrap',
      }}>{bodyText(n)}</div>
      {n.link && (
        <a href={n.link} style={{
          display: 'inline-flex', alignItems: 'center', gap: 6, alignSelf: 'flex-start',
          fontSize: 11.5, fontFamily: FONT, color: '#89b4fa', textDecoration: 'none',
        }}>
          <span>🔗</span>
          <span style={{ textDecoration: 'underline' }}>{n.linkLabel || 'アーティファクトを開く'}</span>
        </a>
      )}
    </div>
  );
}

/**
 * Props:
 *   n        通知データ（data/report の 1 要素）
 *   answer   送信済みの記録（サイドカー）。未送信なら null
 *   draft    入力中の下書き（サイドカー）
 *              自由入力: { choice, note }
 *              ダイアログ: { mode: 'select'|'escapeThenText', optionIndex, value, note }
 *   blocked  送信できない理由（文字列）。null なら送れる
 *   canSend  いま送れる状態か（`lib/send` の canSend の結果）
 *   inflight この 1 件を送信中か
 *   busy     一括送信の実行中か（入力を止める）
 *   onPick / onNote / onDraft / onRetry
 */
function NotificationCard({ n, answer, draft, blocked, canSend, inflight, busy, onPick, onNote, onDraft, onRetry }) {
  // 報告のみのカードは別コンポーネントへ振る（#228）。
  //
  // **フックより前で返して問題ないのは `n.kind` が不変だから。** カードは
  // `key={n.id}` でマウントされ、`data/report` はスナップショットなので、同じ
  // インスタンスでこの分岐が反転することがない（フックの呼び出し順は保たれる）。
  if (isReportOnly(n)) return <ReportCard n={n} />;

  const d = draft || {};
  const status = answer ? answer.status : 'pending';
  const accent = ACCENT[status] || ACCENT.pending;
  // 送信済みカードは既定で縮小表示にする。レポートは上から順に捌いていくので、
  // 済んだカードが本文全文の高さのまま残ると未返答のカードが画面外へ押し出される。
  // **消さずに畳む**（何を送ったかは 1 行で残し、「展開」で全文へ戻せる）
  const [expanded, setExpanded] = React.useState(false);
  const kindColor = KIND_COLOR[n.kind] || KIND_COLOR.general;
  const shape = shapeOf(n);
  const dialog = isDialog(n);
  const prompt = n.prompt || null;
  // 「その他」を選んだのに補足が空 → 送信対象にできない
  const otherNeedsNote = !dialog && d.choice === OTHER && !(d.note || '').trim();
  // 送信済み（成功）は読み取り専用にする。失敗は選び直して再送できる。
  // pastedOnly は本文が宛先に残っているので、内容を変えられては困る（Enter を
  // 送り直すだけの状態）。入力も読み取り専用にする
  const resumeEnter = !dialog && status === 'pastedOnly';
  // ダイアログのカードは一度送ったら読み取り専用。**再送させない**
  // （画面が変わっているか矢印が既に動いているので、同じ回答が別の選択肢を確定しうる）。
  // 例外は `failed` — 定義上キーを 1 つも送っていないので選び直して再送できる
  const readOnly =
    status === 'sent' || resumeEnter || (dialog && status !== 'pending' && status !== 'failed');
  const locked = readOnly || busy || !!blocked;
  const keys = previewKeys(n, d);
  // **形状も見る。** `escapeHatch` だけで判断すると `numbered` / `yesno` にも ESC 欄を
  // 出してしまい、押した瞬間 Rust が `unsupported` を返してそのカードが死ぬ
  const escapeAvailable = canEscape(n);
  // ダイアログが宛先の画面に収まっていない。読めたぶんだけで選ばせてはいけない
  const truncated = isTruncated(n);

  // 縮小表示は `sent` のときだけ。`failed` / `stale` / `unverified` / `pastedOnly` は
  // 人が次の手を決める必要がある（何が起きたかを畳むと気づかれない）ので畳まない。
  //
  // **`sent` でも設問が続いているものは畳まない。** `afterShape` が `askUserQuestion` の
  // ままなら「1 問答えたが 2 問目が残っている」状態で、その旨は full view にしか出ない。
  // 畳むと「返答済み」に見えるまま次のレポートを待つ導線が消える
  const questionRemains = !!(answer && answer.afterShape === 'askUserQuestion');
  const collapsible = status === 'sent' && !questionRemains;
  if (collapsible && !expanded) {
    return (
      <div style={{
        border: '1px solid #262637', borderLeft: `4px solid ${accent}`,
        borderRadius: 8, background: '#16161f', opacity: 0.62,
        padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap',
      }}>
        <span style={{ color: accent, fontSize: 12 }}>{MARK.sent}</span>
        <span style={{ fontSize: 12.5, fontWeight: 700, color: '#bac2de', fontFamily: FONT }}>
          {n.worktreeName}
        </span>
        {n.issueRef && (
          <span style={{ fontSize: 11.5, color: '#7f849c', fontFamily: MONO }}>{n.issueRef}</span>
        )}
        <span style={{ fontSize: 10.5, color: '#585b70', fontFamily: MONO }}>{n.at}</span>
        <span style={{
          fontSize: 11.5, color: '#7f849c', fontFamily: FONT,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 420,
        }}>
          送信内容: {describeSent(n, answer)}
          {answer && answer.note ? ` / ${answer.note}` : ''}
        </span>
        <div style={{ flex: 1 }} />
        <Badge label={`${STATUS_LABEL.sent} ${(answer && answer.at) || ''}`.trim()} color={accent} />
        <button type="button" onClick={() => setExpanded(true)} style={{
          border: '1px solid #45475a', borderRadius: 6, padding: '3px 10px',
          background: 'transparent', color: '#9399b2',
          fontSize: 11, fontWeight: 600, fontFamily: FONT, cursor: 'pointer',
        }}>展開</button>
      </div>
    );
  }

  return (
    <div style={{
      border: '1px solid #313244', borderLeft: `4px solid ${accent}`,
      borderRadius: 8, background: readOnly ? '#16161f' : '#181825',
      opacity: readOnly ? 0.8 : 1,
      padding: '14px 18px', display: 'flex', flexDirection: 'column', gap: 10,
    }}>
      {/* 見出し: 状態 / 発信元ワークツリー / issue / 時刻 / 種別 / 問いの形状 */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
        <span style={{ color: accent, fontSize: 13 }}>{MARK[status] || '●'}</span>
        <span style={{ fontSize: 13, fontWeight: 700, color: '#cdd6f4', fontFamily: FONT }}>
          {n.worktreeName}
        </span>
        {n.issueRef && (
          <span style={{ fontSize: 12, color: '#9399b2', fontFamily: MONO }}>{n.issueRef}</span>
        )}
        <span style={{ fontSize: 11, color: '#585b70', fontFamily: MONO }}>{n.at}</span>
        <Badge label={n.kind} color={kindColor} />
        {prompt && (
          <Badge
            label={SHAPE_LABEL[shape] || shape}
            color={SHAPE_COLOR[shape] || '#6c7086'}
            title={`宛先の画面の形状: ${shape}（${prompt.detectedAtMs ? '検出済み' : ''}レポート生成時点）`} />
        )}
        <div style={{ flex: 1 }} />
        {answer && (
          <Badge
            label={status === 'sent'
              ? `${STATUS_LABEL.sent} ${answer.at || ''}`.trim()
              : (STATUS_LABEL[status] || status)}
            color={accent} />
        )}
        {inflight && <Badge label="送信中…" color="#89b4fa" />}
        {collapsible && (
          <button type="button" onClick={() => setExpanded(false)} style={{
            border: '1px solid #45475a', borderRadius: 6, padding: '3px 10px',
            background: 'transparent', color: '#9399b2',
            fontSize: 11, fontWeight: 600, fontFamily: FONT, cursor: 'pointer',
          }}>畳む</button>
        )}
      </div>

      <WorktreeIdentity n={n} />

      <div style={{ borderTop: '1px solid #262637' }} />

      {/* 通知本文（人の判断に必要十分な全文） */}
      <div style={{ fontSize: 13, color: '#cdd6f4', fontFamily: FONT, lineHeight: 1.8, whiteSpace: 'pre-wrap' }}>
        {bodyText(n)}
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

      {/* ダイアログが開いている場合の承認対象。全文を出す */}
      {dialog && prompt && <PromptContext prompt={prompt} />}

      {/* 分類不能: 画面末尾を読み取り専用で見せて、ターミナルでの手動操作へ誘導する */}
      {shape === 'unknown' && prompt && (
        <div style={{
          border: '1px solid #f9e2af44', background: '#f9e2af0d',
          borderRadius: 6, padding: '10px 12px',
          display: 'flex', flexDirection: 'column', gap: 6,
        }}>
          <div style={{ fontSize: 11.5, color: '#f9e2af', fontFamily: FONT, lineHeight: 1.7 }}>
            <b>画面の形状を分類できませんでした。</b>
            推測でキーを送ると意図しない選択を確定しうるため、このカードからは送信できません。
            下は宛先の画面末尾です。ターミナルを開いて直接操作してください。
          </div>
          <pre style={{
            margin: 0, fontSize: 11, fontFamily: MONO, color: '#9399b2',
            background: '#11111b', border: '1px solid #313244', borderRadius: 4,
            padding: '8px 10px', overflowX: 'auto', lineHeight: 1.5,
          }}>{prompt.tail || '(画面を読み取れませんでした)'}</pre>
        </div>
      )}

      {/* 送信できない理由（購読なし / 端末不在 / 同一セッションへの 2 枚目） */}
      {blocked && (
        <div style={{
          fontSize: 12, fontFamily: FONT, color: '#f9e2af',
          background: '#f9e2af14', border: '1px solid #f9e2af44', borderRadius: 6,
          padding: '8px 12px', lineHeight: 1.7,
        }}>{blocked}</div>
      )}

      {/* 送信失敗のエラー / 未送信の理由。stale と unsupported は
          **リトライボタンを出さない**（画面が変わっているのでレポートを作り直す） */}
      {answer && ['failed', 'stale', 'unsupported', 'unverified'].indexOf(status) >= 0 && (
        <div style={{
          fontSize: 12, fontFamily: FONT, color: accent,
          background: `${accent}14`, border: `1px solid ${accent}44`, borderRadius: 6,
          padding: '8px 12px', lineHeight: 1.7, whiteSpace: 'pre-wrap',
        }}>
          {status === 'stale' && (
            <div style={{ marginBottom: 4 }}>
              <b>画面が変わったため、キーは送っていません。</b>
              リトライしても同じです（照合する画面が既に別物になっています）。
              AI にレポートの作り直しを頼んでから返答してください（既読化済みの通知も拾い直せます）。
            </div>
          )}
          {status === 'unverified' && (
            <div style={{ marginBottom: 4 }}>
              <b>キーは送りましたが、宛先の画面が変わりませんでした。</b>
              通っていない可能性があります。**同じ回答を再送しないでください**
              （矢印が二重に動いて別の選択肢を確定しえます）。ターミナルで状態を確認してください。
            </div>
          )}
          {status === 'unsupported' && (
            <div style={{ marginBottom: 4 }}>
              <b>この形状にはこの回答を送れないため、何も送っていません。</b>
            </div>
          )}
          {answer.error}
          {answer.keysSent && answer.keysSent.length > 0 && (
            <div style={{ marginTop: 4, color: '#9399b2', fontFamily: MONO, fontSize: 11 }}>
              送ったキー: {answer.keysSent.join(' → ')}
            </div>
          )}
        </div>
      )}

      {/* 本文だけ届いた状態。同じ内容を送り直すと二重になるので Enter だけ送る */}
      {resumeEnter && (
        <div style={{
          fontSize: 12, fontFamily: FONT, color: '#fab387',
          background: '#fab38714', border: '1px solid #fab38744', borderRadius: 6,
          padding: '8px 12px', lineHeight: 1.7,
        }}>
          <b>本文は宛先の入力欄に届いています。</b>Enter の送信だけが失敗しました。
          「Enter を再送」で送信し直してください（同じ内容をもう一度送るとテキストが二重になります）。
          {answer.error && (
            <div style={{ marginTop: 4, color: '#9399b2', fontFamily: MONO, fontSize: 11, whiteSpace: 'pre-wrap' }}>
              {answer.error}
            </div>
          )}
        </div>
      )}

      <div style={{ borderTop: '1px solid #262637' }} />

      {readOnly ? (
        /* 返答済み / 再送不可: 送信内容だけを残す（候補・入力欄は出さない） */
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div style={{ fontSize: 12, fontFamily: FONT, color: '#bac2de', lineHeight: 1.8 }}>
            <span style={{ color: '#6c7086' }}>送信内容: </span>
            <b>{describeSent(n, answer)}</b>
            {answer && answer.note ? <span style={{ color: '#9399b2' }}> / {answer.note}</span> : null}
          </div>
          {answer && answer.afterShape && (
            <div style={{ fontSize: 11, fontFamily: MONO, color: '#6c7086' }}>
              送信後の画面: {SHAPE_LABEL[answer.afterShape] || answer.afterShape}
              {answer.afterShape === 'askUserQuestion' &&
                '（まだ設問が残っています。次のレポートで続きに答えてください）'}
            </div>
          )}
          {resumeEnter && (
            <div style={{ display: 'flex', gap: 8 }}>
              <button type="button" disabled={busy || !canSend} onClick={onRetry} style={{
                border: '1px solid #fab387', borderRadius: 6, padding: '7px 14px',
                background: 'transparent', color: (busy || !canSend) ? '#45475a' : '#fab387',
                fontSize: 12, fontWeight: 600, fontFamily: FONT,
                cursor: (busy || !canSend) ? 'default' : 'pointer',
              }}>Enter を再送</button>
            </div>
          )}
        </div>
      ) : dialog ? (
        /* ── ダイアログ: 画面に実在する選択肢だけを出す ────────────────────── */
        <React.Fragment>
          {shape === 'yesno' ? (
            <div style={{ display: 'flex', gap: 8 }}>
              {['y', 'n'].map(v => (
                <ChoiceChip key={v} label={v === 'y' ? 'y（はい）' : 'n（いいえ）'}
                  disabled={locked}
                  selected={d.value === v && d.mode !== 'escapeThenText'}
                  onClick={() => onDraft({
                    mode: 'select',
                    value: d.value === v ? null : v,
                    optionIndex: null,
                  })} />
              ))}
            </div>
          ) : (
            truncated ? (
              <div style={{
                border: '1px solid #f38ba844', background: '#f38ba80d',
                borderRadius: 6, padding: '10px 12px',
                display: 'flex', flexDirection: 'column', gap: 6,
              }}>
                <div style={{ fontSize: 11.5, color: '#f38ba8', fontFamily: FONT, lineHeight: 1.7 }}>
                  <b>ダイアログが宛先の画面に収まっていません。</b>
                  選択肢を全部読めていないため、選択は提供しません
                  （読めたぶんだけで選ばせると、拒否の選択肢を見ないまま承認させることになります）。
                  下の「ESC で抜けて指示を書く」か、ターミナルを開いて直接操作してください。
                </div>
                <div style={{ fontSize: 11, color: '#9399b2', fontFamily: FONT }}>
                  読めた選択肢（<b>一部のみ</b>）: {optionsOf(n).map(o => `${o.index}. ${o.label}`).join(' / ') || '（なし）'}
                </div>
                <pre style={{
                  margin: 0, fontSize: 11, fontFamily: MONO, color: '#9399b2',
                  background: '#11111b', border: '1px solid #313244', borderRadius: 4,
                  padding: '8px 10px', overflowX: 'auto', lineHeight: 1.5,
                }}>{prompt.tail || '(画面を読み取れませんでした)'}</pre>
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{ fontSize: 11, color: '#7f849c', fontFamily: FONT }}>
                  宛先の画面に実在する選択肢（既定選択はありません）
                </div>
                <OptionRadios n={n} draft={d.mode === 'escapeThenText' ? {} : d} disabled={locked}
                  onPickOption={i => onDraft({ mode: 'select', optionIndex: i, value: null })} />
              </div>
            )
          )}

          {/* 拒否して指示を書く欄。選択肢の文言に依存せず ESC で自由入力へ落とす */}
          {escapeAvailable && (
            <div style={{
              border: '1px dashed #45475a', borderRadius: 6, padding: '10px 12px',
              display: 'flex', flexDirection: 'column', gap: 8,
            }}>
              <label style={{
                display: 'flex', alignItems: 'center', gap: 8,
                fontSize: 12, fontFamily: FONT, color: '#cdd6f4', cursor: locked ? 'default' : 'pointer',
              }}>
                <input
                  type="checkbox"
                  disabled={locked}
                  checked={d.mode === 'escapeThenText'}
                  onChange={e => onDraft(e.target.checked
                    ? { mode: 'escapeThenText', optionIndex: null, value: null }
                    : { mode: 'select' })}
                />
                <span>選択肢ではなく、<b>ESC で抜けて指示を書く</b>（拒否 + 指示）</span>
              </label>
              {d.mode === 'escapeThenText' && (
                <textarea
                  value={d.note || ''}
                  disabled={locked}
                  onChange={e => onNote(e.target.value)}
                  rows={2}
                  placeholder="ESC でダイアログを閉じたあと、この内容が自由入力へ送られます（必須）"
                  style={{
                    width: '100%', boxSizing: 'border-box', resize: 'vertical',
                    border: '1px solid ' + (!(d.note || '').trim() ? '#f38ba8' : '#45475a'),
                    borderRadius: 6, background: '#11111b', padding: '9px 12px',
                    fontSize: 13, fontFamily: FONT, color: '#cdd6f4', lineHeight: 1.7,
                  }}
                />
              )}
            </div>
          )}

          <KeyPreview keys={keys} />

          {!keys && !locked && (
            <div style={{ fontSize: 11, color: '#6c7086', fontFamily: FONT }}>
              {truncated
                ? 'ESC + 指示を書くと、送信されるキー列がここに出ます'
                : '選択肢を選ぶ（または ESC + 指示を書く）と、送信されるキー列がここに出ます'}
            </div>
          )}

          {/* `failed` はキーを 1 つも送っていないので再送できる。
              仮に送信済みだったとしても fingerprint 照合が stale で止める */}
          {status === 'failed' && !blocked && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <button type="button" disabled={busy || !canSend} onClick={onRetry} style={{
                border: '1px solid #45475a', borderRadius: 6, padding: '7px 14px',
                background: 'transparent', color: (busy || !canSend) ? '#45475a' : '#cdd6f4',
                fontSize: 12, fontWeight: 600, fontFamily: FONT,
                cursor: (busy || !canSend) ? 'default' : 'pointer',
              }}>この 1 件を再送</button>
              <span style={{ fontSize: 11, color: '#6c7086', fontFamily: FONT }}>
                キーは 1 つも送られていません
              </span>
            </div>
          )}
        </React.Fragment>
      ) : (
        /* ── 自由入力: 従来どおりの候補ボタン + 補足プロンプト ────────────── */
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

          {/* 再送は canSend でも塞ぐ。候補を解除したまま押せると、前置きと元通知
              だけの中身が無いプロンプトが宛先のエージェントへ飛ぶ */}
          {status === 'failed' && !blocked && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <button type="button" disabled={busy || !canSend} onClick={onRetry} style={{
                border: '1px solid #45475a', borderRadius: 6, padding: '7px 14px',
                background: 'transparent', color: (busy || !canSend) ? '#45475a' : '#cdd6f4',
                fontSize: 12, fontWeight: 600, fontFamily: FONT,
                cursor: (busy || !canSend) ? 'default' : 'pointer',
              }}>この 1 件を再送</button>
              {!canSend && !busy && (
                <span style={{ fontSize: 11, color: '#6c7086', fontFamily: FONT }}>
                  候補を選ぶと再送できます
                </span>
              )}
            </div>
          )}
        </React.Fragment>
      )}
    </div>
  );
}

/** 読み取り専用表示用に「何を送ったか」を 1 行で表す */
function describeSent(n, answer) {
  if (!answer) return '—';
  if (!isDialog(n)) {
    return answer.choice === OTHER ? '（補足で直接指示）' : (answer.choice || '—');
  }
  if (answer.mode === 'escapeThenText') return 'ESC で抜けて指示を送信';
  if (typeof answer.optionIndex === 'number') {
    const hit = optionsOf(n).find(o => o.index === answer.optionIndex);
    return hit ? `${hit.index}. ${hit.label}` : `選択肢 ${answer.optionIndex}`;
  }
  if (answer.value) return answer.value;
  return '—';
}

exports.default = NotificationCard;
exports.ReportCard = ReportCard;
exports.ACCENT = ACCENT;
exports.PHASE_COLOR = PHASE_COLOR;
exports.SHAPE_COLOR = SHAPE_COLOR;
exports.KIND_COLOR = KIND_COLOR;
exports.REPORT_KIND_LABEL = REPORT_KIND_LABEL;
exports.Badge = Badge;
