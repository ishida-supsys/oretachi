//! ターミナルに出ている「問い」を解析し、安全に答えられる形へ落とす（#215）。
//!
//! # なぜ画面グリッドが必要か
//!
//! `oretachi_read_terminal` が返すのは `strip_ansi(出力履歴)` で、**これではダイアログを
//! 復元できない。** Claude Code は差分描画のためにカーソル移動 (CSI) で既存行を書き換え、
//! `strip_ansi` はまさにその CSI を捨てるため、バイト列には再描画の断片だけが残る
//! （実測: 選択肢を 1 つ動かすと 4 バイト書き込まれ、`strip_ansi` 後は空文字列になる）。
//!
//! そこで [`render_logical_screen`] が出力履歴を VT エミュレータへ流し直して**画面グリッド**を作り、
//! [`parse_prompt`] はその画面テキストだけを見る純粋関数にしてある。フロント側の
//! `autoApproval.ts` が xterm.js の `buffer.active` を読んでいるのと同じ土台を、
//! UI に依存せず Rust 側で用意する形。
//!
//! # 実測に基づく前提（`docs` ではなく実機から採った）
//!
//! - Claude Code の選択ダイアログのフッタは `Enter to select · Tab/Arrow keys to navigate ·
//!   Esc to cancel`。**数字キーは確定キーではない**（`1` を送っても画面は進まなかった）。
//!   そのため選択は**矢印キーで `❯` を動かして CR** で行う。`❯` の位置は画面から読めるので
//!   移動量が決まり、選択肢が 10 件以上あっても同じ手順で通る。
//! - `AskUserQuestion` の末尾の逃げ道は `Other` ではなく **`Chat about this`** と描画される。
//!
//! # 意図的に扱わないもの
//!
//! - **`multiSelect` の判別。** 画面に単一選択との差が見当たらず、判別根拠が無い。
//!   トグルキーを推測して送ると意図しない選択を確定しうるので、複数選択は
//!   「1 つ選んで CR」だけを提供する（残りは人がターミナルで操作する）。
//! # 複数設問の `AskUserQuestion`（実測。#264）
//!
//! 複数設問はタブ UI で描かれ、**画面には常に 1 問ぶんしか出ない**:
//!
//! ```text
//! ←  ☒ Color  ☐ Size  ✔ Submit  →     ☐ 未回答 / ☒ 回答済み
//! Which size?
//!  1. Large            ← タブが自動で進んだ直後は ❯ が描かれない
//!      大きいサイズを選択します。
//!   2. Small
//! ```
//!
//! 全問答えると確認画面（`Review your answers` / `❯ 1. Submit answers` / `2. Cancel`）
//! へ進む。**この画面には Claude Code のフッタが出ない**ので、フッタだけを手がかりに
//! すると素の番号リスト扱い（数字キー）へ落ちて、確定できないまま止まる。
//! そこで**タブバー行もフッタと同格の「Claude Code のダイアログである」印**として扱う。
//!
//! 選択肢に `preview` が付くと**右へプレビュー枠が並ぶ横並びレイアウト**になり、
//! ラベルの右に枠線が食い込む（実測: `"Grid    ┌────────┐"`）。
//! プレビュー枠の桁を見つけてラベルを切る（[`find_preview_column`]）。

use serde::Serialize;
use sha2::{Digest, Sha256};

/// 画面再生に流し込む出力履歴の最大バイト数。
///
/// `pty_manager::OUTPUT_HISTORY_BYTES` と同じ値。**リングバッファ全部を流す。**
/// これより大きくしても取れるものは増えず、小さくすると全画面再描画 1 回ぶんを
/// 取り逃す確率が上がるだけ。
///
/// 先頭がエスケープシーケンスの途中から始まる（＝リングバッファに切られている）ことは
/// あるが、VT エミュレータは不正なシーケンスを読み飛ばして直後から追従するので、
/// 入力待ちで止まっている画面はそのまま再現できる。
pub const REPLAY_BYTES: usize = 65_536;

/// カーソル移動やフッタを探すときに画面末尾から見る行数。
const TAIL_WINDOW: usize = 12;

/// 選択肢の上にある「何を承認するのか」を拾う最大行数。
const CONTEXT_WINDOW: usize = 24;

/// タブバーが選択肢からどれだけ離れていてよいか（行数）。
///
/// タブが 1 つだけの単一設問では、TodoWrite パネルの `☒`/`☐` と形が同じで
/// 区別できない。本物は設問文を挟んで選択肢のすぐ上にあるので、距離で絞る。
const TAB_BAR_MAX_GAP: usize = 4;

/// 見出しが折り返しているとみなして繋げる最大行数。
///
/// 実測（13 桁のターミナル）では `Do you want to proceed?` が 3 行に割れた。
/// 大きくしすぎると承認対象の説明まで見出しへ吸い込む。
const HEADER_WRAP_LINES: usize = 4;

// ─── 問いの形状 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PromptShape {
    /// 自由入力プロンプト（ダイアログ無し）。本文 → CR で答える
    #[serde(rename = "text")]
    Text,
    /// ツール許可ダイアログ（`PermissionRequest`）
    #[serde(rename = "permission")]
    Permission,
    /// プラン承認（`ExitPlanMode`）
    #[serde(rename = "plan")]
    Plan,
    /// `AskUserQuestion` の設問
    #[serde(rename = "askUserQuestion")]
    AskUserQuestion,
    /// 素の `(y/N)` プロンプト（gh / npm / git などシェル側）
    #[serde(rename = "yesno")]
    YesNo,
    /// Claude Code 以外の TUI の番号選択リスト（`1) foo`）
    #[serde(rename = "numbered")]
    Numbered,
    /// 分類できなかった。**キーを送ってはいけない**
    #[serde(rename = "unknown")]
    Unknown,
}

impl PromptShape {
    pub fn as_str(self) -> &'static str {
        match self {
            PromptShape::Text => "text",
            PromptShape::Permission => "permission",
            PromptShape::Plan => "plan",
            PromptShape::AskUserQuestion => "askUserQuestion",
            PromptShape::YesNo => "yesno",
            PromptShape::Numbered => "numbered",
            PromptShape::Unknown => "unknown",
        }
    }

    /// Claude Code の選択ダイアログか（矢印 + CR で答える系）。
    fn is_cc_select(self) -> bool {
        matches!(
            self,
            PromptShape::Permission | PromptShape::Plan | PromptShape::AskUserQuestion
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptOption {
    /// 画面に描かれている番号そのまま（1 始まりだが、欠番があってもそのまま持つ）
    pub index: u32,
    /// 画面のラベル全文（折り返された続き行も連結済み）
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptQuestion {
    /// 設問の見出し。取れなければ空文字
    pub header: String,
    /// 設問文
    pub question: String,
    /// **常に false。** 画面から複数選択を判別する根拠が無いため（モジュール冒頭参照）
    pub multi_select: bool,
    pub options: Vec<PromptOption>,
    /// 末尾の逃げ道（`Chat about this` / `No, and tell Claude what to do differently`）があるか
    pub allow_other: bool,
    /// いま `❯` が指している選択肢の `index`。**取れなければ `None` で、選択は不可**
    /// （矢印の移動量が決まらないため）
    pub cursor_index: Option<u32>,
}

/// 選択肢の選び方。**`shape` とは別に持つ。**
///
/// Claude Code のダイアログは矢印で `❯` を動かして CR（実測で確認済み: 許可ダイアログへ
/// Down を 2 回送ってから CR を送ると 3 番目の選択肢が確定した）。素の TUI の番号リストは
/// 行入力ベースなので数字 + CR。**取り違えるとどちらも効かない**か、最悪別の解釈をされる。
///
/// 判定根拠を `shape` に相乗りさせると、見出しが折り返して形状の推定が外れたときに
/// 一緒にキーの種類まで間違える（実測: 13 桁のターミナルでは `Do you want to proceed?` が
/// 3 行に折り返され、見出し一致による形状判定が外れた）。そこで独立させ、
/// **Claude Code のフッタという構造的な手がかり**から決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Navigation {
    /// 矢印で `❯` を動かして CR（Claude Code のダイアログ）
    #[serde(rename = "arrows")]
    Arrows,
    /// 数字 + CR（素の TUI の番号リスト）
    #[serde(rename = "digits")]
    Digits,
    /// 選択肢が無い（`text` / `yesno` / `unknown`）
    #[serde(rename = "none")]
    None,
}

/// 複数設問 `AskUserQuestion` のタブバーの 1 タブ（#264）。
///
/// 画面の `←  ☒ Color  ☐ Size  ✔ Submit  →` から起こす。**設問文と選択肢は
/// いま開いているタブのぶんしか画面に無い**ので、ここから取れるのは
/// 「何問あって、どこまで答えたか」だけ。それでも十分に価値がある:
/// カードが進捗を出せるうえ、この行の存在自体が
/// 「Claude Code のダイアログである」という決定的な印になる（確認画面には
/// フッタが無く、これが無いと素の番号リストへ誤分類される）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionTab {
    /// タブの見出し（`AskUserQuestion` の `questions[].header`）
    pub label: String,
    /// `☒` なら回答済み、`☐` なら未回答
    pub answered: bool,
    /// 末尾の `✔ Submit`（回答を確定するタブ）
    pub is_submit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPrompt {
    pub shape: PromptShape,
    /// 選択肢の選び方。`shape` からは決めない（上の説明参照）
    pub navigation: Navigation,
    /// 問いの見出し（`Do you want to proceed?` など）。全文
    pub header: String,
    /// 承認対象。`Bash(...)` / ツール名 / cwd / 設問文など、選択肢の上にある説明の全文。
    /// **何を承認するのか分からないまま `Yes` を押させないためにカードへ全文出す**
    pub context: String,
    pub questions: Vec<PromptQuestion>,
    /// 複数設問 `AskUserQuestion` のタブバー（#264）。単一設問なら 1 要素、
    /// タブバーが無い画面（許可ダイアログなど）では空。
    pub tabs: Vec<QuestionTab>,
    /// `Some("esc")` なら ESC で自由入力へ抜けられる
    pub escape_hatch: Option<String>,
    /// **ダイアログが画面に収まっておらず、選択肢を全部読めていない疑いがある。**
    ///
    /// `true` のとき選択は拒否する（[`plan_keys`]）。読めた選択肢だけを人へ見せると、
    /// 「`1. Yes` しか無い」と誤認して承認させてしまう。実測: dev インスタンスの
    /// 7 行 13 桁のタブでは許可ダイアログの `2.` 以降と見出しが画面外へ流れ、
    /// `options` が `[{1, "Yes"}]` だけになった。
    ///
    /// ESC で抜ける経路（`escapeThenText`）は画面に何が見えていても成立するので塞がない。
    pub truncated: bool,
    /// 画面から抽出した安定キー。`answer_prompt` の再検証に使う。
    /// **`❯` の位置も含める** — 位置が動いていたら矢印の移動量が変わるため、
    /// 「一致したら移動量も同じ」を保証する
    pub fingerprint: String,
    /// 画面末尾。`unknown` のとき人に見せて手動操作へ誘導する
    pub tail: String,
}

// ─── 送るキー ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keystroke {
    /// 人間可読表現（`"Down"` / `"CR"` / `"text(42文字)"`）。UI のキー列プレビューに出す
    pub label: String,
    pub bytes: Vec<u8>,
}

impl Keystroke {
    fn down() -> Self {
        Self { label: "Down".into(), bytes: b"\x1b[B".to_vec() }
    }
    fn up() -> Self {
        Self { label: "Up".into(), bytes: b"\x1b[A".to_vec() }
    }
    fn cr() -> Self {
        Self { label: "CR".into(), bytes: b"\r".to_vec() }
    }
    fn esc() -> Self {
        Self { label: "Esc".into(), bytes: b"\x1b".to_vec() }
    }
    fn text(s: &str) -> Self {
        Self {
            label: format!("text({}文字)", s.chars().count()),
            bytes: s.as_bytes().to_vec(),
        }
    }
    fn ch(c: char) -> Self {
        Self { label: c.to_string(), bytes: c.to_string().into_bytes() }
    }
}

/// 構造化された回答。`answer_prompt` のフラットなパラメータから [`Answer::from_parts`] で組む。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// 選択肢を 1 つ選ぶ
    Select { option_index: u32 },
    /// 自由入力へ本文を送る
    Text { text: String },
    /// ESC でダイアログを抜けてから本文を送る（`No, and tell Claude ...` の代わり）
    EscapeThenText { text: String },
    /// 素の y/n プロンプトへ答える
    YesNo { yes: bool },
}

impl Answer {
    /// MCP のフラットなパラメータから組み立てる。
    ///
    /// タグ付き enum を直接受け取らずフラットにしているのは、アーティファクトの JS が
    /// 素の JSON オブジェクトで呼ぶため（内部タグ付き enum は schemars の表現が読みにくい）。
    pub fn from_parts(
        kind: &str,
        option_index: Option<u32>,
        text: Option<&str>,
        value: Option<&str>,
    ) -> Result<Self, String> {
        match kind {
            "select" => {
                let i = option_index.ok_or_else(|| {
                    "kind=\"select\" には option_index が必須です（oretachi_inspect_prompt が返した options[].index の値）".to_string()
                })?;
                Ok(Answer::Select { option_index: i })
            }
            "text" | "escapeThenText" => {
                let t = text
                    .map(str::to_string)
                    .filter(|t| !t.trim().is_empty())
                    .ok_or_else(|| format!("kind=\"{}\" には空でない text が必須です", kind))?;
                // 本文は「通知の中身 + ユーザーの補足」で、ブラケットペーストで囲んでいない。
                // ESC がそのまま届くと宛先の TUI へ任意のエスケープシーケンスを注入できる
                // ので、呼び出し側の畳み込みに頼らずここでも弾く（`lib/send` の flatten と
                // 二重の防波堤。片方が外れても注入にならないようにする）
                if let Some(bad) = t.chars().find(|c| c.is_control()) {
                    return Err(format!(
                        "text に制御文字 (U+{:04X}) が含まれています。宛先の TUI へエスケープシーケンスを注入しうるため受け付けません。改行や ESC を除いた 1 行に畳んでから渡してください",
                        bad as u32
                    ));
                }
                if kind == "text" {
                    Ok(Answer::Text { text: t })
                } else {
                    Ok(Answer::EscapeThenText { text: t })
                }
            }
            "yesno" => match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
                Some("y") | Some("yes") => Ok(Answer::YesNo { yes: true }),
                Some("n") | Some("no") => Ok(Answer::YesNo { yes: false }),
                other => Err(format!(
                    "kind=\"yesno\" の value は \"y\" / \"n\" のいずれかです（受け取った値: {:?}）",
                    other
                )),
            },
            other => Err(format!(
                "未知の kind '{}' です。使えるのは \"select\" / \"text\" / \"escapeThenText\" / \"yesno\" です",
                other
            )),
        }
    }
}

/// キー列を組めなかった理由。呼び出し側が `unsupported` として返す文言に使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError(pub String);

// ─── 複数設問の一括回答（#264）──────────────────────────────────────────────

/// 複数設問の `AskUserQuestion` を 1 回の呼び出しで答え切るときの、
/// 「いまの画面に対して次に何をするか」。
///
/// **I/O から切り離した純粋関数にしてある**（[`plan_select_all_step`]）。
/// ここの判断を誤ると答えが別の設問へ入るので、テストで固定できる形にしておく。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectAllStep {
    /// `qidx` 問目（0 始まり）へ `option_index` を選ぶ
    Answer { qidx: usize, option_index: u32 },
    /// 確認画面。`option_index`（`Submit answers`）を選んで確定する
    Submit { option_index: u32 },
    /// ダイアログが閉じた。やることは残っていない
    Done,
    /// 送ってはいけない状態。文言をそのまま `reason` に使う
    Refuse(String),
}

/// 「1 手ぶん進んだか」の判定（#264）。
///
/// **fingerprint の差分で判定してはいけない。** fingerprint には `❯` の位置が
/// 入っているので、矢印だけ届いて CR がまだ処理されていない**中間画面**でも変わる。
/// そこで「進んだ」と誤判定すると、次の周回で同じ設問へ CR をもう 1 回送ってしまい、
/// 1 発目で確定して次の設問へ進んだ画面に対して**2 発目が既定の選択肢を確定する**
/// （セルフレビューで検出）。
///
/// 進んだと言えるのは次のどちらか:
///
/// - ダイアログが閉じた（`askUserQuestion` でなくなった）
/// - 回答済み（`☒`）のタブが増えた
pub fn select_all_progressed(before: &ParsedPrompt, after: &ParsedPrompt) -> bool {
    if after.shape != PromptShape::AskUserQuestion {
        return true;
    }
    answered_tabs(after) > answered_tabs(before)
}

/// 回答済み（`☒`）のタブ数。`✔ Submit` は数えない。
pub fn answered_tabs(parsed: &ParsedPrompt) -> usize {
    parsed.tabs.iter().filter(|t| t.answered && !t.is_submit).count()
}

/// 設問タブの数（`✔ Submit` を除く）。
pub fn question_tabs(parsed: &ParsedPrompt) -> usize {
    parsed.tabs.iter().filter(|t| !t.is_submit).count()
}

/// いまの画面に対して次に何をするかを決める（純粋関数）。
///
/// `indices` は**設問の並び順**（= タブの並び順）に並べた画面上の選択肢番号。
/// `last_qidx` は直前にこの呼び出しが答えた設問の位置。
///
/// **同じ設問へ 2 回答えない。** `last_qidx` と同じ位置がまた来たら「進んでいない」
/// ということなので、送らずに止める（進んでいない画面へ次のキーを送ると、
/// 遅れて処理された 1 発目のあとに 2 発目が別の設問を確定する）。
pub fn plan_select_all_step(
    parsed: &ParsedPrompt,
    indices: &[u32],
    last_qidx: Option<usize>,
) -> SelectAllStep {
    if parsed.shape != PromptShape::AskUserQuestion {
        return SelectAllStep::Done;
    }

    // 設問数と渡された回答数が合わないなら**何も送らない**。ずれたまま送ると
    // 別の設問へ別の答えが入る（`request.questions` は生成側 AI が書き起こす
    // データなので、設問を落とす事故が現実にありうる）
    let total = question_tabs(parsed);
    if total > 0 && total != indices.len() {
        return SelectAllStep::Refuse(format!(
            "宛先の画面には設問が {} 問ありますが、渡された回答は {} 件です。ずれたまま送ると別の設問へ答えが入るため、何も送っていません（レポートを作り直してください）",
            total,
            indices.len()
        ));
    }

    // タブバーを読めなくても、見出しと `Submit answers` の並びで確認画面と分かる
    // （#264。タブバーが画面外へ流れると `tabs` が空になる）
    let review_without_tabs = parsed.tabs.is_empty()
        && contains_ci(&parsed.header, SUBMIT_REVIEW_HEADING)
        && parsed
            .questions
            .first()
            .is_some_and(|q| q.options.iter().any(|o| o.label.to_lowercase().starts_with("submit")));
    let on_review = review_without_tabs
        || (parsed.tabs.iter().any(|t| t.is_submit)
            && parsed.tabs.iter().all(|t| t.is_submit || t.answered));
    if on_review {
        // **見出しでも裏取りする。** 「Submit タブがあって全部 ☒」だけを条件に
        // `submit` を含むラベルを探すと、人がタブを戻して回答済みの設問を表示していて
        // その設問に `Submit for review` のような選択肢があったときに別のものを確定する。
        // 確認画面の見出しは `Ready to submit your answers?`（実測）
        let heading_ok = contains_ci(&parsed.header, "submit");
        debug_assert!(!review_without_tabs || heading_ok);
        let submit = parsed
            .questions
            .first()
            .and_then(|q| q.options.iter().find(|o| o.label.to_lowercase().starts_with("submit")));
        return match (heading_ok, submit) {
            (true, Some(o)) => SelectAllStep::Submit { option_index: o.index },
            _ => SelectAllStep::Refuse(
                "全問の回答は送りましたが、確認画面の『Submit answers』を確かめられませんでした（見出しと選択肢が想定と違います）。取り違えを避けるため何も送っていません。ターミナルを開いて確定してください".to_string(),
            ),
        };
    }

    let qidx = match parsed.tabs.iter().position(|t| !t.answered && !t.is_submit) {
        Some(i) => i,
        // **「タブが読めない」と「全部答え終わった」を混ぜない。**
        // 単一設問には `✔ Submit` タブが無いので、答え終わった画面は
        // 「未回答タブなし・Submit タブなし」になる。ここを先送りの分岐に落とすと
        // 存在しない 2 問目を探して `Refuse` になり、**成功しているのに
        // `unverified` でカードが読み取り専用になる**（セルフレビューで検出）
        None if !parsed.tabs.is_empty() => return SelectAllStep::Done,
        // タブバーそのものを読めなかった画面だけ、送った回数で先へ進める
        None => last_qidx.map(|i| i + 1).unwrap_or(0),
    };
    if Some(qidx) == last_qidx {
        return SelectAllStep::Refuse(format!(
            "{} 問目を送ったあと画面が次の設問へ進みませんでした。**同じ設問へもう一度送ると、遅れて確定した先の設問で別の選択肢を確定しえます**。ターミナルで状態を確認してください",
            qidx + 1
        ));
    }
    match indices.get(qidx) {
        Some(i) => SelectAllStep::Answer { qidx, option_index: *i },
        None => SelectAllStep::Refuse(format!(
            "設問 {} 問目の回答が渡された {} 件の中にありません。途中まで送った状態で止めました",
            qidx + 1,
            indices.len()
        )),
    }
}

/// 画面の形状と回答から、送るキー列を組み立てる（純粋関数）。
///
/// **`Unknown` には何も組み立てない。** 分類できていない画面へ推測でキーを送ると、
/// 別のダイアログの既定選択（許可ダイアログなら `1. Yes`）を確定しうる。
///
/// キーの間には呼び出し側が猶予を入れる。Claude Code は同じ読み取りチャンクに来た CR を
/// 送信として扱わないため、1 キー 1 write に分ける必要がある。
pub fn plan_keys(parsed: &ParsedPrompt, answer: &Answer) -> Result<Vec<Keystroke>, PlanError> {
    let unsupported = |what: &str| {
        PlanError(format!(
            "画面の形状は '{}' で、{}。キーは送っていません",
            parsed.shape.as_str(),
            what
        ))
    };

    match (parsed.shape, answer) {
        // **選択肢を全部読めていない疑いがあるなら選ばせない。** 読めたぶんだけ見せると
        // 「`1. Yes` しか無い」と誤認させ、拒否の選択肢を見ないまま承認させてしまう
        (_, Answer::Select { .. }) if parsed.truncated => Err(PlanError(
            "ダイアログが宛先の画面に収まっておらず、選択肢を全部読めていません（画面外に流れた選択肢がある）。読めたぶんだけで選ばせると拒否の選択肢を見ないまま承認させることになるため、選択は受け付けません。ターミナルを開いて直接操作するか、ESC で抜けて指示を送る (kind=\"escapeThenText\") を使ってください".to_string(),
        )),

        // 矢印で `❯` を動かして CR（Claude Code のダイアログ）
        (_, Answer::Select { option_index }) if parsed.navigation == Navigation::Arrows => {
            let q = parsed
                .questions
                .first()
                .ok_or_else(|| unsupported("選択肢を読み取れませんでした"))?;
            let target = q
                .options
                .iter()
                .position(|o| o.index == *option_index)
                .ok_or_else(|| {
                    PlanError(format!(
                        "option_index {} は画面に存在しません（画面の選択肢: {}）。**画面に無い選択肢は送れません**",
                        option_index,
                        q.options
                            .iter()
                            .map(|o| o.index.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?;
            // `❯` が読めないと移動量が決まらない。数字キーへ倒すと確定キーでない以上
            // 何も起きないか、TUI 次第では別の解釈をされるので送らない
            let cursor = q.cursor_index.ok_or_else(|| {
                unsupported("いまどの選択肢が選ばれているか (❯) が読み取れず、矢印の移動量を決められません")
            })?;
            let current = q
                .options
                .iter()
                .position(|o| o.index == cursor)
                .ok_or_else(|| unsupported("❯ が指す選択肢が選択肢一覧の中に見つかりません"))?;
            let mut keys = Vec::new();
            if target > current {
                keys.extend(std::iter::repeat_with(Keystroke::down).take(target - current));
            } else {
                keys.extend(std::iter::repeat_with(Keystroke::up).take(current - target));
            }
            keys.push(Keystroke::cr());
            Ok(keys)
        }

        // 素の TUI の番号選択は行入力ベース。矢印ではなく数字 + CR
        (_, Answer::Select { option_index }) if parsed.navigation == Navigation::Digits => {
            let q = parsed
                .questions
                .first()
                .ok_or_else(|| unsupported("選択肢を読み取れませんでした"))?;
            if !q.options.iter().any(|o| o.index == *option_index) {
                return Err(PlanError(format!(
                    "option_index {} は画面に存在しません",
                    option_index
                )));
            }
            let mut keys: Vec<Keystroke> =
                option_index.to_string().chars().map(Keystroke::ch).collect();
            keys.push(Keystroke::cr());
            Ok(keys)
        }

        (PromptShape::Text, Answer::Text { text }) => {
            Ok(vec![Keystroke::text(text), Keystroke::cr()])
        }

        // ダイアログを ESC で畳んでから自由入力へ落とす。`No, and tell Claude what to do
        // differently` を選ぶのと同じ着地点で、選択肢の文言に依存しない
        (s, Answer::EscapeThenText { text }) if s.is_cc_select() => {
            if parsed.escape_hatch.is_none() {
                return Err(unsupported("ESC で抜けられる表示 (Esc to cancel) が画面に無く、ESC 後の挙動が読めません"));
            }
            Ok(vec![Keystroke::esc(), Keystroke::text(text), Keystroke::cr()])
        }

        (PromptShape::YesNo, Answer::YesNo { yes }) => Ok(vec![
            Keystroke::ch(if *yes { 'y' } else { 'n' }),
            Keystroke::cr(),
        ]),

        // **ダイアログで止まっている宛先へ自由テキストを送らせない。**
        // テキストはダイアログに吸われ、CR が既定選択（許可ダイアログなら `1. Yes`）の
        // 確定として解釈されうる。これが #215 の出発点
        (s, Answer::Text { .. }) if s.is_cc_select() => Err(PlanError(format!(
            "画面の形状は '{}' でダイアログが開いています。自由テキストを送るとダイアログに吸われ、末尾の CR が意図しない選択肢の確定として解釈されるため受け付けません。選択肢を選ぶ (kind=\"select\") か、ESC で抜けてから送る (kind=\"escapeThenText\") を使ってください",
            s.as_str()
        ))),

        (PromptShape::Unknown, _) => Err(PlanError(
            "画面の形状を分類できませんでした (unknown)。推測でキーを送ると別のダイアログの既定選択を確定しうるため、何も送りません。tail を人に見せてターミナルで直接操作してもらってください".to_string(),
        )),

        (shape, answer) => Err(PlanError(format!(
            "画面の形状 '{}' に対して kind={} は使えません",
            shape.as_str(),
            match answer {
                Answer::Select { .. } => "\"select\"",
                Answer::Text { .. } => "\"text\"",
                Answer::EscapeThenText { .. } => "\"escapeThenText\"",
                Answer::YesNo { .. } => "\"yesno\"",
            }
        ))),
    }
}

/// 送るキー列の人間可読プレビュー（`"Down → Down → CR"`）。
pub fn keys_preview(keys: &[Keystroke]) -> Vec<String> {
    keys.iter().map(|k| k.label.clone()).collect()
}

// ─── 画面再生 ────────────────────────────────────────────────────────────────

/// 出力履歴のバイト列を VT エミュレータへ流し直して画面テキストを作る。
///
/// `rows` / `cols` は**その PTY に実際に通知されている値**を渡すこと。履歴は「その幅で
/// 描かれた」バイト列なので、別の幅で再生すると折り返し位置がずれて選択肢行が壊れる。
///
/// 返すのは可視画面のみ（スクロールバックは見ない）。ダイアログは画面下部に収まる前提で、
/// 収まらず見出しが流れた場合は `parse_prompt` が `unknown` へ倒れる（安全側）。
/// 出力履歴を再生して、**折り返しを解いた論理行**の画面テキストを作る。
///
/// vt100 の `contents()` は物理行をそのまま返すので、狭いターミナルでは 1 つの論理行が
/// 複数行に割れる。実測（13 桁）では次のように壊れた:
///
/// - `Do you want to proceed?` → `Do you want` / `to` / `proceed?`
/// - `Esc to cancel · Tab to amend` → `Esc to` / `cancel ·` / `Tab to` / `amend`
/// - `Continue with the merge? (y/N)` → `Continue with` / ` the merge? (` / `y/N)`
///
/// 最後の例が効いてくる。`(y/N)` というマーカー自体が割れるので、行を空白で連結しても
/// `( y/N)` になって一致しない。**vt100 は折り返した行に印を持っている**ので、
/// `row_wrapped` が立っている行は**区切り無し**で次の行へ繋ぐ。これで論理行が復元でき、
/// 見出し・フッタ・選択肢ラベルの折り返しをまとめて扱わなくて済む。
pub fn render_logical_screen(bytes: &[u8], rows: u16, cols: u16) -> String {
    // vt100 0.16 は極端に小さいグリッドで減算オーバーフローして panic する
    // （`grid.rs` 内）。spawn 直後などに 0 が来ることが実際にあるため、
    // 現実のターミナルとして意味のある下限まで持ち上げる。ここで持ち上げた場合は
    // 折り返し位置が実機と食い違うので解析は `unknown` へ倒れやすくなるが、
    // panic して MCP ツールごと落とすより良い
    let rows = rows.max(MIN_REPLAY_ROWS);
    let cols = cols.max(MIN_REPLAY_COLS);
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(bytes);
    let screen = parser.screen();

    let mut out = String::new();
    for row in 0..rows {
        let text: String = screen
            .rows(0, cols)
            .nth(row as usize)
            .unwrap_or_default();
        out.push_str(&text);
        // 折り返しの継続がある行は改行を入れずに次の行へ繋ぐ。
        // **右端で切れた文字列を空白で繋いではいけない**（`(y/N)` が `( y/N)` になる）
        if !screen.row_wrapped(row) {
            out.push('\n');
        }
    }
    // 画面下部の空行は落とす（`contents()` と同じ振る舞い）。残すと `tail` が
    // 空行だらけになって、人へ見せる画面末尾が読みにくい
    out.trim_end_matches('\n').to_string()
}

const MIN_REPLAY_ROWS: u16 = 4;
const MIN_REPLAY_COLS: u16 = 20;

// ─── 行の下処理 ──────────────────────────────────────────────────────────────

/// 枠線（`│ ... │`）を剥がした本文と、その本文が始まる桁位置を返す。
///
/// 桁位置は選択肢ラベルの折り返し続き行を判定するために使う。
fn strip_frame(line: &str) -> (String, usize) {
    let chars: Vec<char> = line.chars().collect();
    let mut start = 0usize;
    let mut end = chars.len();
    // 先頭の空白と縦罫線
    while start < end && (chars[start].is_whitespace() || is_vertical_rule(chars[start])) {
        start += 1;
    }
    while end > start && (chars[end - 1].is_whitespace() || is_vertical_rule(chars[end - 1])) {
        end -= 1;
    }
    (chars[start..end].iter().collect(), start)
}

fn is_vertical_rule(c: char) -> bool {
    matches!(c, '│' | '┃' | '|' | '║')
}

/// 罫線だけの行（`─────` / `╭───╮`）か。
fn is_rule_line(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            matches!(c, '─' | '━' | '═' | '-' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘')
        })
}

/// カーソルマーカー（`❯` / `►`）を剥がした残りを返す。剥がしたかどうかも返す。
///
/// **`>` はマーカーとして扱わない。** `>` は Claude Code の入力欄とシェルのプロンプトの
/// 記号で、選択ダイアログのカーソルは実物では `❯`。「`>` の直後が数字ならマーカー」と
/// していたところ、入力欄に `1. やること` と打ち込んだ状態が「1 件の番号選択ダイアログ」に
/// 化け、その番号を送れてしまった（セルフレビューで検出）。
fn strip_cursor_marker(s: &str) -> (&str, bool) {
    for marker in ['❯', '►', '▶'] {
        if let Some(rest) = s.strip_prefix(marker) {
            return (rest.trim_start(), true);
        }
    }
    (s, false)
}

/// その行が Claude Code の**入力欄**か（＝選択肢行ではない）。
///
/// # なぜ形だけでは区別できないか（実機で判明）
///
/// 入力欄に `1. やること` と打ち込んだ状態は、実機では次のように描画される:
///
/// ```text
/// ────────────────────────────────
/// ❯1. do this thing first
/// ────────────────────────────────
///   ⏸ manual mode on
/// ```
///
/// **カーソル記号は `>` ではなく `❯` で、直後にスペースも無い。** つまり選択肢行
/// （`❯ 1. Yes`）と文字種では区別できず、素朴に解析すると「1 件の番号選択ダイアログ」に
/// 化けてその番号を送れてしまう（セルフレビューで指摘され、実機で再現した）。
///
/// 効く手がかりは**位置**。入力欄は上下を罫線（`────` / `╭╮` / `╰╯`）で挟まれている。
/// ダイアログの選択肢は見出しや他の選択肢に挟まれているので、両側が罫線になることはない。
fn is_input_box_line(lines: &[&str], i: usize) -> bool {
    let (body, _) = strip_frame(lines[i]);
    if !matches!(body.chars().next(), Some('❯') | Some('>')) {
        return false;
    }
    // 直近の非空行が上下ともに罫線か
    let neighbour_is_rule = |range: &mut dyn Iterator<Item = usize>| -> bool {
        for j in range {
            let (b, _) = strip_frame(lines[j]);
            if b.is_empty() {
                continue;
            }
            return is_rule_line(&b);
        }
        false
    };
    let above = neighbour_is_rule(&mut (0..i).rev());
    let below = neighbour_is_rule(&mut (i + 1..lines.len()));
    above && below
}

/// タブバー行（`←  ☒ Color  ☐ Size  ✔ Submit  →`）を解析する（#264）。
///
/// **`☐` / `☒` が 1 つも無い行はタブバーではない。** `✔` だけを手がかりにすると
/// 本文中のチェックマークを拾う。単一設問では `☐ Color` の 1 タブだけで、
/// `←` `→` と `✔ Submit` は描かれない（実測）。
fn parse_tab_bar(body: &str) -> Option<Vec<QuestionTab>> {
    if !body.contains('☐') && !body.contains('☒') {
        return None;
    }
    let mut tabs: Vec<QuestionTab> = Vec::new();
    let mut current: Option<(bool, bool, String)> = None; // (answered, is_submit, label)
    for c in body.chars() {
        match c {
            '☐' | '☑' | '☒' | '✔' | '✓' => {
                if let Some((answered, is_submit, label)) = current.take() {
                    push_tab(&mut tabs, answered, is_submit, label);
                }
                current = Some((matches!(c, '☑' | '☒'), matches!(c, '✔' | '✓'), String::new()));
            }
            // タブバーの左右送り記号はラベルではない
            '←' | '→' => {}
            _ => {
                if let Some((_, _, label)) = current.as_mut() {
                    label.push(c);
                }
            }
        }
    }
    if let Some((answered, is_submit, label)) = current.take() {
        push_tab(&mut tabs, answered, is_submit, label);
    }
    if tabs.is_empty() {
        None
    } else {
        Some(tabs)
    }
}

/// 確認画面（全問回答後）の見出し。タブバーを読めないときの唯一の手がかり。
///
/// **`parse_prompt` と [`plan_select_all_step`] で同じ文字列を照合する。**
/// 片方だけ直すと、画面は確認画面と認識できているのに一括回答から
/// Submit を押せない（あるいはその逆）という食い違いになる。
pub const SUBMIT_REVIEW_HEADING: &str = "submit your answers";

/// 選択肢の上にある行を「末尾の見出しブロック」と「その上の補足」に割る。
///
/// **見出しは折り返す。** 実測（13 桁のターミナル）では
/// `Do you want to proceed?` が 3 行に割れた。1 行だけ拾うと `proceed?` になり、
/// 見出しでの判定（確認画面の検出など）が**狭いタブでだけ外れる**。
/// 末尾の空行と罫線を飛ばしてから、連続する非空行をまとめて 1 つの見出しにする。
fn split_heading_and_context(above: &[String]) -> (String, String) {
    let mut end = above.len();
    while end > 0 && (above[end - 1].is_empty() || is_rule_line(&above[end - 1])) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && !above[start - 1].is_empty() && !is_rule_line(&above[start - 1]) {
        start -= 1;
    }
    let heading = above[start..end]
        .iter()
        .map(|b| b.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let context = above[..start]
        .iter()
        .filter(|b| !b.is_empty() && !is_rule_line(b))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    (heading, context)
}

/// タブバーとして採ってよいだけの形をしているか（#264。セルフレビューで検出）。
///
/// **`☐` / `☒` が 1 個あるだけでは足りない。** Claude Code の TodoWrite パネルは
/// `☒ テストを書く` / `☐ 実装する` を 1 行ずつ描くので、選択肢の上に todo が出ている
/// だけでその行がタブバーとして採られる。そうなると:
///
/// - `has_cc_footer` が立って `navigation` が Digits → Arrows へ反転し、
///   **素の TUI の番号リストへ矢印 + CR を送りうる**
/// - 偽のタブが `question_tabs` / `answered_tabs` に流れ込み、一括回答の件数照合が狂う
/// - `above` をそこで切るので承認対象（context）が落ちる
///
/// 本物のタブバーは 1 行にタブが並ぶので、次のどちらかで見分ける:
///
/// - タブが 2 つ以上ある、または `←` / `→` / `✔ Submit` を伴う（複数設問・確認画面）
/// - タブが 1 つだけ（単一設問）のときは、**呼び出し側が
///   「`AskUserQuestion` のフッタが出ている」「選択肢のすぐ上にある」を追加で確かめる**
fn is_strong_tab_bar(tabs: &[QuestionTab], body: &str) -> bool {
    tabs.len() >= 2 || body.contains('←') || body.contains('→') || tabs.iter().any(|t| t.is_submit)
}

/// タブのラベルとして許す最大文字数。
///
/// `AskUserQuestion` の `header` は短い見出し（`Color` / `URL表示` / `Submit`）で、
/// 仕様上も 12 文字までとされている。長いものは**設問文や todo の項目**なので、
/// 行頭がチェックボックスでもタブバーとして採らない（4 回目のセルフレビューで
/// 検出: `☐ の項目のうちどれを先にやりますか?` が本物のタブバーに勝っていた）。
const MAX_TAB_LABEL_CHARS: usize = 16;

/// タブバー**らしい行の形**をしているか。
///
/// 行頭（trim 後）が `←` かチェックボックスで始まること、ラベルが短いこと。
/// これで散文が落ちる（4 回目のセルフレビューで検出: `凡例: ☒ 完了 / ☐ 未完了` が
/// 「タブ 2 つ = 強い候補」として本物より優先されていた）。
fn looks_like_tab_bar_line(tabs: &[QuestionTab], body: &str) -> bool {
    let head = body.trim_start().chars().next();
    let starts_right = matches!(head, Some('←') | Some('☐') | Some('☑') | Some('☒'));
    starts_right && tabs.iter().all(|t| t.label.chars().count() <= MAX_TAB_LABEL_CHARS)
}

fn push_tab(tabs: &mut Vec<QuestionTab>, answered: bool, is_submit: bool, label: String) {
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        return;
    }
    tabs.push(QuestionTab { label, answered, is_submit });
}

/// プレビュー枠が始まる桁を探す（#264）。
///
/// 選択肢に `preview` が付くと、選択肢リストの**右側**へ枠が並ぶ:
///
/// ```text
/// ❯ 1. Grid                         ┌──────────────────────────┐
///   2. List                         │ ┌───┬───┬───┐            │
///                                   ├─── ✂ ─── 2 lines hidden ─┤
/// ```
///
/// 桁を見つけずに解析すると、枠がラベルへ食い込んだうえ
/// （`"Grid    ┌────────┐"`）、枠の続き行が「折り返した続き行」として
/// 直前の選択肢のラベルへ吸い込まれる（実測）。
///
/// # 誤検出は「選択肢のラベルを黙って切る」ので危険（セルフレビューで検出）
///
/// 「同じ桁に縦枠が 3 行以上」だけを条件にすると、次のどちらでも取り違える:
///
/// - 承認対象に混ざったツリー図（`┌── src` / `├── lib` …）
/// - 画面の上の方に残っている**別のダイアログの残骸**のプレビュー枠
///
/// どちらでも `Yes, and don't ask again for tree commands` が `Yes, and do` へ
/// 切り詰められ、**「以後無条件で承認」であることが隠れたまま人に承認させる。**
/// `truncated` は立たないので安全弁も効かない。
///
/// そこで 2 つで絞る:
///
/// 1. **探すのはいま解析している選択肢の並びの行だけ**（呼び出し側が範囲を渡す）。
///    画面上部の残骸もツリー図も選択肢の外なので、これで落ちる
/// 2. **枠の左に中身がある**こと。本物の横並びレイアウトでは枠の左に選択肢の
///    ラベルがある。ツリー図では枠の左は空白しかないので、切ると何も残らない
///
/// 桁は**表示幅**で数える。CJK は 1 文字 2 桁なので、文字数で数えるとラベルに
/// 日本語が混ざった行だけ桁がずれて候補が揃わない。
fn find_preview_column(lines: &[&str]) -> Option<usize> {
    const MIN_PREVIEW_COL: usize = 16;
    const MIN_PREVIEW_ROWS: usize = 3;
    const MIN_ROWS_WITH_CONTENT_LEFT: usize = 2;

    let mut counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for line in lines {
        if let Some(col) = first_box_column(line, MIN_PREVIEW_COL) {
            *counts.entry(col).or_insert(0) += 1;
        }
    }
    let mut candidates: Vec<usize> = counts
        .into_iter()
        .filter(|&(_, n)| n >= MIN_PREVIEW_ROWS)
        .map(|(col, _)| col)
        .collect();
    candidates.sort_unstable();

    candidates.into_iter().find(|&col| {
        let rows: Vec<&&str> = lines
            .iter()
            .filter(|line| first_box_column(line, MIN_PREVIEW_COL) == Some(col))
            .collect();
        // 枠の左に選択肢の本体があること（ツリー図では左は空白しかない）
        let with_content_left = rows
            .iter()
            .filter(|line| {
                let (body, _) = strip_frame(&cut_at_column(line, col));
                !body.is_empty() && !is_rule_line(&body)
            })
            .count();
        // 枠の右にプレビューの中身があること。**これが無いとダイアログ自身の
        // 右枠線を候補にしてしまう**（切る桁が右枠線なので実害は出ていなかったが、
        // 「プレビューがある」と誤って判断している状態には変わりない）
        let with_content_right = rows
            .iter()
            .filter(|line| {
                let cut = cut_at_column(line, col);
                let rest: String = line.chars().skip(cut.chars().count() + 1).collect();
                let (body, _) = strip_frame(&rest);
                !body.is_empty()
            })
            .count();
        with_content_left >= MIN_ROWS_WITH_CONTENT_LEFT
            && with_content_right >= MIN_ROWS_WITH_CONTENT_LEFT
    })
}

/// その行で**最初に**縦枠が現れる表示桁（`min_col` より左は見ない）。
///
/// 行の最初の枠だけを見るのは、右端の閉じ枠を候補にしないため。
fn first_box_column(line: &str, min_col: usize) -> Option<usize> {
    let mut col = 0usize;
    for c in line.chars() {
        if col >= min_col && matches!(c, '┌' | '│' | '├' | '└' | '┃' | '╭' | '╰') {
            return Some(col);
        }
        col += char_width(c);
    }
    None
}

/// 表示幅（桁数）。制御文字や結合文字は 0 桁として扱う。
fn char_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// 表示桁 `col` より右を落とす（プレビュー枠を切り離す）。
///
/// **文字数ではなく表示幅で切る。** CJK を含む行を文字数で切ると、枠の桁が
/// 揃っていても切り口が行ごとにずれる。
fn cut_at_column(line: &str, col: usize) -> String {
    let mut out = String::new();
    let mut w = 0usize;
    for c in line.chars() {
        let cw = char_width(c);
        if w + cw > col {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

/// 選択肢行（`❯ 1. Yes` / `  2) foo`）を解析する。
///
/// 返すのは `(番号, ラベル, ❯ が付いているか, ラベルが始まる桁, 番号が始まる桁)`。
/// 最後の「番号が始まる桁」は `❯` が描かれなかったときのカーソル推定に使う
/// （[`find_last_option_run`] 参照。#264）。
fn parse_option_line(line: &str) -> Option<(u32, String, bool, usize, usize)> {
    let (body, frame_offset) = strip_frame(line);
    if body.is_empty() {
        return None;
    }
    let (after_marker, has_cursor) = strip_cursor_marker(&body);
    let consumed_by_marker = body.chars().count() - after_marker.chars().count();

    let digits: String = after_marker.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let rest: String = after_marker.chars().skip(digits.len()).collect();
    let mut rest_chars = rest.chars();
    match rest_chars.next() {
        Some('.') | Some(')') => {}
        _ => return None,
    }
    let after_sep: String = rest_chars.collect();
    // `1.Yes` のような区切り無しは選択肢とみなさない（`1.5` などの誤検出を避ける）
    if !after_sep.starts_with(' ') {
        return None;
    }
    let label = after_sep.trim().to_string();
    if label.is_empty() {
        return None;
    }
    let label_col = frame_offset
        + consumed_by_marker
        + digits.len()
        + 1
        + (after_sep.chars().count() - after_sep.trim_start().chars().count());
    let index = digits.parse::<u32>().ok()?;
    Some((index, label, has_cursor, label_col, frame_offset + consumed_by_marker))
}

/// 番号の連番として成立している選択肢の並び。
#[derive(Debug)]
struct OptionRun {
    /// 画面上の開始行 / 終了行（終了行は inclusive。折り返し続き行を含む）
    start: usize,
    end: usize,
    options: Vec<PromptOption>,
    cursor_index: Option<u32>,
    /// 横並びのプレビュー枠を切り離した表示桁。**この run を解析したときの値**で、
    /// run の外（`find_unnumbered_escape`）でも同じ桁を使うために持ち回る。
    /// 画面全体から探し直すと別のダイアログの残骸の桁を拾う（#264）
    preview_col: Option<usize>,
}

/// 画面の**最後の**選択肢の並びを取り出す。
///
/// 再描画の残骸で同じダイアログが複数回現れることは画面グリッドでは起きないが、
/// 画面内に過去のダイアログのログが残っていることはある。**最後のものを採る。**
fn find_last_option_run(lines: &[&str]) -> Option<OptionRun> {
    // ── 2 パスで解く（#264）──────────────────────────────────────────────
    //
    // プレビュー枠の桁を**画面全体**から探すと、画面上部に残っている別の
    // ダイアログの残骸の桁を拾い、いま出ているダイアログの選択肢をそこで切って
    // しまう（セルフレビューで検出。`Yes, and don't ask again for …` が
    // `Yes, and do` になり、`truncated` も立たないので安全弁が効かない）。
    //
    // そこで一度**切らずに**選択肢の並びを取り、**その行範囲の中でだけ**桁を探す。
    // 枠なしで解析すると枠の続き行がラベルへ吸われて `end` が下へ伸びるので、
    // 枠の行はこの範囲に収まる。
    //
    // 探す範囲は run の下へ少しだけ伸ばす。枠の閉じ行（`└───┘`）は罫線だけの行なので
    // 継続行として吸われず run の外に出るが、桁を数えるには要る。**上へは伸ばさない**
    // （上にあるのが残骸なので、伸ばしたら塞いだ意味が無くなる）
    const PREVIEW_LOOKAHEAD: usize = 4;
    let plain = scan_option_runs(lines, None);
    let preview_col = plain.as_ref().and_then(|run| {
        let end = (run.end + 1 + PREVIEW_LOOKAHEAD).min(lines.len());
        find_preview_column(&lines[run.start..end])
    });
    match preview_col {
        Some(_) => scan_option_runs(lines, preview_col),
        None => plain,
    }
}

/// 選択肢の並びを走査して**最後のもの**を返す。
///
/// `preview_col` が `Some` なら各行をその表示桁で切ってから解析する
/// （横並びのプレビュー枠を切り離す）。
fn scan_option_runs(lines: &[&str], preview_col: Option<usize>) -> Option<OptionRun> {
    let cut = |line: &str| -> String {
        match preview_col {
            Some(col) => cut_at_column(line, col),
            None => line.to_string(),
        }
    };

    let mut runs: Vec<OptionRun> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        // 入力欄に `1. …` と打ち込んだ行を選択肢として拾わない（`is_input_box_line` 参照）
        if is_input_box_line(lines, i) {
            i += 1;
            continue;
        }
        let Some((index, label, has_cursor, label_col, num_col)) = parse_option_line(&cut(lines[i]))
        else {
            i += 1;
            continue;
        };
        // 連番は 1 から始まるものだけを選択肢の並びとみなす。`2.` から始まる断片を
        // 拾うと、選択肢の一部だけを見て移動量を誤る
        if index != 1 {
            i += 1;
            continue;
        }
        let start = i;
        let mut options = vec![PromptOption { index, label }];
        let mut num_cols = vec![num_col];
        let mut cursor_index = if has_cursor { Some(index) } else { None };
        let mut expected = 2u32;
        let mut current_label_col = label_col;
        let mut end = i;
        let mut j = i + 1;
        // 罫線をまたいで連番が続くことがある（実測: `3. Type something.` の下に
        // 罫線が引かれ、その下に `4. Chat about this` が来る）。またげるのは 1 回だけ
        let mut skipped_rule = false;
        while j < lines.len() {
            if let Some((idx, label, has_cursor, col, ncol)) = parse_option_line(&cut(lines[j])) {
                if idx != expected {
                    break;
                }
                if has_cursor {
                    cursor_index = Some(idx);
                }
                options.push(PromptOption { index: idx, label });
                num_cols.push(ncol);
                expected += 1;
                current_label_col = col;
                end = j;
                j += 1;
                skipped_rule = false;
                continue;
            }
            let (body, offset) = strip_frame(&cut(lines[j]));
            // 折り返された続き行: ラベルの桁位置以上に字下げされた非空行
            if !body.is_empty() && !is_rule_line(&body) && offset >= current_label_col {
                if let Some(last) = options.last_mut() {
                    last.label.push(' ');
                    last.label.push_str(&body);
                }
                end = j;
                j += 1;
                continue;
            }
            // 罫線 / 空行を 1 回だけまたいで連番の続きを探す（#264）。
            // **`end` は進めない** — 続きが見つからなければ run はここで終わる
            if !skipped_rule && (body.is_empty() || is_rule_line(&body)) {
                skipped_rule = true;
                j += 1;
                continue;
            }
            break;
        }
        // `❯` が 1 つも描かれていないとき、選択中の行だけマーカーぶん左へ寄っている
        // ことを手がかりにする（#264）。複数設問でタブが自動で進んだ直後、Claude Code は
        // `❯` を描き直さないが、行頭のスペースは 1 つぶん詰まったまま残る（実測）:
        //
        // ```text
        //  1. Large      ← 選択中（マーカーの桁が空白で埋まっている）
        //   2. Small
        // ```
        //
        // これを拾わないと `cursor_index` が `None` になり、矢印の移動量を決められず
        // **2 問目以降が一切答えられなくなる**。誤爆を避けるため
        // 「1 行だけが他より左」という形にきっちり当てはまるときしか採らない
        // **桁数の多い番号を巻き込まない。** 選択肢がちょうど 10 件で番号が
        // 右寄せ描画されると `10.` の行だけ 1 桁左から始まり、カーソル扱いになる。
        // 右寄せで左へ出るのは**桁が増えた番号**だけなので、外れ値の番号が
        // 他より桁数が多いなら採らない（4 回目のセルフレビューで検出）
        if cursor_index.is_none() && options.len() >= 2 {
            let base = *num_cols.iter().max().unwrap_or(&0);
            let outliers: Vec<usize> =
                (0..num_cols.len()).filter(|&k| num_cols[k] + 1 == base).collect();
            let others_aligned =
                (0..num_cols.len()).all(|k| outliers.contains(&k) || num_cols[k] == base);
            if outliers.len() == 1 && others_aligned {
                let k = outliers[0];
                let digits = |i: u32| i.to_string().len();
                let widest_other = (0..options.len())
                    .filter(|&j| j != k)
                    .map(|j| digits(options[j].index))
                    .max()
                    .unwrap_or(1);
                if digits(options[k].index) <= widest_other {
                    cursor_index = Some(options[k].index);
                }
            }
        }
        runs.push(OptionRun { start, end, options, cursor_index, preview_col });
        i = end + 1;
    }
    runs.pop()
}

// ─── 解析本体 ────────────────────────────────────────────────────────────────

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

/// 選択肢の並びの直後に**番号なし**で置かれた逃げ道を探す（#264）。
///
/// 実測: 選択肢に `preview` が付くレイアウトでは、`Chat about this` が番号を持たず
/// 罫線の下へ出る。
///
/// ```text
///   3. Type something.
/// ────────────────────────────────
///   Chat about this
/// ```
///
/// 拾わないと**画面に実在する選択肢を人へ見せられない**。文言を決め打ちにしてあるのは、
/// 「番号の無い行を選択肢として足す」を一般化すると、本文の断片を選択肢に化けさせて
/// しまうため（そのまま矢印を送れば別の選択肢を確定しうる）。
fn find_unnumbered_escape(
    lines: &[&str],
    run_end: usize,
    preview_col: Option<usize>,
) -> Option<String> {
    // 選択肢より下にもプレビュー枠が続くので、ここでも同じ桁で切る。
    // 切らないと枠の行が「別の内容」に見えて、その先の `Chat about this` に届かない。
    // **桁を探し直さず run のものを使う** — 探し直すと画面上部の残骸を拾う（#264）
    const LOOKAHEAD: usize = 10;
    for line in lines.iter().skip(run_end + 1).take(LOOKAHEAD) {
        let cut = match preview_col {
            Some(col) => cut_at_column(line, col),
            None => (*line).to_string(),
        };
        let (body, _) = strip_frame(&cut);
        if body.is_empty() || is_rule_line(&body) {
            continue;
        }
        // 既に番号付きなら run 側で拾えているはず。ここでは番号なしだけを見る
        if parse_option_line(&cut).is_some() {
            return None;
        }
        if contains_ci(&body, "chat about this") {
            return Some(body);
        }
        return None;
    }
    None
}

/// `AskUserQuestion` のフッタ（実測: `Enter to select · Tab/Arrow keys to navigate · Esc to cancel`）。
fn is_ask_user_question_footer(s: &str) -> bool {
    contains_ci(s, "enter to select") && contains_ci(s, "to navigate")
}

/// y/n マーカーの後ろに許す文字数。
///
/// 実物のプロンプトは `Continue? (y/N) ` / `Overwrite? [Y/n]: ` のように、マーカーの
/// **直後で入力待ちになる**。後ろに長い文字列が続く行はプロンプトではない。
const YESNO_TRAILING_SLACK: usize = 4;

/// 素の y/n プロンプトか。
///
/// **マーカーが行末付近にあることを要求する。** 単に含まれるだけで判定すると、
/// エコーされたコマンド行を拾ってしまう（実測: `bash -c 'read -p "… (y/N) " x; echo …;
/// sleep 300'` というコマンド行が `yesno` と判定された）。プロンプトでない行へ `y` +
/// CR を送ると、シェルが `y` をコマンドとして実行することになる。
fn is_yesno_line(s: &str) -> bool {
    let l = s.trim_end().to_lowercase();
    for marker in ["(y/n)", "[y/n]", "(yes/no)", "[yes/no]", "(y|n)"] {
        if let Some(pos) = l.rfind(marker) {
            let trailing = l.len() - (pos + marker.len());
            // マーカーの後ろに残るのは `: ` や ` ` 程度のはず
            if trailing <= YESNO_TRAILING_SLACK {
                return true;
            }
        }
    }
    false
}

/// 入力欄より下に出る行を何行まで読み飛ばすか。
///
/// Claude Code の入力欄の下にはステータス行（`⏵⏵ auto-accept edits on` / トークン数 /
/// `? for shortcuts`）が出る。1 行で打ち切ると入力欄に届かず `unknown` へ倒れる。
/// 一方で無制限に遡ると、スクロールバックに残った古いプロンプト行を拾って
/// 「自由入力できる」と誤判定するので、少数行に限る。
const FREE_INPUT_FOOTER_TOLERANCE: usize = 4;

/// 自由入力の受け手の種類。`shape: text` の fingerprint に混ぜる。
///
/// **混ぜないと `text` の fingerprint がどの画面でも同一になる。** そうなると
/// 「レポート生成時は Claude Code の入力欄だったが、送信時には CC が終了して同じ PTY に
/// シェルのプロンプトだけが残っている」状況で照合が通り、**返答テキストがシェルコマンドとして
/// 実行される**（セルフレビューで実測）。
///
/// **入力中のテキストは混ぜない。** 混ぜると、レポートを開いてから人が宛先の端末に何か
/// 打っただけで fingerprint が変わり、送信が常に `stale` になって使えなくなる。
/// そのため CC の入力欄は中身を捨てて種別だけを持ち、シェルは（安定している）
/// プロンプト行そのものを持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
enum FreeInputKind {
    /// Claude Code の入力欄（`❯` / `>` で始まる行）。中身は持たない
    ClaudeCodeBox,
    /// シェルのプロンプト（`PS X:\...>` / `$` / `#` 終わり）。行そのものを持つ
    ShellPrompt(String),
}

impl FreeInputKind {
    /// fingerprint と `header` に載せる表現。
    fn as_header(&self) -> String {
        match self {
            FreeInputKind::ClaudeCodeBox => "[Claude Code の入力欄]".to_string(),
            FreeInputKind::ShellPrompt(line) => format!("[シェルのプロンプト] {}", line),
        }
    }
}

/// Claude Code の入力欄、またはシェルのプロンプトが出ているか（＝自由入力できる）。
///
/// # 不変条件: 選択肢行は入力欄ではない
///
/// **ここを外すと #215 が防ごうとした事象そのものが起きる。** 許可ダイアログの
/// カーソル行 `❯ 2. Yes, and don't ask again …` は `❯` で始まるので、素朴に
/// 「`❯` で始まれば入力欄」と判定すると**開いているダイアログを自由入力と誤認**し、
/// 本文 + CR を撃ち込んで `❯` が指す選択肢を確定させてしまう（セルフレビューで検出）。
/// `1.` が画面外へ流れて連番の起点が見つからないダイアログで実際に起きた。
///
/// そこで**選択肢行として解析できる行は入力欄候補から除外する**。除外した結果
/// 入力欄が見つからなければ `parse_prompt` は `Unknown` へ倒れ、キーを一切送らない。
fn looks_like_free_input(tail: &[&str]) -> Option<FreeInputKind> {
    let mut examined = 0usize;
    for i in (0..tail.len()).rev() {
        let line = tail[i];
        let (body, _) = strip_frame(line);
        if body.is_empty() || is_rule_line(&body) {
            continue;
        }
        // **入力欄の判定を選択肢判定より先に行う。** 実機の入力欄は `❯1. …` のように
        // 選択肢行と同じ形になりうるので、位置（上下が罫線）で先に確定させる
        if is_input_box_line(tail, i) {
            return Some(FreeInputKind::ClaudeCodeBox);
        }
        // 選択肢行は入力欄ではない（上の不変条件）。ここで打ち切って `Unknown` へ倒す
        if parse_option_line(line).is_some() {
            return None;
        }
        // Claude Code のダイアログのフッタが**直近の意味のある行**なら、ダイアログが
        // 開いている。`escape_hatch` のように末尾 12 行を広く見ると、スクロールバックに
        // 残った古いダイアログの残骸でも立って自由入力を塞いでしまうので、ここだけに絞る
        if is_ask_user_question_footer(&body)
            || contains_ci(&body, "tab to amend")
            || contains_ci(&body, "esc to cancel")
        {
            return None;
        }
        // Claude Code の入力欄。`❯` / `>` のあとは入力中のテキスト（空でもよい）。
        // **中身は捨てる**（人が打っただけで fingerprint が変わると常に stale になる）
        let first = body.chars().next();
        if matches!(first, Some('❯') | Some('>')) {
            return Some(FreeInputKind::ClaudeCodeBox);
        }
        // シェルのプロンプト（`PS X:\...>` / `$` / `#`）。行は安定なのでそのまま持つ
        if body.ends_with('>') || body.ends_with('$') || body.ends_with('#') {
            return Some(FreeInputKind::ShellPrompt(body));
        }
        examined += 1;
        if examined > FREE_INPUT_FOOTER_TOLERANCE {
            return None;
        }
    }
    None
}

/// 画面テキストから問いを解析する（純粋関数）。
pub fn parse_prompt(screen: &str) -> ParsedPrompt {
    let lines: Vec<&str> = screen.lines().collect();
    let tail_start = lines.len().saturating_sub(TAIL_WINDOW);
    let tail_lines = &lines[tail_start..];
    let tail = tail_lines
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string();

    // 折り返しに耐えるよう、末尾をまとめて 1 つの文字列にしてから照合する
    // （13 桁のターミナルでは `Esc to` / `cancel ·` に割れる）
    let tail_joined = tail_lines
        .iter()
        .map(|l| strip_frame(l).0)
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let escape_hatch = if contains_ci(&tail_joined, "esc to cancel") {
        Some("esc".to_string())
    } else {
        None
    };

    let run = find_last_option_run(&lines);

    let Some(run) = run else {
        // 選択肢が無い画面。y/n → 自由入力 → unknown の順で倒す
        let last_meaningful = tail_lines
            .iter()
            .rev()
            .map(|l| strip_frame(l).0)
            .find(|b| !b.is_empty() && !is_rule_line(b));
        if last_meaningful.as_deref().is_some_and(is_yesno_line) {
            let header = last_meaningful.unwrap_or_default();
            return seal(ParsedPrompt {
                shape: PromptShape::YesNo,
                navigation: Navigation::None,
                header: header.clone(),
                context: String::new(),
                questions: vec![PromptQuestion {
                    header: header.clone(),
                    question: header,
                    multi_select: false,
                    options: Vec::new(),
                    allow_other: false,
                    cursor_index: None,
                }],
                tabs: Vec::new(),
                escape_hatch,
                truncated: false,
                fingerprint: String::new(),
                tail,
            });
        }
        if let Some(kind) = looks_like_free_input(tail_lines) {
            return seal(ParsedPrompt {
                shape: PromptShape::Text,
                navigation: Navigation::None,
                // 受け手の種類を載せる（fingerprint に効く。上の `FreeInputKind` 参照）
                header: kind.as_header(),
                context: String::new(),
                questions: Vec::new(),
                tabs: Vec::new(),
                escape_hatch,
                truncated: false,
                fingerprint: String::new(),
                tail,
            });
        }
        return seal(ParsedPrompt {
            shape: PromptShape::Unknown,
            navigation: Navigation::None,
            header: String::new(),
            context: String::new(),
            questions: Vec::new(),
            tabs: Vec::new(),
            escape_hatch,
            truncated: false,
            fingerprint: String::new(),
            tail,
        });
    };

    // **フッタも折り返す。** 行ごとに照合すると、13 桁のターミナルでは
    // `Esc to` / `cancel ·` / `Tab to` / `amend` に割れてどのパターンにも当たらず、
    // Claude Code のダイアログを素の番号リスト扱い（数字キー）に落としてしまう。
    // 選択肢より下をまとめて 1 つの文字列にしてから照合する
    // **`run.end` は inclusive なので +1 する。** 含めると最後の選択肢ラベル自身が
    // フッタ判定に混ざり、`Esc to cancel` を含むラベルを持つ素の番号リストが
    // Claude Code のダイアログに化けて矢印キーを送ってしまう（セルフレビューで検出）
    let below_text = lines[(run.end + 1).min(lines.len())..]
        .iter()
        .take(TAIL_WINDOW)
        .map(|l| strip_frame(l).0)
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let below_has_aq_footer = is_ask_user_question_footer(&below_text);
    // `Tab to amend` は許可ダイアログ固有のフッタ。`Esc to cancel` は Claude Code の
    // ダイアログ全般に出る。**どちらかがあれば矢印で答える**（数字ではない）
    let has_amend_footer = contains_ci(&below_text, "tab to amend");

    // 選択肢の上にある見出しと承認対象を拾う
    let ctx_start = run.start.saturating_sub(CONTEXT_WINDOW);
    let mut above: Vec<String> = lines[ctx_start..run.start]
        .iter()
        .map(|l| strip_frame(l).0)
        .collect();

    // ── タブバーより上はダイアログの外（＝スクロールバック）────────────────
    //
    // タブバーが見つかったら**そこで切る**。切らないと、その上に残っている
    // ユーザーのプロンプトのエコーや直前のツール出力まで `context` に入り、
    // カードが画面のダンプになる（実測。#264）。
    //
    // **`☐` / `☒` を含む行なら何でもタブバー、にはしない（→ `is_strong_tab_bar`）。**
    // TodoWrite パネルが同じ記号を使うため、選択肢の上に todo が出ているだけで
    // 素の番号リストが `AskUserQuestion` に化け、矢印キーを送ることになる。
    // タブが 1 つだけの形（単一設問）は強い形と区別できないので、
    // **`AskUserQuestion` のフッタが出ていて、かつ選択肢のすぐ上にある**ことも求める。
    //
    // **下から見て最初に「タブバーらしくて受理できる」行を採る。**
    //
    // 「強い候補を画面全体から先に探す」形にすると、本物より上にある偽の強い候補
    // （`凡例: ☒ 完了 / ☐ 未完了` など）が、選択肢の直上にある本物に勝つ
    // （4 回目のセルフレビューで検出）。逆に「下から最初の受理可能な行」だけだと、
    // 本物より下の設問文に含まれる `☐` が勝つ（2 回目のセルフレビューで検出）。
    //
    // 両方を塞ぐのは**行の形**の条件（`looks_like_tab_bar_line`）で、
    // 散文はここで落ちる。そのうえで下から最初のものを採る。
    let tabs: Vec<QuestionTab> = above
        .iter()
        .enumerate()
        .rev()
        .find_map(|(row, body)| {
            let parsed = parse_tab_bar(body)?;
            if !looks_like_tab_bar_line(&parsed, body) {
                return None;
            }
            if is_strong_tab_bar(&parsed, body) {
                return Some((row, parsed));
            }
            // タブが 1 つだけの単一設問。TodoWrite の 1 行と形が同じなので、
            // `AskUserQuestion` のフッタと選択肢からの近さで裏を取る
            let close_enough = above.len() - 1 - row <= TAB_BAR_MAX_GAP;
            if below_has_aq_footer && close_enough {
                Some((row, parsed))
            } else {
                None
            }
        })
        .map(|(row, parsed)| {
            above.drain(..=row);
            parsed
        })
        .unwrap_or_default();

    // ── タブバーを読めなかった確認画面の救済（#264。3 回目のセルフレビューで検出）──
    //
    // 全問答えたあとの確認画面には Claude Code のフッタが無い。タブバーだけが
    // 「これは Claude Code のダイアログだ」という印なので、それが画面外へ流れたり
    // `CONTEXT_WINDOW`（24 行）より上へ押し出されたりすると**素の番号リストへ
    // 落ちる**。そうなると `2. Cancel` を選んだつもりで数字キーが飛び、確定キー
    // ではない数字のあとの CR が `❯` の当たっている `1. Submit answers` を
    // 確定する（＝押していない方が通る）。
    //
    // 見出しと `Submit answers` という並びは確認画面固有なので、これ自体を印にする。
    //
    // **見出しは折り返す。** しかもこの救済が要る状況（狭い / 短いタブ）は
    // まさに `Ready to submit your answers?` が複数行へ割れる状況なので、
    // 最後の 1 行だけを見ると**救済が必要なときに限って発火しない**
    // （4 回目のセルフレビューで検出）。末尾の非空ブロックを連結してから照合する。
    let (block_heading, block_context) = split_heading_and_context(&above);
    let submit_review = contains_ci(&block_heading, SUBMIT_REVIEW_HEADING)
        && run
            .options
            .iter()
            .any(|o| o.label.to_lowercase().starts_with("submit"));

    // **見出しは折り返しうる。** 選択肢の直上にある連続した非空行をまとめて 1 つの
    // 見出しとして扱う（実測: 13 桁のターミナルでは `Do you want to proceed?` が
    // `Do you want` / `to` / `proceed?` の 3 行に割れた）。1 行だけ拾うと `proceed?` に
    // なって形状の推定が外れる。通常の幅では空行で区切られた 1 行になるので影響は無い
    let mut block_start = above.len();
    while block_start > 0
        && above.len() - block_start < HEADER_WRAP_LINES
        && !above[block_start - 1].is_empty()
        && !is_rule_line(&above[block_start - 1])
    {
        block_start -= 1;
    }
    let (header, context) = if tabs.is_empty() && !submit_review {
        let header = above[block_start..]
            .iter()
            .map(|b| b.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // 見出しより上の非空行が「何を承認するのか」。全文をカードへ出す
        let context = above[..block_start]
            .iter()
            .filter(|b| !b.is_empty() && !is_rule_line(b))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        (header, context)
    } else {
        // タブバーがある画面（`AskUserQuestion`）と、タブバーを読めなかった
        // 確認画面。**末尾の非空ブロックが設問文（確認画面なら見出し）**で、
        // その上は補足（`Review your answers` / `● Which color? → Red` など）。
        // 折り返しに耐えるようブロックごと連結する
        (block_heading, block_context)
    };

    let footer_says_cc =
        below_has_aq_footer || has_amend_footer || contains_ci(&below_text, "esc to cancel");
    // **タブバーもフッタと同格の印**（#264）。全問答えたあとの確認画面
    // （`Review your answers` / `❯ 1. Submit answers`）には Claude Code のフッタが
    // 出ないため、フッタだけを見ると素の番号リストへ落ちて数字キーを送ってしまい、
    // **確定できないまま止まる**（実測）。
    let has_cc_footer = footer_says_cc || !tabs.is_empty() || submit_review;

    // 罫線の下に番号なしで置かれた逃げ道を拾う（#264）。実測: 選択肢に preview が
    // 付くレイアウトでは `Chat about this` が番号を持たずに罫線の下へ出る。
    // **矢印の移動量は画面の並び順で決まる**ので、続きの番号を振っておけば
    // そのまま選べる。Claude Code のダイアログだと分かっているときだけ足す
    let mut options = run.options;
    if has_cc_footer {
        if let Some(label) = find_unnumbered_escape(&lines, run.end, run.preview_col) {
            let next = options.last().map(|o| o.index + 1).unwrap_or(1);
            options.push(PromptOption { index: next, label });
        }
    }
    let last_label = options.last().map(|o| o.label.as_str()).unwrap_or("");
    // 実測: `AskUserQuestion` の逃げ道は `Other` ではなく `Chat about this`
    let has_chat_about_this = contains_ci(last_label, "chat about this");

    // **Claude Code のフッタが無く、その下に入力欄があるなら選択プロンプトではない。**
    // CC が本文中に散文の番号リスト（`1. まず設計する` …）を出したあと入力欄で待っている
    // 画面を「番号選択ダイアログ」と誤分類すると、(a) 本文の返答 (`kind="text"`) が
    // 拒否されて従来の返答経路が死に、(b) カードが散文行を「画面に実在する選択肢」として
    // 提示してしまう（セルフレビューで検出）。
    //
    // `looks_like_free_input` は末尾から見て**選択肢行に当たった時点で false** を返すので、
    // 選択肢が画面最下部にある本物のダイアログではここが true になることはない。
    if !has_cc_footer {
        if let Some(kind) = looks_like_free_input(tail_lines) {
            return seal(ParsedPrompt {
                shape: PromptShape::Text,
                navigation: Navigation::None,
                header: kind.as_header(),
                context: String::new(),
                questions: Vec::new(),
                tabs: Vec::new(),
                escape_hatch,
                truncated: false,
                fingerprint: String::new(),
                tail,
            });
        }
    }

    let shape = if below_has_aq_footer || has_chat_about_this || !tabs.is_empty() || submit_review {
        PromptShape::AskUserQuestion
    } else if has_amend_footer || header.to_lowercase().starts_with("do you want to") {
        PromptShape::Permission
    } else if contains_ci(&header, "would you like to proceed") {
        PromptShape::Plan
    } else if has_cc_footer {
        // Claude Code のダイアログだが見出しから種類を特定できなかった。
        // **キーの種類は当てられている**ので `permission` として扱い、承認対象を全文出す
        // （名前を当てられないことより、矢印で答えられないことのほうが害が大きい）
        PromptShape::Permission
    } else {
        PromptShape::Numbered
    };
    // ここが要点: キーの種類は見出しの一致ではなく Claude Code のフッタで決める。
    // 見出しが折り返して形状の推定を外しても、キーの種類までは間違えない
    let navigation = if has_cc_footer {
        Navigation::Arrows
    } else {
        Navigation::Digits
    };

    // ── ダイアログが画面に収まっているか ────────────────────────────────────
    //
    // **画面グリッドには「画面に出ているぶん」しか無い。** ダイアログが画面より高いと
    // 上側が流れ、読めた選択肢だけが `options` に入る。それを完全な一覧として人へ見せると
    // 「`1. Yes` しか無い」と誤認させ、拒否の選択肢を見ないまま承認させることになる。
    //
    // 実測: dev インスタンスの 7 行 13 桁のタブでは、許可ダイアログの見出しと `2.` 以降が
    // すべて画面外へ流れて `options` が `[{1, "Yes"}]` だけになった。
    //
    // 手がかりは 2 つ:
    // - 選択肢が画面の先頭行から始まっている（＝その上にあったはずの枠と見出しが無い）
    // - Claude Code のダイアログなのに選択肢が 1 つしか無い（実物は必ず 2 つ以上ある。
    //   `Yes` だけの許可ダイアログは存在しない）
    let clipped_at_top = run.start == 0;
    let implausibly_few = has_cc_footer && options.len() < 2;
    let truncated = clipped_at_top || implausibly_few;

    let allow_other = has_chat_about_this
        || contains_ci(last_label, "tell claude what to do differently")
        || contains_ci(last_label, "other");

    // `AskUserQuestion` は見出しではなく設問文が上に来る。header と context を
    // そのまま設問として持たせる（見出しチップは画面から確実に切り出せない）
    let question = if shape == PromptShape::AskUserQuestion && !context.is_empty() {
        format!("{}\n{}", context, header).trim().to_string()
    } else {
        header.clone()
    };

    seal(ParsedPrompt {
        shape,
        navigation,
        header,
        context,
        questions: vec![PromptQuestion {
            // タブバーがあれば、いま開いているタブの見出しがそのまま設問の見出し。
            // どれが「いま」かは画面の色でしか区別できないので、**未回答の先頭**を採る
            // （1 問答えると自動で次の未回答タブへ進む挙動に一致する。#264）
            header: tabs
                .iter()
                .find(|t| !t.answered && !t.is_submit)
                .map(|t| t.label.clone())
                .unwrap_or_default(),
            question,
            multi_select: false,
            options,
            allow_other,
            cursor_index: run.cursor_index,
        }],
        tabs,
        escape_hatch,
        truncated,
        fingerprint: String::new(),
        tail,
    })
}

/// `fingerprint` を計算して封をする。
///
/// 呼び出し側は `fingerprint` を空にした [`ParsedPrompt`] を組んでここへ渡す。
/// **fingerprint を自分で埋めた `ParsedPrompt` を作らないこと** — 照合の土台が
/// 1 か所に無いと、フィールドを足したときに fingerprint へ混ぜ忘れる。
fn seal(mut parsed: ParsedPrompt) -> ParsedPrompt {
    parsed.fingerprint = fingerprint_of(&parsed);
    parsed
}

/// 画面の同一性キー。
///
/// **`❯` の位置を必ず含める。** キー列は「いまの `❯` から目標まで矢印を n 回」なので、
/// 位置が動いていたら移動量が変わる。fingerprint が一致するなら移動量も同じ、を保証する。
/// `tail` は含めない（スピナーの 1 コマで毎回変わってしまい、常に `stale` になる）。
fn fingerprint_of(parsed: &ParsedPrompt) -> String {
    let mut hasher = Sha256::new();
    hasher.update(parsed.shape.as_str().as_bytes());
    hasher.update([0x1f]);
    // キーの種類が変われば「送るもの」が変わる。同じ画面として扱ってはいけない
    hasher.update(format!("{:?}", parsed.navigation).as_bytes());
    hasher.update([0x1f]);
    // 画面が広がって選択肢が全部見えるようになったら「別の画面」として扱い、
    // 切れていたときの fingerprint での送信を stale で弾く
    hasher.update(if parsed.truncated { b"trunc" as &[u8] } else { b"full" });
    hasher.update([0x1f]);
    hasher.update(parsed.header.as_bytes());
    hasher.update([0x1f]);
    hasher.update(parsed.context.as_bytes());
    // タブの回答状況が変われば「次に答える設問」が変わる。1 問答えた直後の画面を
    // 前の fingerprint で撃たせないために混ぜる（#264）
    for t in &parsed.tabs {
        hasher.update([0x1c]);
        hasher.update(t.label.as_bytes());
        hasher.update([b':']);
        hasher.update(if t.answered { b"x" as &[u8] } else { b"-" });
    }
    for q in &parsed.questions {
        hasher.update([0x1e]);
        hasher.update(q.question.as_bytes());
        hasher.update([0x1f]);
        hasher.update(
            q.cursor_index.map(|i| i.to_string()).unwrap_or_else(|| "-".into()).as_bytes(),
        );
        for o in &q.options {
            hasher.update([0x1d]);
            hasher.update(o.index.to_string().as_bytes());
            hasher.update([b':']);
            hasher.update(o.label.as_bytes());
        }
    }
    let digest = hasher.finalize();
    digest.iter().take(16).map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実物のツール許可ダイアログ。`src/utils/autoApproval.test.ts` の `ccPrompt()` と
    /// 同じ形（あちらは xterm バッファから採ったもの）に枠線を足してある。
    fn permission_screen(subject: &str, cwd: &str) -> String {
        [
            "╭──────────────────────────────────────────────────╮",
            "│ Bash command                                     │",
            "│                                                  │",
            &format!("│   {:<46} │", subject),
            "│   Print a probe marker                           │",
            "│                                                  │",
            "│ Do you want to proceed?                          │",
            "│ ❯ 1. Yes                                         │",
            &format!("│   2. Yes, and don't ask again for echo commands  │"),
            &format!("│      in {:<40} │", cwd),
            "│   3. No, and tell Claude what to do differently   │",
            "│      (esc)                                       │",
            "╰──────────────────────────────────────────────────╯",
            "  Esc to cancel · Tab to amend",
        ]
        .join("\n")
    }

    fn plan_screen() -> String {
        [
            "╭─────────────────────────────────────────╮",
            "│ Ready to code?                          │",
            "│                                         │",
            "│ Here is the plan:                       │",
            "│   1. touch a file                       │",
            "│                                         │",
            "│ Would you like to proceed?              │",
            "│   1. Yes, and auto-accept edits         │",
            "│ ❯ 2. Yes, and manually approve edits    │",
            "│   3. No, keep planning                  │",
            "╰─────────────────────────────────────────╯",
            "  Esc to cancel",
        ]
        .join("\n")
    }

    /// 実測したフッタと `Chat about this` を持つ `AskUserQuestion` 画面。
    fn ask_user_question_screen() -> String {
        [
            "╭─────────────────────────────────────────╮",
            "│ Which approach should we take?          │",
            "│                                         │",
            "│ ❯ 1. Xx                                 │",
            "│   2. Yy                                 │",
            "│   3. Zz                                 │",
            "│   4. Chat about this                    │",
            "╰─────────────────────────────────────────╯",
            "  Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
        ]
        .join("\n")
    }

    fn free_input_screen() -> String {
        [
            "  ⎿  done",
            "",
            "╭─────────────────────────────────────────╮",
            "│ >                                       │",
            "╰─────────────────────────────────────────╯",
            "  ⏵⏵ auto-accept edits on",
        ]
        .join("\n")
    }

    #[test]
    fn detects_permission_dialog() {
        let p = parse_prompt(&permission_screen(
            "echo probe-215",
            "X:\\devel\\worktree\\oretachi-y14b",
        ));
        assert_eq!(p.shape, PromptShape::Permission);
        assert_eq!(p.header, "Do you want to proceed?");
        // 何を承認するのかが全文入っている
        assert!(p.context.contains("Bash command"), "context={:?}", p.context);
        assert!(p.context.contains("echo probe-215"), "context={:?}", p.context);
        let q = &p.questions[0];
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Yes");
        assert_eq!(q.cursor_index, Some(1));
        assert!(q.allow_other);
        assert_eq!(p.escape_hatch.as_deref(), Some("esc"));
    }

    #[test]
    fn joins_wrapped_option_labels() {
        let p = parse_prompt(&permission_screen("echo hi", "X:\\devel\\worktree\\oretachi-y14b"));
        // 2 行に折り返された選択肢 2 が 1 つのラベルとして繋がっている
        assert!(
            p.questions[0].options[1].label.contains("in X:\\devel\\worktree\\oretachi-y14b"),
            "label={:?}",
            p.questions[0].options[1].label
        );
        assert!(p.questions[0].options[2].label.contains("(esc)"));
    }

    #[test]
    fn detects_plan_dialog_with_cursor_not_on_first() {
        let p = parse_prompt(&plan_screen());
        assert_eq!(p.shape, PromptShape::Plan);
        assert_eq!(p.questions[0].cursor_index, Some(2));
        assert_eq!(p.questions[0].options.len(), 3);
    }

    #[test]
    fn detects_ask_user_question_by_real_footer() {
        let p = parse_prompt(&ask_user_question_screen());
        assert_eq!(p.shape, PromptShape::AskUserQuestion);
        assert_eq!(p.questions[0].options.len(), 4);
        assert!(p.questions[0].allow_other);
        assert!(p.questions[0].question.contains("Which approach"));
        // 複数選択は判別根拠が無いので常に false
        assert!(!p.questions[0].multi_select);
    }

    #[test]
    fn detects_free_input() {
        let p = parse_prompt(&free_input_screen());
        assert_eq!(p.shape, PromptShape::Text);
    }

    // ── 複数設問の AskUserQuestion（#264。すべて実機の画面から採った）──────────

    /// 1 問目。タブバーの上にはユーザーのプロンプトのエコーが残っている。
    fn multi_question_first_screen() -> String {
        [
            "❯ AskUserQuestion ツールを1回だけ呼んで、questions に2問まとめて入れてください。1問目: header 'Color'",
            "'Red'/'Blue'。2問目: header 'Size' question 'Which size?' options 'Large'/'Small'。",
            "←  ☐ Color  ☐ Size  ✔ Submit  →",
            "Which color?",
            "",
            "❯ 1. Red",
            "     赤を選択します。",
            "  2. Blue",
            "     青を選択します。",
            "  3. Type something.",
            "────────────────────────────────────────",
            "  4. Chat about this",
            "",
            "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
        ]
        .join("\n")
    }

    /// 1 問答えて自動でタブが進んだ直後。**`❯` が描き直されていない。**
    fn multi_question_second_screen() -> String {
        [
            "←  ☒ Color  ☐ Size  ✔ Submit  →",
            "Which size?",
            "",
            " 1. Large",
            "     大きいサイズを選択します。",
            "  2. Small",
            "     小さいサイズを選択します。",
            "  3. Type something.",
            "────────────────────────────────────────",
            "  4. Chat about this",
            "",
            "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
        ]
        .join("\n")
    }

    /// 全問答えたあとの確認画面。**Claude Code のフッタが出ない。**
    fn submit_review_screen() -> String {
        [
            "←  ☒ Color  ☒ Size  ✔ Submit  →",
            "Review your answers",
            "",
            " ● Which color?",
            "   → Red",
            " ● Which size?",
            "   → Large",
            "",
            "Ready to submit your answers?",
            "",
            "❯ 1. Submit answers",
            "  2. Cancel",
        ]
        .join("\n")
    }

    /// 選択肢に preview が付くと、右へプレビュー枠が並ぶ横並びレイアウトになる。
    /// `Chat about this` は罫線の下に**番号なし**で置かれる。
    fn preview_question_screen() -> String {
        [
            "←  ☐ Layout  ☐ Theme  ✔ Submit  →",
            "Which layout?",
            "",
            "❯ 1. Grid                         ┌──────────────────────────────────────────┐",
            "  2. List                         │ ┌───┬───┬───┐                            │",
            "                                  ├─── ✂ ─── 2 lines hidden ─────────────────┤",
            "                                  └──────────────────────────────────────────┘",
            "",
            "                                  Notes: press n to add notes",
            "",
            "────────────────────────────────────────",
            "  Chat about this",
            "",
            "Enter to select · ↑/↓ to navigate · n to add notes · Tab to switch questions · Esc to cancel",
        ]
        .join("\n")
    }

    #[test]
    fn reads_the_question_tab_bar() {
        let p = parse_prompt(&multi_question_first_screen());
        assert_eq!(p.shape, PromptShape::AskUserQuestion);
        assert_eq!(p.navigation, Navigation::Arrows);
        let labels: Vec<&str> = p.tabs.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["Color", "Size", "Submit"]);
        assert!(!p.tabs[0].answered);
        assert!(p.tabs[2].is_submit);
        // いま開いているタブ（未回答の先頭）の見出しが設問の見出しになる
        assert_eq!(p.questions[0].header, "Color");
    }

    /// タブバーの上に残ったスクロールバックを設問に混ぜない（#264）。
    ///
    /// 混ぜるとカードが画面のダンプになり、人が何を聞かれているのか読めなくなる。
    #[test]
    fn keeps_scrollback_out_of_the_question() {
        let p = parse_prompt(&multi_question_first_screen());
        assert_eq!(p.header, "Which color?");
        assert_eq!(p.context, "");
        assert!(
            !p.questions[0].question.contains("AskUserQuestion ツールを"),
            "question={:?}",
            p.questions[0].question
        );
    }

    /// 罫線の下に続く番号付きの選択肢を落とさない（#264）。
    #[test]
    fn picks_up_a_numbered_option_below_a_rule() {
        let p = parse_prompt(&multi_question_first_screen());
        let labels: Vec<&str> = p.questions[0].options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels.len(), 4, "options={:?}", labels);
        assert_eq!(labels[3], "Chat about this");
        assert!(p.questions[0].allow_other);
        assert_eq!(p.questions[0].cursor_index, Some(1));
    }

    /// `❯` が描き直されていない画面でもカーソルを復元する（#264）。
    ///
    /// 復元できないと矢印の移動量が決まらず、**2 問目以降が一切答えられない**。
    /// 手がかりは「選択中の行だけマーカーぶん左に寄っている」こと。
    #[test]
    fn recovers_the_cursor_when_the_marker_is_not_drawn() {
        let p = parse_prompt(&multi_question_second_screen());
        assert_eq!(p.shape, PromptShape::AskUserQuestion);
        assert_eq!(p.questions[0].cursor_index, Some(1));
        assert!(p.tabs[0].answered, "1 問目は ☒ になっている");
        assert!(!p.tabs[1].answered);
        assert_eq!(p.questions[0].header, "Size");
        // 復元できていれば矢印の移動量が組める
        let keys = plan_keys(&p, &Answer::Select { option_index: 2 }).expect("select");
        assert_eq!(keys_preview(&keys), vec!["Down", "CR"]);
    }

    /// マーカーが 1 つも無く、字下げも揃っている画面ではカーソルを推測しない。
    #[test]
    fn does_not_guess_a_cursor_when_every_option_is_aligned() {
        let p = parse_prompt(
            "Pick a target:\n  1) staging\n  2) production\nSelection: ",
        );
        assert_eq!(p.questions[0].cursor_index, None);
    }

    /// 確認画面は Claude Code のフッタを出さない。**タブバーで見分ける。**
    ///
    /// 見分けられないと素の番号リスト扱いになって数字キーを送り、
    /// Claude Code では確定しないので回答が宛先へ渡らない（実測）。
    #[test]
    fn the_submit_review_screen_is_a_claude_code_dialog() {
        let p = parse_prompt(&submit_review_screen());
        assert_eq!(p.shape, PromptShape::AskUserQuestion);
        assert_eq!(p.navigation, Navigation::Arrows, "数字キーでは確定しない");
        assert_eq!(p.header, "Ready to submit your answers?");
        assert!(p.context.contains("Which color?"), "context={:?}", p.context);
        assert!(p.tabs.iter().all(|t| t.is_submit || t.answered));
        assert_eq!(p.questions[0].options[0].label, "Submit answers");
        let keys = plan_keys(&p, &Answer::Select { option_index: 1 }).expect("submit");
        assert_eq!(keys_preview(&keys), vec!["CR"]);
    }

    /// 横並びのプレビュー枠を選択肢のラベルから切り離す（#264）。
    ///
    /// 切らないと `"Grid    ┌────────┐"` のようなラベルになり、枠の続き行まで
    /// 直前の選択肢へ吸い込まれる。
    #[test]
    fn strips_the_preview_panel_from_option_labels() {
        let p = parse_prompt(&preview_question_screen());
        assert_eq!(p.shape, PromptShape::AskUserQuestion);
        let labels: Vec<&str> = p.questions[0].options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec!["Grid", "List", "Chat about this"]);
        assert_eq!(p.questions[0].cursor_index, Some(1));
        assert!(p.questions[0].allow_other);
        assert_eq!(p.header, "Which layout?");
    }

    /// プレビューの無い普通のダイアログを preview 判定に巻き込まない。
    #[test]
    fn a_plain_permission_dialog_has_no_preview_column() {
        let p = parse_prompt(&permission_screen("echo probe-264", "Print a probe"));
        assert_eq!(p.shape, PromptShape::Permission);
        assert!(p.tabs.is_empty());
        assert_eq!(p.questions[0].options.len(), 3);
    }

    /// 承認対象に混ざったツリー図をプレビュー枠と取り違えない（セルフレビューで検出）。
    ///
    /// 取り違えるとその桁で選択肢のラベルが切られ、
    /// `Yes, and don't ask again for tree commands` が `Yes, and do` になる。
    /// **`truncated` は立たないので安全弁も効かず**、「以後無条件で承認」であることが
    /// 隠れたまま人に承認させることになる。
    #[test]
    fn a_tree_diagram_in_the_context_is_not_a_preview_panel() {
        let screen = [
            "Bash command",
            "tree -L 2 src",
            "                ┌── src",
            "                ├── lib",
            "                ├── bin",
            "                └── tests",
            "",
            "Do you want to proceed?",
            "",
            "❯ 1. Yes",
            "  2. Yes, and don't ask again for tree commands",
            "  3. No, and tell Claude what to do differently (esc)",
            "",
            "  Esc to cancel · Tab to amend",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Permission);
        let labels: Vec<&str> = p.questions[0].options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Yes",
                "Yes, and don't ask again for tree commands",
                "No, and tell Claude what to do differently (esc)",
            ],
            "ツリー図の桁でラベルが切られている"
        );
        assert!(p.questions[0].allow_other);
    }

    // ── 複数設問の一括回答の手順（#264）──────────────────────────────────────

    #[test]
    fn select_all_answers_the_first_unanswered_tab() {
        let p = parse_prompt(&multi_question_second_screen());
        assert_eq!(p.tabs.len(), 3);
        // 1 問目は ☒ なので 2 問目（qidx=1）へ 2 番目の選択肢を送る
        let step = plan_select_all_step(&p, &[1, 2], None);
        assert_eq!(step, SelectAllStep::Answer { qidx: 1, option_index: 2 });
    }

    #[test]
    fn select_all_confirms_on_the_review_screen() {
        let p = parse_prompt(&submit_review_screen());
        let step = plan_select_all_step(&p, &[1, 1], Some(1));
        assert_eq!(step, SelectAllStep::Submit { option_index: 1 });
    }

    #[test]
    fn select_all_is_done_when_the_dialog_closed() {
        let p = parse_prompt(&free_input_screen());
        assert_eq!(plan_select_all_step(&p, &[1], Some(0)), SelectAllStep::Done);
    }

    /// **同じ設問へ 2 回送らない。**
    ///
    /// CR がまだ処理されていない中間画面を「進んだ」と読んでしまうと、同じ設問が
    /// もう一度来る。そこで送ると、遅れて確定した**次の設問**の既定選択肢を
    /// 確定してしまう（セルフレビューで検出した経路）。
    #[test]
    fn select_all_refuses_to_answer_the_same_question_twice() {
        let p = parse_prompt(&multi_question_first_screen());
        // 1 問目（qidx=0）を送った直後、まだ画面が進んでいない
        match plan_select_all_step(&p, &[1, 2], Some(0)) {
            SelectAllStep::Refuse(reason) => {
                assert!(reason.contains("進みませんでした"), "reason={}", reason);
            }
            other => panic!("送ってはいけない: {:?}", other),
        }
    }

    /// 設問数と渡された回答数がずれていたら**何も送らない**。
    ///
    /// ずれたまま送ると、ある設問へ別の設問の答えが入る。`request.questions` は
    /// レポート生成側の AI が書き起こすデータなので、設問を落とす事故が現実にありうる。
    #[test]
    fn select_all_refuses_when_the_answer_count_does_not_match() {
        let p = parse_prompt(&multi_question_first_screen());
        match plan_select_all_step(&p, &[1], None) {
            SelectAllStep::Refuse(reason) => {
                assert!(reason.contains("2 問"), "reason={}", reason);
                assert!(reason.contains("1 件"), "reason={}", reason);
            }
            other => panic!("送ってはいけない: {:?}", other),
        }
    }

    /// 「進んだ」を fingerprint の差分で測らない（セルフレビューで検出）。
    ///
    /// 矢印だけ届いて CR がまだ処理されていない中間画面は fingerprint が変わるが、
    /// **設問は 1 つも片付いていない**。ここを取り違えると同じ設問へ CR を二重に送る。
    #[test]
    fn moving_the_cursor_alone_is_not_progress() {
        let before = parse_prompt(&multi_question_first_screen());
        let mid = parse_prompt(&multi_question_first_screen().replace("❯ 1. Red", "  1. Red").replace("  2. Blue", "❯ 2. Blue"));
        assert_ne!(before.fingerprint, mid.fingerprint, "❯ が動けば fingerprint は変わる");
        assert!(!select_all_progressed(&before, &mid), "設問は片付いていない");

        let after = parse_prompt(&multi_question_second_screen());
        assert!(select_all_progressed(&before, &after), "☒ が増えたら進んだ");
    }

    /// 画面上部に残った**別のダイアログのプレビュー枠**の桁で、いま出ている
    /// ダイアログの選択肢を切らない（2 回目のセルフレビューで検出）。
    ///
    /// 切ると `Yes, and don't ask again for tree commands in X:\…` が
    /// `Yes, and don't ask again for tree` になり、`truncated` も立たないので
    /// **「以後無条件で承認」であることが隠れたまま承認させる。**
    #[test]
    fn a_stale_preview_panel_above_does_not_cut_the_current_dialog() {
        let screen = [
            "  Which layout?",
            "❯ 1. Grid                         ┌──────────────────────────────┐",
            "  2. List                         │ ┌───┬───┬───┐                │",
            "                                  ├──────────────────────────────┤",
            "                                  └──────────────────────────────┘",
            "",
            "● Bash(tree -L 2 src)",
            "",
            "Do you want to proceed?",
            "",
            "❯ 1. Yes",
            "  2. Yes, and don't ask again for tree commands in X:\\devel\\worktree\\oretachi-34yd",
            "  3. No, and tell Claude what to do differently (esc)",
            "",
            "  Esc to cancel · Tab to amend",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Permission);
        assert_eq!(
            p.questions[0].options[1].label,
            "Yes, and don't ask again for tree commands in X:\\devel\\worktree\\oretachi-34yd",
            "上の残骸の桁でラベルが切られている"
        );
        assert_eq!(p.questions[0].options.len(), 3);
    }

    /// TodoWrite パネルの `☒` / `☐` をタブバーと取り違えない
    /// （2 回目のセルフレビューで検出）。
    ///
    /// 取り違えると `navigation` が Digits → Arrows へ反転し、
    /// **素の TUI の番号リストへ矢印 + CR を送る**ことになる。
    #[test]
    fn a_todo_panel_is_not_a_question_tab_bar() {
        let screen = [
            "● Update Todos",
            "  ⎿  ☒ テストを書く",
            "     ☐ 実装する",
            "     ☐ レビューする",
            "",
            "Pick a target:",
            "  1) staging",
            "  2) production",
            "Selection: ",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert!(p.tabs.is_empty(), "tabs={:?}", p.tabs);
        assert_eq!(p.shape, PromptShape::Numbered);
        assert_eq!(p.navigation, Navigation::Digits, "素の番号リストへ矢印を送ってはいけない");
    }

    /// todo が上にあっても、許可ダイアログは許可ダイアログのまま。
    #[test]
    fn a_todo_panel_above_a_permission_dialog_changes_nothing() {
        let screen = [
            "  ⎿  ☒ テストを書く",
            "     ☐ 実装する",
            "",
            "Bash command",
            "cargo test",
            "",
            "Do you want to proceed?",
            "",
            "❯ 1. Yes",
            "  2. No, and tell Claude what to do differently (esc)",
            "",
            "  Esc to cancel · Tab to amend",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert!(p.tabs.is_empty(), "tabs={:?}", p.tabs);
        assert_eq!(p.shape, PromptShape::Permission);
        assert!(p.context.contains("cargo test"), "context={:?}", p.context);
    }

    /// 単一設問のタブバー（`☐ Color` の 1 つだけ）は、`AskUserQuestion` の
    /// フッタが出ていて選択肢のすぐ上にあるときだけ採る。
    #[test]
    fn a_single_question_tab_bar_is_accepted_next_to_its_options() {
        let screen = [
            "❯ 直前のプロンプトのエコー",
            "☐ Color",
            "Which color?",
            "",
            "❯ 1. Red",
            "  2. Blue",
            "",
            "Enter to select · ↑/↓ to navigate · Esc to cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.tabs.len(), 1);
        assert_eq!(p.tabs[0].label, "Color");
        assert_eq!(p.header, "Which color?");
        assert_eq!(p.context, "", "エコーを混ぜない");
    }

    /// 単一設問（`✔ Submit` タブが無い）で全問答え終えた画面は `Done`。
    ///
    /// ここを「2 問目を探す」経路へ落とすと、成功しているのに
    /// `Refuse` になってカードが読み取り専用になる（2 回目のセルフレビューで検出）。
    #[test]
    fn select_all_is_done_when_every_tab_is_answered_without_a_submit_tab() {
        let screen = [
            "☒ Color",
            "Which color?",
            "",
            "❯ 1. Red",
            "  2. Blue",
            "",
            "Enter to select · ↑/↓ to navigate · Esc to cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.tabs.len(), 1);
        assert!(p.tabs[0].answered);
        assert_eq!(plan_select_all_step(&p, &[1], Some(0)), SelectAllStep::Done);
    }

    /// 確認画面は見出しでも裏取りする（回答済みタブを表示中の設問と取り違えない）。
    #[test]
    fn select_all_refuses_a_submit_lookalike_without_the_review_heading() {
        let screen = [
            "←  ☒ Color  ☒ Size  ✔ Submit  →",
            "Which workflow should we use?",
            "",
            "❯ 1. Submit for review",
            "  2. Merge directly",
            "",
            "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        match plan_select_all_step(&p, &[1, 1], Some(1)) {
            SelectAllStep::Refuse(reason) => assert!(reason.contains("確かめられませんでした")),
            other => panic!("確定してはいけない: {:?}", other),
        }
    }

    /// CJK が混ざっても横並びのプレビュー枠を切り離せる（桁は表示幅で数える）。
    #[test]
    fn the_preview_column_is_measured_in_display_width() {
        let screen = [
            "☐ Layout",
            "Which layout?",
            "",
            "❯ 1. グリッド表示                 ┌──────────────────┐",
            "  2. リスト                       │ preview line 1   │",
            "                                  ├──────────────────┤",
            "                                  └──────────────────┘",
            "",
            "Enter to select · ↑/↓ to navigate · Esc to cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        let labels: Vec<&str> = p.questions[0].options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec!["グリッド表示", "リスト"], "labels={:?}", labels);
    }

    /// プレビュー枠の上辺が 1 番目の選択肢と揃っていなくても切り離せる。
    ///
    /// 「`┌` の行の左が 1 番目の選択肢として読めること」を要求していた時期は
    /// ここを取り逃していた（枠文字がラベルへ混ざる）。
    #[test]
    fn the_preview_panel_is_found_when_its_top_is_above_the_first_option() {
        let screen = [
            "☐ Layout",
            "Which layout?                     ┌──────────────────┐",
            "❯ 1. Grid                         │ preview line 1   │",
            "  2. List                         │ preview line 2   │",
            "                                  └──────────────────┘",
            "",
            "Enter to select · ↑/↓ to navigate · Esc to cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        let labels: Vec<&str> = p.questions[0].options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec!["Grid", "List"], "labels={:?}", labels);
    }

    /// タブバーが画面外へ流れた確認画面でも、素の番号リストへ落とさない
    /// （3 回目のセルフレビューで検出）。
    ///
    /// 落とすと `2. Cancel` を選んだつもりで数字キーが飛び、確定キーではない
    /// 数字のあとの CR が `❯` の当たっている `1. Submit answers` を確定する
    /// （＝押していない方が通る）。
    #[test]
    fn the_submit_review_screen_is_recognised_without_its_tab_bar() {
        let screen = [
            "Review your answers",
            "",
            " ● Which color?",
            "   → Red",
            " ● Which size?",
            "   → Large",
            "",
            "Ready to submit your answers?",
            "",
            "❯ 1. Submit answers",
            "  2. Cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert!(p.tabs.is_empty(), "タブバーは画面に無い");
        assert_eq!(p.shape, PromptShape::AskUserQuestion);
        assert_eq!(p.navigation, Navigation::Arrows, "数字キーでは確定しない");
        assert_eq!(p.header, "Ready to submit your answers?");
        let keys = plan_keys(&p, &Answer::Select { option_index: 2 }).expect("cancel");
        assert_eq!(keys_preview(&keys), vec!["Down", "CR"], "数字を送ってはいけない");
        // 一括回答からも確定できる
        assert_eq!(
            plan_select_all_step(&p, &[1, 1], Some(1)),
            SelectAllStep::Submit { option_index: 1 }
        );
    }

    /// 本物のタブバーより下に `☐` を含む行があっても、本物を採る
    /// （3 回目のセルフレビューで検出）。
    ///
    /// 取り違えると `is_submit` が失われて確認画面へ進めなくなり、偽タブは
    /// `☒` に変わらないので「進んでいない」と誤判定され続ける。
    #[test]
    fn a_strong_tab_bar_wins_over_a_checkbox_in_the_question_text() {
        let screen = [
            "←  ☐ Task  ✔ Submit  →",
            "☐ の項目のうちどれを先にやりますか?",
            "",
            "❯ 1. テストを書く",
            "  2. 実装する",
            "",
            "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        let labels: Vec<&str> = p.tabs.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["Task", "Submit"], "tabs={:?}", p.tabs);
        assert!(p.tabs[1].is_submit);
        assert_eq!(p.header, "☐ の項目のうちどれを先にやりますか?");
    }

    /// 選択肢がちょうど 10 件で番号が右寄せでも、`10.` をカーソルと誤認しない
    /// （3 回目のセルフレビューで検出）。
    #[test]
    fn right_aligned_two_digit_numbers_are_not_mistaken_for_a_cursor() {
        let mut lines = vec!["Pick a target:".to_string()];
        for i in 1..=10 {
            // 番号だけ右寄せ（`  1.` … ` 10.`）。ラベルの桁は揃う
            lines.push(format!("{:>3}. option-{}", i, i));
        }
        lines.push("Selection: ".to_string());
        let p = parse_prompt(&lines.join("\n"));
        assert_eq!(p.questions[0].options.len(), 10);
        assert_eq!(p.questions[0].cursor_index, None, "❯ が無いのに位置を決めない");
    }

    /// 見出しが折り返していても確認画面と分かる（4 回目のセルフレビューで検出）。
    ///
    /// この救済が要るのは狭い / 短いタブで、**まさに見出しが割れる状況**。
    /// 最後の 1 行だけを見ていると、必要なときに限って発火しなかった。
    #[test]
    fn the_submit_review_screen_is_recognised_when_its_heading_wraps() {
        let screen = [
            "Review your",
            "answers",
            "",
            "Ready to",
            "submit your",
            "answers?",
            "",
            "❯ 1. Submit",
            "     answers",
            "  2. Cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::AskUserQuestion);
        assert_eq!(p.navigation, Navigation::Arrows, "数字キーでは確定しない");
        assert_eq!(p.header, "Ready to submit your answers?");
        let keys = plan_keys(&p, &Answer::Select { option_index: 2 }).expect("cancel");
        assert_eq!(keys_preview(&keys), vec!["Down", "CR"]);
        assert_eq!(
            plan_select_all_step(&p, &[1, 1], Some(1)),
            SelectAllStep::Submit { option_index: 1 }
        );
    }

    /// 本物のタブバーより**上**にある散文の `☒` / `☐` に負けない
    /// （4 回目のセルフレビューで検出）。
    ///
    /// 「強い候補を画面全体から先に探す」形にしたときの新しい穴だった。
    #[test]
    fn prose_with_checkboxes_above_does_not_beat_the_real_tab_bar() {
        let screen = [
            "凡例: ☒ 完了 / ☐ 未完了",
            "",
            "☐ Color",
            "Which color?",
            "",
            "❯ 1. Red",
            "  2. Blue",
            "",
            "Enter to select · ↑/↓ to navigate · Esc to cancel",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        let labels: Vec<&str> = p.tabs.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["Color"], "tabs={:?}", p.tabs);
        assert_eq!(p.header, "Which color?");
    }

    /// 番号が左寄せなら、選択肢が 10 件以上でもカーソルを推定できる
    /// （`10.` の行だけラベルの桁がずれることで取り逃していた）。
    #[test]
    fn the_cursor_is_still_recovered_with_ten_or_more_options() {
        let mut lines = vec!["←  ☒ A  ☐ B  ✔ Submit  →".to_string(), "Pick one".to_string(), String::new()];
        for i in 1..=10 {
            // カーソル行だけマーカーぶん左（`❯` は描かれていない）
            let prefix = if i == 3 { " " } else { "  " };
            lines.push(format!("{}{}. option-{}", prefix, i, i));
        }
        lines.push("Enter to select · Tab/Arrow keys to navigate · Esc to cancel".to_string());
        let p = parse_prompt(&lines.join("\n"));
        assert_eq!(p.questions[0].options.len(), 10);
        assert_eq!(p.questions[0].cursor_index, Some(3));
    }

    #[test]
    fn closing_the_dialog_counts_as_progress() {
        let before = parse_prompt(&submit_review_screen());
        let after = parse_prompt(&free_input_screen());
        assert!(select_all_progressed(&before, &after));
    }

    #[test]
    fn detects_shell_yesno() {
        for line in [
            "Continue with the merge? (y/N) ",
            "Overwrite existing file? [Y/n]: ",
            "Proceed? (yes/no)",
        ] {
            let p = parse_prompt(&format!("Cloning into 'x'...\n{}", line));
            assert_eq!(p.shape, PromptShape::YesNo, "line={:?}", line);
        }
    }

    /// エコーされたコマンド行を y/n プロンプトと取り違えない。
    ///
    /// 実測: `oretachi_inspect_prompt` が下のコマンド行を `yesno` と判定した。
    /// プロンプトでない行へ `y` + CR を送ると、シェルが `y` をコマンドとして実行する。
    #[test]
    fn does_not_treat_an_echoed_command_line_as_a_yesno_prompt() {
        let screen = concat!(
            "PS X:\\devel\\worktree\\oretachi-y14b> ",
            "bash -c 'read -p \"Continue with the merge? (y/N) \" x; echo GOT=[$x]; sleep 300'",
        );
        let p = parse_prompt(screen);
        assert_ne!(p.shape, PromptShape::YesNo, "tail={:?}", p.tail);
    }

    #[test]
    fn detects_bare_numbered_list() {
        let p = parse_prompt(
            "Pick a target:\n  1) staging\n  2) production\nSelection: ",
        );
        assert_eq!(p.shape, PromptShape::Numbered);
        assert_eq!(p.questions[0].options.len(), 2);
    }

    #[test]
    fn unknown_screen_yields_unknown_and_keeps_tail() {
        let p = parse_prompt("Compiling foo v0.1.0\n   Building [====>    ] 12/40\n");
        assert_eq!(p.shape, PromptShape::Unknown);
        assert!(p.tail.contains("Building"));
    }

    #[test]
    fn empty_screen_is_unknown() {
        assert_eq!(parse_prompt("").shape, PromptShape::Unknown);
        assert_eq!(parse_prompt("   \n\n  ").shape, PromptShape::Unknown);
    }

    #[test]
    fn takes_the_last_dialog_when_an_older_one_is_still_on_screen() {
        let screen = [
            "│ Do you want to proceed?     │",
            "│ ❯ 1. Yes                    │",
            "│   2. No                     │",
            "  (answered)",
            "│ Would you like to proceed?  │",
            "│   1. Yes, and auto-accept   │",
            "│ ❯ 2. Yes, and manually      │",
            "│   3. No, keep planning      │",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Plan);
        assert_eq!(p.questions[0].cursor_index, Some(2));
    }

    // ── キー列の組み立て ───────────────────────────────────────────────────

    #[test]
    fn select_moves_cursor_down_then_confirms() {
        let p = parse_prompt(&permission_screen("echo hi", "X:\\wt"));
        let keys = plan_keys(&p, &Answer::Select { option_index: 3 }).unwrap();
        assert_eq!(keys_preview(&keys), vec!["Down", "Down", "CR"]);
    }

    #[test]
    fn select_moves_cursor_up_when_target_is_above() {
        let p = parse_prompt(&plan_screen()); // ❯ は 2 番
        let keys = plan_keys(&p, &Answer::Select { option_index: 1 }).unwrap();
        assert_eq!(keys_preview(&keys), vec!["Up", "CR"]);
    }

    #[test]
    fn select_current_option_sends_only_cr() {
        let p = parse_prompt(&plan_screen());
        let keys = plan_keys(&p, &Answer::Select { option_index: 2 }).unwrap();
        assert_eq!(keys_preview(&keys), vec!["CR"]);
    }

    /// 選択肢が 10 件以上でも矢印方式なら同じ手順で通る（数字キーだと届かない）。
    #[test]
    fn select_works_past_ten_options() {
        let mut lines = vec!["Do you want to proceed?".to_string()];
        for i in 1..=12 {
            lines.push(format!("{}{}. option {}", if i == 1 { "❯ " } else { "  " }, i, i));
        }
        // 実物のダイアログには必ずフッタが付く。キーの種類はここから決まる
        lines.push("  Esc to cancel · Tab to amend".to_string());
        let p = parse_prompt(&lines.join("\n"));
        assert_eq!(p.navigation, Navigation::Arrows);
        assert_eq!(p.questions[0].options.len(), 12);
        let keys = plan_keys(&p, &Answer::Select { option_index: 11 }).unwrap();
        assert_eq!(keys.len(), 11);
        assert!(keys_preview(&keys).iter().take(10).all(|k| k == "Down"));
    }

    /// 13 桁のターミナルで実際に採った許可ダイアログ。見出しが
    /// `Do you want` / `to` / `proceed?` の 3 行に割れる。
    ///
    /// 見出しの一致だけで形状を決めていた実装では `numbered` へ落ち、**矢印ではなく
    /// 数字キーを送っていた**（どちらも効かないか、TUI 次第で別の解釈をされる）。
    #[test]
    fn narrow_terminal_wraps_the_header_but_keys_stay_arrows() {
        let screen = [
            " This        ",
            " command     ",
            " requires    ",
            " approval    ",
            "",
            " Do you want ",
            " to",
            " proceed?    ",
            " ❯ 1. Yes    ",
            "   2. Yes,   ",
            "      and    ",
            "      don't  ",
            "      ask    ",
            "      again  ",
            "   3. No     ",
            "",
            " Esc to      ",
            " cancel ·    ",
            " Tab to      ",
            " amend       ",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        // 折り返した見出しが 1 つに繋がっている
        assert_eq!(p.header, "Do you want to proceed?");
        assert_eq!(p.shape, PromptShape::Permission);
        assert_eq!(p.navigation, Navigation::Arrows);
        assert!(p.context.contains("requires"), "context={:?}", p.context);
        assert_eq!(p.questions[0].options.len(), 3);
        let keys = plan_keys(&p, &Answer::Select { option_index: 3 }).unwrap();
        assert_eq!(keys_preview(&keys), vec!["Down", "Down", "CR"]);
    }

    /// 見出しをまったく読めなくても、Claude Code のフッタがあれば数字キーへ倒さない。
    /// 数字を送っても確定しないので、`numbered` と取り違えると回答が届かない。
    #[test]
    fn cc_footer_alone_keeps_arrow_navigation() {
        let screen = [
            " (見出しは画面外へ流れた)",
            "",
            " ❯ 1. Yes",
            "   2. No",
            " Esc to cancel · Tab to amend",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.navigation, Navigation::Arrows);
        assert!(!p.truncated, "上に行があるので切れていない扱い");
        assert_eq!(
            keys_preview(&plan_keys(&p, &Answer::Select { option_index: 2 }).unwrap()),
            vec!["Down", "CR"]
        );
    }

    /// dev インスタンスの 7 行 13 桁のタブで実際に起きたケース。ダイアログが画面より
    /// 高く、見出しと `2.` 以降が流れて `options` が `[{1, "Yes"}]` だけになった。
    ///
    /// **読めたぶんだけで選ばせてはいけない。** 人は「`Yes` しか無い」と誤認して、
    /// 拒否の選択肢を見ないまま承認してしまう。
    #[test]
    fn refuses_selection_when_the_dialog_is_clipped_off_screen() {
        let screen = [" ❯ 1. Yes", "", " Esc to", " cancel ·", " Tab to", " amend"].join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Permission);
        assert_eq!(p.navigation, Navigation::Arrows);
        assert!(p.truncated, "画面先頭から選択肢が始まり、しかも 1 件しか無い");

        let err = plan_keys(&p, &Answer::Select { option_index: 1 }).unwrap_err();
        assert!(err.0.contains("収まっておらず"), "err={}", err.0);

        // ESC で抜ける経路は画面に何が見えていても成立するので塞がない
        assert_eq!(
            keys_preview(&plan_keys(&p, &Answer::EscapeThenText { text: "手で見ます".into() }).unwrap()),
            vec!["Esc", "text(5文字)", "CR"]
        );
    }

    /// 画面が広がって選択肢が全部見えるようになったら「別の画面」として扱い、
    /// 切れていたときの fingerprint での送信を `stale` で弾く。
    #[test]
    fn fingerprint_changes_when_a_clipped_dialog_becomes_fully_visible() {
        let clipped = parse_prompt(&[" ❯ 1. Yes", " Esc to cancel · Tab to amend"].join("\n"));
        let full = parse_prompt(
            &[
                " Do you want to proceed?",
                " ❯ 1. Yes",
                "   2. No",
                " Esc to cancel · Tab to amend",
            ]
            .join("\n"),
        );
        assert!(clipped.truncated);
        assert!(!full.truncated);
        assert_ne!(clipped.fingerprint, full.fingerprint);
    }

    /// **#215 のセルフレビューで見つかった最悪のケース。**
    ///
    /// `1.` が画面外へ流れた許可ダイアログ（`❯ 2. …` から始まる画面）では
    /// `find_last_option_run` が連番の起点を見つけられず `None` を返す。すると
    /// `looks_like_free_input` が**ダイアログのカーソルマーカー `❯` を CC の入力欄**と
    /// 誤認して `Text` へ倒し、開いているダイアログへ自由テキスト + CR が送られて
    /// `❯` が指す選択肢（ここでは「以後 echo を無条件承認」）が確定してしまう。
    /// これは #215 が防ごうとした事象そのもの。
    #[test]
    fn a_clipped_dialog_is_never_mistaken_for_a_free_input_box() {
        let screen = [
            "│ ❯ 2. Yes, and don't ask again for echo commands │",
            "│   3. No, and tell Claude what to do differently │",
            "╰─────────────────────────────────────────────────╯",
            "  Esc to cancel · Tab to amend",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_ne!(
            p.shape,
            PromptShape::Text,
            "ダイアログの選択肢行を入力欄と誤認している: {:?}",
            p
        );
        // 分類できないので何も送らない
        assert_eq!(p.shape, PromptShape::Unknown);
        assert!(plan_keys(&p, &Answer::Text { text: "了解".into() }).is_err());
        assert!(plan_keys(&p, &Answer::Select { option_index: 2 }).is_err());
    }

    /// CC が本文中に散文の番号リストを出したあと入力欄で待っている画面。
    ///
    /// 散文行を選択肢の並びとして拾うと (a) `kind="text"` が拒否されて本来の返答経路が
    /// 死に、(b) カードが散文を「画面に実在する選択肢」として提示してしまう。
    #[test]
    fn a_prose_numbered_list_above_the_input_box_is_still_free_input() {
        let screen = [
            "● 進め方はこうします:",
            "  1. design first",
            "  2. then implement",
            "  3. finally test",
            "",
            "╭─────────────────────────────╮",
            "│ >                           │",
            "╰─────────────────────────────╯",
            "  ⏵⏵ auto-accept edits on",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Text, "parsed={:?}", p);
        assert_eq!(p.navigation, Navigation::None);
        // 本文の返答が通ること（ここが死ぬと従来の自由入力経路が壊れる）
        assert!(plan_keys(&p, &Answer::Text { text: "了解".into() }).is_ok());
    }

    /// 入力欄に `1. …` と打ち込んだ状態を「1 件の番号選択ダイアログ」と誤認しない。
    ///
    /// **画面は実機から採ったもの。** カーソル記号は `>` ではなく `❯` で、**直後に
    /// スペースが無い**（`❯1. do this thing first`）。合成画面（`> 1. …`）でテストして
    /// いたため、`>` をマーカーから外すだけの修正では実機で直っていなかった。
    #[test]
    fn text_typed_into_the_input_box_is_not_an_option_list() {
        let rule = "─".repeat(60);
        let screen = [
            "▝▝ ▝▝    X:\\devel\\worktree\\oretachi-y14b",
            "",
            &rule,
            "❯1. do this thing first",
            &rule,
            "  ⏸ manual mode on",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Text, "parsed={:?}", p);
        assert_eq!(p.navigation, Navigation::None);
        assert!(p.questions.is_empty() || p.questions[0].options.is_empty());
        // 番号を送れてしまわないこと
        assert!(plan_keys(&p, &Answer::Select { option_index: 1 }).is_err());
    }

    /// 枠で囲まれた入力欄（`│ > … │`）でも同じ。
    #[test]
    fn a_boxed_input_line_is_not_an_option_list() {
        let screen = [
            "╭──────────────────────────────────╮",
            "│ > 1. do this thing first         │",
            "╰──────────────────────────────────╯",
            "  ? for shortcuts",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Text, "parsed={:?}", p);
        assert!(p.questions.is_empty() || p.questions[0].options.is_empty());
    }

    /// フッタ判定に**選択肢ラベル自身**を混ぜない（`run.end` は inclusive）。
    /// ラベルに `Esc to cancel` を含む番号リストが Claude Code のダイアログに化ける。
    #[test]
    fn an_option_label_is_not_mistaken_for_a_cc_footer() {
        let screen = [
            "Pick a target:",
            "  1) staging",
            "  2) how to use Esc to cancel the deploy",
            "Selection: ",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.shape, PromptShape::Numbered, "parsed={:?}", p);
        assert_eq!(p.navigation, Navigation::Digits);
    }

    #[test]
    fn numbered_list_uses_digits_not_arrows() {
        let p = parse_prompt("Pick:\n  1) staging\n  2) production\nSelection: ");
        let keys = plan_keys(&p, &Answer::Select { option_index: 2 }).unwrap();
        assert_eq!(keys_preview(&keys), vec!["2", "CR"]);
    }

    #[test]
    fn yesno_sends_char_then_cr() {
        let p = parse_prompt("Continue? (y/N) ");
        assert_eq!(
            keys_preview(&plan_keys(&p, &Answer::YesNo { yes: false }).unwrap()),
            vec!["n", "CR"]
        );
    }

    #[test]
    fn text_into_free_input_sends_body_then_cr() {
        let p = parse_prompt(&free_input_screen());
        let keys = plan_keys(&p, &Answer::Text { text: "了解".into() }).unwrap();
        assert_eq!(keys_preview(&keys), vec!["text(2文字)", "CR"]);
    }

    /// #215 の中核。ダイアログが開いている宛先へ自由テキストは送らせない。
    #[test]
    fn refuses_free_text_while_a_dialog_is_open() {
        for screen in [
            permission_screen("rm -rf /", "X:\\wt"),
            plan_screen(),
            ask_user_question_screen(),
        ] {
            let p = parse_prompt(&screen);
            let err = plan_keys(&p, &Answer::Text { text: "了解".into() }).unwrap_err();
            assert!(err.0.contains("ダイアログ"), "err={}", err.0);
        }
    }

    #[test]
    fn escape_then_text_is_allowed_on_dialogs_with_an_escape_hatch() {
        let p = parse_prompt(&permission_screen("echo hi", "X:\\wt"));
        let keys =
            plan_keys(&p, &Answer::EscapeThenText { text: "別の方法で".into() }).unwrap();
        assert_eq!(keys_preview(&keys), vec!["Esc", "text(5文字)", "CR"]);
    }

    #[test]
    fn never_sends_keys_to_unknown_screens() {
        let p = parse_prompt("Compiling foo v0.1.0\n");
        for answer in [
            Answer::Select { option_index: 1 },
            Answer::Text { text: "x".into() },
            Answer::YesNo { yes: true },
            Answer::EscapeThenText { text: "x".into() },
        ] {
            assert!(plan_keys(&p, &answer).is_err());
        }
    }

    #[test]
    fn refuses_option_index_that_is_not_on_screen() {
        let p = parse_prompt(&plan_screen());
        let err = plan_keys(&p, &Answer::Select { option_index: 9 }).unwrap_err();
        assert!(err.0.contains("画面に存在しません"), "err={}", err.0);
    }

    /// `❯` が読めない画面では移動量が決まらないので選択させない。
    #[test]
    fn refuses_select_without_a_cursor_marker() {
        let screen = [
            "Do you want to proceed?",
            "  1. Yes",
            "  2. No",
            "  Esc to cancel · Tab to amend",
        ]
        .join("\n");
        let p = parse_prompt(&screen);
        assert_eq!(p.navigation, Navigation::Arrows);
        assert_eq!(p.questions[0].cursor_index, None);
        let err = plan_keys(&p, &Answer::Select { option_index: 2 }).unwrap_err();
        assert!(err.0.contains("❯"), "err={}", err.0);
    }

    // ── fingerprint ────────────────────────────────────────────────────────

    #[test]
    fn fingerprint_is_stable_for_the_same_screen() {
        let a = parse_prompt(&permission_screen("echo hi", "X:\\wt"));
        let b = parse_prompt(&permission_screen("echo hi", "X:\\wt"));
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn fingerprint_changes_when_the_approval_subject_changes() {
        let a = parse_prompt(&permission_screen("echo hi", "X:\\wt"));
        let b = parse_prompt(&permission_screen("rm -rf /", "X:\\wt"));
        assert_ne!(
            a.fingerprint, b.fingerprint,
            "承認対象が変わったのに fingerprint が同じだと、別のコマンドを承認してしまう"
        );
    }

    /// `❯` が動いたら矢印の移動量が変わるので、fingerprint も変わらなければならない。
    #[test]
    fn fingerprint_changes_when_the_cursor_moves() {
        let base = plan_screen();
        let moved = base.replace("│   1. Yes", "│ ❯ 1. Yes").replace("│ ❯ 2.", "│   2.");
        let a = parse_prompt(&base);
        let b = parse_prompt(&moved);
        assert_eq!(a.questions[0].cursor_index, Some(2));
        assert_eq!(b.questions[0].cursor_index, Some(1));
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    /// `shape: text` の fingerprint が定数だと、「レポート生成時は Claude Code の入力欄
    /// だったが、送信時には CC が終了して同じ PTY にシェルのプロンプトだけが残っている」
    /// 状況で照合が通り、**返答テキストがシェルコマンドとして実行される**。
    #[test]
    fn text_fingerprint_distinguishes_a_cc_box_from_a_shell_prompt() {
        let cc = parse_prompt(&free_input_screen());
        let shell = parse_prompt("PS X:\\devel\\worktree\\other> ");
        assert_eq!(cc.shape, PromptShape::Text);
        assert_eq!(shell.shape, PromptShape::Text);
        assert_ne!(
            cc.fingerprint, shell.fingerprint,
            "CC の入力欄とシェルのプロンプトが同じ fingerprint だと、CC 終了後のシェルへ本文が流れる"
        );
    }

    /// 別のシェル（別ディレクトリ）も別画面として扱う。
    #[test]
    fn text_fingerprint_distinguishes_two_shell_prompts() {
        let a = parse_prompt("PS X:\\devel\\worktree\\aaa> ");
        let b = parse_prompt("PS X:\\devel\\worktree\\bbb> ");
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    /// **入力中のテキストは fingerprint に混ぜない。** 混ぜると、レポートを開いてから
    /// 人が宛先の端末に何か打っただけで送信が常に `stale` になって使えなくなる。
    #[test]
    fn text_fingerprint_ignores_what_is_typed_in_the_cc_box() {
        let typed_screen = free_input_screen().replace("│ >  ", "│ > あ");
        // 置換が効いていないと、この test は何も検証しないまま通ってしまう
        assert_ne!(typed_screen, free_input_screen(), "テストの前提が崩れている");
        let empty = parse_prompt(&free_input_screen());
        let typed = parse_prompt(&typed_screen);
        assert_eq!(typed.shape, PromptShape::Text);
        assert_eq!(
            empty.fingerprint, typed.fingerprint,
            "入力中のテキストで fingerprint が変わると、人が打つだけで送れなくなる"
        );
    }

    /// スピナーだけが動いた画面で fingerprint が変わると、常に `stale` になって
    /// 何も送れなくなる。
    #[test]
    fn fingerprint_ignores_spinner_churn_below_the_dialog() {
        let base = ask_user_question_screen();
        let a = parse_prompt(&format!("{}\n  ✻ Hashing…", base));
        let b = parse_prompt(&format!("{}\n  ✶ Hullaballooing…", base));
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    // ── Answer::from_parts ─────────────────────────────────────────────────

    #[test]
    fn from_parts_builds_each_kind() {
        assert_eq!(
            Answer::from_parts("select", Some(2), None, None).unwrap(),
            Answer::Select { option_index: 2 }
        );
        assert_eq!(
            Answer::from_parts("text", None, Some("hi"), None).unwrap(),
            Answer::Text { text: "hi".into() }
        );
        assert_eq!(
            Answer::from_parts("yesno", None, None, Some("Y")).unwrap(),
            Answer::YesNo { yes: true }
        );
    }

    #[test]
    fn from_parts_rejects_control_characters_in_text() {
        // ESC を通すと宛先の TUI へ任意のエスケープシーケンスを注入できる
        for bad in ["a\u{1b}[B", "line1\nline2", "x\r"] {
            let err = Answer::from_parts("text", None, Some(bad), None).unwrap_err();
            assert!(err.contains("制御文字"), "input={:?} err={}", bad, err);
        }
    }

    #[test]
    fn from_parts_rejects_missing_and_unknown_pieces() {
        assert!(Answer::from_parts("select", None, None, None).is_err());
        assert!(Answer::from_parts("text", None, Some("   "), None).is_err());
        assert!(Answer::from_parts("yesno", None, None, Some("maybe")).is_err());
        assert!(Answer::from_parts("nope", None, None, None).is_err());
    }

    // ── 画面再生 ───────────────────────────────────────────────────────────

    /// カーソル移動を含む差分描画は `strip_ansi` では復元できないが、VT 再生なら通る。
    /// これが `read_terminal` の text ではなく画面グリッドを使う理由。
    #[test]
    fn logical_screen_reconstructs_a_cursor_addressed_redraw() {
        // 1 行目に "1. Yes" を描いたあと、カーソルを戻して "❯" を書き足す
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(b"\x1b[2J\x1b[H");
        bytes.extend(b"  1. Yes\r\n  2. No\r\n");
        bytes.extend(b"\x1b[1;1H\xe2\x9d\xaf"); // 1行1桁へ移動して ❯
        let screen = render_logical_screen(&bytes, 6, 40);
        assert!(screen.contains("❯ 1. Yes"), "screen={:?}", screen);
        // strip_ansi 相当（エスケープを捨てるだけ）では ❯ が末尾に落ちて解析できない
        let p = parse_prompt(&screen);
        assert_eq!(p.questions[0].cursor_index, Some(1));
    }

    /// 13 桁のターミナルでは `(y/N)` マーカー自体が右端で割れる（実測:
    /// `Continue with` / ` the merge? (` / `y/N)`）。物理行のままだと一致せず
    /// `unknown` へ落ちるので、`row_wrapped` を見て**区切り無しで**繋ぎ直す。
    /// 空白で繋ぐと `( y/N)` になって同じく一致しない。
    #[test]
    fn logical_screen_rejoins_a_marker_split_by_wrapping() {
        // 13 桁に 31 文字を書くと 3 行へ折り返される
        let bytes = b"Continue with the merge? (y/N) ";
        let logical = render_logical_screen(bytes, 7, 13);
        assert!(
            logical.contains("Continue with the merge? (y/N)"),
            "折り返しを解けていない: logical={:?}",
            logical
        );
        // 空白で繋いでいたら `( y/N)` になって一致しない
        assert_eq!(parse_prompt(&logical).shape, PromptShape::YesNo);
    }

    /// 折り返しを解いても、ハードな改行はそのまま行として残る。
    #[test]
    fn logical_screen_keeps_hard_newlines() {
        let logical = render_logical_screen(b"one\r\ntwo\r\nthree", 8, 40);
        let lines: Vec<&str> = logical.lines().map(str::trim_end).collect();
        assert_eq!(&lines[..3], &["one", "two", "three"]);
    }

    /// `spawn` 直後などに 0 が来ても panic しない（vt100 は小さいグリッドで
    /// 減算オーバーフローする）。MCP ツールごと落とすわけにはいかない。
    #[test]
    fn logical_screen_tolerates_degenerate_sizes() {
        for (rows, cols) in [(0u16, 0u16), (1, 1), (0, 80), (24, 0), (1, 200)] {
            let screen = render_logical_screen(b"hello", rows, cols);
            assert!(screen.contains("hello"), "rows={} cols={} screen={:?}", rows, cols, screen);
        }
    }
}
