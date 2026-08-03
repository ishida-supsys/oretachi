
// ── slides/<SlideName> テンプレート例 ────────────────────────────────
// このファイルはスキーマ確認用のサンプル。実際の各スライドは
// Step 3 のアウトライン分析に基づいて新規生成する。
//
// ── 共通 props ────────────────────────────────────────────────────────
//   n     (number) 現在ページ番号（1 始まり）
//   total (number) 総ページ数
//   label (string) フッタ左に出す発表タイトル
//   → そのまま <Slide n={n} total={total} label={label}> に渡す
//
// ── import / export パターン ──────────────────────────────────────────
//   const { Slide, ... } = require('../components/deck');
//   exports.default = XxxSlide;                       // 1ファイル1スライド
//   exports.IntroSlide = IntroSlide;                  // 1ファイルに複数まとめる場合
//
// ── 画像について（重要） ──────────────────────────────────────────────
//   プレビュー iframe の CSP は default-src 'none'; img-src data: blob: のため
//   外部 URL の画像・フォント・CDN スクリプトは一切読み込めない。
//   図版は「インライン SVG」で描くか、data: URI を埋め込むこと。

const { Slide, SlideTitle, Bullets, TwoCol, Big, Quote, CodeBlock, Note, Center, THEME, u } =
  require('../components/deck');

// ── パターン 1: タイトルスライド ──────────────────────────────────────
// 中央寄せ + フッタなし。kind: 'title'
function TitleSlide({ label }) {
  return (
    <Slide footer={false}>
      <Center>
        <div style={{ fontSize: u(6.4), fontWeight: 900, lineHeight: 1.15, letterSpacing: '-0.02em' }}>
          アーティファクト対応形式の拡張
        </div>
        <div style={{ height: u(0.45), width: u(9), background: THEME.accent, borderRadius: u(0.3) }} />
        <div style={{ fontSize: u(2.2), color: THEME.muted }}>
          CSV / TSV とプレゼンテーション
        </div>
        <div style={{ fontSize: u(1.5), color: THEME.faint, marginTop: u(1.4) }}>
          {label} · 2026-08-02
        </div>
      </Center>
    </Slide>
  );
}

// ── パターン 2: 箇条書き ──────────────────────────────────────────────
// 最も基本的な型。項目は 4〜6 個まで。sub で補足を1行足せる。kind: 'bullets'
function AgendaSlide({ n, total, label }) {
  return (
    <Slide n={n} total={total} label={label}>
      <SlideTitle sub="本日お話しすること">アジェンダ</SlideTitle>
      <Bullets items={[
        { text: '現状のアーティファクト', sub: 'code / markdown / html / svg / mermaid / react の 6 形式' },
        { text: '追加する 2 つの形式', sub: 'CSV/TSV とプレゼンテーション' },
        { text: '設計方針', sub: 'ビューア拡張とスキル方式の使い分け' },
        'ロードマップ',
      ]} />
    </Slide>
  );
}

// ── パターン 3: 2カラム（左テキスト / 右図版） ────────────────────────
// ratio で左の比率を指定。図版はインライン SVG。kind: 'two-col'
function ArchSlide({ n, total, label }) {
  return (
    <Slide n={n} total={total} label={label}>
      <SlideTitle>アーキテクチャ概要</SlideTitle>
      <TwoCol
        ratio={0.52}
        left={
          <Bullets dense items={[
            'Tauri 2 + Vue 3 のデスクトップ構成',
            'アーティファクトは JSON 単位で保存',
            'MCP 経由でエージェントが生成',
            'ビューアは形式ごとに分岐描画',
          ]} />
        }
        right={
          <div style={{
            border: `1px solid ${THEME.border}`, borderRadius: u(0.8),
            background: THEME.panel, padding: u(1.6),
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <svg viewBox="0 0 188 140" style={{ width: '100%', height: 'auto' }}>
              <rect x="8" y="10" width="76" height="34" rx="5" fill="#fff" stroke="#89b4fa" strokeWidth="1.6" />
              <text x="46" y="31" textAnchor="middle" fontSize="10" fontFamily="system-ui" fill="#3b4252">Agent</text>
              <rect x="104" y="10" width="76" height="34" rx="5" fill="#fff" stroke="#fab387" strokeWidth="1.6" />
              <text x="142" y="31" textAnchor="middle" fontSize="10" fontFamily="system-ui" fill="#3b4252">MCP</text>
              <rect x="56" y="90" width="76" height="34" rx="5" fill="#fff" stroke="#a6e3a1" strokeWidth="1.6" />
              <text x="94" y="111" textAnchor="middle" fontSize="10" fontFamily="system-ui" fill="#3b4252">Viewer</text>
              <line x1="84" y1="27" x2="102" y2="27" stroke="#9aa0aa" strokeWidth="1.4" />
              <line x1="142" y1="44" x2="100" y2="88" stroke="#9aa0aa" strokeWidth="1.4" />
            </svg>
          </div>
        }
      />
    </Slide>
  );
}

// ── パターン 4: 数値・キーメッセージの強調 ────────────────────────────
// 1スライド1メッセージ。区切りとして効く。kind: 'big'
function ImpactSlide({ n, total, label }) {
  return (
    <Slide n={n} total={total} label={label}>
      <Center>
        <Big
          label="対応形式"
          value="6 → 8"
          sub="CSV/TSV をテーブルビューアで、スライドを React アーティファクトで"
        />
      </Center>
    </Slide>
  );
}

// ── パターン 5: コード ────────────────────────────────────────────────
// 15行程度まで。それ以上は要点だけ抜粋する。kind: 'code'
function CodeSlide({ n, total, label }) {
  return (
    <Slide n={n} total={total} label={label}>
      <SlideTitle sub="ロジックは SFC の外に出して単体テストする">CSV パースの入口</SlideTitle>
      <CodeBlock
        caption="src/utils/csvArtifact.ts"
        code={[
          'export function parseCsvArtifact(',
          '  content: string,',
          '  contentType: string,',
          '): CsvTable {',
          '  const result = Papa.parse<string[]>(content, {',
          '    header: false,',
          '    skipEmptyLines: "greedy",',
          '    delimiter: delimiterFor(contentType),',
          '  });',
          '  // 1行目をヘッダ、以降をデータ行として扱う',
          '}',
        ].join('\n')}
      />
      <Note>列キーは c0, c1, … を採番するため、ヘッダ名が重複・空欄でもテーブルが壊れない。</Note>
    </Slide>
  );
}

// ── パターン 6: 引用・方針の言い切り ──────────────────────────────────
// kind: 'quote'
function PolicySlide({ n, total, label }) {
  return (
    <Slide n={n} total={total} label={label}>
      <SlideTitle>設計方針</SlideTitle>
      <Center gap={1.6}>
        <Quote cite="実装方針メモ">
          新しい表現は、まずスキルで作れないか考える。<br />
          ビューアに手を入れるのは、既存の描画では表現できないときだけ。
        </Quote>
      </Center>
    </Slide>
  );
}

// ── パターン 7: クロージング ──────────────────────────────────────────
// kind: 'closing'
function ClosingSlide({ n, total, label }) {
  return (
    <Slide n={n} total={total} label={label}>
      <Center>
        <div style={{ fontSize: u(5.2), fontWeight: 900, letterSpacing: '-0.02em' }}>まとめ</div>
        <div style={{ height: u(0.45), width: u(7.6), background: THEME.accent, borderRadius: u(0.3) }} />
        <div style={{ maxWidth: u(72), textAlign: 'left', marginTop: u(1.2) }}>
          <Bullets dense items={[
            'CSV/TSV は新しい content_type としてテーブルビューアを追加',
            'プレゼンは既存の React アーティファクト上にスキルで構築',
            'ビューアの変更を最小に保ったまま表現力を広げる',
          ]} />
        </div>
      </Center>
    </Slide>
  );
}

exports.default = TitleSlide;
exports.TitleSlide = TitleSlide;
exports.AgendaSlide = AgendaSlide;
exports.ArchSlide = ArchSlide;
exports.ImpactSlide = ImpactSlide;
exports.CodeSlide = CodeSlide;
exports.PolicySlide = PolicySlide;
exports.ClosingSlide = ClosingSlide;
