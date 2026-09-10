
// 通知 1 件のカード。カスタマイズ不要（そのまま利用）。
//
// ── 設計の要（#264）───────────────────────────────────────────────────────
//
// **カードはターミナル画面の写しではない。** 生の hook JSON も画面のダンプも
// 出さない（それが読めないからレポートを作っている）。通知に入っている構造化
// データを HTML のフォームとして組み直す。
//
// 選択肢の出どころは 2 通りある:
//
// 1. **通知（hook JSON）由来** — `AskUserQuestion` の `tool_input.questions`。
//    全設問・全選択肢・`description`・`preview` が入っている。**画面より情報が多い**
//    （ターミナルは 1 問ずつしか出さず、preview は `✂ N lines hidden` で切られる）。
//    このときは全問を 1 枚のフォームに出して一括で答える
// 2. **画面由来** — 許可ダイアログ / プラン承認 / y-n / 番号選択。
//    `oretachi_inspect_prompt` が返した**画面に実在するラベルだけ**を出す（#215）。
//    レポートを生成した AI が創作した候補文を見せると人の判断を誤らせる
const {
  OTHER,
  SHAPE_LABEL,
  shapeOf,
  isDialog,
  isReportOnly,
  isQuestionForm,
  askQuestions,
  answeredCount,
  tabAnsweredAt,
  paragraphsOf,
  richSegments,
  questionOf,
  optionsOf,
  previewKeys,
  isTruncated,
  canEscape,
  cursorReadable,
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

// 生の kind より「何が届いたのか」が読み取れる日本語ラベル（#264）。
// `worktree.message` のような内部識別子をそのままバッジに出さない
const KIND_LABEL = {
  general: '通知',
  approval: '承認待ち',
  completed: '作業完了',
  hook: 'フック',
  'worktree.message': 'メッセージ',
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

/**
 * 本文中の Markdown 記法・issue 番号・URL を組んで出す（#264）。
 *
 * エージェントは `notify_worktree` の本文に Markdown を書いてくる。素で出すと
 * `**分割して**` のような記号が並んで読みにくく、`#203` も `https://…` も
 * ただの文字列のままでリンクにならない。
 *
 * **リンク先は `lib/send` の `richSegments` が組んだものだけを使う。** 本文は
 * エージェントが書いた文字列なので、ここで任意の文字列を `href` にしない。
 */
function Rich({ text, repoUrl }) {
  const segs = richSegments(text, repoUrl);
  return (
    <React.Fragment>
      {segs.map((s, i) => {
        if (s.type === 'bold') return <b key={i}>{s.text}</b>;
        if (s.type === 'code') {
          return (
            <code key={i} style={{
              fontFamily: MONO, fontSize: '0.92em', background: '#11111b',
              border: '1px solid #313244', borderRadius: 3, padding: '1px 5px',
            }}>{s.text}</code>
          );
        }
        if (s.type === 'link') {
          return (
            <a key={i} href={s.href} style={{ color: '#89b4fa', wordBreak: 'break-all' }}>
              {s.text}
            </a>
          );
        }
        return <React.Fragment key={i}>{s.text}</React.Fragment>;
      })}
    </React.Fragment>
  );
}

/** ラベルと値の明細。ツール許可の `tool_input` などを表で出す */
function FieldTable({ fields }) {
  if (!fields || fields.length === 0) return null;
  return (
    <div style={{
      display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '6px 16px',
      border: '1px solid #262637', borderRadius: 6, background: '#11111b', padding: '10px 12px',
    }}>
      {fields.map((f, i) => (
        <React.Fragment key={i}>
          <div style={{ fontSize: 11, color: '#7f849c', fontFamily: FONT, paddingTop: 3 }}>
            {f.label}
          </div>
          {f.code ? (
            <pre style={{
              margin: 0, fontSize: 11.5, fontFamily: MONO, color: '#cdd6f4',
              lineHeight: 1.6, whiteSpace: 'pre-wrap', wordBreak: 'break-all', overflowX: 'auto',
            }}>{f.value}</pre>
          ) : (
            <div style={{ fontSize: 12.5, color: '#cdd6f4', fontFamily: FONT, lineHeight: 1.7 }}>
              {f.value}
            </div>
          )}
        </React.Fragment>
      ))}
    </div>
  );
}

function LinkRow({ links }) {
  if (!links || links.length === 0) return null;
  const icon = k => (k === 'artifact' ? '📄' : k === 'pr' ? '🔀' : k === 'issue' ? '🐞' : '🔗');
  return (
    <div style={{ display: 'flex', gap: 14, flexWrap: 'wrap' }}>
      {links.map((l, i) => (
        <a key={i} href={l.href} style={{
          display: 'inline-flex', alignItems: 'center', gap: 6,
          fontSize: 12, fontFamily: FONT, color: '#89b4fa', textDecoration: 'none',
        }}>
          <span>{icon(l.kind)}</span>
          <span style={{ textDecoration: 'underline' }}>{l.label || l.href}</span>
        </a>
      ))}
    </div>
  );
}

/** 通知の中身。**生データは出さない**（→ モジュール冒頭） */
function NotificationBody({ n, meta }) {
  const paragraphs = paragraphsOf(n);
  const bullets = (n.bullets || []).filter(b => typeof b === 'string' && b.trim());
  const repoUrl = meta && meta.repoUrl;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
      {paragraphs.map((p, i) => (
        <div key={i} style={{ fontSize: 13, color: '#cdd6f4', fontFamily: FONT, lineHeight: 1.85 }}>
          <Rich text={p} repoUrl={repoUrl} />
        </div>
      ))}
      {bullets.length > 0 && (
        <ul style={{ margin: 0, paddingLeft: 20, display: 'flex', flexDirection: 'column', gap: 4 }}>
          {bullets.map((b, i) => (
            <li key={i} style={{ fontSize: 12.5, color: '#bac2de', fontFamily: FONT, lineHeight: 1.8 }}>
              <Rich text={b} repoUrl={repoUrl} />
            </li>
          ))}
        </ul>
      )}
      <FieldTable fields={n.fields} />
      <LinkRow links={n.links} />
    </div>
  );
}

/** 見出し付きの囲み。カード内のセクションを揃える */
function Section({ title, color, note, children }) {
  return (
    <div style={{
      border: `1px solid ${color}44`, background: `${color}0d`, borderRadius: 6,
      padding: '11px 13px', display: 'flex', flexDirection: 'column', gap: 10,
    }}>
      <div style={{ fontSize: 11.5, color, fontFamily: FONT, fontWeight: 700 }}>{title}</div>
      {note && (
        <div style={{ fontSize: 11, color: '#7f849c', fontFamily: FONT, lineHeight: 1.7 }}>{note}</div>
      )}
      {children}
    </div>
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

/** 選択肢 1 つぶんのラジオ行。`description` と `preview` は通知由来のときだけ付く */
function OptionRow({ index, label, description, preview, selected, disabled, cursorHint, onClick }) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      style={{
        display: 'flex', alignItems: 'flex-start', gap: 10, textAlign: 'left',
        border: '1px solid ' + (selected ? '#89b4fa' : '#313244'),
        background: selected ? '#89b4fa14' : 'transparent',
        borderRadius: 6, padding: '9px 12px',
        cursor: disabled ? 'default' : 'pointer',
        fontFamily: FONT, lineHeight: 1.6,
      }}
    >
      <span style={{ color: selected ? '#89b4fa' : '#585b70', fontSize: 13, flexShrink: 0 }}>
        {selected ? '◉' : '○'}
      </span>
      <span style={{ fontSize: 11, fontFamily: MONO, color: '#6c7086', flexShrink: 0, paddingTop: 1 }}>
        {index}.
      </span>
      <span style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 5, minWidth: 0 }}>
        <span style={{
          fontSize: 12.5, color: disabled ? '#585b70' : '#cdd6f4',
          fontWeight: description ? 600 : 400, wordBreak: 'break-word',
        }}>{label}</span>
        {description && (
          <span style={{ fontSize: 11.5, color: '#9399b2', wordBreak: 'break-word' }}>
            {description}
          </span>
        )}
        {/* ターミナルでは `✂ N lines hidden` で切られて読めない preview を全文で出す（#264） */}
        {preview && (
          <pre style={{
            margin: '2px 0 0', fontSize: 11, fontFamily: MONO, color: '#a6adc8',
            background: '#11111b', border: '1px solid #313244', borderRadius: 4,
            padding: '8px 10px', overflowX: 'auto', lineHeight: 1.5,
          }}>{preview}</pre>
        )}
      </span>
      {cursorHint && (
        <span
          title="いま宛先の画面でこの選択肢に ❯ が当たっています（ここからの移動量でキー列が決まります）"
          style={{ fontSize: 10, fontFamily: MONO, color: '#585b70', flexShrink: 0 }}
        >❯ 現在位置</span>
      )}
    </button>
  );
}

/** 画面に実在する選択肢のラジオ。**既定選択は無し**（`Yes` をプリセットしない） */
function OptionRadios({ n, draft, disabled, onPickOption }) {
  const q = questionOf(n);
  const opts = optionsOf(n);
  const d = draft || {};
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      {opts.map(o => (
        <OptionRow
          key={o.index}
          index={o.index}
          label={o.label}
          selected={d.optionIndex === o.index}
          disabled={disabled}
          cursorHint={!!q && q.cursorIndex === o.index}
          onClick={() => onPickOption(d.optionIndex === o.index ? null : o.index)}
        />
      ))}
    </div>
  );
}

/**
 * 通知由来の設問フォーム（#264）。**全設問を 1 枚に出して一括で答える。**
 *
 * 宛先の画面には 1 問ぶんしか出ないが、`AskUserQuestion` の通知には全設問が
 * 入っている。1 問ずつ答えさせると「1 問答える → レポートを作り直す」を
 * 設問の数だけ繰り返すことになり、実質答えられない。
 *
 * 送信は `lib/send` の `answerAll` が 1 回で済ませ、Rust 側が
 * 「選ぶ → 画面が次へ進むのを待つ」を繰り返して確認画面の Submit まで確定する。
 */
function QuestionForm({ n, draft, disabled, onDraft }) {
  const questions = askQuestions(n);
  const picks = (draft && draft.picks) || {};
  const done = answeredCount(n, draft);
  return (
    <Section
      title={`設問に答えてください — 全 ${questions.length} 問（回答済み ${done}）`}
      color="#89b4fa"
      note={
        questions.length > 1
          ? '宛先の画面には 1 問ずつしか出ませんが、通知に全設問が入っているのでここで一度に答えられます。送信すると 1 問目から順に確定し、最後の確定（Submit）まで自動で進めます。'
          : '選択肢は宛先が実際に提示しているものです。'
      }
    >
      {questions.map((q, qi) => (
        <div key={qi} style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <span style={{
              width: 20, height: 20, borderRadius: 999, flexShrink: 0,
              display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              background: typeof picks[qi] === 'number' ? '#a6e3a1' : '#313244',
              color: typeof picks[qi] === 'number' ? '#11111b' : '#9399b2',
              fontSize: 11, fontWeight: 700, fontFamily: MONO,
            }}>{typeof picks[qi] === 'number' ? '✓' : qi + 1}</span>
            {q.header && <Badge label={q.header} color="#89b4fa" />}
            <span style={{ fontSize: 13, fontWeight: 700, color: '#cdd6f4', fontFamily: FONT, lineHeight: 1.6 }}>
              {q.question}
            </span>
            {/* 人が先にターミナルで答えていた設問。宛先はこの設問を飛ばすので、
                ここでの選択は使われない。**選択の要求は緩めない** —— タブの状態は
                生成時のスナップショットなので、これを根拠に選択を省くと、実際には
                未回答だったときに何も答えないまま画面が進む */}
            {tabAnsweredAt(n, qi) && (
              <Badge
                label="宛先で回答済み"
                color="#a6e3a1"
                title="レポート生成時点で、この設問は宛先の画面で既に回答済みでした。ここでの選択は送られません" />
            )}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, paddingLeft: 28 }}>
            {q.options.map((o, oi) => (
              <OptionRow
                key={oi}
                index={oi + 1}
                label={o.label}
                description={o.description}
                preview={o.preview}
                selected={picks[qi] === oi}
                disabled={disabled}
                onClick={() => onDraft({
                  mode: 'selectAll',
                  optionIndex: null,
                  value: null,
                  picks: { ...picks, [qi]: picks[qi] === oi ? null : oi },
                })}
              />
            ))}
          </div>
        </div>
      ))}
    </Section>
  );
}

/**
 * 報告カード（#228）。人の判断を必要としない購読イベントを「読むだけ」で出す。
 *
 * **送信 UI を一切持たない。** 選択肢・補足欄・再送ボタン・キー列プレビューを
 * 出さないのは、これらが「押さないと片付かない」という圧を作るため。判断が不要な
 * イベントに返答欄を出すと、人は全カードを捌こうとして無い判断を探すことになる。
 *
 * 参照するフィールドは `kind` / `worktreeName` / `branchName` / `at` / 本文 /
 * `links` だけ。`sessionId` / `subscribed` / `prompt` / `desc` / `phase` は
 * **報告カードでは収集していない**ので触らない（`worktree.closed` は発信元が
 * 既に削除済みで、`get_worktree_status` も `read_terminal` も引けない）。
 */
function ReportCard({ n, meta }) {
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
        <IssueRef n={n} meta={meta} />
        <span style={{ fontSize: 10.5, color: '#585b70', fontFamily: MONO }}>{n.at}</span>
        <Badge label={KIND_LABEL[n.kind] || n.kind} color={accent} />
        <div style={{ flex: 1 }} />
        <Badge
          label="報告のみ"
          color="#6c7086"
          title="人の判断を必要としないイベントなので、返答欄はありません（読むだけで完結します）" />
      </div>
      <NotificationBody n={n} meta={meta} />
    </div>
  );
}

/** issue 番号。リポジトリが分かっていればリンクにする（#264） */
function IssueRef({ n, meta }) {
  if (!n.issueRef) return null;
  const repoUrl = meta && meta.repoUrl;
  const num = String(n.issueRef).replace(/^#/, '');
  if (!repoUrl || !/^\d+$/.test(num)) {
    return <span style={{ fontSize: 12, color: '#9399b2', fontFamily: MONO }}>{n.issueRef}</span>;
  }
  return (
    <a href={`${repoUrl}/issues/${num}`} style={{ fontSize: 12, color: '#89b4fa', fontFamily: MONO }}>
      {n.issueRef}
    </a>
  );
}

/**
 * Props:
 *   n        通知データ（data/report の 1 要素）
 *   meta     data/report の META（repoUrl をリンク組み立てに使う）
 *   answer   送信済みの記録（サイドカー）。未送信なら null
 *   draft    入力中の下書き（サイドカー）
 *              自由入力: { choice, note }
 *              ダイアログ: { mode: 'select'|'selectAll'|'escapeThenText', optionIndex, picks, value, note }
 *   blocked  送信できない理由（文字列）。null なら送れる
 *   canSend  いま送れる状態か（`lib/send` の canSend の結果）
 *   inflight この 1 件を送信中か
 *   busy     一括送信の実行中か（入力を止める）
 *   onPick / onNote / onDraft / onRetry
 */
function NotificationCard({ n, meta, answer, draft, blocked, canSend, inflight, busy, onPick, onNote, onDraft, onRetry }) {
  // 報告のみのカードは別コンポーネントへ振る（#228）。
  //
  // **フックより前で返して問題ないのは `n.kind` が不変だから。** カードは
  // `key={n.id}` でマウントされ、`data/report` はスナップショットなので、同じ
  // インスタンスでこの分岐が反転することがない（フックの呼び出し順は保たれる）。
  if (isReportOnly(n)) return <ReportCard n={n} meta={meta} />;

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
  const questionForm = isQuestionForm(n);
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
  // ダイアログが宛先の画面に収まっていない。読めたぶんだけで選ばせてはいけない。
  // **通知由来の設問フォームは影響を受けない**（選択肢を画面から読んでいない。#264）
  const truncated = isTruncated(n) && !questionForm;
  const questions = questionForm ? askQuestions(n) : [];
  // `❯` の位置が読めない画面では選べない（Rust が移動量を決められない）。
  // ESC で抜ける経路だけが残るので、そう見えるようにする
  const cannotSelect = dialog && !cursorReadable(n);

  // 縮小表示は `sent` のときだけ。`failed` / `stale` / `unverified` / `pastedOnly` は
  // 人が次の手を決める必要がある（何が起きたかを畳むと気づかれない）ので畳まない。
  //
  // **`sent` でも設問が残っているものは畳まない。** `afterShape` が `askUserQuestion` の
  // ままなら「まだ答え切れていない」状態で、その旨は full view にしか出ない。
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
        <IssueRef n={n} meta={meta} />
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
      padding: '14px 18px', display: 'flex', flexDirection: 'column', gap: 11,
    }}>
      {/* 見出し: 状態 / 発信元ワークツリー / issue / 時刻 / 種別 / 問いの形状 */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
        <span style={{ color: accent, fontSize: 13 }}>{MARK[status] || '●'}</span>
        <span style={{ fontSize: 13, fontWeight: 700, color: '#cdd6f4', fontFamily: FONT }}>
          {n.worktreeName}
        </span>
        <IssueRef n={n} meta={meta} />
        <span style={{ fontSize: 11, color: '#585b70', fontFamily: MONO }}>{n.at}</span>
        <Badge label={KIND_LABEL[n.kind] || n.kind} color={kindColor} />
        {prompt && (
          <Badge
            label={SHAPE_LABEL[shape] || shape}
            color={SHAPE_COLOR[shape] || '#6c7086'}
            title={`宛先の画面の形状: ${shape}（レポート生成時点）`} />
        )}
        {questions.length > 1 && (
          <Badge
            label={`設問 ${answeredCount(n, d)}/${questions.length}`}
            color="#89b4fa"
            title="このカードから全問まとめて答えられます" />
        )}
        <div style={{ flex: 1 }} />
        {answer && (
          <Badge
            label={status === 'sent'
              ? `${STATUS_LABEL.sent} ${answer.at || ''}`.trim()
              : (dialog && status === 'pastedOnly'
                ? '送信途中で停止'
                : (STATUS_LABEL[status] || status))}
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

      <NotificationBody n={n} meta={meta} />

      {/* 分類不能: 画面末尾を読み取り専用で見せて、ターミナルでの手動操作へ誘導する。
          **ここだけは画面のテキストを出す** — 何を出せばいいのか分からない画面なので、
          人がターミナルで何を見ることになるかを示すしかない */}
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
      {answer && ['failed', 'stale', 'unsupported', 'unverified'].indexOf(status) >= 0
        // ダイアログ経路の `pastedOnly`（selectAll が途中で止まった）もここへ出す。
        // 出さないと「何問目まで確定したか」も理由もカードに現れず、
        // `Enter 未送信` というバッジだけが残って人が状況を読めない
        || (dialog && status === 'pastedOnly') ? (
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
              通っていない可能性があります。<b>同じ回答を再送しないでください</b>
              （矢印が二重に動いて別の選択肢を確定しえます）。ターミナルで状態を確認してください。
            </div>
          )}
          {status === 'unsupported' && (
            <div style={{ marginBottom: 4 }}>
              <b>この形状にはこの回答を送れないため、何も送っていません。</b>
            </div>
          )}
          {dialog && status === 'pastedOnly' && (
            <div style={{ marginBottom: 4 }}>
              <b>送信の途中で止まりました。</b>
              <b>同じ回答を再送しないでください</b>（既に送ったキーで ❯ が動いています）。
              残りはターミナルを開いて答えてください。
            </div>
          )}
          {typeof answer.answeredCount === 'number' && answer.answeredCount > 0 && (
            <div style={{ marginBottom: 4 }}>
              {/* Rust 側は画面のタブ（☒）から数える。人が先に答えていたぶんも含む */}
              <b>宛先では {answer.answeredCount} 問が確定済みです。</b>
              残りはターミナルを開いて答えてください。
            </div>
          )}
          {answer.error}
          {answer.keysSent && answer.keysSent.length > 0 && (
            <div style={{ marginTop: 4, color: '#9399b2', fontFamily: MONO, fontSize: 11 }}>
              送ったキー: {answer.keysSent.join(' → ')}
            </div>
          )}
        </div>
      ) : null}

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
                '（まだ設問が残っています。ターミナルを開くか、次のレポートで続きに答えてください）'}
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
        /* ── ダイアログ ────────────────────────────────────────────────────── */
        <React.Fragment>
          {questionForm ? (
            /* 通知由来の設問フォーム。全設問を 1 枚で（#264） */
            <QuestionForm n={n} draft={d} disabled={locked} onDraft={onDraft} />
          ) : shape === 'yesno' ? (
            <Section title="この確認に答える" color="#f9e2af">
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
            </Section>
          ) : truncated ? (
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
            </div>
          ) : (
            /* 画面由来の選択肢。**創作しない**（#215） */
            <Section
              title={promptTitle(n)}
              color={SHAPE_COLOR[shape] || '#6c7086'}
              note="宛先の画面に実在する選択肢です（既定選択はありません）">
              <OptionRadios n={n} draft={d.mode === 'escapeThenText' ? {} : d} disabled={locked}
                onPickOption={i => onDraft({ mode: 'select', optionIndex: i, value: null, picks: {} })} />
            </Section>
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
                  // **`picks` を消さない。** `setDraft` はパッチのマージなので、
                  // ここで空にすると多設問フォームで全問埋めたあと ESC を
                  // ON→OFF しただけで回答が全部消える（OFF 側では戻せない）。
                  // 送信経路は `d.mode` で分かれるので、残しておいても誤送信にならない
                  onChange={e => onDraft(e.target.checked
                    ? { mode: 'escapeThenText', optionIndex: null, value: null }
                    : { mode: questionForm ? 'selectAll' : 'select' })}
                />
                <span>
                  選択肢ではなく、<b>ESC で抜けて指示を書く</b>
                  {questionForm ? '（全設問をキャンセルして自由入力）' : '（拒否 + 指示）'}
                </span>
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

          {cannotSelect && !locked && (
            <div style={{
              fontSize: 11.5, color: '#f9e2af',
              background: '#f9e2af12', border: '1px solid #f9e2af44', borderRadius: 6,
              padding: '8px 12px', lineHeight: 1.7,
            }}>
              <b>宛先の画面でいまどの選択肢が選ばれているか（❯）を読み取れませんでした。</b>
              矢印の移動量を決められないため<b>選択肢は送れません</b>。
              {escapeAvailable
                ? '下の「ESC で抜けて指示を書く」を使うか、ターミナルを開いて直接操作してください。'
                : 'ターミナルを開いて直接操作してください。'}
            </div>
          )}

          <KeyPreview keys={keys} />

          {questionForm && !cannotSelect && d.mode !== 'escapeThenText' && !locked && (
            <div style={{ fontSize: 11, color: '#6c7086', fontFamily: FONT, lineHeight: 1.7 }}>
              送信すると、宛先の画面を 1 問ずつ読み直しながら
              {askQuestions(n).length > 1 ? ' 全設問を順に確定し、最後の Submit まで進めます' : ' 回答を確定します'}。
              画面が生成時と変わっていた場合は<b>何も送りません</b>。
            </div>
          )}

          {!keys && !questionForm && !locked && (
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
        /* ── 自由入力: 候補ボタン + 補足プロンプト ──────────────────────────── */
        <Section title="この通知へ返答する" color="#6c7086">
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
        </Section>
      )}
    </div>
  );
}

/** 画面由来の選択肢セクションの見出し。何を聞かれているのかを 1 行で示す */
function promptTitle(n) {
  const q = questionOf(n);
  const text = (q && q.question) || (n.prompt && n.prompt.header) || '';
  const label = SHAPE_LABEL[shapeOf(n)] || '確認';
  return text ? `${label} — ${text}` : label;
}

/** 読み取り専用表示用に「何を送ったか」を 1 行で表す */
function describeSent(n, answer) {
  if (!answer) return '—';
  if (!isDialog(n)) {
    return answer.choice === OTHER ? '（補足で直接指示）' : (answer.choice || '—');
  }
  if (answer.mode === 'escapeThenText') return 'ESC で抜けて指示を送信';
  if (answer.mode === 'selectAll' && answer.picks) {
    const qs = askQuestions(n);
    const parts = qs.map((q, qi) => {
      const oi = answer.picks[qi];
      const label = typeof oi === 'number' && q.options[oi] ? q.options[oi].label : '—';
      return `${q.header || `設問${qi + 1}`}: ${label}`;
    });
    return parts.join(' / ') || '—';
  }
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
exports.KIND_LABEL = KIND_LABEL;
exports.Badge = Badge;
exports.Rich = Rich;
exports.Section = Section;
