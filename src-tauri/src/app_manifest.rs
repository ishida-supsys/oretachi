//! アプリ形式アーティファクトの manifest（アーティファクト JSON の任意フィールド `app`）。
//!
//! このモジュールは**純粋関数だけ**を持つ（`AppHandle` も settings も取らない）ので、
//! パース・正規化・権限判定をすべて単体テストできる。
//!
//! # 権限セットの 2 つの形
//!
//! - [`RequestedPermissions`]: manifest の `permissions` を正規化したもの。**変化検知にだけ**使う
//!   （grant に保存した `requested` と比べ、違えば再承認を求める）。ワークツリー名は生文字列のまま
//!   持つので、改名で解決できなくなっても再承認は出ない。
//! - [`ResolvedPermissions`]: 承認時に解決した形（ワークツリー名 → ID など）。**関門はこちらだけ**を
//!   見る。アーティファクト JSON 内の manifest は関門からは見ない（AI が書き換えられるため）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 将来の sub-issue が受理するようになる権限キー。#370 の時点では「未対応」として manifest 不正扱い
/// にする（黙って無視すると、申請したつもりの権限が効かないまま承認だけ通ることになる）。
const RESERVED_PERMISSION_KEYS: &[&str] = &["wasm"];

/// `aiTerminal.agents` で指定できるエージェント（v1 は Claude Code のみ）。
pub const AI_TERMINAL_AGENTS: &[&str] = &["claude"];
/// `aiTerminal.maxConcurrent` の上限（インスタンスごと）。
pub const AI_TERMINAL_MAX_CONCURRENT_CAP: u32 = 4;

/// `scheduler.jobs` の最大件数と、tool ジョブの `params` の最大バイト数（シリアライズ後）。
pub const SCHEDULER_MAX_JOBS: usize = 10;
pub const SCHEDULER_PARAMS_MAX_BYTES: usize = 16 * 1024;
const SCHEDULER_MAX_ID_CHARS: usize = 40;

/// `subWindow.maxCount` の上限（インスタンスごと）。
pub const SUB_WINDOW_MAX_COUNT_CAP: u32 = 4;
/// `subWindow.maxWidth` / `maxHeight` の下限（書いた場合のみ）。上限は設けない（省略 = 上限なし）
pub const SUB_WINDOW_DIM_MIN: u32 = 64;
/// 大きさを省略して開くときの既定（`maxWidth` / `maxHeight` が省略されているとき）
pub const SUB_WINDOW_DEFAULT_WIDTH: u32 = 480;
pub const SUB_WINDOW_DEFAULT_HEIGHT: u32 = 640;

/// `audio.sampleRates` に書けるレート（Hz）。音声認識で使う帯域に絞る。
pub const AUDIO_SAMPLE_RATES: &[u32] = &[8000, 16000, 24000, 48000];
/// `audio.maxSeconds` の上限と既定（1 回の録音の最大秒数）
pub const AUDIO_MAX_SECONDS_CAP: u32 = 600;
const AUDIO_DEFAULT_MAX_SECONDS: u32 = 60;
const AUDIO_DEFAULT_SAMPLE_RATE: u32 = 16000;

/// `process` の上限。コマンド名・引数の数・環境変数の数・enum の候補数。
pub const PROCESS_MAX_COMMANDS: usize = 16;
const PROCESS_MAX_ARGS: usize = 64;
const PROCESS_MAX_ENV: usize = 32;
const PROCESS_MAX_ENUM_VALUES: usize = 64;
const PROCESS_MAX_FIXED_CHARS: usize = 4096;
const PROCESS_MAX_EXEC_CHARS: usize = 1024;
/// `maxConcurrent`（コマンドごと）の上限と既定
pub const PROCESS_MAX_CONCURRENT_CAP: u32 = 8;
/// `timeoutSec` の上限（7 日）。省略 = 無制限
pub const PROCESS_MAX_TIMEOUT_SEC: u64 = 604_800;
/// `maxOutputBytes`（累計）の下限・上限・既定。超えるとツリーごと止める
pub const PROCESS_OUTPUT_MIN: u64 = 1024;
pub const PROCESS_OUTPUT_MAX: u64 = 1 << 30;
pub const PROCESS_OUTPUT_DEFAULT: u64 = 64 << 20;
/// 子プロセスへ渡さない環境変数の名前の接頭辞（oretachi の MCP のポート・トークンなど。大文字小文字は無視）。
/// manifest の `env.inherit` に書かれても拒否し、起動時にも二重に除外する。
pub const PROCESS_FORBIDDEN_ENV_PREFIXES: &[&str] = &["ORETACHI_", "MCP_", "CLAUDE_"];

const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_VERSION_CHARS: usize = 50;
const MAX_TARGETS: usize = 20;
const MAX_TARGET_CHARS: usize = 200;
const MAX_FS_SCOPES: usize = 20;

/// 生成元スキルの記録。**表示専用**で、権限セットには含めない。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedMcp {
    /// ソート・重複除去済み。`ARTIFACT_CALLABLE_TOOLS` の部分集合。
    pub tools: Vec<String>,
    /// 生文字列（トリム・ソート・重複除去済み）。空 = 自ワークツリーのみ。
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPermission {
    pub max_bytes: u64,
}

/// インスタンス専用 AI ターミナル。ユーザーと同じ権限で動くエージェントを起動できるので、
/// 承認ダイアログでは最も強い警告にする。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTerminalPermission {
    /// ソート・重複除去済み。`AI_TERMINAL_AGENTS` の部分集合。
    pub agents: Vec<String>,
    /// 同時に開ける数（1〜`AI_TERMINAL_MAX_CONCURRENT_CAP`）。
    pub max_concurrent: u32,
}

/// manifest の `network`。`allow` は URL プレフィックス（`app_net::normalize_prefix` で正規化済み・
/// ソート・重複除去済み）。承認時にそのまま解決済みの形にもなる（解決で変わるものがない）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPermission {
    pub allow: Vec<String>,
}

/// スケジューラージョブの実行内容。承認ダイアログに**そのまま**見せる（無人で動くので、何が走るかを
/// 承認時点で固定する）。`tool` は `mcp.tools` の、`aiTerminal` は `aiTerminal` 権限の範囲でだけ動く
/// （parse 時に宣言を要求し、実行時も既存の関門を通る）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SchedulerAction {
    #[serde(rename = "tool")]
    Tool {
        tool: String,
        /// JSON オブジェクト（省略は `{}`）
        #[serde(default = "empty_object")]
        params: serde_json::Value,
    },
    #[serde(rename = "aiTerminal", rename_all = "camelCase")]
    AiTerminal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        initial_prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerJob {
    pub id: String,
    /// 5 フィールドの cron 式（空白 1 つに正規化済み。ローカル時刻で評価する）
    pub cron: String,
    pub action: SchedulerAction,
}

/// manifest の `scheduler`。アプリのインスタンスが**起動中**（Running）のあいだだけ、ホスト側が
/// `jobs` を cron 式どおりに実行する。再起動をまたいで復活しない・取りこぼした回の追いかけ実行はしない。
/// `jobs` は id でソート済み。解決で変わるものがない（承認した値がそのまま実行の入力になる）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerPermission {
    pub jobs: Vec<SchedulerJob>,
}

/// manifest の `media`。`<video>` / `<audio>` の `src` に直接渡せるローカルファイルと URL の申請。
/// fs / network の権限とは独立（承認ダイアログでは「メディア再生」として示す）。
/// `read` は fs と同じ `worktree://` / `artifact://` のディレクトリプレフィックス（読み取り専用）、
/// `allow` は network と同じ http(s) の URL プレフィックス。どちらも正規化済み・ソート・重複除去済み。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedMedia {
    pub read: Vec<String>,
    pub allow: Vec<String>,
}

/// 承認時に解決した `media`。`fs` は `read` を `app_fs::resolve_requested_fs` で解決したもの
/// （`read` が空なら無し）。`allow` は解決で変わらない。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMedia {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<ResolvedFs>,
    pub allow: Vec<String>,
}

/// manifest の `subWindow`。インスタンスが専用サブウィンドウ（`appinst-<n>-w<k>`）を開くための申請。
///
/// 数・大きさの上限と、見た目・位置の権限（透明・枠なし・常に前面・位置の指定）を**個別に**申請させる。
/// 省略した値は既定で埋めて正規化する（変化検知が既定の書き分けで揺れないように）。
/// 透明・枠なしのウィンドウは、小さくても他のウィンドウの上に重なって操作を横取りできるので、
/// 承認ダイアログでは注意の対象にする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubWindowPermission {
    /// 同時に開ける数（1〜`SUB_WINDOW_MAX_COUNT_CAP`）
    pub max_count: u32,
    /// 開くウィンドウの幅・高さの上限（論理ピクセル。`SUB_WINDOW_DIM_MIN` 以上）。**省略 = 上限なし**
    /// （ユーザーの手動リサイズ・最大化も縛らない）。以前の grant は必ず値を持つのでそのまま読める
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    #[serde(default)]
    pub transparent: bool,
    #[serde(default)]
    pub frameless: bool,
    #[serde(default)]
    pub always_on_top: bool,
    /// 位置の指定（開くときの x / y・移動・ドラッグ）。無いと位置は OS 任せ
    #[serde(default)]
    pub position: bool,
}

/// manifest の `audio`。マイクの生音声（PCM）の取得。**ユーザーの発話を聞ける権限**なので、
/// 承認ダイアログでは最も強い警告にする。取得は Rust 側（`app_audio.rs`）で行い、iframe には
/// getUserMedia を使わせない（sandbox / CSP を緩めない。前提 3）。
///
/// 省略した値は既定で埋めて正規化する（変化検知が既定の書き分けで揺れないように）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPermission {
    /// マイク入力の申請。必須で `true`（将来、出力やループバックを足しても区別できるように）
    pub microphone: bool,
    /// 1 回の録音の最大秒数（1〜`AUDIO_MAX_SECONDS_CAP`）。host が強制する
    pub max_seconds: u32,
    /// `start` で指定できるサンプルレート（Hz）。ソート・重複除去済み。`AUDIO_SAMPLE_RATES` の部分集合
    pub sample_rates: Vec<u32>,
}

/// manifest の `fs`。`read` / `write` は `worktree://<相対>` / `artifact://<相対>` のディレクトリ
/// プレフィックス（正規化済み・ソート・重複除去済み）。`write` は `read` を含意しない。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestedFs {
    pub read: Vec<String>,
    pub write: Vec<String>,
    /// 既定拒否パス（`.git/` など）への書き込みを許すか。最も強い警告の対象
    #[serde(default)]
    pub allow_sensitive_paths: bool,
}

/// 承認時に解決した FS のスコープ 1 件。`path` は解決済みの絶対パス（ルートは canonicalize 済み）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedFsScope {
    /// "worktree" | "artifact"
    pub scheme: String,
    /// 正規化済みの相対プレフィックス（`out/` など。ルート全体は空）
    pub prefix: String,
    pub path: String,
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedFs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_root: Option<String>,
    pub scopes: Vec<ResolvedFsScope>,
    #[serde(default)]
    pub allow_sensitive_paths: bool,
}

/// コマンドの引数 1 件（manifest の `process.commands.<name>.args` の要素）。
///
/// 固定の文字列か、型つきのパラメータだけ。呼び出し側が渡せるのはパラメータの値だけで、
/// 実行ファイルや固定の引数は変えられない。`Fixed` の値が `worktree://` / `artifact://` で始まるものは
/// パスとして承認時に絶対パスへ解決する（それ以外は文字列のまま）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProcessArg {
    Fixed { value: String },
    /// `under` 配下（正規化済みの `worktree://…/` / `artifact://…/`）に解決できる値だけ
    Path { param: String, under: String },
    /// `[A-Za-z0-9_-]{1,64}`（`-` 始まりは不可）
    Token { param: String },
    /// 宣言した候補のどれか（ソート・重複除去済み）
    Enum { param: String, values: Vec<String> },
}

/// 申請したコマンド 1 件（正規化済み）。変化検知にだけ使う。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessCommand {
    /// 絶対パス / `worktree://…` / `artifact://…` / 裸の名前（承認時に PATH から絶対パスへ解決する）
    pub exec: String,
    pub args: Vec<ProcessArg>,
    /// `worktree://…` / `artifact://…`（省略時は `worktree://`）
    pub cwd: String,
    /// 継承する環境変数の名前（ソート・重複除去済み）。これ以外は子へ渡さない
    pub env_inherit: Vec<String>,
    pub max_concurrent: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    pub max_output_bytes: u64,
}

/// manifest の `process`。名前つきで宣言したコマンドだけを、アプリが起動できる（`app_process.rs`）。
/// **ユーザーと同じ権限でコマンドを実行できる**ので、承認ダイアログでは最も強い警告にする。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessPermission {
    pub commands: std::collections::BTreeMap<String, ProcessCommand>,
}

/// 承認時に解決した引数。`Path` は `under` を `app_fs::resolve_requested_fs` で解決したスコープを持ち、
/// 呼び出しのたびに `app_fs::resolve_scoped_path` で照合する。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ResolvedProcessArg {
    /// パスとして解決したものは絶対パス（`\\?\` は除いた形）
    Fixed { value: String },
    Path { param: String, under: String, fs: ResolvedFs },
    Token { param: String },
    Enum { param: String, values: Vec<String> },
}

/// 承認時に解決したコマンド。`exec` は**承認時に解決した絶対パス**（PATH の差し替えへの対策）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedProcessCommand {
    pub exec: String,
    pub args: Vec<ResolvedProcessArg>,
    pub cwd: String,
    pub env_inherit: Vec<String>,
    pub max_concurrent: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProcess {
    pub commands: std::collections::BTreeMap<String, ResolvedProcessCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestedPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<RequestedMcp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryPermission>,
    #[serde(default, rename = "aiTerminal", skip_serializing_if = "Option::is_none")]
    pub ai_terminal: Option<AiTerminalPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<RequestedFs>,
    /// manifest の `db`。形は `fs` と同じ（DB ファイルの置き場所のスコープ）。read のスコープの DB は
    /// 読み取り専用で開く。`allowSensitivePaths` は read / write の両方の既定拒否を解く
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<RequestedFs>,
    /// manifest の `network`。Rust 側のプロキシ（`app_net.rs`）が通す URL プレフィックス
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPermission>,
    /// manifest の `subWindow`。専用サブウィンドウ（`app_subwindow.rs`）の上限と、申請した見た目の権限
    #[serde(default, rename = "subWindow", skip_serializing_if = "Option::is_none")]
    pub sub_window: Option<SubWindowPermission>,
    /// manifest の `audio`。マイクの生音声の取得（`app_audio.rs`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioPermission>,
    /// manifest の `media`。動画・音声の再生（`app_media.rs` の appmedia プロトコル）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<RequestedMedia>,
    /// manifest の `scheduler`。cron 式で動く無人ジョブ（`app_scheduler.rs`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<SchedulerPermission>,
    /// manifest の `process`。宣言したコマンドの起動（`app_process.rs`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessPermission>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub source: Option<AppSource>,
    pub permissions: RequestedPermissions,
}

/// 越境先 1 件の解決結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTarget {
    /// manifest に書かれていた生文字列（表示用）。
    pub raw: String,
    /// 照合に使う正規化済みの形。ワークツリー ID / `*` / `workgroup:<gid>` / `repo:<小文字名>`。
    pub stored: String,
    /// ダイアログ用の表示名（承認時点のワークツリー名など）。
    pub label: String,
    /// `*` / `workgroup:` / `repo:`。承認後に作られるワークツリーにも当たる。
    pub wildcard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMcp {
    pub tools: Vec<String>,
    pub targets: Vec<ResolvedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResolvedPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<ResolvedMcp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryPermission>,
    #[serde(default, rename = "aiTerminal", skip_serializing_if = "Option::is_none")]
    pub ai_terminal: Option<AiTerminalPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<ResolvedFs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<ResolvedFs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPermission>,
    /// 解決で変わるものがない（承認した値がそのまま関門の入力になる）
    #[serde(default, rename = "subWindow", skip_serializing_if = "Option::is_none")]
    pub sub_window: Option<SubWindowPermission>,
    /// 解決で変わるものがない（承認した値がそのまま関門の入力になる）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioPermission>,
    /// `read` はパスを解決した形（`fs` と同様）。`allow` はそのまま
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<ResolvedMedia>,
    /// 解決で変わるものがない（承認した値がそのまま実行の入力になる）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<SchedulerPermission>,
    /// 承認時に解決した形（exec・固定パス・cwd は絶対パス）。`app_instance::build_resolved` が埋める
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ResolvedProcess>,
}

/// host 側に保存する承認。キーはアーティファクト識別子（`<worktreeId>/<artifactId>`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGrant {
    pub granted_at: u64,
    pub requested: RequestedPermissions,
    pub resolved: ResolvedPermissions,
}

/// アーティファクト JSON の `app` が「アプリのマーカー」として立っているか。
/// `null` は立っていない扱い。**型の検証はしない**（壊れた manifest でも、マーカーがある以上は
/// 通常のビューアからの呼び出しを塞ぐ側へ倒すため）。
pub fn has_app_marker(artifact_json: &serde_json::Value) -> bool {
    artifact_json.get("app").map(|v| !v.is_null()).unwrap_or(false)
}

/// アーティファクト JSON の**文字列**に対するマーカー判定。関門・メモリ書き込みの両方がここを通る。
///
/// - `"app"` を含まない文字列はパースを省いて偽（ほとんどの書き込みはこちら）。
/// - 含むならパースしてマーカーを見る。
/// - **パースできないときは真**（塞ぐ側へ倒す）。壊れた JSON を理由に関門をすり抜けさせない。
pub fn raw_has_app_marker(raw: &str) -> bool {
    if !raw.contains("\"app\"") {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(raw)
        .map(|v| has_app_marker(&v))
        .unwrap_or(true)
}

fn expect_object<'a>(
    v: &'a serde_json::Value,
    what: &str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, String> {
    v.as_object().ok_or_else(|| format!("{} は JSON オブジェクトである必要があります", what))
}

fn optional_text(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    max_chars: usize,
) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.chars().count() > max_chars {
                return Err(format!("{} は {} 文字以内にしてください", key, max_chars));
            }
            Ok(Some(t.to_string()))
        }
        Some(_) => Err(format!("{} は文字列である必要があります", key)),
    }
}

/// `app` オブジェクトを検証して manifest にする。
///
/// `permissions` 内の未知キーは**拒否**する（fail closed）。受理するのは `mcp` / `memory` / `fs` / `db` / `network` / `aiTerminal` だけで、
/// 後続 sub-issue が足す予定のキーは「未対応」として別メッセージで拒否する。
pub fn parse_manifest(app: &serde_json::Value) -> Result<AppManifest, String> {
    let obj = expect_object(app, "app")?;

    let name = optional_text(obj, "name", MAX_NAME_CHARS)?
        .ok_or_else(|| "app.name は必須です（空でない文字列）".to_string())?;
    let description = optional_text(obj, "description", MAX_DESCRIPTION_CHARS)?;
    let version = optional_text(obj, "version", MAX_VERSION_CHARS)?;

    let source = match obj.get("source") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => {
            let s = expect_object(v, "app.source")?;
            Some(AppSource {
                skill: optional_text(s, "skill", MAX_NAME_CHARS)?,
                version: optional_text(s, "version", MAX_VERSION_CHARS)?,
            })
        }
    };

    let permissions = match obj.get("permissions") {
        None | Some(serde_json::Value::Null) => RequestedPermissions::default(),
        Some(v) => parse_permissions(v)?,
    };

    Ok(AppManifest { name, description, version, source, permissions })
}

fn parse_permissions(v: &serde_json::Value) -> Result<RequestedPermissions, String> {
    let obj = expect_object(v, "app.permissions")?;
    let mut out = RequestedPermissions::default();
    for (key, val) in obj {
        match key.as_str() {
            "mcp" => out.mcp = parse_mcp(val)?,
            "memory" => out.memory = parse_memory(val)?,
            "aiTerminal" => out.ai_terminal = parse_ai_terminal(val)?,
            "fs" => out.fs = parse_scoped(val, ScopedKind::Fs)?,
            "db" => out.db = parse_scoped(val, ScopedKind::Db)?,
            "network" => out.network = parse_network(val)?,
            "subWindow" => out.sub_window = parse_sub_window(val)?,
            "audio" => out.audio = parse_audio(val)?,
            "media" => out.media = parse_media(val)?,
            "scheduler" => out.scheduler = parse_scheduler(val)?,
            "process" => out.process = parse_process(val)?,
            k if RESERVED_PERMISSION_KEYS.contains(&k) => {
                return Err(format!(
                    "権限 '{}' はこの版ではまだ使えません（対応する機能が未実装です）。manifest から外してください",
                    k
                ));
            }
            other => {
                return Err(format!(
                    "未知の権限 '{}' です。指定できるのは mcp / memory / fs / db / network / subWindow / aiTerminal / audio / media / scheduler / process だけです",
                    other
                ));
            }
        }
    }
    check_scheduler_against_permissions(&out)?;
    check_process_against_write_scopes(&out)?;
    Ok(out)
}

/// `exec` の拡張子が `.bat` / `.cmd` か（末尾のドット・空白は Windows が黙って剥がすので無視して見る）。
/// `CreateProcess` はこれらを暗黙に `cmd.exe` 経由で動かし、引数の解釈による注入の恐れがあるので、
/// どの OS でも manifest の時点で拒否する（OS で通る・通らないが分かれる manifest にしない）。
pub fn is_forbidden_exec_ext(exec: &str) -> bool {
    let name = exec.rsplit(['/', '\\']).next().unwrap_or(exec).trim_end_matches(['.', ' ']);
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".bat") || lower.ends_with(".cmd")
}

/// 環境変数の名前が、子へ渡してはいけない接頭辞か。
pub fn is_forbidden_env_name(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    PROCESS_FORBIDDEN_ENV_PREFIXES.iter().any(|p| up.starts_with(p))
}

fn is_path_uri(s: &str) -> bool {
    s.starts_with("worktree://") || s.starts_with("artifact://")
}

fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}')
}

/// 区切り（`/` と `\`）で分けた成分に `..` があるか。固定引数・絶対パスの exec は、書き込みスコープとの
/// 照合を字句で行うので、`..` で外へ回り込むものは受け付けない（`a/../b` を畳んだ結果が覆われるかを
/// 照合側で正しく見るより、拒否するほうが確実）。
fn has_dotdot(s: &str) -> bool {
    s.split(['/', '\\']).any(|c| c == "..")
}

/// Windows のドライブ相対パス（`C:scripts\run.py`。ドライブ文字のあとにルートが無い）。
/// 子の作業フォルダ基準で解決され、`Path::join` は基準を置き換えるため、照合の対象から外れてしまう。
fn is_drive_relative(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && !(b.len() >= 3 && (b[2] == b'\\' || b[2] == b'/'))
}

fn is_drive_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// `exec` を検査して正規化する。許すのは絶対パス・`worktree://` / `artifact://` のファイル・裸の名前だけ
/// （`./x` や `bin\x` のような相対パスは、何を指すかが cwd 次第になるので拒否）。
fn parse_exec(raw: &str, what: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(format!("{} は空にできません", what));
    }
    if s.chars().count() > PROCESS_MAX_EXEC_CHARS || has_control_chars(s) {
        return Err(format!("{} が長すぎるか、制御文字を含んでいます", what));
    }
    if is_forbidden_exec_ext(s) {
        return Err(format!(
            "{} '{}' は .bat / .cmd です。コマンドライン引数の解釈による注入の恐れがあるため、exec にできません（.exe などを指定してください）",
            what, s
        ));
    }
    if s.ends_with('.') {
        return Err(format!("{} '{}' は末尾がドットです", what, s));
    }
    if is_path_uri(s) {
        let n = crate::app_fs::normalize_path_uri(s)?;
        if n.ends_with("://") {
            return Err(format!("{} にはファイルを指定してください（ルート自身は指定できません）", what));
        }
        return Ok(n);
    }
    if s.starts_with("\\\\") {
        return Err(format!("{} に UNC パス / \\\\?\\ 形式は使えません: {}", what, s));
    }
    if is_drive_absolute(s) || s.starts_with('/') {
        if has_dotdot(s) {
            return Err(format!("{} '{}' に '..' は使えません（絶対パスは畳んだ形で書いてください）", what, s));
        }
        return Ok(s.to_string());
    }
    if s.contains(['/', '\\', ':']) || s.contains(['"', '*', '?', '<', '>', '|']) || s.starts_with('-') {
        return Err(format!(
            "{} '{}' は絶対パス・worktree:// / artifact:// のファイル・コマンド名（PATH で探す）のどれかで指定してください",
            what, s
        ));
    }
    Ok(s.to_string())
}

fn valid_param_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 32 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_process_arg(v: &serde_json::Value, what: &str) -> Result<ProcessArg, String> {
    match v {
        serde_json::Value::String(s) => {
            if s.chars().count() > PROCESS_MAX_FIXED_CHARS || s.contains(['\0', '\r', '\n']) {
                return Err(format!(
                    "{} は {} 文字以内で、NUL・改行を含まない文字列にしてください",
                    what, PROCESS_MAX_FIXED_CHARS
                ));
            }
            // worktree:// / artifact:// で始まるものは承認時に絶対パスへ解決する固定のパス
            let value = if is_path_uri(s) {
                crate::app_fs::normalize_path_uri(s)?
            } else {
                // 書き込みスコープとの照合（cwd 基準の相対パスとして解決される値）を `..` や Windows の
                // ドライブ相対パスで外れないよう、この 2 つの形だけは manifest の時点で拒否する
                // （`:` や `*` を含む普通の文字列まで一律に拒否はしない）
                if has_dotdot(s) {
                    return Err(format!(
                        "{} に '..' は使えません（パスは worktree://… の形か、cwd からの相対パスで、'..' を含めずに書いてください）",
                        what
                    ));
                }
                if is_drive_relative(s) {
                    return Err(format!(
                        "{} はドライブ相対パス（C:foo の形）です。worktree://… か、ルートつきの絶対パスで書いてください",
                        what
                    ));
                }
                s.clone()
            };
            Ok(ProcessArg::Fixed { value })
        }
        serde_json::Value::Object(obj) => {
            if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "param" | "type" | "under" | "values")) {
                return Err(format!("{} の未知のキー '{}' です（param / type / under / values だけ）", what, k));
            }
            let param = obj
                .get("param")
                .and_then(|p| p.as_str())
                .ok_or_else(|| format!("{}.param（パラメータ名）は必須の文字列です", what))?
                .to_string();
            if !valid_param_name(&param) {
                return Err(format!("{}.param '{}' は [A-Za-z0-9_]{{1,32}} で指定してください", what, param));
            }
            let ty = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let reject_extra = |keys: &[&str]| -> Result<(), String> {
                match keys.iter().find(|k| obj.contains_key(**k)) {
                    Some(k) => Err(format!("{} の type '{}' には '{}' を指定できません", what, ty, k)),
                    None => Ok(()),
                }
            };
            match ty {
                "path" => {
                    reject_extra(&["values"])?;
                    let under = obj
                        .get("under")
                        .and_then(|u| u.as_str())
                        .ok_or_else(|| format!("{}.under（worktree://… / artifact://… のプレフィックス）は path に必須です", what))?;
                    Ok(ProcessArg::Path { param, under: crate::app_fs::normalize_scope(under)? })
                }
                "token" => {
                    reject_extra(&["under", "values"])?;
                    Ok(ProcessArg::Token { param })
                }
                "enum" => {
                    reject_extra(&["under"])?;
                    let items = obj
                        .get("values")
                        .and_then(|a| a.as_array())
                        .ok_or_else(|| format!("{}.values（候補の配列）は enum に必須です", what))?;
                    let mut set = BTreeSet::new();
                    for item in items {
                        let s = item
                            .as_str()
                            .ok_or_else(|| format!("{}.values の要素は文字列である必要があります", what))?;
                        if s.is_empty() || s.chars().count() > 256 || has_control_chars(s) {
                            return Err(format!("{}.values に空・長すぎる・制御文字を含む候補があります", what));
                        }
                        if s.starts_with('-') {
                            return Err(format!("{}.values の候補 '{}' は '-' で始められません（オプションとして解釈されるため）", what, s));
                        }
                        set.insert(s.to_string());
                    }
                    if set.is_empty() || set.len() > PROCESS_MAX_ENUM_VALUES {
                        return Err(format!("{}.values は 1〜{} 件にしてください", what, PROCESS_MAX_ENUM_VALUES));
                    }
                    Ok(ProcessArg::Enum { param, values: set.into_iter().collect() })
                }
                other => Err(format!("{}.type '{}' は使えません（path / token / enum だけ）", what, other)),
            }
        }
        _ => Err(format!("{} は文字列（固定の引数）かオブジェクト（パラメータ）である必要があります", what)),
    }
}

fn parse_process_command(name: &str, v: &serde_json::Value) -> Result<ProcessCommand, String> {
    let what = format!("app.permissions.process.commands.{}", name);
    let obj = expect_object(v, &what)?;
    const KEYS: &[&str] = &["exec", "args", "cwd", "env", "stdin", "maxConcurrent", "timeoutSec", "maxOutputBytes", "pin"];
    if let Some(k) = obj.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("{} の未知のキー '{}' です（{} だけ）", what, k, KEYS.join(" / ")));
    }
    if obj.contains_key("pin") {
        // エントリのハッシュ固定（前提 9 の案 A）は将来の拡張。黙って無視しない
        return Err(format!("{}.pin はこの版ではまだ使えません（対応する機能が未実装です）。manifest から外してください", what));
    }
    let exec = parse_exec(
        obj.get("exec")
            .and_then(|e| e.as_str())
            .ok_or_else(|| format!("{}.exec（実行ファイル）は必須の文字列です", what))?,
        &format!("{}.exec", what),
    )?;

    let args = match obj.get("args") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => {
            if items.len() > PROCESS_MAX_ARGS {
                return Err(format!("{}.args は {} 件までです", what, PROCESS_MAX_ARGS));
            }
            let mut out = Vec::with_capacity(items.len());
            let mut names = BTreeSet::new();
            for (i, item) in items.iter().enumerate() {
                let arg = parse_process_arg(item, &format!("{}.args[{}]", what, i))?;
                let pname = match &arg {
                    ProcessArg::Path { param, .. } | ProcessArg::Token { param } | ProcessArg::Enum { param, .. } => Some(param),
                    ProcessArg::Fixed { .. } => None,
                };
                if let Some(p) = pname {
                    if !names.insert(p.clone()) {
                        return Err(format!("{}.args にパラメータ名 '{}' が重複しています", what, p));
                    }
                }
                out.push(arg);
            }
            out
        }
        Some(_) => return Err(format!("{}.args は配列である必要があります", what)),
    };

    let cwd = match obj.get("cwd") {
        None | Some(serde_json::Value::Null) => "worktree://".to_string(),
        Some(serde_json::Value::String(s)) => {
            if !is_path_uri(s.trim()) {
                return Err(format!("{}.cwd は worktree://… または artifact://… で指定してください（絶対パスは受け付けません）", what));
            }
            crate::app_fs::normalize_path_uri(s)?
        }
        Some(_) => return Err(format!("{}.cwd は文字列である必要があります", what)),
    };

    let env_inherit = match obj.get("env") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(e) => {
            let eo = expect_object(e, &format!("{}.env", what))?;
            if let Some(k) = eo.keys().find(|k| k.as_str() != "inherit") {
                return Err(format!("{}.env の未知のキー '{}' です（inherit だけ。値を直接書くことはできません）", what, k));
            }
            let names = string_list(eo, "inherit", &format!("{}.env.inherit", what))?;
            if names.len() > PROCESS_MAX_ENV {
                return Err(format!("{}.env.inherit は {} 件までです", what, PROCESS_MAX_ENV));
            }
            for n in &names {
                let valid = n.len() <= 64
                    && n.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !valid {
                    return Err(format!("{}.env.inherit の名前 '{}' は [A-Za-z_][A-Za-z0-9_]{{0,63}} で指定してください", what, n));
                }
                if is_forbidden_env_name(n) {
                    return Err(format!(
                        "{}.env.inherit に '{}' は指定できません（{} で始まる変数は oretachi 自身の接続情報などを含むため、子へ渡しません）",
                        what,
                        n,
                        PROCESS_FORBIDDEN_ENV_PREFIXES.join(" / ")
                    ));
                }
            }
            names
        }
    };

    match obj.get("stdin") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(s)) if s == "none" => {}
        Some(_) => return Err(format!("{}.stdin は \"none\" だけです（標準入力は渡せません）", what)),
    }

    let uint = |key: &str, min: u64, max: u64| -> Result<Option<u64>, String> {
        match obj.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(n) => {
                let n = n
                    .as_u64()
                    .ok_or_else(|| format!("{}.{} は正の整数である必要があります", what, key))?;
                if n < min || n > max {
                    return Err(format!("{}.{} は {}〜{} の範囲で指定してください（指定: {}）", what, key, min, max, n));
                }
                Ok(Some(n))
            }
        }
    };
    Ok(ProcessCommand {
        exec,
        args,
        cwd,
        env_inherit,
        max_concurrent: uint("maxConcurrent", 1, PROCESS_MAX_CONCURRENT_CAP as u64)?.unwrap_or(1) as u32,
        timeout_sec: uint("timeoutSec", 1, PROCESS_MAX_TIMEOUT_SEC)?,
        max_output_bytes: uint("maxOutputBytes", PROCESS_OUTPUT_MIN, PROCESS_OUTPUT_MAX)?.unwrap_or(PROCESS_OUTPUT_DEFAULT),
    })
}

fn parse_process(v: &serde_json::Value) -> Result<Option<ProcessPermission>, String> {
    let what = "app.permissions.process";
    let obj = expect_object(v, what)?;
    if let Some(k) = obj.keys().find(|k| k.as_str() != "commands") {
        return Err(format!("{} の未知のキー '{}' です（commands だけ）", what, k));
    }
    let cmds = match obj.get("commands") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(c) => expect_object(c, "app.permissions.process.commands")?,
    };
    if cmds.is_empty() {
        return Ok(None);
    }
    if cmds.len() > PROCESS_MAX_COMMANDS {
        return Err(format!("{}.commands は {} 件までです", what, PROCESS_MAX_COMMANDS));
    }
    let mut commands = std::collections::BTreeMap::new();
    for (name, val) in cmds {
        let valid = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            return Err(format!("コマンド名 '{}' は [A-Za-z0-9_-]{{1,64}} で指定してください", name));
        }
        commands.insert(name.clone(), parse_process_command(name, val)?);
    }
    Ok(Some(ProcessPermission { commands }))
}

/// `process` が同じ manifest の書き込み権限の迂回路にならないようにする。
///
/// 前提 9（案 C）の論拠「書き込める相手は、そもそも MCP クライアントとして同等以上の権限を持つ」は
/// **外部の書き手**（AI・MCP クライアント）にしか効かない。アプリ自身が `fs.write` / `db`（書き込み）で
/// スクリプトを書き換えられ、かつ `process` で起動できるなら、承認なしの iframe から任意のコードを
/// 実行できてしまう。そこで、書き込みスコープが exec・固定のパス引数・cwd（の配下）を覆う組み合わせは拒否する
/// （親ディレクトリを覆うスコープは、配下であるこれらも覆うのでこの検査に含まれる）。
/// 先頭の引数が `path` パラメータで、その `under` が書き込めるものも拒否する
/// （`exec: python, args: [{path}]` は、書いたファイルをそのまま実行する形になる）。
/// **これは限定的なヒューリスティック**で、`-X utf8` のようなオプションを先に置く形などは通る
/// （`path` パラメータの `under` に書けること自体が、この API の主な使い方〔request.json を書いて渡す〕なので、
/// 一律には拒否できない）。そこは承認ダイアログの警告（`procWritableParams`）で人に見せる。
/// 他所から `import` されるファイルまでは守れない（案 A と同じ限界）。
fn check_process_against_write_scopes(p: &RequestedPermissions) -> Result<(), String> {
    let Some(proc_perm) = &p.process else { return Ok(()) };
    let writes: Vec<(&str, &String)> = p
        .fs
        .iter()
        .flat_map(|f| f.write.iter().map(|s| ("fs", s)))
        .chain(p.db.iter().flat_map(|d| d.write.iter().map(|s| ("db", s))))
        .collect();
    if writes.is_empty() {
        return Ok(());
    }
    let covers = |scope: &str, target: &str| -> bool {
        match (crate::app_fs::split_uri(scope), crate::app_fs::split_uri(target)) {
            (Ok((s1, c1)), Ok((s2, c2))) => s1 == s2 && c2.len() >= c1.len() && c2[..c1.len()] == c1[..],
            _ => false,
        }
    };
    for (name, cmd) in &proc_perm.commands {
        let mut targets: Vec<(&str, &str)> = vec![("exec", cmd.exec.as_str()), ("cwd", cmd.cwd.as_str())];
        // cwd 基準の相対パスとして解決される固定引数（`python scripts/run.py`、`python -m pkg` の `pkg` など）。
        // `-` 始まり（オプション）と、`://` を含む・絶対パスのものは対象外。誤検知は、書き込みスコープが
        // その相対パスを実際に覆うときだけ（`utf8` のような引数は `worktree://utf8` として照合され、通常は覆われない）
        let mut relative: Vec<String> = Vec::new();
        for a in &cmd.args {
            if let ProcessArg::Fixed { value } = a {
                if is_path_uri(value) {
                    targets.push(("固定のパス引数", value.as_str()));
                } else if is_path_uri(&cmd.cwd)
                    && !value.starts_with('-')
                    && !value.contains("://")
                    && !value.starts_with('/')
                    && !is_drive_absolute(value)
                {
                    // cwd は `worktree://`（ルート）か `worktree://a/b`（末尾の区切り無し）
                    let sep = if cmd.cwd.ends_with("://") { "" } else { "/" };
                    let joined = format!("{}{}{}", cmd.cwd, sep, value.replace('\\', "/"));
                    if crate::app_fs::split_uri(&joined).is_ok() {
                        relative.push(joined);
                    }
                }
            }
        }
        for r in &relative {
            targets.push(("cwd 基準の相対パスの固定引数", r.as_str()));
        }
        if let Some(ProcessArg::Path { under, .. }) = cmd.args.first() {
            targets.push(("先頭の path パラメータの under", under.as_str()));
        }
        for (kind, uri) in targets {
            if !is_path_uri(uri) {
                continue;
            }
            if let Some((perm, scope)) = writes.iter().find(|(_, s)| covers(s, uri)) {
                return Err(format!(
                    "app.permissions.process のコマンド '{}' の {}（{}）は、app.permissions.{}.write の '{}' に書き込める場所にあります。アプリ自身が書き換えて実行できてしまうため、書き込めない場所に置くか、書き込みのスコープを狭めてください",
                    name, kind, uri, perm, scope
                ));
            }
        }
    }
    Ok(())
}

/// スケジューラーが他の権限の迂回路にならないようにする。tool ジョブは `mcp.tools` に宣言済みのツールだけ、
/// aiTerminal ジョブは `aiTerminal` 権限を申請したときだけ書ける（承認ダイアログで別々に見えるものを、
/// スケジューラー経由で黙って使わせない）。
fn check_scheduler_against_permissions(p: &RequestedPermissions) -> Result<(), String> {
    let Some(sch) = &p.scheduler else { return Ok(()) };
    for job in &sch.jobs {
        match &job.action {
            SchedulerAction::Tool { tool, .. } => {
                let declared = p.mcp.as_ref().map(|m| m.tools.iter().any(|t| t == tool)).unwrap_or(false);
                if !declared {
                    return Err(format!(
                        "app.permissions.scheduler のジョブ '{}' が呼ぶツール '{}' は、app.permissions.mcp.tools にも宣言してください",
                        job.id, tool
                    ));
                }
            }
            SchedulerAction::AiTerminal { .. } => {
                if p.ai_terminal.is_none() {
                    return Err(format!(
                        "app.permissions.scheduler のジョブ '{}' は AI ターミナルを開きます。app.permissions.aiTerminal も申請してください",
                        job.id
                    ));
                }
            }
        }
    }
    Ok(())
}

fn string_list(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    what: &str,
) -> Result<Vec<String>, String> {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => {
            let mut set = BTreeSet::new();
            for item in items {
                let s = item
                    .as_str()
                    .ok_or_else(|| format!("{} の要素は文字列である必要があります", what))?
                    .trim();
                if s.is_empty() {
                    return Err(format!("{} に空の要素があります", what));
                }
                set.insert(s.to_string());
            }
            Ok(set.into_iter().collect())
        }
        Some(_) => Err(format!("{} は文字列の配列である必要があります", what)),
    }
}

fn parse_mcp(v: &serde_json::Value) -> Result<Option<RequestedMcp>, String> {
    let obj = expect_object(v, "app.permissions.mcp")?;
    if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "tools" | "targets")) {
        return Err(format!("app.permissions.mcp の未知のキー '{}' です（tools / targets だけ）", k));
    }
    let tools = string_list(obj, "tools", "app.permissions.mcp.tools")?;
    let targets = string_list(obj, "targets", "app.permissions.mcp.targets")?;

    for t in &tools {
        if !crate::mcp_server::ARTIFACT_CALLABLE_TOOLS.contains(&t.as_str()) {
            return Err(format!(
                "ツール '{}' はアプリから呼べません。呼べるのは次のツールだけです: {}",
                t,
                crate::mcp_server::ARTIFACT_CALLABLE_TOOLS.join(", ")
            ));
        }
    }
    if tools.is_empty() {
        if !targets.is_empty() {
            return Err("app.permissions.mcp.targets は tools と一緒に指定してください".to_string());
        }
        return Ok(None);
    }
    if targets.len() > MAX_TARGETS {
        return Err(format!("app.permissions.mcp.targets は {} 件までです", MAX_TARGETS));
    }
    if let Some(t) = targets.iter().find(|t| t.chars().count() > MAX_TARGET_CHARS) {
        return Err(format!("targets の要素が長すぎます（{} 文字まで）: {}", MAX_TARGET_CHARS, t));
    }
    Ok(Some(RequestedMcp { tools, targets }))
}

fn parse_network(v: &serde_json::Value) -> Result<Option<NetworkPermission>, String> {
    let obj = expect_object(v, "app.permissions.network")?;
    if let Some(k) = obj.keys().find(|k| k.as_str() != "allow") {
        // ループバック・プライベート IP・oretachi 自身のポートを許す設定は v1 にはない
        return Err(format!("app.permissions.network の未知のキー '{}' です（allow だけ）", k));
    }
    let raw = string_list(obj, "allow", "app.permissions.network.allow")?;
    let mut set = BTreeSet::new();
    for item in &raw {
        set.insert(crate::app_net::normalize_prefix(item)?);
    }
    if set.len() > crate::app_net::MAX_PREFIXES {
        return Err(format!("app.permissions.network.allow は {} 件までです", crate::app_net::MAX_PREFIXES));
    }
    if set.is_empty() {
        return Ok(None);
    }
    Ok(Some(NetworkPermission { allow: set.into_iter().collect() }))
}

fn parse_media(v: &serde_json::Value) -> Result<Option<RequestedMedia>, String> {
    let what = "app.permissions.media";
    let obj = expect_object(v, what)?;
    if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "read" | "allow")) {
        // 書き込み・既定拒否パスの解除・ループバック等の許可は media にはない
        return Err(format!("{} の未知のキー '{}' です（read / allow だけ）", what, k));
    }
    let mut read = BTreeSet::new();
    for item in &string_list(obj, "read", "app.permissions.media.read")? {
        read.insert(crate::app_fs::normalize_scope(item)?);
    }
    if read.len() > MAX_FS_SCOPES {
        return Err(format!("{}.read は {} 件までです", what, MAX_FS_SCOPES));
    }
    let mut allow = BTreeSet::new();
    for item in &string_list(obj, "allow", "app.permissions.media.allow")? {
        let prefix = crate::app_net::normalize_prefix(item)?;
        if !(prefix.starts_with("http://") || prefix.starts_with("https://")) {
            return Err(format!("{}.allow は http(s) の URL だけです: {}", what, item));
        }
        allow.insert(prefix);
    }
    if allow.len() > crate::app_net::MAX_PREFIXES {
        return Err(format!("{}.allow は {} 件までです", what, crate::app_net::MAX_PREFIXES));
    }
    if read.is_empty() && allow.is_empty() {
        return Ok(None);
    }
    Ok(Some(RequestedMedia { read: read.into_iter().collect(), allow: allow.into_iter().collect() }))
}

fn parse_sub_window(v: &serde_json::Value) -> Result<Option<SubWindowPermission>, String> {
    let obj = expect_object(v, "app.permissions.subWindow")?;
    const KEYS: &[&str] = &["maxCount", "maxWidth", "maxHeight", "transparent", "frameless", "alwaysOnTop", "position"];
    if let Some(k) = obj.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!(
            "app.permissions.subWindow の未知のキー '{}' です（{} だけ）",
            k,
            KEYS.join(" / ")
        ));
    }
    // 省略 / null は None（maxWidth / maxHeight では「上限なし」）。書いたときだけ範囲を検査する
    let uint = |key: &str, min: u32, max: u32| -> Result<Option<u32>, String> {
        match obj.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(n) => {
                let n = n
                    .as_u64()
                    .ok_or_else(|| format!("app.permissions.subWindow.{} は正の整数である必要があります", key))?;
                if n < min as u64 || n > max as u64 {
                    let range = if max == u32::MAX { format!("{} 以上", min) } else { format!("{}〜{}", min, max) };
                    return Err(format!(
                        "app.permissions.subWindow.{} は {} の範囲で指定してください（指定: {}）",
                        key, range, n
                    ));
                }
                Ok(Some(n as u32))
            }
        }
    };
    let flag = |key: &str| -> Result<bool, String> {
        match obj.get(key) {
            None | Some(serde_json::Value::Null) => Ok(false),
            Some(serde_json::Value::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("app.permissions.subWindow.{} は true / false である必要があります", key)),
        }
    };
    Ok(Some(SubWindowPermission {
        max_count: uint("maxCount", 1, SUB_WINDOW_MAX_COUNT_CAP)?.unwrap_or(1),
        max_width: uint("maxWidth", SUB_WINDOW_DIM_MIN, u32::MAX)?,
        max_height: uint("maxHeight", SUB_WINDOW_DIM_MIN, u32::MAX)?,
        transparent: flag("transparent")?,
        frameless: flag("frameless")?,
        always_on_top: flag("alwaysOnTop")?,
        position: flag("position")?,
    }))
}

fn parse_audio(v: &serde_json::Value) -> Result<Option<AudioPermission>, String> {
    let obj = expect_object(v, "app.permissions.audio")?;
    const KEYS: &[&str] = &["microphone", "maxSeconds", "sampleRates"];
    if let Some(k) = obj.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("app.permissions.audio の未知のキー '{}' です（{} だけ）", k, KEYS.join(" / ")));
    }
    // マイクの申請は明示させる（`{}` や `false` で黙って何も起きない承認にしない）
    match obj.get("microphone") {
        Some(serde_json::Value::Bool(true)) => {}
        _ => return Err("app.permissions.audio.microphone に true を指定してください（マイクの音声を取得する申請）".to_string()),
    }
    let max_seconds = match obj.get("maxSeconds") {
        None | Some(serde_json::Value::Null) => AUDIO_DEFAULT_MAX_SECONDS,
        Some(n) => {
            let n = n
                .as_u64()
                .ok_or_else(|| "app.permissions.audio.maxSeconds は正の整数である必要があります".to_string())?;
            if n < 1 || n > AUDIO_MAX_SECONDS_CAP as u64 {
                return Err(format!(
                    "app.permissions.audio.maxSeconds は 1〜{} の範囲で指定してください（指定: {}）",
                    AUDIO_MAX_SECONDS_CAP, n
                ));
            }
            n as u32
        }
    };
    let sample_rates = match obj.get("sampleRates") {
        None | Some(serde_json::Value::Null) => vec![AUDIO_DEFAULT_SAMPLE_RATE],
        Some(serde_json::Value::Array(items)) => {
            let mut set = BTreeSet::new();
            for item in items {
                let r = item
                    .as_u64()
                    .ok_or_else(|| "app.permissions.audio.sampleRates の要素は整数（Hz）である必要があります".to_string())?;
                if !AUDIO_SAMPLE_RATES.iter().any(|a| *a as u64 == r) {
                    return Err(format!(
                        "app.permissions.audio.sampleRates に使えないレート {} があります（使えるのは {} だけ）",
                        r,
                        AUDIO_SAMPLE_RATES.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" / ")
                    ));
                }
                set.insert(r as u32);
            }
            if set.is_empty() {
                return Err("app.permissions.audio.sampleRates は 1 つ以上指定してください".to_string());
            }
            set.into_iter().collect()
        }
        Some(_) => return Err("app.permissions.audio.sampleRates は整数の配列である必要があります".to_string()),
    };
    Ok(Some(AudioPermission { microphone: true, max_seconds, sample_rates }))
}

fn parse_scheduler(v: &serde_json::Value) -> Result<Option<SchedulerPermission>, String> {
    let what = "app.permissions.scheduler";
    let obj = expect_object(v, what)?;
    if let Some(k) = obj.keys().find(|k| k.as_str() != "jobs") {
        return Err(format!("{} の未知のキー '{}' です（jobs だけ）", what, k));
    }
    let items = match obj.get("jobs") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(serde_json::Value::Array(a)) => a,
        Some(_) => return Err(format!("{}.jobs はオブジェクトの配列である必要があります", what)),
    };
    if items.is_empty() {
        return Ok(None);
    }
    if items.len() > SCHEDULER_MAX_JOBS {
        return Err(format!("{}.jobs は {} 件までです", what, SCHEDULER_MAX_JOBS));
    }
    let mut jobs: Vec<SchedulerJob> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let jwhat = format!("{}.jobs[{}]", what, i);
        let jobj = expect_object(item, &jwhat)?;
        if let Some(k) = jobj.keys().find(|k| !matches!(k.as_str(), "id" | "cron" | "action")) {
            return Err(format!("{} の未知のキー '{}' です（id / cron / action だけ）", jwhat, k));
        }
        let id = optional_text(jobj, "id", SCHEDULER_MAX_ID_CHARS)?
            .ok_or_else(|| format!("{}.id は必須です（空でない文字列）", jwhat))?;
        if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("{}.id は英数字・-・_ だけにしてください: {}", jwhat, id));
        }
        if jobs.iter().any(|j| j.id == id) {
            return Err(format!("{}.id '{}' が重複しています", what, id));
        }
        let cron_raw = optional_text(jobj, "cron", 100)?
            .ok_or_else(|| format!("{}.cron は必須です（5 フィールドの cron 式）", jwhat))?;
        let cron = crate::app_scheduler::normalize_cron(&cron_raw);
        crate::app_scheduler::CronSpec::parse(&cron).map_err(|e| format!("{}.cron が不正です: {}", jwhat, e))?;
        let action = parse_scheduler_action(jobj.get("action"), &format!("{}.action", jwhat))?;
        jobs.push(SchedulerJob { id, cron, action });
    }
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Some(SchedulerPermission { jobs }))
}

fn parse_scheduler_action(v: Option<&serde_json::Value>, what: &str) -> Result<SchedulerAction, String> {
    let obj = expect_object(v.ok_or_else(|| format!("{} は必須です", what))?, what)?;
    let ty = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match ty {
        "tool" => {
            if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "type" | "tool" | "params")) {
                return Err(format!("{} の未知のキー '{}' です（type / tool / params だけ）", what, k));
            }
            let tool = optional_text(obj, "tool", MAX_TARGET_CHARS)?
                .ok_or_else(|| format!("{}.tool は必須です", what))?;
            let params = match obj.get("params") {
                None | Some(serde_json::Value::Null) => empty_object(),
                Some(p @ serde_json::Value::Object(_)) => p.clone(),
                Some(_) => return Err(format!("{}.params は JSON オブジェクトである必要があります", what)),
            };
            let size = serde_json::to_string(&params).map_err(|e| e.to_string())?.len();
            if size > SCHEDULER_PARAMS_MAX_BYTES {
                return Err(format!("{}.params は {} バイト以内にしてください（{} バイト）", what, SCHEDULER_PARAMS_MAX_BYTES, size));
            }
            Ok(SchedulerAction::Tool { tool, params })
        }
        "aiTerminal" => {
            const KEYS: &[&str] = &["type", "title", "initialPrompt", "systemPrompt", "model", "cwd"];
            if let Some(k) = obj.keys().find(|k| !KEYS.contains(&k.as_str())) {
                return Err(format!("{} の未知のキー '{}' です（{} だけ）", what, k, KEYS.join(" / ")));
            }
            let text = |key: &str, max: usize| -> Result<Option<String>, String> {
                match obj.get(key) {
                    None | Some(serde_json::Value::Null) => Ok(None),
                    Some(serde_json::Value::String(s)) => {
                        if s.chars().count() > max {
                            return Err(format!("{}.{} は {} 文字以内にしてください", what, key, max));
                        }
                        if s.contains('\0') {
                            return Err(format!("{}.{} に NUL は使えません", what, key));
                        }
                        Ok(Some(s.clone()).filter(|s| !s.trim().is_empty()))
                    }
                    Some(_) => Err(format!("{}.{} は文字列である必要があります", what, key)),
                }
            };
            // 無人で起動するエージェントが何をするかは initialPrompt で決まる。空は「何もしない端末を開く」
            // だけなので、書き忘れとして拒否する
            let initial_prompt = text("initialPrompt", crate::ai_terminal::MAX_PROMPT_CHARS)?
                .ok_or_else(|| format!("{}.initialPrompt は必須です（空でない文字列）", what))?;
            let model = text("model", MAX_NAME_CHARS)?;
            if let Some(m) = model.as_deref() {
                crate::ai_terminal::validate_model(m).map_err(|e| format!("{}.model: {}", what, e))?;
            }
            Ok(SchedulerAction::AiTerminal {
                title: text("title", crate::ai_terminal::MAX_TITLE_CHARS)?,
                initial_prompt,
                system_prompt: text("systemPrompt", crate::ai_terminal::MAX_PROMPT_CHARS)?,
                model,
                cwd: text("cwd", 260)?,
            })
        }
        other => Err(format!("{}.type '{}' は指定できません（tool / aiTerminal）", what, other)),
    }
}

fn parse_memory(v: &serde_json::Value) -> Result<Option<MemoryPermission>, String> {
    let obj = expect_object(v, "app.permissions.memory")?;
    if let Some(k) = obj.keys().find(|k| k.as_str() != "maxBytes") {
        return Err(format!("app.permissions.memory の未知のキー '{}' です（maxBytes だけ）", k));
    }
    // memory 自体は任意。書くなら maxBytes は必須（未指定ならメモリ API は使えない、が親 issue の方針）
    let max_bytes = obj
        .get("maxBytes")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| "app.permissions.memory には maxBytes（正の整数）が必須です".to_string())?;
    let hard_cap = crate::ARTIFACT_MEMORY_MAX_BYTES as u64;
    if max_bytes == 0 || max_bytes > hard_cap {
        return Err(format!(
            "app.permissions.memory.maxBytes は 1〜{} の範囲で指定してください（指定: {}）",
            hard_cap, max_bytes
        ));
    }
    Ok(Some(MemoryPermission { max_bytes }))
}

fn parse_ai_terminal(v: &serde_json::Value) -> Result<Option<AiTerminalPermission>, String> {
    let obj = expect_object(v, "app.permissions.aiTerminal")?;
    if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "agents" | "maxConcurrent")) {
        // 権限モード・権限スキップ・許可ツール一覧などは v1 ではアプリから指定できない。
        return Err(format!(
            "app.permissions.aiTerminal の未知のキー '{}' です（agents / maxConcurrent だけ。権限モード・権限スキップ・許可ツールの指定は v1 ではできません）",
            k
        ));
    }
    let agents = string_list(obj, "agents", "app.permissions.aiTerminal.agents")?;
    if agents.is_empty() {
        return Err("app.permissions.aiTerminal.agents は必須です（例: [\"claude\"]）".to_string());
    }
    if let Some(a) = agents.iter().find(|a| !AI_TERMINAL_AGENTS.contains(&a.as_str())) {
        return Err(format!(
            "エージェント '{}' は指定できません。v1 で指定できるのは {} だけです",
            a,
            AI_TERMINAL_AGENTS.join(", ")
        ));
    }
    let max_concurrent = obj
        .get("maxConcurrent")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| "app.permissions.aiTerminal には maxConcurrent（正の整数）が必須です".to_string())?;
    if max_concurrent == 0 || max_concurrent > AI_TERMINAL_MAX_CONCURRENT_CAP as u64 {
        return Err(format!(
            "app.permissions.aiTerminal.maxConcurrent は 1〜{} の範囲で指定してください（指定: {}）",
            AI_TERMINAL_MAX_CONCURRENT_CAP, max_concurrent
        ));
    }
    Ok(Some(AiTerminalPermission { agents, max_concurrent: max_concurrent as u32 }))
}

/// アプリのメモリ（シリアライズした JSON のバイト数）が承認済みの `maxBytes` に収まるか。
/// アプリ自身の書き込み（`app_set_memory`）と MCP の `artifact_store` の両方がここを通る。
///
/// 見るのは**書き込み後の合計サイズ**だけ。上限を下げる manifest 変更のあとで既存データが新しい
/// 上限を超えていても、データは消さない。超過している間は、新しい上限に収まらない書き込みを
/// 拒否するだけで（収まる書き込み＝縮める書き込みは通る）、読み取りは影響を受けない。
/// 戻り値は書き込み後のバイト数。
pub fn check_memory_size(memory: &serde_json::Value, max_bytes: u64) -> Result<u64, String> {
    let size = serde_json::to_string(memory).map_err(|e| e.to_string())?.len() as u64;
    if size > max_bytes {
        return Err(format!(
            "メモリーがこのアプリの承認済み最大サイズを超えています: {} バイト（maxBytes {}）",
            size, max_bytes
        ));
    }
    Ok(size)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopedKind {
    Fs,
    Db,
}

impl ScopedKind {
    fn key(self) -> &'static str {
        match self {
            ScopedKind::Fs => "fs",
            ScopedKind::Db => "db",
        }
    }
}

/// `fs` と `db` の共通の解析。どちらも `read` / `write` のスコープと `allowSensitivePaths` を持つ。
///
/// 既定拒否パスの事前検査だけ違う: `fs` は write のスコープだけ、`db` は read も対象にする
/// （DB ファイルの置き場所としても使えない。読み取り専用で開いても、同じディレクトリに副産物ができる）。
fn parse_scoped(v: &serde_json::Value, kind: ScopedKind) -> Result<Option<RequestedFs>, String> {
    let key_name = kind.key();
    let what = format!("app.permissions.{}", key_name);
    let obj = expect_object(v, &what)?;
    if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "read" | "write" | "allowSensitivePaths")) {
        return Err(format!(
            "{} の未知のキー '{}' です（read / write / allowSensitivePaths だけ）",
            what, k
        ));
    }
    let allow_sensitive_paths = match obj.get("allowSensitivePaths") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(b)) => *b,
        Some(_) => return Err(format!("{}.allowSensitivePaths は真偽値である必要があります", what)),
    };
    let normalize = |key: &str| -> Result<Vec<String>, String> {
        let raw = string_list(obj, key, &format!("{}.{}", what, key))?;
        let mut set = BTreeSet::new();
        for item in &raw {
            set.insert(crate::app_fs::normalize_scope(item)?);
        }
        if set.len() > MAX_FS_SCOPES {
            return Err(format!("{}.{} は {} 件までです", what, key, MAX_FS_SCOPES));
        }
        Ok(set.into_iter().collect())
    };
    let read = normalize("read")?;
    let write = normalize("write")?;
    if read.is_empty() && write.is_empty() {
        if allow_sensitive_paths {
            return Err(format!("{}.allowSensitivePaths は read / write と一緒に指定してください", what));
        }
        return Ok(None);
    }
    if !allow_sensitive_paths {
        // 承認ダイアログへ行く前に、使えないと分かっている申請は manifest 不正として返す
        let checked: Vec<(&str, &String)> = match kind {
            ScopedKind::Fs => write.iter().map(|s| ("write", s)).collect(),
            ScopedKind::Db => read
                .iter()
                .map(|s| ("read", s))
                .chain(write.iter().map(|s| ("write", s)))
                .collect(),
        };
        if let Some((list, s)) = checked.into_iter().find(|(_, s)| crate::app_fs::scope_hits_sensitive(s)) {
            return Err(format!(
                "{}.{} の '{}' は既定で{}を拒否するパス（{}）を含みます。本当に必要なら allowSensitivePaths: true を指定してください（承認ダイアログで最も強い警告が出ます）",
                what,
                list,
                s,
                if kind == ScopedKind::Db { "DB の置き場所にすること" } else { "書き込み" },
                crate::app_fs::SENSITIVE_NAMES.join(" / ")
            ));
        }
    }
    Ok(Some(RequestedFs { read, write, allow_sensitive_paths }))
}

/// 保存済みの `requested` と、いまの manifest の `requested` が違うか（＝再承認が要るか）。
/// 権限が狭まる変更でも `true` を返す（単純さを優先）。名前・説明・版は比較に入らない。
pub fn requested_changed(saved: &RequestedPermissions, current: &RequestedPermissions) -> bool {
    saved != current
}

/// 関門 (1): 呼ぼうとしたツールが承認済みか。**保存済みの `resolved` だけ**を見る。
pub fn authorize_app_tool(resolved: &ResolvedPermissions, tool: &str) -> Result<(), String> {
    let granted = resolved
        .mcp
        .as_ref()
        .map(|m| m.tools.iter().any(|t| t == tool))
        .unwrap_or(false);
    if granted {
        Ok(())
    } else {
        Err(format!(
            "ツール '{}' はこのアプリに承認されていません（manifest で宣言し、起動時の承認ダイアログで許可されたツールだけ呼べます）",
            tool
        ))
    }
}

/// 関門 (3a): 宛先ワークツリー（自分以外）が、承認済みの越境先に当たるか。
///
/// `dest_targets` は `cross_worktree_dest_targets` の戻り値（宛先にマッチしうる購読 target の全集合）。
/// 承認した `stored` のどれかがそこに含まれれば当たる。`targets` を省略した（空の）アプリは常に偽
/// ＝自ワークツリーのみ。これだけでは通さず、呼び出し側が既存の購読関係（#211）も別途確認する。
pub fn authorize_app_cross_target(
    resolved: &ResolvedPermissions,
    dest_targets: &[String],
) -> Result<(), String> {
    let hit = resolved
        .mcp
        .as_ref()
        .map(|m| m.targets.iter().any(|t| dest_targets.contains(&t.stored)))
        .unwrap_or(false);
    if hit {
        Ok(())
    } else {
        Err("宛先のワークツリーは、このアプリが承認された越境先（manifest の mcp.targets）に含まれていません。購読があっても、承認されていない宛先へは届きません".to_string())
    }
}

/// `*` / `workgroup:` / `repo:` のいずれかか（承認後に作られるワークツリーにも当たる）。
pub fn is_wildcard_target(stored: &str) -> bool {
    stored == crate::event_db::TARGET_ALL
        || stored.starts_with(crate::event_db::TARGET_WORKGROUP_PREFIX)
        || stored.starts_with(crate::event_db::TARGET_REPO_PREFIX)
}

/// 承認ダイアログで最も強く警告するツール（ユーザーと同等の権限＝任意コマンド実行になりうる）。
pub const DANGEROUS_TOOLS: &[&str] = &[
    "oretachi_write_terminal",
    "oretachi_send_keys",
    "oretachi_answer_prompt",
    "oretachi_add_task",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok(v: serde_json::Value) -> AppManifest {
        parse_manifest(&v).expect("valid manifest")
    }

    fn err(v: serde_json::Value) -> String {
        parse_manifest(&v).expect_err("invalid manifest")
    }

    #[test]
    fn minimal_manifest_has_no_permissions() {
        let m = ok(json!({ "name": "Dash" }));
        assert_eq!(m.name, "Dash");
        assert_eq!(m.permissions, RequestedPermissions::default());
    }

    #[test]
    fn name_is_required() {
        assert!(err(json!({})).contains("app.name"));
        assert!(err(json!({ "name": "  " })).contains("app.name"));
    }

    #[test]
    fn normalizes_tools_and_targets() {
        let m = ok(json!({
            "name": "x",
            "permissions": { "mcp": {
                "tools": ["notify_worktree", "oretachi_poll_inbox", "notify_worktree"],
                "targets": [" wt-b ", "*", "wt-b"]
            }}
        }));
        let mcp = m.permissions.mcp.unwrap();
        assert_eq!(mcp.tools, vec!["notify_worktree", "oretachi_poll_inbox"]);
        assert_eq!(mcp.targets, vec!["*", "wt-b"]);
    }

    #[test]
    fn order_of_tools_does_not_change_requested() {
        let a = ok(json!({ "name": "x", "permissions": { "mcp": { "tools": ["notify_worktree", "oretachi_poll_inbox"] }}}));
        let b = ok(json!({ "name": "y", "description": "d", "permissions": { "mcp": { "tools": ["oretachi_poll_inbox", "notify_worktree"] }}}));
        assert!(!requested_changed(&a.permissions, &b.permissions));
    }

    #[test]
    fn narrowing_still_counts_as_change() {
        let a = ok(json!({ "name": "x", "permissions": { "mcp": { "tools": ["notify_worktree", "oretachi_poll_inbox"] }}}));
        let b = ok(json!({ "name": "x", "permissions": { "mcp": { "tools": ["notify_worktree"] }}}));
        assert!(requested_changed(&a.permissions, &b.permissions));
    }

    #[test]
    fn rejects_tool_outside_callable_list() {
        let e = err(json!({ "name": "x", "permissions": { "mcp": { "tools": ["oretachi_kill_terminal"] }}}));
        assert!(e.contains("oretachi_kill_terminal"));
    }

    #[test]
    fn rejects_unknown_permission_key() {
        let e = err(json!({ "name": "x", "permissions": { "root": {} }}));
        assert!(e.contains("未知の権限"));
    }

    #[test]
    fn reserved_permission_keys_are_not_yet_supported() {
        for key in RESERVED_PERMISSION_KEYS {
            let e = err(json!({ "name": "x", "permissions": { *key: {} }}));
            assert!(e.contains("まだ使えません"), "{key}: {e}");
        }
    }

    #[test]
    fn parses_sub_window_permission_with_defaults() {
        let m = ok(json!({ "name": "x", "permissions": { "subWindow": {} }}));
        let s = m.permissions.sub_window.unwrap();
        // 省略は「上限なし」（480×640 は開くときの既定の大きさであって上限ではない）
        assert_eq!((s.max_count, s.max_width, s.max_height), (1, None, None));
        assert!(!s.transparent && !s.frameless && !s.always_on_top && !s.position);
        let m = ok(json!({ "name": "x", "permissions": { "subWindow": {
            "maxCount": 2, "maxWidth": 300, "maxHeight": 400,
            "transparent": true, "frameless": true, "alwaysOnTop": true, "position": true
        }}}));
        let s = m.permissions.sub_window.unwrap();
        assert_eq!((s.max_count, s.max_width, s.max_height), (2, Some(300), Some(400)));
        assert!(s.transparent && s.frameless && s.always_on_top && s.position);
    }

    #[test]
    fn rejects_bad_sub_window_permission() {
        let p = |v: serde_json::Value| err(json!({ "name": "x", "permissions": { "subWindow": v }}));
        assert!(p(json!([])).contains("JSON オブジェクト"));
        assert!(p(json!({ "skipTaskbar": true })).contains("未知のキー"));
        assert!(p(json!({ "maxCount": 0 })).contains("maxCount"));
        assert!(p(json!({ "maxCount": SUB_WINDOW_MAX_COUNT_CAP + 1 })).contains("maxCount"));
        // 上限は無い（大きな値は通る）。下限だけ検査する
        assert!(p(json!({ "maxWidth": 0 })).contains("maxWidth"));
        assert!(p(json!({ "maxHeight": SUB_WINDOW_DIM_MIN - 1 })).contains("maxHeight"));
        assert!(p(json!({ "maxWidth": "big" })).contains("maxWidth"));
        assert!(p(json!({ "transparent": "yes" })).contains("transparent"));
    }

    #[test]
    fn sub_window_change_requires_reapproval_only_when_the_set_changes() {
        let a = ok(json!({ "name": "x", "permissions": { "subWindow": { "transparent": true }}}));
        // 既定を明示しても同じ集合
        let same = ok(json!({ "name": "y", "permissions": { "subWindow": { "transparent": true, "maxCount": 1 }}}));
        // 上限の有無は権限の違い（省略 = 上限なし。書けば上限つきなので集合が変わる）
        let capped = ok(json!({ "name": "x", "permissions": { "subWindow": { "transparent": true, "maxWidth": 480 }}}));
        assert_ne!(a.permissions, capped.permissions);
        // 巨大な値も書ける（上限は撤廃）
        let huge = ok(json!({ "name": "x", "permissions": { "subWindow": { "maxWidth": 100000 }}}));
        assert_eq!(huge.permissions.sub_window.unwrap().max_width, Some(100000));
        let wider = ok(json!({ "name": "x", "permissions": { "subWindow": { "transparent": true, "alwaysOnTop": true }}}));
        assert_eq!(a.permissions, same.permissions);
        assert_ne!(a.permissions, wider.permissions);
    }

    #[test]
    fn parses_audio_permission_with_defaults_and_normalizes() {
        let m = ok(json!({ "name": "x", "permissions": { "audio": { "microphone": true }}}));
        let a = m.permissions.audio.unwrap();
        assert_eq!((a.microphone, a.max_seconds, a.sample_rates.clone()), (true, 60, vec![16000]));
        // ソート・重複除去。既定を明示しても同じ集合
        let m = ok(json!({ "name": "x", "permissions": { "audio": {
            "microphone": true, "maxSeconds": 120, "sampleRates": [48000, 16000, 16000]
        }}}));
        let a = m.permissions.audio.unwrap();
        assert_eq!((a.max_seconds, a.sample_rates), (120, vec![16000, 48000]));
        let same = ok(json!({ "name": "y", "permissions": { "audio": { "microphone": true, "maxSeconds": 60, "sampleRates": [16000] }}}));
        let dflt = ok(json!({ "name": "x", "permissions": { "audio": { "microphone": true }}}));
        assert_eq!(same.permissions, dflt.permissions);
    }

    #[test]
    fn rejects_bad_audio_permission() {
        let p = |v: serde_json::Value| err(json!({ "name": "x", "permissions": { "audio": v }}));
        assert!(p(json!([])).contains("JSON オブジェクト"));
        // マイクの申請は明示が必須（空・false・文字列は拒否）
        assert!(p(json!({})).contains("microphone"));
        assert!(p(json!({ "microphone": false })).contains("microphone"));
        assert!(p(json!({ "microphone": "yes" })).contains("microphone"));
        assert!(p(json!({ "microphone": true, "loopback": true })).contains("未知のキー"));
        assert!(p(json!({ "microphone": true, "maxSeconds": 0 })).contains("maxSeconds"));
        assert!(p(json!({ "microphone": true, "maxSeconds": AUDIO_MAX_SECONDS_CAP + 1 })).contains("maxSeconds"));
        assert!(p(json!({ "microphone": true, "maxSeconds": "long" })).contains("maxSeconds"));
        assert!(p(json!({ "microphone": true, "sampleRates": [] })).contains("sampleRates"));
        assert!(p(json!({ "microphone": true, "sampleRates": [44100] })).contains("44100"));
        assert!(p(json!({ "microphone": true, "sampleRates": "16000" })).contains("sampleRates"));
        assert!(p(json!({ "microphone": true, "sampleRates": ["16000"] })).contains("sampleRates"));
    }

    #[test]
    fn audio_change_requires_reapproval_only_when_the_set_changes() {
        let a = ok(json!({ "name": "x", "permissions": { "audio": { "microphone": true }}}));
        let longer = ok(json!({ "name": "x", "permissions": { "audio": { "microphone": true, "maxSeconds": 61 }}}));
        let wider = ok(json!({ "name": "x", "permissions": { "audio": { "microphone": true, "sampleRates": [16000, 48000] }}}));
        let none = ok(json!({ "name": "x" }));
        assert!(requested_changed(&a.permissions, &longer.permissions));
        assert!(requested_changed(&a.permissions, &wider.permissions));
        // 権限が狭まる変更（外す）でも再承認
        assert!(requested_changed(&a.permissions, &none.permissions));
        // 名前・説明の変更だけでは出ない
        let renamed = ok(json!({ "name": "other", "description": "d", "permissions": { "audio": { "microphone": true }}}));
        assert!(!requested_changed(&a.permissions, &renamed.permissions));
    }

    #[test]
    fn parses_network_permission() {
        let m = ok(json!({ "name": "x", "permissions": { "network": { "allow": [
            "https://b.example.com", "HTTPS://api.example.com/v1/", "https://b.example.com/"
        ] }}}));
        // 正規化・ソート・重複除去
        assert_eq!(
            m.permissions.network.unwrap().allow,
            vec!["https://api.example.com/v1/", "https://b.example.com/"]
        );
        // 空なら権限なし
        assert!(ok(json!({ "name": "x", "permissions": { "network": { "allow": [] }}})).permissions.network.is_none());
    }

    #[test]
    fn rejects_bad_network_permission() {
        let p = |v: serde_json::Value| err(json!({ "name": "x", "permissions": { "network": v }}));
        // 解除オプションは無い（未知キー）
        assert!(p(json!({ "allow": ["https://a.example.com/"], "allowPrivateNetwork": true })).contains("未知のキー"));
        assert!(p(json!({ "allow": "https://a.example.com/" })).contains("配列"));
        assert!(p(json!({ "allow": ["ftp://a.example.com/"] })).contains("http"));
        assert!(p(json!({ "allow": ["http://127.0.0.1:8080/"] })).contains("ループバック"));
        assert!(p(json!({ "allow": ["http://localhost/"] })).contains("ループバック"));
        assert!(p(json!({ "allow": ["https://u:p@a.example.com/"] })).contains("認証情報"));
        let many: Vec<String> = (0..=crate::app_net::MAX_PREFIXES).map(|i| format!("https://h{}.example.com/", i)).collect();
        assert!(p(json!({ "allow": many })).contains("件まで"));
    }

    #[test]
    fn network_change_requires_reapproval() {
        let a = ok(json!({ "name": "x", "permissions": { "network": { "allow": ["https://a.example.com/"] }}}));
        let b = ok(json!({ "name": "x", "permissions": { "network": { "allow": ["https://a.example.com/", "https://b.example.com/"] }}}));
        let c = ok(json!({ "name": "renamed", "permissions": { "network": { "allow": ["https://a.example.com/"] }}}));
        assert!(requested_changed(&a.permissions, &b.permissions));
        assert!(requested_changed(&b.permissions, &a.permissions)); // 狭まる変更でも出す
        assert!(!requested_changed(&a.permissions, &c.permissions)); // 名前だけの変更では出さない
    }

    #[test]
    fn parses_and_normalizes_media_permission() {
        let m = ok(json!({ "name": "x", "permissions": { "media": {
            "read": ["worktree://videos/", "worktree://videos/", "artifact://clips"],
            "allow": ["https://cdn.example.com/v", "https://cdn.example.com/v"]
        }}}));
        let media = m.permissions.media.unwrap();
        assert_eq!(media.read, vec!["artifact://clips/", "worktree://videos/"]);
        assert_eq!(media.allow, vec!["https://cdn.example.com/v"]);
        // どちらかだけでもよい
        assert!(ok(json!({ "name": "x", "permissions": { "media": { "allow": ["https://a.example.com/"] }}})).permissions.media.is_some());
        assert!(ok(json!({ "name": "x", "permissions": { "media": { "read": ["worktree://v/"] }}})).permissions.media.is_some());
        // 空は権限なし
        assert!(ok(json!({ "name": "x", "permissions": { "media": { "read": [], "allow": [] }}})).permissions.media.is_none());
    }

    #[test]
    fn rejects_bad_media_permission() {
        let p = |v: serde_json::Value| err(json!({ "name": "x", "permissions": { "media": v }}));
        // 書き込み・既定拒否パスの解除は無い（未知キー）
        assert!(p(json!({ "read": ["worktree://v/"], "write": ["worktree://v/"] })).contains("未知のキー"));
        assert!(p(json!({ "read": ["worktree://v/"], "allowSensitivePaths": true })).contains("未知のキー"));
        assert!(p(json!({ "allow": "https://a.example.com/" })).contains("配列"));
        // http(s) だけ（ws は通信路が違う）
        assert!(p(json!({ "allow": ["wss://a.example.com/"] })).contains("http(s)"));
        assert!(p(json!({ "allow": ["ftp://a.example.com/"] })).contains("http"));
        assert!(p(json!({ "allow": ["http://127.0.0.1:8080/"] })).contains("ループバック"));
        assert!(p(json!({ "allow": ["https://u:p@a.example.com/"] })).contains("認証情報"));
        assert!(p(json!({ "read": ["../x/"] })).len() > 0);
        let many: Vec<String> = (0..=crate::app_net::MAX_PREFIXES).map(|i| format!("https://h{}.example.com/", i)).collect();
        assert!(p(json!({ "allow": many })).contains("件まで"));
        let scopes: Vec<String> = (0..=MAX_FS_SCOPES).map(|i| format!("worktree://d{}/", i)).collect();
        assert!(p(json!({ "read": scopes })).contains("件まで"));
    }

    #[test]
    fn media_change_requires_reapproval() {
        let a = ok(json!({ "name": "x", "permissions": { "media": { "allow": ["https://a.example.com/"] }}}));
        let b = ok(json!({ "name": "x", "permissions": { "media": { "allow": ["https://a.example.com/"], "read": ["worktree://v/"] }}}));
        let c = ok(json!({ "name": "renamed", "permissions": { "media": { "allow": ["https://a.example.com/"] }}}));
        assert!(requested_changed(&a.permissions, &b.permissions));
        assert!(requested_changed(&b.permissions, &a.permissions));
        assert!(!requested_changed(&a.permissions, &c.permissions));
    }

    fn sched(jobs: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
        let mut perms = json!({ "scheduler": { "jobs": jobs } });
        if let (Some(o), Some(e)) = (perms.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        json!({ "name": "x", "permissions": perms })
    }

    fn tool_job(id: &str, cron: &str) -> serde_json::Value {
        json!({ "id": id, "cron": cron, "action": { "type": "tool", "tool": "notify_worktree", "params": { "kind": "message" } } })
    }

    fn mcp_notify() -> serde_json::Value {
        json!({ "mcp": { "tools": ["notify_worktree"] } })
    }

    #[test]
    fn parses_scheduler_and_normalizes() {
        let m = ok(sched(
            json!([
                tool_job("b-job", "  0   9 * *  1-5 "),
                {
                    "id": "a_job", "cron": "*/30 * * * *",
                    "action": { "type": "aiTerminal", "initialPrompt": "run report", "title": "  ", "model": null }
                }
            ]),
            json!({ "mcp": { "tools": ["notify_worktree"] }, "aiTerminal": { "agents": ["claude"], "maxConcurrent": 1 } }),
        ));
        let jobs = m.permissions.scheduler.unwrap().jobs;
        // id でソート、cron は空白 1 つに正規化
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), vec!["a_job", "b-job"]);
        assert_eq!(jobs[1].cron, "0 9 * * 1-5");
        match &jobs[0].action {
            SchedulerAction::AiTerminal { title, initial_prompt, model, .. } => {
                assert_eq!((title.as_deref(), initial_prompt.as_str(), model.as_deref()), (None, "run report", None));
            }
            other => panic!("{other:?}"),
        }
        // params 省略は {}
        let m = ok(sched(
            json!([{ "id": "t", "cron": "* * * * *", "action": { "type": "tool", "tool": "notify_worktree" } }]),
            mcp_notify(),
        ));
        match &m.permissions.scheduler.unwrap().jobs[0].action {
            SchedulerAction::Tool { params, .. } => assert_eq!(params, &json!({})),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_scheduler_means_no_permission() {
        assert!(ok(json!({ "name": "x", "permissions": { "scheduler": { "jobs": [] } } })).permissions.scheduler.is_none());
        assert!(ok(json!({ "name": "x", "permissions": { "scheduler": {} } })).permissions.scheduler.is_none());
    }

    #[test]
    fn rejects_bad_scheduler() {
        let e = |jobs: serde_json::Value, extra: serde_json::Value| err(sched(jobs, extra));
        let m = mcp_notify();
        assert!(err(json!({ "name": "x", "permissions": { "scheduler": { "jobs": [], "tz": "UTC" } } })).contains("未知のキー"));
        assert!(err(json!({ "name": "x", "permissions": { "scheduler": { "jobs": "x" } } })).contains("配列"));
        assert!(e(json!([tool_job("a", "* * * *")]), m.clone()).contains("cron"));
        assert!(e(json!([tool_job("a", "61 * * * *")]), m.clone()).contains("cron"));
        assert!(e(json!([tool_job("a b", "* * * * *")]), m.clone()).contains("id"));
        assert!(e(json!([tool_job("a", "* * * * *"), tool_job("a", "0 * * * *")]), m.clone()).contains("重複"));
        assert!(e(json!([{ "cron": "* * * * *", "action": { "type": "tool", "tool": "notify_worktree" } }]), m.clone()).contains("id"));
        assert!(e(json!([{ "id": "a", "cron": "* * * * *" }]), m.clone()).contains("action"));
        assert!(e(json!([{ "id": "a", "cron": "* * * * *", "action": { "type": "shell", "cmd": "x" } }]), m.clone()).contains("type"));
        assert!(e(json!([{ "id": "a", "cron": "* * * * *", "action": { "type": "tool", "tool": "notify_worktree", "x": 1 } }]), m.clone()).contains("未知のキー"));
        assert!(e(json!([{ "id": "a", "cron": "* * * * *", "action": { "type": "tool", "tool": "notify_worktree", "params": [] } }]), m.clone()).contains("オブジェクト"));
        assert!(e(json!([{ "id": "a", "cron": "* * * * *", "extra": 1, "action": { "type": "tool", "tool": "notify_worktree" } }]), m.clone()).contains("未知のキー"));
        let big = "x".repeat(SCHEDULER_PARAMS_MAX_BYTES + 1);
        assert!(e(json!([{ "id": "a", "cron": "* * * * *", "action": { "type": "tool", "tool": "notify_worktree", "params": { "k": big } } }]), m.clone()).contains("バイト"));
        let many: Vec<_> = (0..=SCHEDULER_MAX_JOBS).map(|i| tool_job(&format!("j{i}"), "* * * * *")).collect();
        assert!(e(json!(many), m).contains("件まで"));
    }

    #[test]
    fn rejects_bad_scheduler_ai_terminal_action() {
        let ai = json!({ "aiTerminal": { "agents": ["claude"], "maxConcurrent": 1 } });
        let job = |action: serde_json::Value| json!([{ "id": "a", "cron": "* * * * *", "action": action }]);
        // initialPrompt は必須（空・空白も不可）
        assert!(err(sched(job(json!({ "type": "aiTerminal" })), ai.clone())).contains("initialPrompt"));
        assert!(err(sched(job(json!({ "type": "aiTerminal", "initialPrompt": "  " })), ai.clone())).contains("initialPrompt"));
        assert!(err(sched(job(json!({ "type": "aiTerminal", "initialPrompt": "x", "permissionMode": "bypass" })), ai.clone())).contains("未知のキー"));
        assert!(err(sched(job(json!({ "type": "aiTerminal", "initialPrompt": "x\u{0}" })), ai.clone())).contains("NUL"));
        assert!(err(sched(job(json!({ "type": "aiTerminal", "initialPrompt": "x", "model": "--dangerous flag" })), ai)).contains("model"));
    }

    #[test]
    fn scheduler_cannot_bypass_other_permissions() {
        // tool ジョブは mcp.tools に宣言したツールだけ
        let e = err(sched(json!([tool_job("a", "* * * * *")]), json!({})));
        assert!(e.contains("mcp.tools"), "{e}");
        let e = err(sched(json!([tool_job("a", "* * * * *")]), json!({ "mcp": { "tools": ["oretachi_poll_inbox"] } })));
        assert!(e.contains("mcp.tools"), "{e}");
        // aiTerminal ジョブは aiTerminal 権限が要る
        let e = err(sched(
            json!([{ "id": "a", "cron": "* * * * *", "action": { "type": "aiTerminal", "initialPrompt": "x" } }]),
            json!({}),
        ));
        assert!(e.contains("aiTerminal"), "{e}");
    }

    #[test]
    fn scheduler_change_requires_reapproval() {
        let a = ok(sched(json!([tool_job("a", "0 9 * * *")]), mcp_notify()));
        let same = ok(sched(json!([tool_job("a", "0  9 * * *")]), mcp_notify())); // 空白の揺れ
        let later = ok(sched(json!([tool_job("a", "0 10 * * *")]), mcp_notify()));
        let none = ok(json!({ "name": "x", "permissions": mcp_notify() }));
        assert!(!requested_changed(&a.permissions, &same.permissions));
        assert!(requested_changed(&a.permissions, &later.permissions));
        assert!(requested_changed(&a.permissions, &none.permissions));
        assert!(requested_changed(&none.permissions, &a.permissions));
    }

    #[test]
    fn legacy_grant_without_scheduler_deserializes_as_no_scheduler() {
        let raw = r#"{"grantedAt":1,"requested":{"memory":{"maxBytes":10}},"resolved":{"memory":{"maxBytes":10}}}"#;
        let g: AppGrant = serde_json::from_str(raw).unwrap();
        assert!(g.resolved.scheduler.is_none() && g.requested.scheduler.is_none());
    }

    #[test]
    fn scheduler_grant_roundtrips_through_json() {
        let m = ok(sched(
            json!([
                tool_job("t", "5 4 * * *"),
                { "id": "ai", "cron": "0 * * * *", "action": { "type": "aiTerminal", "initialPrompt": "p", "systemPrompt": "s", "cwd": "sub" } }
            ]),
            json!({ "mcp": { "tools": ["notify_worktree"] }, "aiTerminal": { "agents": ["claude"], "maxConcurrent": 1 } }),
        ));
        let sch = m.permissions.scheduler.clone().unwrap();
        let s = serde_json::to_string(&sch).unwrap();
        assert!(s.contains("\"type\":\"aiTerminal\"") && s.contains("initialPrompt") && s.contains("\"type\":\"tool\""));
        assert_eq!(serde_json::from_str::<SchedulerPermission>(&s).unwrap(), sch);
    }

    #[test]
    fn legacy_grant_without_media_deserializes_as_no_media() {
        // #402 より前に保存された grant（media キー無し）は、media 権限なしとして読める
        let raw = r#"{"grantedAt":1,"requested":{"memory":{"maxBytes":10}},"resolved":{"memory":{"maxBytes":10}}}"#;
        let g: AppGrant = serde_json::from_str(raw).unwrap();
        assert!(g.resolved.media.is_none() && g.requested.media.is_none());
    }

    #[test]
    fn parses_ai_terminal_permission() {
        let m = ok(json!({ "name": "x", "permissions": { "aiTerminal": { "agents": ["claude", "claude"], "maxConcurrent": 2 }}}));
        let t = m.permissions.ai_terminal.unwrap();
        assert_eq!(t.agents, vec!["claude"]);
        assert_eq!(t.max_concurrent, 2);
    }

    #[test]
    fn rejects_bad_ai_terminal_permission() {
        let p = |v: serde_json::Value| err(json!({ "name": "x", "permissions": { "aiTerminal": v }}));
        assert!(p(json!({ "agents": ["gemini"], "maxConcurrent": 1 })).contains("gemini"));
        assert!(p(json!({ "maxConcurrent": 1 })).contains("agents"));
        assert!(p(json!({ "agents": ["claude"] })).contains("maxConcurrent"));
        assert!(p(json!({ "agents": ["claude"], "maxConcurrent": 0 })).contains("maxConcurrent"));
        assert!(p(json!({ "agents": ["claude"], "maxConcurrent": 5 })).contains("maxConcurrent"));
        // 権限モード・権限スキップ・許可ツールは v1 では指定できない
        for k in ["permissionMode", "skipPermissions", "allowedTools"] {
            let e = p(json!({ "agents": ["claude"], "maxConcurrent": 1, k: true }));
            assert!(e.contains("未知のキー"), "{k}: {e}");
        }
    }

    #[test]
    fn ai_terminal_change_requires_reapproval() {
        let a = ok(json!({ "name": "x", "permissions": { "aiTerminal": { "agents": ["claude"], "maxConcurrent": 1 }}}));
        let b = ok(json!({ "name": "x", "permissions": { "aiTerminal": { "agents": ["claude"], "maxConcurrent": 2 }}}));
        assert!(requested_changed(&a.permissions, &b.permissions));
    }

    #[test]
    fn rejects_unknown_keys_inside_mcp_and_memory() {
        assert!(err(json!({ "name": "x", "permissions": { "mcp": { "tools": ["notify_worktree"], "all": true }}})).contains("未知のキー"));
        assert!(err(json!({ "name": "x", "permissions": { "memory": { "maxBytes": 10, "extra": 1 }}})).contains("未知のキー"));
    }

    #[test]
    fn targets_without_tools_are_rejected() {
        let e = err(json!({ "name": "x", "permissions": { "mcp": { "targets": ["*"] }}}));
        assert!(e.contains("tools"));
    }

    #[test]
    fn empty_tools_means_no_mcp_permission() {
        let m = ok(json!({ "name": "x", "permissions": { "mcp": { "tools": [] }}}));
        assert!(m.permissions.mcp.is_none());
    }

    #[test]
    fn memory_requires_max_bytes_when_present() {
        assert!(err(json!({ "name": "x", "permissions": { "memory": {} }})).contains("maxBytes"));
        assert!(err(json!({ "name": "x", "permissions": { "memory": { "maxBytes": 0 } }})).contains("範囲"));
        let too_big = crate::ARTIFACT_MEMORY_MAX_BYTES as u64 + 1;
        assert!(err(json!({ "name": "x", "permissions": { "memory": { "maxBytes": too_big } }})).contains("範囲"));
        let m = ok(json!({ "name": "x", "permissions": { "memory": { "maxBytes": 4096 } }}));
        assert_eq!(m.permissions.memory, Some(MemoryPermission { max_bytes: 4096 }));
    }

    #[test]
    fn memory_size_is_checked_on_the_serialized_result() {
        let m = json!({ "k": "あ" });
        let size = serde_json::to_string(&m).unwrap().len() as u64;
        // マルチバイトはバイト数で数える（文字数ではない）
        assert!(size > 7);
        assert_eq!(check_memory_size(&m, size), Ok(size));
        assert!(check_memory_size(&m, size - 1).unwrap_err().contains("最大サイズ"));
        // 上限を下げたあと、収まらないデータは書けず、縮めた書き込みは通る
        let big = json!({ "k": "x".repeat(100) });
        assert!(check_memory_size(&big, 50).is_err());
        assert!(check_memory_size(&json!({ "k": "x" }), 50).is_ok());
    }

    #[test]
    fn memory_is_optional() {
        let m = ok(json!({ "name": "x", "permissions": { "mcp": { "tools": ["notify_worktree"] }}}));
        assert!(m.permissions.memory.is_none());
    }

    #[test]
    fn source_is_display_only() {
        let a = ok(json!({ "name": "x", "source": { "skill": "s", "version": "1" } }));
        let b = ok(json!({ "name": "x", "source": { "skill": "s", "version": "2" } }));
        assert!(!requested_changed(&a.permissions, &b.permissions));
        assert_eq!(a.source.unwrap().version.as_deref(), Some("1"));
    }

    #[test]
    fn marker_detection() {
        assert!(has_app_marker(&json!({ "id": "a", "app": { "name": "x" } })));
        assert!(has_app_marker(&json!({ "app": 1 })), "壊れた manifest でもマーカーは立つ");
        assert!(!has_app_marker(&json!({ "id": "a" })));
        assert!(!has_app_marker(&json!({ "app": null })));
    }

    /// 同梱スキル（app-starter）の manifest は、この版のパーサで受理されること。
    /// 受理されない manifest を配ると、スキルの最初の手順で `artifact create` が失敗する
    // ─── process ───

    fn proc_manifest(cmd: serde_json::Value) -> serde_json::Value {
        json!({ "name": "x", "permissions": { "process": { "commands": { "run-workflow": cmd }}}})
    }

    #[test]
    fn parses_process_permission() {
        let m = ok(json!({ "name": "x", "permissions": { "process": { "commands": {
            "run-workflow": {
                "exec": "python",
                "args": [
                    "-X", "utf8",
                    "worktree://.claude\\skills/x/run.py",
                    { "param": "request", "type": "path", "under": "worktree://runs" },
                    { "param": "mode", "type": "token" },
                    { "param": "size", "type": "enum", "values": ["s", "l", "s"] }
                ],
                "cwd": "worktree://",
                "env": { "inherit": ["USERPROFILE", "PATH", "PATH"] },
                "stdin": "none",
                "maxConcurrent": 4,
                "timeoutSec": 3600,
                "maxOutputBytes": 1048576
            },
            "ping": { "exec": "C:\\Windows\\System32\\ping.exe" }
        }}}}));
        let p = m.permissions.process.unwrap();
        let c = &p.commands["run-workflow"];
        assert_eq!(c.exec, "python");
        assert_eq!(
            c.args,
            vec![
                ProcessArg::Fixed { value: "-X".into() },
                ProcessArg::Fixed { value: "utf8".into() },
                // worktree:// で始まる固定の引数は正規化する（区切りの統一）
                ProcessArg::Fixed { value: "worktree://.claude/skills/x/run.py".into() },
                ProcessArg::Path { param: "request".into(), under: "worktree://runs/".into() },
                ProcessArg::Token { param: "mode".into() },
                ProcessArg::Enum { param: "size".into(), values: vec!["l".into(), "s".into()] },
            ]
        );
        assert_eq!(c.cwd, "worktree://");
        assert_eq!(c.env_inherit, vec!["PATH", "USERPROFILE"]);
        assert_eq!((c.max_concurrent, c.timeout_sec, c.max_output_bytes), (4, Some(3600), 1048576));
        // 省略は既定で埋める
        let d = &p.commands["ping"];
        assert_eq!(
            (d.cwd.as_str(), d.max_concurrent, d.timeout_sec, d.max_output_bytes),
            ("worktree://", 1, None, PROCESS_OUTPUT_DEFAULT)
        );
        // 空は権限なし
        assert!(ok(json!({ "name": "x", "permissions": { "process": { "commands": {} }}})).permissions.process.is_none());
    }

    #[test]
    fn rejects_bad_process_permission() {
        let e = |cmd: serde_json::Value| err(proc_manifest(cmd));
        // .bat / .cmd は承認前に弾く（大文字・末尾のドット・パスの途中も）
        for exec in ["run.bat", "tool.CMD", "worktree://scripts/x.bat", "C:\\x\\y.cmd", "run.bat.", "run.cmd ."] {
            assert!(e(json!({ "exec": exec })).contains(".bat / .cmd"), "{}", exec);
        }
        // exec の形
        assert!(e(json!({})).contains("exec"));
        assert!(e(json!({ "exec": "" })).contains("空"));
        assert!(e(json!({ "exec": "./run.exe" })).contains("絶対パス"));
        assert!(e(json!({ "exec": "bin\\run.exe" })).contains("絶対パス"));
        assert!(e(json!({ "exec": "-rf" })).contains("絶対パス"));
        assert!(e(json!({ "exec": "\\\\server\\share\\run.exe" })).contains("UNC"));
        assert!(e(json!({ "exec": "worktree://" })).contains("ファイル"));
        assert!(e(json!({ "exec": "worktree://../x.exe" })).contains("不正"));
        // 未知のキー・pin・stdin
        assert!(e(json!({ "exec": "python", "shell": true })).contains("未知のキー"));
        assert!(e(json!({ "exec": "python", "pin": "sha256" })).contains("pin"));
        assert!(e(json!({ "exec": "python", "stdin": "inherit" })).contains("stdin"));
        // 引数
        assert!(e(json!({ "exec": "python", "args": [1] })).contains("文字列"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a" }] })).contains("path / token / enum"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a b", "type": "token" }] })).contains("param"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a", "type": "path" }] })).contains("under"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a", "type": "path", "under": "/etc/" }] })).contains("worktree://"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a", "type": "path", "under": "worktree://a/../b" }] })).contains("不正"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a", "type": "token", "under": "worktree://a/" }] })).contains("under"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a", "type": "enum", "values": [] }] })).contains("1〜"));
        assert!(e(json!({ "exec": "python", "args": [{ "param": "a", "type": "enum", "values": ["ok", "-x"] }] })).contains("'-'"));
        assert!(e(json!({ "exec": "python", "args": [
            { "param": "a", "type": "token" }, { "param": "a", "type": "token" }
        ] })).contains("重複"));
        assert!(e(json!({ "exec": "python", "args": ["a\nb"] })).contains("改行"));
        // cwd は worktree:// / artifact:// だけ
        assert!(e(json!({ "exec": "python", "cwd": "C:\\work" })).contains("cwd"));
        assert!(e(json!({ "exec": "python", "cwd": "worktree://a/../.." })).contains("不正"));
        // env: 値は書けない。oretachi の接続情報を含む接頭辞は拒否
        assert!(e(json!({ "exec": "python", "env": { "set": { "A": "1" } } })).contains("未知のキー"));
        assert!(e(json!({ "exec": "python", "env": { "inherit": ["ORETACHI_MCP_PORT"] } })).contains("指定できません"));
        assert!(e(json!({ "exec": "python", "env": { "inherit": ["mcp_token"] } })).contains("指定できません"));
        assert!(e(json!({ "exec": "python", "env": { "inherit": ["Claude_Code_Entrypoint"] } })).contains("指定できません"));
        assert!(e(json!({ "exec": "python", "env": { "inherit": ["A=B"] } })).contains("inherit"));
        // 上限の範囲
        assert!(e(json!({ "exec": "python", "maxConcurrent": 0 })).contains("maxConcurrent"));
        assert!(e(json!({ "exec": "python", "maxConcurrent": 9 })).contains("maxConcurrent"));
        assert!(e(json!({ "exec": "python", "timeoutSec": 0 })).contains("timeoutSec"));
        assert!(e(json!({ "exec": "python", "timeoutSec": 604801 })).contains("timeoutSec"));
        assert!(e(json!({ "exec": "python", "maxOutputBytes": 10 })).contains("maxOutputBytes"));
        // コマンド名・形
        let bad_name = err(json!({ "name": "x", "permissions": { "process": { "commands": { "a b": { "exec": "python" } }}}}));
        assert!(bad_name.contains("コマンド名"));
        assert!(err(json!({ "name": "x", "permissions": { "process": { "shell": true }}})).contains("未知のキー"));
    }

    #[test]
    fn process_cannot_run_what_the_app_itself_can_write() {
        let with = |fs: serde_json::Value, cmd: serde_json::Value| {
            json!({ "name": "x", "permissions": { "fs": fs, "process": { "commands": { "c": cmd }}}})
        };
        let script = json!({
            "exec": "python",
            "args": ["worktree://scripts/run.py", { "param": "request", "type": "path", "under": "worktree://runs/" }]
        });
        // runs/ だけ書ける: スクリプトにも cwd にも届かない
        assert!(parse_manifest(&with(json!({ "write": ["worktree://runs/"] }), script.clone())).is_ok());
        // スクリプトのあるディレクトリ・その親・ルートに書ける
        for scope in ["worktree://scripts/", "worktree://", "worktree://scripts/run.py"] {
            let e = parse_manifest(&with(json!({ "write": [scope] }), script.clone())).unwrap_err();
            assert!(e.contains("書き込める場所"), "{}: {}", scope, e);
        }
        // `..` で書き込みスコープの検査を外す形は、manifest の時点で拒否する（cwd は scripts/ の外の work/）
        let dotdot = |arg: &str| json!({ "exec": "python", "args": [arg], "cwd": "worktree://work/" });
        for arg in ["../scripts/run.py", "scripts/../scripts/run.py", "..\\scripts\\run.py", ".."] {
            let e = parse_manifest(&with(json!({ "write": ["worktree://scripts/"] }), dotdot(arg))).unwrap_err();
            assert!(e.contains("'..'"), "{}: {}", arg, e);
        }
        // 絶対パスの exec も `..` を含められない
        for exec in ["C:\\repo\\work\\..\\scripts\\x.exe", "/repo/work/../scripts/x"] {
            let e = parse_manifest(&with(json!({ "write": ["worktree://scripts/"] }), json!({ "exec": exec }))).unwrap_err();
            assert!(e.contains("'..'"), "{}: {}", exec, e);
        }
        // Windows のドライブ相対パスは、cwd 基準の相対パスとして照合できないので拒否
        let e = parse_manifest(&with(json!({ "write": ["worktree://scripts/"] }), dotdot("C:scripts\\run.py"))).unwrap_err();
        assert!(e.contains("ドライブ相対"), "{}", e);
        // 普通の文字列は拒否しない（`..` を成分としてでなく含む・`:` や `*` を含む・オプション）
        for arg in ["--range=a..b", "key:value", "*.py", "-X", "utf8", "1.5"] {
            assert!(
                parse_manifest(&with(json!({ "write": ["worktree://runs/"] }), dotdot(arg))).is_ok(),
                "{} は通る",
                arg
            );
        }
        // cwd 基準の相対パスの固定引数（`python scripts/run.py`、`python -m pkg`）も、書ける場所なら拒否。
        // `-X utf8` の `utf8` のようなオプションの値は、覆われない限り誤検知しない
        let rel = json!({ "exec": "python", "args": ["-X", "utf8", "scripts\\run.py"] });
        assert!(parse_manifest(&with(json!({ "write": ["worktree://runs/"] }), rel.clone())).is_ok());
        let e = parse_manifest(&with(json!({ "write": ["worktree://scripts/"] }), rel)).unwrap_err();
        assert!(e.contains("相対パス"), "{}", e);
        let module = json!({ "exec": "python", "args": ["-m", "pkg"] });
        assert!(parse_manifest(&with(json!({ "write": ["worktree://pkg/"] }), module)).is_err());
        // db の書き込みスコープも同じ
        let db = json!({ "name": "x", "permissions": { "db": { "write": ["worktree://scripts/"] },
            "process": { "commands": { "c": script.clone() }}}});
        assert!(parse_manifest(&db).unwrap_err().contains("db.write"));
        // 読み取りだけなら問題ない
        assert!(parse_manifest(&with(json!({ "read": ["worktree://"] }), script.clone())).is_ok());
        // artifact:// も同じ
        let art = json!({ "exec": "artifact://bin/tool.exe" });
        assert!(parse_manifest(&with(json!({ "write": ["artifact://bin/"] }), art)).is_err());
        // cwd が書ける場所にあるものも拒否
        let cwd = json!({ "exec": "python", "cwd": "worktree://runs/" });
        assert!(parse_manifest(&with(json!({ "write": ["worktree://runs/"] }), cwd)).is_err());
        // 先頭の引数が path パラメータで、その under に書ける形（書いたファイルをそのまま実行できる）
        let first = json!({ "exec": "python", "args": [{ "param": "script", "type": "path", "under": "worktree://runs/" }] });
        assert!(parse_manifest(&with(json!({ "write": ["worktree://runs/"] }), first)).is_err());
    }

    #[test]
    fn process_change_requires_reapproval() {
        let base = ok(proc_manifest(json!({ "exec": "python", "args": ["a.py"] })));
        let same_renamed = ok(json!({ "name": "renamed", "permissions": { "process": { "commands": {
            "run-workflow": { "exec": "python", "args": ["a.py"] }
        }}}}));
        assert!(!requested_changed(&base.permissions, &same_renamed.permissions));
        for changed in [
            json!({ "exec": "python3", "args": ["a.py"] }),
            json!({ "exec": "python", "args": ["b.py"] }),
            json!({ "exec": "python", "args": ["a.py"], "cwd": "worktree://x/" }),
            json!({ "exec": "python", "args": ["a.py"], "env": { "inherit": ["PATH"] } }),
            json!({ "exec": "python", "args": ["a.py"], "maxConcurrent": 2 }),
            json!({ "exec": "python", "args": ["a.py", { "param": "t", "type": "token" }] }),
        ] {
            let m = ok(proc_manifest(changed.clone()));
            assert!(requested_changed(&base.permissions, &m.permissions), "{}", changed);
            assert!(requested_changed(&m.permissions, &base.permissions), "狭まる変更でも出す: {}", changed);
        }
    }

    #[test]
    fn legacy_grant_without_process_deserializes_as_no_process() {
        let raw = r#"{"grantedAt":1,"requested":{"memory":{"maxBytes":10}},"resolved":{"memory":{"maxBytes":10}}}"#;
        let g: AppGrant = serde_json::from_str(raw).unwrap();
        assert!(g.resolved.process.is_none() && g.requested.process.is_none());
    }

    #[test]
    fn process_grant_roundtrips_through_json() {
        let m = ok(proc_manifest(json!({
            "exec": "python",
            "args": [{ "param": "request", "type": "path", "under": "worktree://runs/" },
                     { "param": "size", "type": "enum", "values": ["s", "l"] }]
        })));
        let requested = m.permissions;
        let resolved = ResolvedPermissions {
            process: Some(ResolvedProcess {
                commands: requested
                    .process
                    .as_ref()
                    .unwrap()
                    .commands
                    .iter()
                    .map(|(k, c)| {
                        (
                            k.clone(),
                            ResolvedProcessCommand {
                                exec: "C:\\Python\\python.exe".into(),
                                args: vec![
                                    ResolvedProcessArg::Fixed { value: "-X".into() },
                                    ResolvedProcessArg::Token { param: "mode".into() },
                                ],
                                cwd: "C:\\wt".into(),
                                env_inherit: c.env_inherit.clone(),
                                max_concurrent: c.max_concurrent,
                                timeout_sec: c.timeout_sec,
                                max_output_bytes: c.max_output_bytes,
                            },
                        )
                    })
                    .collect(),
            }),
            ..Default::default()
        };
        let g = AppGrant { granted_at: 1, requested, resolved };
        let back: AppGrant = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn bundled_starter_manifest_is_valid() {
        let raw = include_str!("../skills/app-starter/app.json");
        let value: serde_json::Value = serde_json::from_str(raw).expect("app.json は JSON");
        let m = parse_manifest(&value).expect("同梱の manifest は有効");
        assert_eq!(m.name, "Notification Viewer");
        let mcp = m.permissions.mcp.expect("mcp 権限を宣言している");
        assert_eq!(mcp.tools, vec!["notify_worktree", "oretachi_list_worktree_notifications"]);
        assert!(mcp.targets.is_empty(), "雛形は自ワークツリーだけ");
    }

    #[test]
    fn raw_marker_detection_fails_closed() {
        // 通常の React アーティファクト。コード中に "app" という文字列があっても誤検知しない
        let normal = r#"{"id":"a","type":"application/vnd.ant.react","content":"const k = \"app\";","created_at":1}"#;
        assert!(!raw_has_app_marker(normal));
        assert!(!raw_has_app_marker(r#"{"id":"a","content":"c"}"#));
        assert!(raw_has_app_marker(r#"{"id":"a","app":{"name":"x"}}"#));
        assert!(!raw_has_app_marker(r#"{"id":"a","app":null}"#));
        // 壊れた JSON でも、"app" を含むなら塞ぐ側へ倒す
        assert!(raw_has_app_marker(r#"{"id":"a","app":{"name":"x""#));
        assert!(!raw_has_app_marker("{ broken"));
    }

    #[test]
    fn fs_scopes_are_normalized_sorted_and_deduplicated() {
        let m = ok(json!({ "name": "x", "permissions": { "fs": {
            "read": ["worktree://src\\sub", "artifact://", " worktree://src/sub/ "],
            "write": ["worktree://out/"]
        }}}));
        let fs = m.permissions.fs.unwrap();
        assert_eq!(fs.read, vec!["artifact://", "worktree://src/sub/"]);
        assert_eq!(fs.write, vec!["worktree://out/"]);
        assert!(!fs.allow_sensitive_paths);
    }

    #[test]
    fn fs_rejects_raw_paths_escapes_and_unknown_keys() {
        for bad in ["C:\\x", "/etc", "src/", "worktree://../x", "worktree://a/../b", "worktree://a:b", "http://x"] {
            let e = err(json!({ "name": "x", "permissions": { "fs": { "read": [bad] }}}));
            assert!(e.contains("fs") || e.contains(bad), "{bad}: {e}");
        }
        assert!(err(json!({ "name": "x", "permissions": { "fs": { "read": ["artifact://"], "all": true }}})).contains("未知のキー"));
        assert!(err(json!({ "name": "x", "permissions": { "fs": { "read": "artifact://" }}})).contains("配列"));
        assert!(err(json!({ "name": "x", "permissions": { "fs": { "read": ["artifact://"], "allowSensitivePaths": "yes" }}})).contains("真偽値"));
        let many: Vec<String> = (0..21).map(|i| format!("artifact://d{}/", i)).collect();
        assert!(err(json!({ "name": "x", "permissions": { "fs": { "read": many }}})).contains("件まで"));
    }

    #[test]
    fn fs_write_to_sensitive_paths_needs_allow_sensitive_paths() {
        for p in ["worktree://.git/", "worktree://a/.github/workflows/", "worktree://.claude/", "worktree://.vscode/"] {
            let e = err(json!({ "name": "x", "permissions": { "fs": { "write": [p] }}}));
            assert!(e.contains("allowSensitivePaths"), "{p}: {e}");
            // 読むだけなら要らない
            ok(json!({ "name": "x", "permissions": { "fs": { "read": [p] }}}));
            // 解除を申請すれば受理される（ダイアログで最も強い警告になる）
            let m = ok(json!({ "name": "x", "permissions": { "fs": { "write": [p], "allowSensitivePaths": true }}}));
            assert!(m.permissions.fs.unwrap().allow_sensitive_paths);
        }
        // ルート全体への write は申請できる（拒否パスは呼び出し時に実パスで弾く）
        ok(json!({ "name": "x", "permissions": { "fs": { "write": ["worktree://"] }}}));
    }

    #[test]
    fn fs_empty_means_no_permission_and_flag_alone_is_invalid() {
        let m = ok(json!({ "name": "x", "permissions": { "fs": { "read": [], "write": [] }}}));
        assert!(m.permissions.fs.is_none());
        assert!(err(json!({ "name": "x", "permissions": { "fs": { "allowSensitivePaths": true }}})).contains("read / write"));
    }

    #[test]
    fn fs_change_requires_reapproval_but_order_does_not() {
        let a = ok(json!({ "name": "x", "permissions": { "fs": { "read": ["worktree://a/", "worktree://b/"] }}}));
        let b = ok(json!({ "name": "x", "permissions": { "fs": { "read": ["worktree://b/", "worktree://a/"] }}}));
        assert!(!requested_changed(&a.permissions, &b.permissions));
        let narrower = ok(json!({ "name": "x", "permissions": { "fs": { "read": ["worktree://a/"] }}}));
        assert!(requested_changed(&a.permissions, &narrower.permissions));
        let flagged = ok(json!({ "name": "x", "permissions": { "fs": { "write": ["worktree://.git/"], "allowSensitivePaths": true }}}));
        let flagged_again = ok(json!({ "name": "x", "permissions": { "fs": { "write": ["worktree://.git/"], "allowSensitivePaths": true }}}));
        assert!(!requested_changed(&flagged.permissions, &flagged_again.permissions));
        let read_only = ok(json!({ "name": "x", "permissions": { "fs": { "read": ["worktree://.git/"] }}}));
        assert!(requested_changed(&flagged.permissions, &read_only.permissions));
    }

    #[test]
    fn legacy_grant_without_fs_deserializes_as_no_fs() {
        // #370 / #371 で保存された grant（fs キー無し）は、FS 権限なしとして読める
        let raw = r#"{"grantedAt":1,"requested":{"memory":{"maxBytes":10}},"resolved":{"memory":{"maxBytes":10}}}"#;
        let g: AppGrant = serde_json::from_str(raw).unwrap();
        assert!(g.resolved.fs.is_none() && g.requested.fs.is_none());
    }

    #[test]
    fn legacy_grant_without_db_deserializes_as_no_db() {
        // #373 までに保存された grant（db キー無し）は、DB 権限なしとして読める
        let raw = r#"{"grantedAt":1,"requested":{"fs":{"read":[],"write":["artifact://"]}},"resolved":{"memory":{"maxBytes":10}}}"#;
        let g: AppGrant = serde_json::from_str(raw).unwrap();
        assert!(g.resolved.db.is_none() && g.requested.db.is_none());
        assert!(g.requested.fs.is_some());
    }

    #[test]
    fn db_scopes_are_normalized_like_fs_and_kept_separate_from_fs() {
        let m = ok(json!({ "name": "x", "permissions": {
            "db": { "read": ["worktree://ro\\"], "write": ["artifact://", "artifact://"] }
        }}));
        let db = m.permissions.db.unwrap();
        assert_eq!(db.read, vec!["worktree://ro/"]);
        assert_eq!(db.write, vec!["artifact://"]);
        assert!(!db.allow_sensitive_paths);
        // fs の申請とは独立（db を申請しても fs の権限にはならない）
        assert!(m.permissions.fs.is_none());
    }

    #[test]
    fn db_rejects_raw_paths_unknown_keys_and_lone_flag() {
        for bad in [
            json!({ "read": ["/etc/x.db"] }),
            json!({ "write": ["C:/x.db"] }),
            json!({ "write": ["worktree://../x"] }),
            json!({ "write": ["artifact://"], "mode": "rw" }),
            json!({ "allowSensitivePaths": true }),
        ] {
            // パスの不正はパス側のメッセージ、キーの不正は app.permissions.db 付きのメッセージで返る
            let e = err(json!({ "name": "x", "permissions": { "db": bad.clone() }}));
            assert!(!e.is_empty(), "{bad}");
        }
        let e = err(json!({ "name": "x", "permissions": { "db": { "mode": "rw" } }}));
        assert!(e.contains("app.permissions.db"), "{e}");
        let e = err(json!({ "name": "x", "permissions": { "db": { "allowSensitivePaths": true } }}));
        assert!(e.contains("app.permissions.db"), "{e}");
        // 空は権限なし
        assert!(ok(json!({ "name": "x", "permissions": { "db": {} }})).permissions.db.is_none());
    }

    #[test]
    fn db_sensitive_paths_are_refused_even_for_read() {
        // fs は read なら既定拒否パスでも申請できるが、db は read でも不可（副産物が同じ場所に出る）
        assert!(parse_manifest(&json!({ "name": "x", "permissions": { "fs": { "read": ["worktree://.git/"] } }})).is_ok());
        for list in ["read", "write"] {
            let e = err(json!({ "name": "x", "permissions": { "db": { list: ["worktree://.git/"] } }}));
            assert!(e.contains("allowSensitivePaths") && e.contains(&format!("db.{}", list)), "{list}: {e}");
        }
        let m = ok(json!({ "name": "x", "permissions": {
            "db": { "read": ["worktree://.git/"], "allowSensitivePaths": true }
        }}));
        assert!(m.permissions.db.unwrap().allow_sensitive_paths);
    }

    #[test]
    fn db_change_requires_reapproval() {
        let a = ok(json!({ "name": "x", "permissions": { "db": { "write": ["artifact://"] } }}));
        let b = ok(json!({ "name": "x", "permissions": { "db": { "write": ["artifact://data/"] } }}));
        let none = ok(json!({ "name": "x" }));
        assert!(requested_changed(&a.permissions, &b.permissions));
        assert!(requested_changed(&a.permissions, &none.permissions));
        assert!(!requested_changed(&a.permissions, &a.permissions.clone()));
    }

    #[test]
    fn legacy_artifact_without_app_is_none() {
        let raw = r#"{"id":"a","type":"application/vnd.ant.react","title":"t","content":"c","created_at":1,"updated_at":1}"#;
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(!has_app_marker(&value));
    }

    fn resolved(tools: &[&str], targets: &[&str]) -> ResolvedPermissions {
        ResolvedPermissions {
            mcp: Some(ResolvedMcp {
                tools: tools.iter().map(|s| s.to_string()).collect(),
                targets: targets
                    .iter()
                    .map(|s| ResolvedTarget {
                        raw: s.to_string(),
                        stored: s.to_string(),
                        label: s.to_string(),
                        wildcard: is_wildcard_target(s),
                    })
                    .collect(),
            }),
            memory: None,
            ai_terminal: None,
            fs: None,
            db: None,
            network: None,
            sub_window: None,
            audio: None,
            media: None,
            scheduler: None,
            process: None,
        }
    }

    #[test]
    fn tool_gate_uses_resolved_set_only() {
        let r = resolved(&["notify_worktree"], &[]);
        assert!(authorize_app_tool(&r, "notify_worktree").is_ok());
        assert!(authorize_app_tool(&r, "oretachi_write_terminal").is_err());
        assert!(authorize_app_tool(&ResolvedPermissions::default(), "notify_worktree").is_err());
    }

    #[test]
    fn cross_gate_requires_declared_target() {
        let dest = vec!["wt-b".to_string(), "*".to_string(), "workgroup:g1".to_string(), "repo:oretachi".to_string()];
        // targets 省略 = 自ワークツリーのみ
        assert!(authorize_app_cross_target(&resolved(&["oretachi_show_worktree"], &[]), &dest).is_err());
        // ID 完全一致
        assert!(authorize_app_cross_target(&resolved(&["oretachi_show_worktree"], &["wt-b"]), &dest).is_ok());
        // 別ワークツリーの ID は当たらない
        assert!(authorize_app_cross_target(&resolved(&["oretachi_show_worktree"], &["wt-c"]), &dest).is_err());
        // ワイルドカード
        assert!(authorize_app_cross_target(&resolved(&["oretachi_show_worktree"], &["*"]), &dest).is_ok());
        assert!(authorize_app_cross_target(&resolved(&["oretachi_show_worktree"], &["workgroup:g1"]), &dest).is_ok());
        assert!(authorize_app_cross_target(&resolved(&["oretachi_show_worktree"], &["workgroup:g2"]), &dest).is_err());
        assert!(authorize_app_cross_target(&resolved(&["oretachi_show_worktree"], &["repo:oretachi"]), &dest).is_ok());
    }

    #[test]
    fn grant_roundtrips_through_json() {
        let g = AppGrant {
            granted_at: 1,
            requested: RequestedPermissions {
                mcp: Some(RequestedMcp { tools: vec!["notify_worktree".into()], targets: vec!["wt-b".into()] }),
                memory: Some(MemoryPermission { max_bytes: 10 }),
                ai_terminal: Some(AiTerminalPermission { agents: vec!["claude".into()], max_concurrent: 2 }),
                fs: None,
                db: None,
                network: None,
                sub_window: None,
                audio: None,
                media: Some(RequestedMedia { read: vec!["worktree://videos/".into()], allow: vec!["https://cdn.example.com/".into()] }),
                scheduler: None,
                process: None,
            },
            resolved: resolved(&["notify_worktree"], &["wt-b"]),
        };
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains("grantedAt") && s.contains("maxBytes") && s.contains("aiTerminal"));
        let back: AppGrant = serde_json::from_str(&s).unwrap();
        assert_eq!(back, g);
    }
}
