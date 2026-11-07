//! アプリ形式アーティファクト向けのコマンド起動 API（`process`）。
//!
//! manifest（`app.permissions.process`）に**名前つきで宣言し、承認された固定のコマンドだけ**を、
//! アプリが自分で起動して、標準出力・終了コードを受け取れる。呼び出し側が渡せるのはコマンド名と
//! パラメータの値だけで、実行ファイルや固定の引数は変えられない。
//!
//! # 関門（#369 の前提 1・2・5・7）
//!
//! - 見るのは host 側に保存した grant の [`ResolvedProcess`]（承認時に解決した絶対パス・固定引数・cwd・
//!   env・パラメータの型と `under`）だけ。アーティファクト JSON 内の manifest は見ない。
//! - 呼び出し元インスタンスは webview のウィンドウラベルから決める（[`instance_ctx`]）。
//! - **HTTP（`web_viewer.rs`）には出さない。**
//! - シェルを経由しない（argv で直接起動）。引数は「固定の文字列」か「型つきのパラメータ」だけ。
//!   パラメータの値は `-` で始められない（オプションの注入を防ぐ）。`path` は `app_fs` と同じ検査
//!   （`..`・リンクによる脱出・ルートの移動を拒否）を通す。
//! - 子の環境変数は `env.inherit` に書いた名前だけ。`ORETACHI_` / `MCP_` / `CLAUDE_` で始まるものは
//!   書かれていても渡さない（oretachi 自身の MCP のポート・トークンを漏らさない）。
//!
//! # 前提 9 との関係
//!
//! 宣言したスクリプトが `worktree://` の中にあれば、それを書き換えられる相手（AI や MCP クライアント）が
//! 承認済みの起動を通して任意のコードを実行できる。v1 はこれを**守らず、承認ダイアログで明示して
//! ユーザーの判断に委ねる**（案 C。書ける相手は元々端末で任意のコマンドを実行できる）。ただし
//! **アプリ自身**が同じ manifest の書き込み権限でスクリプトを書き換えられる組み合わせは、
//! `app_manifest::check_process_against_write_scopes`（と承認時の [`check_resolved_against_write_scopes`]）で
//! 拒否する。ハッシュ固定（案 A）は、`ResolvedProcessCommand` にフィールドを足して後から入れられる。
//!
//! # ライフサイクル
//!
//! - **リロードでは殺さない。** 数時間かかる処理があるので、新しい iframe が `list` / `read` で拾い直す。
//! - **インスタンスの停止・取り消し・oretachi の終了では殺す**（Windows は子ごとの Job Object、
//!   Unix はプロセスグループ）。
//! - 終了は**直下の子の `wait()` が返った時点**。出力を継承した孫がパイプを握り続けても「実行中」が
//!   続かないよう、子の終了後はツリーごと止めてからリーダーを短いタイムアウトで吸い切る。
//!
//! # 出力
//!
//! 行単位（`\n` と `\r` の両方で区切る。進捗表示が `\r` だけで更新するため）。1 プロセスあたり
//! リングバッファ（1 MiB）に残し、`read` で `seq` 以降を取り戻せる。アプリへの push
//! （`app-process-event`）は 50〜100 ms ごとにまとめる（行ごとの emit は webview の負荷になる）。

use crate::app_fs::Access;
use crate::app_manifest::{
    is_forbidden_env_name, is_forbidden_exec_ext, ProcessArg, ProcessPermission, RequestedFs, ResolvedFs,
    ResolvedProcess, ResolvedProcessArg, ResolvedProcessCommand,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncRead, AsyncReadExt};

/// アプリへ push するイベント名。ペイロードは `{id, kind: "output", lines}` / `{id, kind: "exit", …}`。
pub const EVENT_NAME: &str = "app-process-event";

/// 同時に動かせる数（インスタンスごと・全体）。コマンドごとの上限は manifest の `maxConcurrent`。
pub const MAX_PER_INSTANCE: usize = 8;
pub const MAX_TOTAL: usize = 16;
/// 終了済みのエントリをインスタンスごとに残す数（`list` / `read` 用）。
const KEEP_FINISHED: usize = 16;
/// プロセスごとの出力のリングバッファ（テキストのバイト数）。
pub const RING_BYTES: usize = 1024 * 1024;
/// 1 行の最大バイト数。超えたら分割する。
const LINE_MAX: usize = 16 * 1024;
/// push をまとめる間隔と、1 回に送る最大行数。
const FLUSH_INTERVAL: Duration = Duration::from_millis(75);
const FLUSH_MAX_LINES: usize = 256;
/// 子の終了後にリーダー・ポンプを待つ上限。
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// パラメータの値の最大文字数。
const MAX_PARAM_CHARS: usize = 1024;

// ─── 承認時の解決（純粋関数。`AppHandle` を取らない） ─────────────────

/// `\\?\` / `\\?\UNC\` を除いた形（子へ渡す・ダイアログへ出す表記）。
pub fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        p.to_string()
    }
}

fn is_path_uri(s: &str) -> bool {
    s.starts_with("worktree://") || s.starts_with("artifact://")
}

/// `worktree://…` / `artifact://…` を、承認時の絶対パスへ解決する（`app_fs::resolve_requested_fs` と同じ
/// 検査。リンクでルートの外へ出るものは拒否）。存在は問わない。
fn uri_to_abs(uri: &str, worktree_path: Option<&Path>, artifact_dir: &Path) -> Result<String, String> {
    let fs = crate::app_fs::resolve_requested_fs(
        &RequestedFs { read: vec![uri.to_string()], write: Vec::new(), allow_sensitive_paths: false },
        worktree_path,
        artifact_dir,
    )?;
    let scope = fs.scopes.first().ok_or_else(|| format!("'{}' を解決できません", uri))?;
    Ok(strip_verbatim(&scope.path))
}

fn is_nonempty_file(p: &Path) -> bool {
    // リンクを辿る。失敗するもの（WindowsApps の AppExecLink が返す os error 1920 など）と、
    // 0 バイトのもの（ストアの実行エイリアスのスタブ）は実行ファイルとして扱わない
    std::fs::metadata(p).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// 裸のコマンド名を、PATH の**絶対パスのディレクトリだけ**から探す（相対・空のエントリは無視する。
/// カレントディレクトリ由来の差し替えを避ける）。Windows は `.exe` / `.com` だけを試す。
pub fn find_in_path(name: &str, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let path_env = path_env?;
    let candidates: Vec<String> = if cfg!(windows) {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".com") {
            vec![name.to_string()]
        } else {
            vec![format!("{}.exe", name), format!("{}.com", name)]
        }
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(path_env) {
        if !dir.is_absolute() {
            continue;
        }
        for c in &candidates {
            let p = dir.join(c);
            if is_nonempty_file(&p) {
                return Some(p);
            }
        }
    }
    None
}

fn resolve_exec(
    exec: &str,
    worktree_path: Option<&Path>,
    artifact_dir: &Path,
    path_env: Option<&OsStr>,
) -> Result<String, String> {
    let abs = if is_path_uri(exec) {
        uri_to_abs(exec, worktree_path, artifact_dir)?
    } else if Path::new(exec).is_absolute() || exec.starts_with('/') {
        strip_verbatim(exec)
    } else {
        find_in_path(exec, path_env)
            .map(|p| strip_verbatim(&p.to_string_lossy()))
            .ok_or_else(|| format!("コマンド '{}' が PATH から見つかりません", exec))?
    };
    if is_forbidden_exec_ext(&abs) {
        return Err(format!("'{}' は .bat / .cmd です。exec にできません", abs));
    }
    if cfg!(windows) {
        let lower = abs.to_ascii_lowercase();
        if !(lower.ends_with(".exe") || lower.ends_with(".com")) {
            return Err(format!("Windows では exec に .exe / .com だけを指定できます: {}", abs));
        }
    }
    if !is_nonempty_file(Path::new(&abs)) {
        return Err(format!("実行ファイル '{}' が見つかりません", abs));
    }
    Ok(abs)
}

/// manifest の `process` を承認時の形へ解決する。ダイアログに出した集合をそのまま grant へ保存する。
pub fn resolve_requested_process(
    requested: &ProcessPermission,
    worktree_path: Option<&Path>,
    artifact_dir: &Path,
    path_env: Option<&OsStr>,
) -> Result<ResolvedProcess, String> {
    let mut commands = std::collections::BTreeMap::new();
    for (name, cmd) in &requested.commands {
        let ctx = |e: String| format!("process のコマンド '{}': {}", name, e);
        let exec = resolve_exec(&cmd.exec, worktree_path, artifact_dir, path_env).map_err(ctx)?;
        let mut args = Vec::with_capacity(cmd.args.len());
        for a in &cmd.args {
            args.push(match a {
                ProcessArg::Fixed { value } => ResolvedProcessArg::Fixed {
                    value: if is_path_uri(value) {
                        uri_to_abs(value, worktree_path, artifact_dir).map_err(ctx)?
                    } else {
                        value.clone()
                    },
                },
                ProcessArg::Path { param, under } => ResolvedProcessArg::Path {
                    param: param.clone(),
                    under: under.clone(),
                    fs: crate::app_fs::resolve_requested_fs(
                        &RequestedFs { read: vec![under.clone()], write: Vec::new(), allow_sensitive_paths: false },
                        worktree_path,
                        artifact_dir,
                    )
                    .map_err(ctx)?,
                },
                ProcessArg::Token { param } => ResolvedProcessArg::Token { param: param.clone() },
                ProcessArg::Enum { param, values } => {
                    ResolvedProcessArg::Enum { param: param.clone(), values: values.clone() }
                }
            });
        }
        let cwd = uri_to_abs(&cmd.cwd, worktree_path, artifact_dir).map_err(ctx)?;
        if !Path::new(&cwd).is_dir() {
            return Err(ctx(format!("cwd '{}' がディレクトリではありません", cwd)));
        }
        commands.insert(
            name.clone(),
            ResolvedProcessCommand {
                exec,
                args,
                cwd,
                env_inherit: cmd.env_inherit.clone(),
                max_concurrent: cmd.max_concurrent,
                timeout_sec: cmd.timeout_sec,
                max_output_bytes: cmd.max_output_bytes,
            },
        );
    }
    Ok(ResolvedProcess { commands })
}

fn norm_path(p: &str) -> PathBuf {
    let s = strip_verbatim(p);
    if cfg!(windows) {
        PathBuf::from(s.to_lowercase().replace('/', "\\"))
    } else {
        PathBuf::from(s)
    }
}

/// 承認時（canonicalize 済みのパス）にも、書き込みスコープが exec・固定引数・cwd を覆わないことを確かめる。
/// manifest の字句検査（`app_manifest::check_process_against_write_scopes`）を、リンクによる迂回に対して
/// 補う。`fs` / `db` は解決済みのスコープ。
pub fn check_resolved_against_write_scopes(
    process: &ResolvedProcess,
    fs: Option<&ResolvedFs>,
    db: Option<&ResolvedFs>,
) -> Result<(), String> {
    let writes: Vec<(&str, PathBuf)> = fs
        .into_iter()
        .map(|f| ("fs", f))
        .chain(db.into_iter().map(|d| ("db", d)))
        .flat_map(|(k, f)| f.scopes.iter().filter(|s| s.write).map(move |s| (k, norm_path(&s.path))))
        .collect();
    if writes.is_empty() {
        return Ok(());
    }
    for (name, cmd) in &process.commands {
        let mut targets: Vec<(&str, &str)> = vec![("exec", cmd.exec.as_str()), ("cwd", cmd.cwd.as_str())];
        // 実パスの形も照合する（リンクを辿ると書き込みスコープの中を指すもの）。実パスにできない
        // （まだ無い・文字として不正）ものは、字句の形だけで照合する
        let real = |p: &Path| -> Option<String> {
            crate::app_fs::canonicalize_allow_missing_tail(p).ok().map(|c| c.to_string_lossy().into_owned())
        };
        let mut owned: Vec<String> = Vec::new();
        owned.extend(real(Path::new(&cmd.exec)));
        owned.extend(real(Path::new(&cmd.cwd)));
        // 固定の引数は、絶対パスならそのまま、そうでなければ cwd 基準の相対パスとして照合する
        // （`python scripts/run.py` の `scripts/run.py`。オプション（`-` 始まり）は対象外）。
        // `..`・cwd 内の junction / symlink は実パス化して畳む（manifest は `..` とドライブ相対を拒否済み）
        for a in &cmd.args {
            if let ResolvedProcessArg::Fixed { value } = a {
                // `--名前=値` は右辺をパスとして見る。それ以外の `-` 始まり（オプション）は対象外
                let value: &str = if value.starts_with('-') {
                    match value.split_once('=') {
                        Some((_, rhs)) if !rhs.is_empty() => rhs,
                        _ => continue,
                    }
                } else {
                    value
                };
                let joined = Path::new(&cmd.cwd).join(value);
                owned.extend(real(&joined));
                owned.push(joined.to_string_lossy().into_owned());
            }
        }
        for o in &owned {
            targets.push(("exec / cwd / 固定の引数（実パス・結合後）", o.as_str()));
        }
        if let Some(ResolvedProcessArg::Path { fs: pfs, .. }) = cmd.args.first() {
            for s in &pfs.scopes {
                targets.push(("先頭の path パラメータの under", s.path.as_str()));
            }
        }
        for (kind, t) in targets {
            let tp = norm_path(t);
            if let Some((perm, scope)) = writes.iter().find(|(_, s)| tp.starts_with(s)) {
                return Err(format!(
                    "process のコマンド '{}' の {}（{}）は、{}.write の '{}' に書き込める場所にあります（リンクの先を含む）。アプリ自身が書き換えて実行できてしまうため承認できません",
                    name,
                    kind,
                    strip_verbatim(t),
                    perm,
                    scope.display()
                ));
            }
        }
    }
    Ok(())
}

// ─── 呼び出し時の引数・環境の組み立て ────────────────────────────────

fn token_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn declared_param(a: &ResolvedProcessArg) -> Option<&str> {
    match a {
        ResolvedProcessArg::Fixed { .. } => None,
        ResolvedProcessArg::Path { param, .. }
        | ResolvedProcessArg::Token { param }
        | ResolvedProcessArg::Enum { param, .. } => Some(param),
    }
}

/// 宣言どおりの argv を作る。宣言外のパラメータ・欠落・型に合わない値・`-` で始まる値・`under` の外の
/// パス・`..`・リンクによる脱出は `Err`。固定の引数は変えられない。
pub fn build_argv(cmd: &ResolvedProcessCommand, params: &serde_json::Value) -> Result<Vec<OsString>, String> {
    let empty = serde_json::Map::new();
    let obj = match params {
        serde_json::Value::Null => &empty,
        serde_json::Value::Object(o) => o,
        _ => return Err("params はオブジェクトで渡してください".to_string()),
    };
    let declared: HashSet<&str> = cmd.args.iter().filter_map(declared_param).collect();
    if let Some(k) = obj.keys().find(|k| !declared.contains(k.as_str())) {
        // キー名は呼び出し側が決める任意の文字列。長さと制御文字を抑えてから出す
        let shown: String = k.chars().filter(|c| !c.is_control()).take(64).collect();
        return Err(format!("宣言されていないパラメータ '{}' です", shown));
    }
    let mut argv = Vec::with_capacity(cmd.args.len());
    for a in &cmd.args {
        let Some(name) = declared_param(a) else {
            if let ResolvedProcessArg::Fixed { value } = a {
                argv.push(OsString::from(value));
            }
            continue;
        };
        let v = obj
            .get(name)
            .ok_or_else(|| format!("パラメータ '{}' がありません", name))?
            .as_str()
            .ok_or_else(|| format!("パラメータ '{}' は文字列で渡してください", name))?;
        if v.is_empty() || v.chars().count() > MAX_PARAM_CHARS || v.contains('\0') {
            return Err(format!("パラメータ '{}' が空・長すぎる・NUL を含んでいます", name));
        }
        // どの型でも `-` で始まる値は渡せない（オプションとして解釈されるのを防ぐ）
        if v.starts_with('-') {
            return Err(format!("パラメータ '{}' は '-' で始まる値にできません", name));
        }
        match a {
            ResolvedProcessArg::Path { fs, under, .. } => {
                // 値（パス）はエラー文に出さない。この文はデバッグログにも残り、起動画面に見えるうえ、
                // 本文を AI の経路に載せたくない入力（プロンプトなど）を指すパスもありうるため。
                // `app_fs` の詳細なエラー文は値を含むので、種類だけ伝える
                let p = crate::app_fs::resolve_scoped_path(fs, v, Access::Read).map_err(|_| {
                    format!(
                        "パラメータ '{}' のパスが不正か、承認された範囲（{}）の外です（worktree:// / artifact:// の形で、.. ・リンクによる脱出・範囲外・ルートの移動は拒否されます）",
                        name, under
                    )
                })?;
                argv.push(OsString::from(strip_verbatim(&p.to_string_lossy())));
            }
            ResolvedProcessArg::Token { .. } => {
                if !token_ok(v) {
                    return Err(format!("パラメータ '{}' は [A-Za-z0-9_-]{{1,64}} の文字列にしてください", name));
                }
                argv.push(OsString::from(v));
            }
            ResolvedProcessArg::Enum { values, .. } => {
                if !values.iter().any(|x| x == v) {
                    return Err(format!("パラメータ '{}' の値は宣言された候補のどれかにしてください", name));
                }
                argv.push(OsString::from(v));
            }
            ResolvedProcessArg::Fixed { .. } => unreachable!("固定の引数はパラメータではない"),
        }
    }
    Ok(argv)
}

/// 子へ渡す環境。`inherit` の名前だけを親から写す（禁止の接頭辞は書かれていても除く）。
/// Windows は `SystemRoot` を常に渡す（無いと Winsock・CryptoAPI を使う多くのツールが起動できない）。
pub fn build_env(
    inherit: &[String],
    parent: impl Iterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let same = |a: &str, b: &str| if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b };
    let mut out: Vec<(OsString, OsString)> = Vec::new();
    for (k, v) in parent {
        let Some(ks) = k.to_str() else { continue };
        if is_forbidden_env_name(ks) {
            continue;
        }
        let wanted = inherit.iter().any(|n| same(n, ks)) || (cfg!(windows) && ks.eq_ignore_ascii_case("SystemRoot"));
        if wanted && !out.iter().any(|(ek, _)| ek.to_str().is_some_and(|e| same(e, ks))) {
            out.push((k, v));
        }
    }
    out
}

// ─── 出力の行分割とリングバッファ ────────────────────────────────────

/// 出力の 1 行。`seq` はプロセスごとに 0 から振る。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OutLine {
    pub stream: &'static str,
    pub seq: u64,
    pub text: String,
}

/// バイト列を行へ分ける。区切りは `\n` と `\r`（`\r\n` は 1 つ）。1 行が `LINE_MAX` に達したら分割する。
#[derive(Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    skip_lf: bool,
}

impl LineSplitter {
    fn take(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned()
    }

    fn take_capped(&mut self) -> String {
        // 末尾が途中で切れたマルチバイト文字なら、その手前までを 1 行にする
        let cut = match std::str::from_utf8(&self.buf) {
            Err(e) if e.error_len().is_none() && e.valid_up_to() > 0 => e.valid_up_to(),
            _ => self.buf.len(),
        };
        let rest = self.buf.split_off(cut);
        let line = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf = rest;
        line
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in data {
            match b {
                b'\n' => {
                    if self.skip_lf {
                        self.skip_lf = false;
                    } else {
                        out.push(self.take());
                    }
                }
                b'\r' => {
                    // 進捗表示の `\r` 更新を 1 行として届ける。空行は作らない
                    if !self.buf.is_empty() {
                        out.push(self.take());
                    }
                    self.skip_lf = true;
                }
                _ => {
                    self.skip_lf = false;
                    self.buf.push(b);
                    if self.buf.len() >= LINE_MAX {
                        out.push(self.take_capped());
                    }
                }
            }
        }
        out
    }

    /// ストリームの終わり。改行で終わっていない残りを返す。
    pub fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            None
        } else {
            Some(self.take())
        }
    }
}

struct Ring {
    lines: VecDeque<OutLine>,
    bytes: usize,
    next_seq: u64,
}

impl Ring {
    fn new() -> Self {
        Self { lines: VecDeque::new(), bytes: 0, next_seq: 0 }
    }

    fn push(&mut self, stream: &'static str, text: String) -> OutLine {
        let line = OutLine { stream, seq: self.next_seq, text };
        self.next_seq += 1;
        self.bytes += line.text.len();
        self.lines.push_back(line.clone());
        while self.bytes > RING_BYTES {
            match self.lines.pop_front() {
                Some(old) => self.bytes -= old.text.len(),
                None => break,
            }
        }
        line
    }

    /// `since`（含む）以降の行と、`since` 以降に古いほうから捨てられた行があるか。
    fn read(&self, since: u64) -> (Vec<OutLine>, bool) {
        let first = self.lines.front().map(|l| l.seq).unwrap_or(self.next_seq);
        let lines = self.lines.iter().filter(|l| l.seq >= since).cloned().collect();
        (lines, since < first)
    }
}

// ─── ツリーごとの停止 ────────────────────────────────────────────────

/// 子とその子孫をまとめて止める手段。Windows は子ごとの Job Object、Unix はプロセスグループ。
pub struct TreeKiller {
    #[cfg(windows)]
    job: crate::job_object::ProcessJob,
    #[cfg(unix)]
    pgid: i32,
}

impl TreeKiller {
    /// 明示の kill・タイムアウト・出力量の超過で止める（何度呼んでもよい）。Unix は SIGTERM のあと、
    /// 猶予を置いて、まだ残っていれば SIGKILL（スクリプトが後始末できるように）。Windows は即時。
    pub fn kill(&self) {
        self.kill_impl(true);
    }

    /// 猶予なしで止める。子が終わったあとの掃除（孫の回収）と、oretachi の終了で使う。
    /// 子を回収したあとに猶予付きの kill を使うと、空になったグループ ID が別のプロセスに再利用された
    /// あとで SIGKILL が飛びうるので、終了後の掃除は必ずこちら。
    pub fn kill_now(&self) {
        self.kill_impl(false);
    }

    #[cfg(windows)]
    fn kill_impl(&self, _graceful: bool) {
        self.job.terminate();
    }

    #[cfg(unix)]
    fn kill_impl(&self, graceful: bool) {
        let pgid = self.pgid;
        if !graceful {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
            return;
        }
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            // グループがまだ残っているときだけ（空なら ID が再利用されているかもしれないので撃たない）
            unsafe {
                if libc::kill(-pgid, 0) == 0 {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
        });
    }
}

// ─── レジストリ ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExitInfo {
    pub code: Option<i32>,
    /// `"killed"`（kill・停止）/ `"timeout"` / `"output-limit"` / `null`
    pub signal: Option<String>,
    pub duration_ms: u64,
}

struct Entry {
    id: String,
    command: String,
    /// 起動時の解決済みコマンド。権限セットが変わったとき、消えた・変わったコマンドのプロセスを止める
    spec: ResolvedProcessCommand,
    started_at_ms: u64,
    exit: Option<ExitInfo>,
    killer: Option<Arc<TreeKiller>>,
    /// `attach_killer` より前に停止が要求された
    stop_requested: bool,
    stop_reason: Option<&'static str>,
    ring: Ring,
    total_bytes: u64,
    limit_hit: bool,
}

#[derive(Default)]
struct Inner {
    by_label: HashMap<String, Vec<Entry>>,
    /// 停止したインスタンス。以後は起動させない（ラベルは再利用されない）
    dead: HashSet<String>,
    seq: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    pub id: String,
    pub started_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub id: String,
    pub command: String,
    pub started_at: u64,
    pub running: bool,
    pub exit: Option<ExitInfo>,
    pub next_seq: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResult {
    pub lines: Vec<OutLine>,
    pub next_seq: u64,
    /// `sinceSeq` 以降のうち、リングバッファから捨てられた行がある
    pub truncated: bool,
    pub running: bool,
    pub exit: Option<ExitInfo>,
}

/// `AppHandle` を持たない状態機械。バックグラウンドのタスクが持てるよう `Arc` で共有する。
#[derive(Clone, Default)]
pub struct AppProcessRegistry {
    inner: Arc<Mutex<Inner>>,
}

enum PushOutcome {
    Stored { line: OutLine, crossed_limit: bool },
    /// エントリが無い（インスタンスの停止で外された）か、上限を超えたあとで捨てた
    Dropped,
}

impl AppProcessRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // 毒されていても状態は壊れていない（各操作が途中で panic しない）ので、そのまま使う
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 起動の枠を取る。停止済みインスタンス・コマンドごと / インスタンスごと / 全体の上限は `Err`。
    pub fn begin(&self, label: &str, command: &str, spec: &ResolvedProcessCommand) -> Result<(String, u64), String> {
        let mut inner = self.lock();
        if inner.dead.contains(label) {
            return Err("アプリが停止しているため、コマンドを起動できません".to_string());
        }
        let running = |e: &Entry| e.exit.is_none();
        let mine = inner.by_label.get(label);
        let per_cmd = mine.map(|v| v.iter().filter(|e| running(e) && e.command == command).count()).unwrap_or(0);
        let per_inst = mine.map(|v| v.iter().filter(|e| running(e)).count()).unwrap_or(0);
        let total: usize = inner.by_label.values().map(|v| v.iter().filter(|e| running(e)).count()).sum();
        if per_cmd >= spec.max_concurrent as usize {
            return Err(format!(
                "コマンド '{}' は同時に {} 個までです（実行中が上限に達しています）",
                command, spec.max_concurrent
            ));
        }
        if per_inst >= MAX_PER_INSTANCE {
            return Err(format!("このアプリが同時に動かせるプロセスは {} 個までです", MAX_PER_INSTANCE));
        }
        if total >= MAX_TOTAL {
            return Err(format!("oretachi 全体で同時に動かせるプロセスは {} 個までです", MAX_TOTAL));
        }
        inner.seq += 1;
        let id = format!("p{}", inner.seq);
        let started_at_ms = now_ms();
        let entries = inner.by_label.entry(label.to_string()).or_default();
        entries.push(Entry {
            id: id.clone(),
            command: command.to_string(),
            spec: spec.clone(),
            started_at_ms,
            exit: None,
            killer: None,
            stop_requested: false,
            stop_reason: None,
            ring: Ring::new(),
            total_bytes: 0,
            limit_hit: false,
        });
        Ok((id, started_at_ms))
    }

    /// 起動に失敗したときに枠を返す。
    pub fn discard(&self, label: &str, id: &str) {
        if let Some(v) = self.lock().by_label.get_mut(label) {
            v.retain(|e| e.id != id);
        }
    }

    /// 起動した子の停止手段を登録する。起動中に停止が要求されていた（またはエントリが外された）ら `false`。
    fn attach_killer(&self, label: &str, id: &str, killer: Arc<TreeKiller>) -> bool {
        let mut inner = self.lock();
        let Some(e) = inner.by_label.get_mut(label).and_then(|v| v.iter_mut().find(|e| e.id == id)) else {
            return false;
        };
        if e.stop_requested {
            return false;
        }
        e.killer = Some(killer);
        true
    }

    fn push_output(&self, label: &str, id: &str, stream: &'static str, text: String, max_bytes: u64) -> PushOutcome {
        let mut inner = self.lock();
        let Some(e) = inner.by_label.get_mut(label).and_then(|v| v.iter_mut().find(|e| e.id == id)) else {
            return PushOutcome::Dropped;
        };
        if e.limit_hit {
            return PushOutcome::Dropped;
        }
        e.total_bytes += text.len() as u64 + 1;
        let crossed_limit = e.total_bytes > max_bytes;
        if crossed_limit {
            e.limit_hit = true;
            if e.stop_reason.is_none() {
                e.stop_reason = Some("output-limit");
            }
        }
        PushOutcome::Stored { line: e.ring.push(stream, text), crossed_limit }
    }

    /// 停止の理由を記録する（最初の 1 つだけ）。
    fn mark_stop(&self, label: &str, id: &str, reason: &'static str) {
        if let Some(e) = self.lock().by_label.get_mut(label).and_then(|v| v.iter_mut().find(|e| e.id == id)) {
            if e.stop_reason.is_none() {
                e.stop_reason = Some(reason);
            }
        }
    }

    fn finish(&self, label: &str, id: &str, code: Option<i32>, started: Instant) -> Option<ExitInfo> {
        let mut inner = self.lock();
        let entries = inner.by_label.get_mut(label)?;
        let e = entries.iter_mut().find(|e| e.id == id)?;
        let info = ExitInfo {
            code,
            signal: e.stop_reason.map(str::to_string),
            duration_ms: started.elapsed().as_millis() as u64,
        };
        e.exit = Some(info.clone());
        e.killer = None;
        // 終了済みは新しいものから `KEEP_FINISHED` 件だけ残す
        let finished = entries.iter().filter(|e| e.exit.is_some()).count();
        if finished > KEEP_FINISHED {
            let mut drop_n = finished - KEEP_FINISHED;
            entries.retain(|e| {
                if drop_n > 0 && e.exit.is_some() {
                    drop_n -= 1;
                    false
                } else {
                    true
                }
            });
        }
        Some(info)
    }

    /// `kill` を要求する。実行中なら停止手段（無ければ起動直後なので要求だけ記録）を返す。
    /// 戻りの `bool` は「実行中だったか」。
    pub fn request_kill(&self, label: &str, id: &str) -> Option<(bool, Option<Arc<TreeKiller>>)> {
        let mut inner = self.lock();
        let e = inner.by_label.get_mut(label)?.iter_mut().find(|e| e.id == id)?;
        if e.exit.is_some() {
            return Some((false, None));
        }
        e.stop_requested = true;
        if e.stop_reason.is_none() {
            e.stop_reason = Some("killed");
        }
        Some((true, e.killer.clone()))
    }

    pub fn list(&self, label: &str) -> Vec<ProcessInfo> {
        let inner = self.lock();
        inner
            .by_label
            .get(label)
            .map(|v| {
                v.iter()
                    .map(|e| ProcessInfo {
                        id: e.id.clone(),
                        command: e.command.clone(),
                        started_at: e.started_at_ms,
                        running: e.exit.is_none(),
                        exit: e.exit.clone(),
                        next_seq: e.ring.next_seq,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn read(&self, label: &str, id: &str, since: u64) -> Result<ReadResult, String> {
        let inner = self.lock();
        let e = inner
            .by_label
            .get(label)
            .and_then(|v| v.iter().find(|e| e.id == id))
            .ok_or_else(|| format!("プロセス '{}' はありません（終了済みで古いものは破棄されます）", id))?;
        let (lines, truncated) = e.ring.read(since);
        Ok(ReadResult {
            lines,
            next_seq: e.ring.next_seq,
            truncated,
            running: e.exit.is_none(),
            exit: e.exit.clone(),
        })
    }

    /// インスタンスの停止。実行中のものを全部止める。`stopped` なら以後の起動も拒否し、エントリも外す。
    pub fn abort_all(&self, label: &str, stopped: bool) -> Vec<Arc<TreeKiller>> {
        let mut inner = self.lock();
        let mut killers = Vec::new();
        if let Some(v) = inner.by_label.get_mut(label) {
            for e in v.iter_mut().filter(|e| e.exit.is_none()) {
                e.stop_requested = true;
                if e.stop_reason.is_none() {
                    e.stop_reason = Some("killed");
                }
                if let Some(k) = e.killer.clone() {
                    killers.push(k);
                }
            }
        }
        if stopped {
            inner.by_label.remove(label);
            inner.dead.insert(label.to_string());
        }
        killers
    }

    /// 新しい権限セットで消えた・解決結果が変わったコマンドの実行中プロセスを止める。
    pub fn kill_changed(&self, label: &str, new: Option<&ResolvedProcess>) -> Vec<Arc<TreeKiller>> {
        let mut inner = self.lock();
        let mut killers = Vec::new();
        if let Some(v) = inner.by_label.get_mut(label) {
            for e in v.iter_mut().filter(|e| e.exit.is_none()) {
                let same = new.and_then(|p| p.commands.get(&e.command)).is_some_and(|c| *c == e.spec);
                if !same {
                    e.stop_requested = true;
                    if e.stop_reason.is_none() {
                        e.stop_reason = Some("killed");
                    }
                    if let Some(k) = e.killer.clone() {
                        killers.push(k);
                    }
                }
            }
        }
        killers
    }

    /// oretachi の終了。全インスタンスの実行中プロセスを猶予なしで止める。
    pub fn kill_everything(&self) {
        let killers: Vec<Arc<TreeKiller>> = {
            let mut inner = self.lock();
            inner
                .by_label
                .values_mut()
                .flat_map(|v| v.iter_mut())
                .filter(|e| e.exit.is_none())
                .filter_map(|e| {
                    e.stop_requested = true;
                    e.killer.clone()
                })
                .collect()
        };
        for k in killers {
            k.kill_now();
        }
    }
}

// ─── 起動と出力の中継 ────────────────────────────────────────────────

/// 起動に必要なものが全部そろった形（承認済みのコマンドから `build_argv` / `build_env` で作る）。
pub struct SpawnSpec {
    pub exec: String,
    pub argv: Vec<OsString>,
    pub cwd: String,
    pub env: Vec<(OsString, OsString)>,
    pub timeout: Option<Duration>,
    pub max_output_bytes: u64,
}

/// アプリへ push する先。本番は `emit_to_instance`、テストは収集用のクロージャ。
pub type Emit = Arc<dyn Fn(serde_json::Value) + Send + Sync>;

async fn read_stream<R: AsyncRead + Unpin>(
    mut r: R,
    stream: &'static str,
    tx: tokio::sync::mpsc::Sender<(&'static str, String)>,
) {
    let mut splitter = LineSplitter::default();
    let mut buf = vec![0u8; 8192];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for line in splitter.feed(&buf[..n]) {
                    if tx.send((stream, line)).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
    if let Some(rest) = splitter.finish() {
        let _ = tx.send((stream, rest)).await;
    }
}

/// 行を受けてリングへ積み、50〜100 ms ごとにまとめて push する。累計が上限を超えたらツリーごと止める。
async fn pump(
    reg: AppProcessRegistry,
    label: String,
    id: String,
    mut rx: tokio::sync::mpsc::Receiver<(&'static str, String)>,
    emit: Emit,
    killer: Arc<TreeKiller>,
    max_bytes: u64,
) {
    let mut batch: Vec<OutLine> = Vec::new();
    let flush = |batch: &mut Vec<OutLine>| {
        if !batch.is_empty() {
            emit(serde_json::json!({ "id": id, "kind": "output", "lines": std::mem::take(batch) }));
        }
    };
    let mut tick = tokio::time::interval(FLUSH_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some((stream, text)) => {
                    if let PushOutcome::Stored { line, crossed_limit } = reg.push_output(&label, &id, stream, text, max_bytes) {
                        batch.push(line);
                        if crossed_limit {
                            // 異常な出力量なので猶予は置かない（子の終了後に読み残した出力で超えた場合に、
                            // 回収済みのグループへ遅れて SIGKILL を撃たないためにもこちら）
                            killer.kill_now();
                        }
                        if batch.len() >= FLUSH_MAX_LINES {
                            flush(&mut batch);
                        }
                    }
                }
                None => break,
            },
            _ = tick.tick() => flush(&mut batch),
        }
    }
    flush(&mut batch);
}

#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 子を起動し、出力の中継と終了の監視をバックグラウンドで始める。`reg.begin` で枠を取ったあとに呼ぶ。
/// 起動に失敗したら `Err`（枠は呼び出し側が `discard` する）。
pub async fn spawn_process(
    reg: AppProcessRegistry,
    label: String,
    id: String,
    spec: SpawnSpec,
    emit: Emit,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(&spec.exec);
    cmd.args(&spec.argv)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(false);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| format!("起動できませんでした: {}", e))?;
    let started = Instant::now();
    let pid = child.id().ok_or_else(|| "起動直後に終了しました".to_string())?;

    #[cfg(windows)]
    let killer = {
        // 一時停止のまま Job へ入れてから再開する（割り当て前に孫が生まれて Job の外へ出るのを防ぐ）
        let setup = (|| -> Result<crate::job_object::ProcessJob, String> {
            let job = crate::job_object::ProcessJob::new()?;
            let handle = child.raw_handle().ok_or_else(|| "プロセスのハンドルを取得できませんでした".to_string())?;
            job.assign(handle)?;
            Ok(job)
        })();
        match setup {
            Ok(job) => Arc::new(TreeKiller { job }),
            Err(e) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(e);
            }
        }
    };
    #[cfg(unix)]
    let killer = Arc::new(TreeKiller { pgid: pid as i32 });

    if !reg.attach_killer(&label, &id, killer.clone()) {
        killer.kill_now();
        let _ = child.start_kill();
        let _ = child.wait().await;
        return Err("アプリの停止中のため、起動を取り消しました".to_string());
    }
    // 停止手段を登録してから再開する（停止要求と競合しても、停止済みのプロセスが一瞬も動かない）
    #[cfg(windows)]
    if let Err(e) = crate::job_object::resume_process_threads(pid) {
        killer.kill_now();
        let _ = child.start_kill();
        let _ = child.wait().await;
        return Err(e);
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = tokio::sync::mpsc::channel::<(&'static str, String)>(1024);
    let mut readers = Vec::new();
    if let Some(o) = stdout {
        readers.push(tokio::spawn(read_stream(o, "stdout", tx.clone())));
    }
    if let Some(e) = stderr {
        readers.push(tokio::spawn(read_stream(e, "stderr", tx.clone())));
    }
    drop(tx);
    let pump_handle = tokio::spawn(pump(
        reg.clone(),
        label.clone(),
        id.clone(),
        rx,
        emit.clone(),
        killer.clone(),
        spec.max_output_bytes,
    ));

    let timeout = spec.timeout;
    tokio::spawn(async move {
        let status = match timeout {
            Some(d) => match tokio::time::timeout(d, child.wait()).await {
                Ok(r) => r,
                Err(_) => {
                    reg.mark_stop(&label, &id, "timeout");
                    killer.kill();
                    child.wait().await
                }
            },
            None => child.wait().await,
        };
        // 終了の定義は直下の子の `wait()`。出力を継承した孫がパイプを握っていても、ここで止めて吸い切る。
        // 子はもう回収済みなので、猶予付きの kill（遅れて SIGKILL を撃つ）は使わない
        killer.kill_now();
        for r in readers {
            let abort = r.abort_handle();
            if tokio::time::timeout(DRAIN_TIMEOUT, r).await.is_err() {
                abort.abort();
            }
        }
        let pump_abort = pump_handle.abort_handle();
        if tokio::time::timeout(DRAIN_TIMEOUT, pump_handle).await.is_err() {
            pump_abort.abort();
        }
        let code = status.ok().and_then(|s| s.code());
        if let Some(info) = reg.finish(&label, &id, code, started) {
            emit(serde_json::json!({
                "id": id,
                "kind": "exit",
                "code": info.code,
                "signal": info.signal,
                "durationMs": info.duration_ms,
            }));
        }
    });
    Ok(())
}

// ─── ライフサイクルのフック ───────────────────────────────────────────

/// インスタンスの停止・取り消し（`app_instance::release`）。全プロセスをツリーごと止め、以後は起動させない。
/// リロード（`request_reload`）からは呼ばない（プロセスはリロードをまたいで生きる）。
pub fn on_instance_released(app: &AppHandle, label: &str) {
    if let Some(reg) = app.try_state::<AppProcessRegistry>() {
        for k in reg.abort_all(label, true) {
            k.kill();
        }
    }
}

/// 再承認で権限セットが入れ替わったあと。消えた・解決結果が変わったコマンドの実行中プロセスを止める。
pub fn on_grant_replaced(app: &AppHandle, label: &str, worktree_id: &str, artifact_id: &str) {
    let Some(reg) = app.try_state::<AppProcessRegistry>() else { return };
    let new = app
        .state::<crate::app_grant::AppGrantStore>()
        .get(worktree_id, artifact_id)
        .and_then(|g| g.resolved.process);
    for k in reg.kill_changed(label, new.as_ref()) {
        k.kill();
    }
}

/// oretachi の終了（`RunEvent::Exit`）。`release` を通らない経路の保険。
pub fn kill_all_fast(app: &AppHandle) {
    if let Some(reg) = app.try_state::<AppProcessRegistry>() {
        reg.kill_everything();
    }
}

// ─── Tauri コマンド（appinst-* ウィンドウからだけ） ────────────────────

fn instance_ctx(window: &tauri::Window, app: &AppHandle) -> Result<crate::app_instance::InstanceContext, String> {
    crate::app_instance::require_instance_window(window)?;
    crate::app_instance::context_for_label(app, window.label())
}

fn granted(app: &AppHandle, ctx: &crate::app_instance::InstanceContext) -> Result<ResolvedProcess, String> {
    let grant = app
        .state::<crate::app_grant::AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .ok_or_else(|| "このアプリには承認がありません".to_string())?;
    grant
        .resolved
        .process
        .ok_or_else(|| "このアプリにはコマンド起動（process）の権限が承認されていません".to_string())
}

fn app_log(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, level: &str, message: String) {
    crate::app_instance::add_log(app, &ctx.worktree_id, &ctx.artifact_id, level, message);
}

/// 拒否・失敗はデバッグログへ残す。パラメータの値は出さない（本文を AI の経路に載せたくない用途がある）。
fn log_deny(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, what: &str, error: &str) {
    let note = if ctx.reapproval_pending { "（再承認待ちのため、新しい権限はまだ使えません）" } else { "" };
    log::debug!("[app-process] 拒否 {} {}/{} — {}", what, ctx.worktree_id, ctx.artifact_id, error);
    app_log(app, ctx, "warn", format!("拒否: process.{} — {}{}", what, error, note));
}

#[tauri::command]
pub async fn app_process_start(
    window: tauri::Window,
    app: AppHandle,
    name: String,
    params: Option<serde_json::Value>,
) -> Result<StartResult, String> {
    let ctx = instance_ctx(&window, &app)?;
    let what = format!("start {}", name.chars().take(64).collect::<String>());
    let fail = |e: String| -> String {
        log_deny(&app, &ctx, &what, &e);
        e
    };
    let perm = granted(&app, &ctx).map_err(&fail)?;
    let cmd = perm
        .commands
        .get(&name)
        .ok_or_else(|| fail(format!("コマンド '{}' は宣言も承認もされていません", name.chars().take(64).collect::<String>())))?
        .clone();
    // 解決時にも見ているが、念のため（.bat / .cmd は cmd.exe 経由の注入になる）
    if is_forbidden_exec_ext(&cmd.exec) {
        return Err(fail("exec が .bat / .cmd です".to_string()));
    }

    let params = params.unwrap_or(serde_json::Value::Null);
    let cmd_for_argv = cmd.clone();
    let argv = tauri::async_runtime::spawn_blocking(move || build_argv(&cmd_for_argv, &params))
        .await
        .map_err(|e| fail(e.to_string()))?
        .map_err(&fail)?;
    let env = build_env(&cmd.env_inherit, std::env::vars_os());

    let reg = app.state::<AppProcessRegistry>().inner().clone();
    let (id, started_at) = reg.begin(&ctx.instance_label, &name, &cmd).map_err(&fail)?;

    let emit: Emit = {
        let app = app.clone();
        let label = ctx.instance_label.clone();
        let ctx2 = ctx.clone();
        let name = name.clone();
        let id = id.clone();
        Arc::new(move |v: serde_json::Value| {
            if v.get("kind").and_then(|k| k.as_str()) == Some("exit") {
                let code = v.get("code").and_then(|c| c.as_i64());
                let signal = v.get("signal").and_then(|s| s.as_str()).unwrap_or("-");
                let ms = v.get("durationMs").and_then(|d| d.as_u64()).unwrap_or(0);
                app_log(
                    &app,
                    &ctx2,
                    "info",
                    format!(
                        "終了: process {}（{}） code={} signal={} {}ms",
                        name,
                        id,
                        code.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
                        signal,
                        ms
                    ),
                );
            }
            crate::app_subwindow::emit_to_instance(&app, &label, EVENT_NAME, v);
        })
    };
    let spec = SpawnSpec {
        exec: cmd.exec.clone(),
        argv,
        cwd: cmd.cwd.clone(),
        env,
        timeout: cmd.timeout_sec.map(Duration::from_secs),
        max_output_bytes: cmd.max_output_bytes,
    };
    if let Err(e) = spawn_process(reg.clone(), ctx.instance_label.clone(), id.clone(), spec, emit).await {
        reg.discard(&ctx.instance_label, &id);
        return Err(fail(e));
    }
    app_log(&app, &ctx, "info", format!("許可: process.start {}（{}、{}）", name, id, cmd.exec));
    Ok(StartResult { id, started_at })
}

#[tauri::command]
pub async fn app_process_kill(window: tauri::Window, app: AppHandle, id: String) -> Result<bool, String> {
    let ctx = instance_ctx(&window, &app)?;
    granted(&app, &ctx).map_err(|e| {
        log_deny(&app, &ctx, "kill", &e);
        e
    })?;
    let reg = app.state::<AppProcessRegistry>().inner().clone();
    match reg.request_kill(&ctx.instance_label, &id) {
        None => Err(format!("プロセス '{}' はありません", id)),
        Some((was_running, killer)) => {
            if let Some(k) = killer {
                k.kill();
            }
            if was_running {
                app_log(&app, &ctx, "info", format!("許可: process.kill {}", id));
            }
            Ok(was_running)
        }
    }
}

#[tauri::command]
pub async fn app_process_list(window: tauri::Window, app: AppHandle) -> Result<Vec<ProcessInfo>, String> {
    let ctx = instance_ctx(&window, &app)?;
    granted(&app, &ctx).map_err(|e| {
        log_deny(&app, &ctx, "list", &e);
        e
    })?;
    Ok(app.state::<AppProcessRegistry>().list(&ctx.instance_label))
}

#[tauri::command]
pub async fn app_process_read(
    window: tauri::Window,
    app: AppHandle,
    id: String,
    since_seq: Option<u64>,
) -> Result<ReadResult, String> {
    let ctx = instance_ctx(&window, &app)?;
    granted(&app, &ctx).map_err(|e| {
        log_deny(&app, &ctx, "read", &e);
        e
    })?;
    app.state::<AppProcessRegistry>().read(&ctx.instance_label, &id, since_seq.unwrap_or(0))
}

// ─── テスト ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_manifest::{ProcessCommand, ResolvedFsScope};

    fn spec(max_concurrent: u32) -> ResolvedProcessCommand {
        ResolvedProcessCommand {
            exec: "/bin/true".into(),
            args: vec![],
            cwd: ".".into(),
            env_inherit: vec![],
            max_concurrent,
            timeout_sec: None,
            max_output_bytes: 1 << 20,
        }
    }

    // ─── 行分割 ───

    fn feed_all(chunks: &[&[u8]]) -> Vec<String> {
        let mut s = LineSplitter::default();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(s.feed(c));
        }
        out.extend(s.finish());
        out
    }

    #[test]
    fn splits_on_lf_cr_and_crlf() {
        assert_eq!(feed_all(&[b"a\nb\r\nc\rd"]), vec!["a", "b", "c", "d"]);
        // `\r\n` が chunk の境界をまたいでも 1 つの区切り
        assert_eq!(feed_all(&[b"a\r", b"\nb\n"]), vec!["a", "b"]);
        // 進捗表示（`\r` だけで更新）が行として届く。空行は作らない
        assert_eq!(feed_all(&[b"10%\r20%\r30%\r"]), vec!["10%", "20%", "30%"]);
        // 改行で終わらない残りも出る
        assert_eq!(feed_all(&[b"tail"]), vec!["tail"]);
        // `\n` だけの空行は残す
        assert_eq!(feed_all(&[b"a\n\nb\n"]), vec!["a", "", "b"]);
    }

    #[test]
    fn splits_long_lines_without_breaking_multibyte_chars() {
        let line = "あ".repeat(LINE_MAX); // 3 バイト文字。LINE_MAX バイトを大きく超える
        let out = feed_all(&[line.as_bytes()]);
        assert!(out.len() > 1);
        assert!(out.iter().all(|l| !l.contains('\u{fffd}')), "文字の途中で切れていない");
        assert_eq!(out.concat(), line);
    }

    // ─── リング ───

    #[test]
    fn ring_drops_oldest_and_reports_truncation() {
        let mut r = Ring::new();
        let big = "x".repeat(LINE_MAX);
        let n = RING_BYTES / LINE_MAX + 10;
        for _ in 0..n {
            r.push("stdout", big.clone());
        }
        assert!(r.bytes <= RING_BYTES);
        assert_eq!(r.next_seq, n as u64);
        let (lines, truncated) = r.read(0);
        assert!(truncated, "先頭から捨てられている");
        assert_eq!(lines.last().map(|l| l.seq), Some(n as u64 - 1));
        let first = lines.first().unwrap().seq;
        let (lines2, truncated2) = r.read(first);
        assert!(!truncated2);
        assert_eq!(lines2.len(), lines.len());
        let (none, t3) = r.read(n as u64);
        assert!(none.is_empty() && !t3);
    }

    // ─── 引数 ───

    fn args_cmd(args: Vec<ResolvedProcessArg>) -> ResolvedProcessCommand {
        ResolvedProcessCommand { args, ..spec(1) }
    }

    fn fs_with(root: &Path, under: &str) -> ResolvedFs {
        let root_s = root.to_string_lossy().into_owned();
        ResolvedFs {
            worktree_root: Some(root_s.clone()),
            artifact_root: None,
            scopes: vec![ResolvedFsScope {
                scheme: "worktree".into(),
                prefix: under.to_string(),
                path: root.join(under).to_string_lossy().into_owned(),
                write: false,
            }],
            allow_sensitive_paths: false,
        }
    }

    fn json(v: serde_json::Value) -> serde_json::Value {
        v
    }

    #[test]
    fn argv_rejects_undeclared_missing_and_mistyped_params() {
        let cmd = args_cmd(vec![
            ResolvedProcessArg::Fixed { value: "-X".into() },
            ResolvedProcessArg::Token { param: "mode".into() },
            ResolvedProcessArg::Enum { param: "size".into(), values: vec!["l".into(), "s".into()] },
        ]);
        let ok = build_argv(&cmd, &json(serde_json::json!({"mode": "fast_1", "size": "s"}))).unwrap();
        assert_eq!(ok, vec![OsString::from("-X"), "fast_1".into(), "s".into()]);
        // 宣言外
        assert!(build_argv(&cmd, &json(serde_json::json!({"mode": "a", "size": "s", "extra": "x"}))).is_err());
        // 欠落
        assert!(build_argv(&cmd, &json(serde_json::json!({"mode": "a"}))).is_err());
        // 型に合わない（token に空白・記号、数値、enum の候補外）
        for bad in [
            serde_json::json!({"mode": "a b", "size": "s"}),
            serde_json::json!({"mode": "a;b", "size": "s"}),
            serde_json::json!({"mode": 1, "size": "s"}),
            serde_json::json!({"mode": "a", "size": "xl"}),
            serde_json::json!({"mode": "", "size": "s"}),
        ] {
            assert!(build_argv(&cmd, &bad).is_err(), "{}", bad);
        }
        // params がオブジェクトでない
        assert!(build_argv(&cmd, &serde_json::json!(["a"])).is_err());
    }

    #[test]
    fn argv_rejects_values_starting_with_dash_for_every_type() {
        let cmd = args_cmd(vec![
            ResolvedProcessArg::Token { param: "t".into() },
            ResolvedProcessArg::Enum { param: "e".into(), values: vec!["-x".into(), "y".into()] },
        ]);
        // token の文字種には `-` があるが、先頭は不可
        assert!(build_argv(&cmd, &json(serde_json::json!({"t": "-rf", "e": "y"}))).is_err());
        // 宣言側が `-x` を持っていても（manifest では拒否されるが）、呼び出しの関門でも拒否する
        assert!(build_argv(&cmd, &json(serde_json::json!({"t": "a", "e": "-x"}))).is_err());
        assert!(build_argv(&cmd, &json(serde_json::json!({"t": "a-b", "e": "y"}))).is_ok());
    }

    #[test]
    fn argv_path_params_stay_under_the_declared_prefix() {
        let dir = tempfile_dir("argv_path");
        std::fs::create_dir_all(dir.join("runs").join("a")).unwrap();
        std::fs::create_dir_all(dir.join("other")).unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        let cmd = args_cmd(vec![ResolvedProcessArg::Path {
            param: "request".into(),
            under: "worktree://runs/".into(),
            fs: fs_with(&root, "runs"),
        }]);
        let good = build_argv(&cmd, &json(serde_json::json!({"request": "worktree://runs/a/request.json"}))).unwrap();
        let shown = good[0].to_string_lossy().into_owned();
        assert!(shown.ends_with("request.json") && !shown.starts_with(r"\\?\"), "{}", shown);
        for bad in [
            "worktree://other/x.json",              // under の外
            "worktree://runs/../other/x.json",      // ..
            "worktree://runs/a/..\\..\\other/x",    // 区切りを混ぜた ..
            "runs/a/request.json",                  // スキーム無し
            "/etc/passwd",                          // 絶対パス
            "C:\\Windows\\win.ini",
            "artifact://x",                         // 承認されていないスキーム
            "-worktree://runs/a",                   // `-` 始まり
        ] {
            let e = build_argv(&cmd, &json(serde_json::json!({"request": bad}))).expect_err(bad);
            // 拒否の理由はデバッグログにも残るので、値（パス）そのものは含めない
            assert!(!e.contains(bad), "エラー文に値が入っている: {}", e);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn argv_path_params_reject_symlink_or_junction_escape() {
        let dir = tempfile_dir("argv_link");
        let outside = tempfile_dir("argv_link_outside");
        std::fs::create_dir_all(dir.join("runs")).unwrap();
        std::fs::write(outside.join("secret.txt"), "x").unwrap();
        let link = dir.join("runs").join("link");
        if !make_dir_link(&outside, &link) {
            eprintln!("リンクを作れない環境のためスキップ");
            return;
        }
        let root = std::fs::canonicalize(&dir).unwrap();
        let cmd = args_cmd(vec![ResolvedProcessArg::Path {
            param: "request".into(),
            under: "worktree://runs/".into(),
            fs: fs_with(&root, "runs"),
        }]);
        let r = build_argv(&cmd, &json(serde_json::json!({"request": "worktree://runs/link/secret.txt"})));
        assert!(r.is_err(), "リンクで外へ出るパスは通らない: {:?}", r);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    fn tempfile_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("oretachi-app-process-{}-{}-{}", tag, std::process::id(), now_ms()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[cfg(windows)]
    fn make_dir_link(target: &Path, link: &Path) -> bool {
        // junction（管理者権限が要らない）
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn make_dir_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    // ─── 環境 ───

    #[test]
    fn env_passes_only_inherited_names_and_never_forbidden_prefixes() {
        let parent = vec![
            ("PATH", "p"),
            ("HOME", "h"),
            ("ORETACHI_MCP_PORT", "4000"),
            ("MCP_TOKEN", "t"),
            ("CLAUDE_CODE_X", "c"),
            ("SECRET", "s"),
        ]
        .into_iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        // 禁止の接頭辞は inherit に書かれていても渡さない
        let inherit: Vec<String> = ["PATH", "ORETACHI_MCP_PORT", "MCP_TOKEN", "CLAUDE_CODE_X"].map(String::from).to_vec();
        let out = build_env(&inherit, parent);
        let names: Vec<String> = out.iter().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["PATH"]);
    }

    #[cfg(windows)]
    #[test]
    fn env_always_passes_system_root_on_windows_and_matches_case_insensitively() {
        let parent = vec![("Path", "p"), ("SystemRoot", "C:\\Windows"), ("Other", "o")]
            .into_iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let out = build_env(&["PATH".to_string()], parent);
        let names: Vec<String> = out.iter().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Path", "SystemRoot"]);
    }

    // ─── 解決 ───

    fn requested_cmd(exec: &str) -> ProcessCommand {
        ProcessCommand {
            exec: exec.into(),
            args: vec![],
            cwd: "worktree://".into(),
            env_inherit: vec![],
            max_concurrent: 1,
            timeout_sec: None,
            max_output_bytes: 1 << 20,
        }
    }

    #[test]
    fn find_in_path_ignores_relative_and_empty_entries() {
        let dir = tempfile_dir("find");
        let exe = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::write(dir.join(exe), "x").unwrap();
        let rel_only = std::env::join_paths([PathBuf::from("."), PathBuf::from("")]).unwrap();
        // カレント由来（相対・空）のエントリだけなら見つからない
        let cwd = std::env::current_dir().unwrap();
        std::fs::write(cwd.join(exe), "x").ok();
        assert!(find_in_path("tool", Some(&rel_only)).is_none());
        let _ = std::fs::remove_file(cwd.join(exe));
        let abs = std::env::join_paths([dir.clone()]).unwrap();
        assert_eq!(find_in_path("tool", Some(&abs)), Some(dir.join(exe)));
        // 0 バイトのファイル（ストアの実行エイリアスのスタブ）は実行ファイルとして扱わない
        std::fs::write(dir.join(exe), "").unwrap();
        assert!(find_in_path("tool", Some(&abs)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_rejects_bat_cmd_and_missing_exec_and_resolves_worktree_paths() {
        let wt = tempfile_dir("resolve_wt");
        let art = tempfile_dir("resolve_art");
        let exe = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::create_dir_all(wt.join("bin")).unwrap();
        std::fs::write(wt.join("bin").join(exe), "x").unwrap();
        std::fs::write(wt.join("bin").join("run.bat"), "x").unwrap();
        let one = |c: ProcessCommand| {
            let mut m = std::collections::BTreeMap::new();
            m.insert("c".to_string(), c);
            ProcessPermission { commands: m }
        };
        // 解決できる: worktree:// の実行ファイルは絶対パスへ
        let ok = resolve_requested_process(&one(requested_cmd(&format!("worktree://bin/{}", exe))), Some(&wt), &art, None).unwrap();
        let exec = &ok.commands["c"].exec;
        assert!(Path::new(exec).is_absolute() && !exec.starts_with(r"\\?\"), "{}", exec);
        // .bat / .cmd は拒否
        assert!(resolve_requested_process(&one(requested_cmd("worktree://bin/run.bat")), Some(&wt), &art, None).is_err());
        // 存在しない
        assert!(resolve_requested_process(&one(requested_cmd("worktree://bin/none")), Some(&wt), &art, None).is_err());
        // PATH に無い裸の名前
        assert!(resolve_requested_process(&one(requested_cmd("definitely-not-a-command")), Some(&wt), &art, None).is_err());
        // ホーム / リポジトリ（worktree_path = None）では worktree:// を使えない
        assert!(resolve_requested_process(&one(requested_cmd(&format!("worktree://bin/{}", exe))), None, &art, None).is_err());
        let _ = std::fs::remove_dir_all(&wt);
        let _ = std::fs::remove_dir_all(&art);
    }

    #[test]
    fn resolved_check_rejects_exec_or_cwd_inside_a_writable_scope() {
        let wt = std::fs::canonicalize(tempfile_dir("wcheck")).unwrap();
        let exec = wt.join("scripts").join("run.exe").to_string_lossy().into_owned();
        let mut cmd = spec(1);
        cmd.exec = strip_verbatim(&exec);
        cmd.cwd = strip_verbatim(&wt.to_string_lossy());
        let process = ResolvedProcess { commands: [("c".to_string(), cmd)].into_iter().collect() };
        let fs_write = |sub: &str| ResolvedFs {
            worktree_root: Some(wt.to_string_lossy().into_owned()),
            artifact_root: None,
            scopes: vec![ResolvedFsScope {
                scheme: "worktree".into(),
                prefix: sub.into(),
                path: wt.join(sub).to_string_lossy().into_owned(),
                write: true,
            }],
            allow_sensitive_paths: false,
        };
        // runs/ だけ書ける → exec（scripts/）も cwd（ルート）も覆われない
        assert!(check_resolved_against_write_scopes(&process, Some(&fs_write("runs")), None).is_ok());
        // scripts/ が書ける → exec を覆う
        assert!(check_resolved_against_write_scopes(&process, Some(&fs_write("scripts")), None).is_err());
        // ルートが書ける → 全部覆う（db の書き込みスコープでも同じ）
        assert!(check_resolved_against_write_scopes(&process, None, Some(&fs_write(""))).is_err());
        // cwd 基準の相対パスの固定引数（`python scripts/run.py`）。オプションは対象外
        let mut rel = spec(1);
        rel.exec = "C:\\Python\\python.exe".into();
        rel.cwd = strip_verbatim(&wt.to_string_lossy());
        rel.args = vec![
            ResolvedProcessArg::Fixed { value: "-X".into() },
            ResolvedProcessArg::Fixed { value: "scripts/run.py".into() },
        ];
        let rel_process = ResolvedProcess { commands: [("c".to_string(), rel)].into_iter().collect() };
        assert!(check_resolved_against_write_scopes(&rel_process, Some(&fs_write("runs")), None).is_ok());
        assert!(check_resolved_against_write_scopes(&rel_process, Some(&fs_write("scripts")), None).is_err());
        // `--名前=値` の右辺も相対パスとして照合する。値のないオプション・別の場所を指す値は対象外
        let mut opt = spec(1);
        opt.exec = "C:\\Python\\python.exe".into();
        opt.cwd = strip_verbatim(&wt.to_string_lossy());
        opt.args = vec![ResolvedProcessArg::Fixed { value: "--config=scripts/x.py".into() }];
        let opt_process = ResolvedProcess { commands: [("c".to_string(), opt.clone())].into_iter().collect() };
        assert!(check_resolved_against_write_scopes(&opt_process, Some(&fs_write("scripts")), None).is_err());
        assert!(check_resolved_against_write_scopes(&opt_process, Some(&fs_write("runs")), None).is_ok());
        opt.args = vec![ResolvedProcessArg::Fixed { value: "--level=2".into() }];
        let level = ResolvedProcess { commands: [("c".to_string(), opt)].into_iter().collect() };
        assert!(check_resolved_against_write_scopes(&level, Some(&fs_write("scripts")), None).is_ok());
        // 相対の固定引数が cwd 内のリンク（work/link → scripts/）を通る形は、実パスで見て拒否する。
        // `..` を畳まない字句の比較だけでは、work/link/run.py は scripts/ の配下に見えない
        std::fs::create_dir_all(wt.join("scripts")).unwrap();
        std::fs::create_dir_all(wt.join("work")).unwrap();
        if make_dir_link(&wt.join("scripts"), &wt.join("work").join("link")) {
            let mut via = spec(1);
            via.exec = "C:\\Python\\python.exe".into();
            via.cwd = strip_verbatim(&wt.join("work").to_string_lossy());
            via.args = vec![ResolvedProcessArg::Fixed { value: "link/run.py".into() }];
            let via_process = ResolvedProcess { commands: [("c".to_string(), via)].into_iter().collect() };
            let e = check_resolved_against_write_scopes(&via_process, Some(&fs_write("scripts")), None).unwrap_err();
            assert!(e.contains("書き込める場所"), "{}", e);
            // 書き込めない場所へ向いたリンクなら通る
            assert!(check_resolved_against_write_scopes(&via_process, Some(&fs_write("runs")), None).is_ok());
        } else {
            eprintln!("リンクを作れない環境のためスキップ");
        }
        // exec が書き込める場所へのリンクを通る形（実パスで見る）
        if make_dir_link(&wt.join("scripts"), &wt.join("work").join("bin")) {
            let mut via = spec(1);
            via.exec = strip_verbatim(&wt.join("work").join("bin").join("run.exe").to_string_lossy());
            via.cwd = strip_verbatim(&wt.join("work").to_string_lossy());
            let p = ResolvedProcess { commands: [("c".to_string(), via)].into_iter().collect() };
            assert!(check_resolved_against_write_scopes(&p, Some(&fs_write("scripts")), None).is_err());
        }
        let _ = std::fs::remove_dir_all(&wt);
    }

    // ─── レジストリ ───

    #[test]
    fn registry_enforces_command_instance_and_total_limits() {
        let reg = AppProcessRegistry::default();
        let s = spec(2);
        assert!(reg.begin("a", "c", &s).is_ok());
        assert!(reg.begin("a", "c", &s).is_ok());
        assert!(reg.begin("a", "c", &s).is_err(), "コマンドごとの上限");
        // 別のコマンドは別枠。インスタンスごとの上限まで
        let wide = spec(8);
        for _ in 0..(MAX_PER_INSTANCE - 2) {
            reg.begin("a", "d", &wide).unwrap();
        }
        assert!(reg.begin("a", "d", &wide).is_err(), "インスタンスごとの上限");
        // 全体の上限は、別インスタンスの分も数える
        for label in ["b", "c"] {
            for _ in 0..MAX_PER_INSTANCE {
                if reg.begin(label, "d", &wide).is_err() {
                    break;
                }
            }
        }
        assert!(reg.begin("z", "d", &wide).is_err(), "全体の上限");
    }

    #[test]
    fn registry_isolates_instances_and_blocks_after_stop() {
        let reg = AppProcessRegistry::default();
        let (id, _) = reg.begin("a", "c", &spec(1)).unwrap();
        assert!(reg.list("b").is_empty());
        assert!(reg.read("b", &id, 0).is_err(), "他のインスタンスのプロセスは読めない");
        assert!(reg.request_kill("b", &id).is_none(), "他のインスタンスのプロセスは止められない");
        assert_eq!(reg.list("a").len(), 1);
        reg.abort_all("a", true);
        assert!(reg.list("a").is_empty());
        assert!(reg.begin("a", "c", &spec(1)).is_err(), "停止後は起動できない");
        assert!(reg.begin("b", "c", &spec(1)).is_ok());
    }

    #[test]
    fn registry_keeps_a_bounded_number_of_finished_entries() {
        let reg = AppProcessRegistry::default();
        let s = spec(8);
        for _ in 0..(KEEP_FINISHED + 5) {
            let (id, _) = reg.begin("a", "c", &s).unwrap();
            assert!(reg.finish("a", &id, Some(0), Instant::now()).is_some());
        }
        assert_eq!(reg.list("a").len(), KEEP_FINISHED);
    }

    #[test]
    fn output_limit_marks_the_stop_reason_once() {
        let reg = AppProcessRegistry::default();
        let (id, _) = reg.begin("a", "c", &spec(1)).unwrap();
        let big = "x".repeat(600);
        let first = reg.push_output("a", &id, "stdout", big.clone(), 1000);
        assert!(matches!(first, PushOutcome::Stored { crossed_limit: false, .. }));
        let second = reg.push_output("a", &id, "stdout", big.clone(), 1000);
        assert!(matches!(second, PushOutcome::Stored { crossed_limit: true, .. }));
        // 超えたあとは捨てる
        assert!(matches!(reg.push_output("a", &id, "stdout", big, 1000), PushOutcome::Dropped));
        reg.mark_stop("a", &id, "timeout");
        let info = reg.finish("a", &id, None, Instant::now()).unwrap();
        assert_eq!(info.signal.as_deref(), Some("output-limit"), "最初の理由が残る");
    }

    #[test]
    fn kill_changed_targets_removed_or_modified_commands_only() {
        let reg = AppProcessRegistry::default();
        let keep = spec(1);
        let mut changed = spec(1);
        changed.cwd = "/old".into();
        reg.begin("a", "keep", &keep).unwrap();
        reg.begin("a", "changed", &changed).unwrap();
        reg.begin("a", "gone", &keep).unwrap();
        let mut new_cmd = changed.clone();
        new_cmd.cwd = "/new".into();
        let new = ResolvedProcess {
            commands: [("keep".to_string(), keep.clone()), ("changed".to_string(), new_cmd)].into_iter().collect(),
        };
        // 停止手段はまだ無い（起動直後）ので返らないが、要求は記録される
        assert!(reg.kill_changed("a", Some(&new)).is_empty());
        let infos = reg.list("a");
        assert_eq!(infos.len(), 3);
        let inner = reg.lock();
        let flags: HashMap<&str, bool> =
            inner.by_label["a"].iter().map(|e| (e.command.as_str(), e.stop_requested)).collect();
        assert_eq!(flags["keep"], false);
        assert_eq!(flags["changed"], true);
        assert_eq!(flags["gone"], true);
    }

    // ─── 実プロセス（Windows） ───

    #[cfg(windows)]
    mod real {
        use super::*;
        use std::sync::Mutex as StdMutex;

        const CMD: &str = r"C:\Windows\System32\cmd.exe";

        fn collector() -> (Emit, Arc<StdMutex<Vec<serde_json::Value>>>) {
            let events = Arc::new(StdMutex::new(Vec::new()));
            let e2 = events.clone();
            (Arc::new(move |v| e2.lock().unwrap().push(v)), events)
        }

        /// 本番と同じ `build_env`（`env.inherit` に書いた名前 + SystemRoot だけ）。PATH を渡さないと `cmd /c ping` は
        /// ping.exe を見つけられない（環境が分離されている証拠でもある）
        fn sys_env() -> Vec<(OsString, OsString)> {
            build_env(&["PATH".to_string()], std::env::vars_os())
        }

        async fn run(
            reg: &AppProcessRegistry,
            label: &str,
            line: &str,
            timeout: Option<Duration>,
            max_output: u64,
        ) -> (String, Arc<StdMutex<Vec<serde_json::Value>>>) {
            let s = spec(4);
            let (id, _) = reg.begin(label, "c", &s).unwrap();
            let (emit, events) = collector();
            spawn_process(
                reg.clone(),
                label.to_string(),
                id.clone(),
                SpawnSpec {
                    exec: CMD.into(),
                    argv: vec!["/c".into(), line.into()],
                    cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                    env: sys_env(),
                    timeout,
                    max_output_bytes: max_output,
                },
                emit,
            )
            .await
            .unwrap();
            (id, events)
        }

        async fn wait_exit(events: &Arc<StdMutex<Vec<serde_json::Value>>>, secs: u64) -> Option<serde_json::Value> {
            let deadline = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < deadline {
                if let Some(e) = events.lock().unwrap().iter().find(|e| e["kind"] == "exit") {
                    return Some(e.clone());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            None
        }

        fn lines_of(events: &Arc<StdMutex<Vec<serde_json::Value>>>) -> Vec<String> {
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == "output")
                .flat_map(|e| e["lines"].as_array().unwrap().clone())
                .map(|l| l["text"].as_str().unwrap().to_string())
                .collect()
        }

        fn ping_processes(marker: &str) -> usize {
            let mut sys = sysinfo::System::new();
            // 既定の更新ではコマンドラインを読まない
            sys.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                true,
                sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
            );
            // コマンドラインは引数ごとに分かれて返る（`ping` `-n` `61` …）ので、空白でつないで探す
            sys.processes()
                .values()
                .filter(|p| {
                    let line: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
                    line.join(" ").contains(marker)
                })
                .count()
        }

        #[tokio::test]
        async fn runs_a_command_and_pushes_lines_and_exit_code() {
            let reg = AppProcessRegistry::default();
            let (id, events) = run(&reg, "a", "echo one&& echo two>&2&& exit 3", None, 1 << 20).await;
            let exit = wait_exit(&events, 10).await.expect("exit が届く");
            assert_eq!(exit["code"], 3);
            assert!(exit["signal"].is_null());
            let lines = lines_of(&events);
            assert!(lines.contains(&"one".to_string()) && lines.contains(&"two".to_string()), "{:?}", lines);
            // read で取り戻せる（seq は 0 始まり）
            let r = reg.read("a", &id, 0).unwrap();
            assert!(!r.running && r.lines.len() >= 2 && r.lines[0].seq == 0);
            assert_eq!(reg.read("a", &id, r.next_seq).unwrap().lines.len(), 0);
        }

        #[tokio::test]
        async fn child_sees_only_inherited_variables_and_never_the_forbidden_ones() {
            // cmd は未定義の変数を `%NAME%` のまま出す。子に渡っていれば値が出る
            std::env::set_var("ORETACHI_MCP_PORT_TEST", "4000");
            std::env::set_var("OTHER_SECRET_TEST", "leak");
            std::env::set_var("ALLOWED_TEST", "ok");
            let reg = AppProcessRegistry::default();
            let s = spec(1);
            let (id, _) = reg.begin("a", "c", &s).unwrap();
            let (emit, events) = collector();
            // inherit に禁止の接頭辞の名前が書かれていても渡さない（manifest で拒否されるが、起動時にも除外する）
            let inherit = ["PATH", "ALLOWED_TEST", "ORETACHI_MCP_PORT_TEST"].map(String::from);
            spawn_process(
                reg.clone(),
                "a".into(),
                id,
                SpawnSpec {
                    exec: CMD.into(),
                    argv: vec![
                        "/c".into(),
                        "echo [%ORETACHI_MCP_PORT_TEST%][%OTHER_SECRET_TEST%][%ALLOWED_TEST%]".into(),
                    ],
                    cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                    env: build_env(&inherit, std::env::vars_os()),
                    timeout: None,
                    max_output_bytes: 1 << 20,
                },
                emit,
            )
            .await
            .unwrap();
            wait_exit(&events, 10).await.expect("exit");
            assert_eq!(
                lines_of(&events),
                vec!["[%ORETACHI_MCP_PORT_TEST%][%OTHER_SECRET_TEST%][ok]"],
                "継承を宣言した変数だけが渡る"
            );
        }

        #[tokio::test]
        async fn kill_stops_the_whole_tree() {
            let reg = AppProcessRegistry::default();
            let marker = "ping -n 61 127.0.0.1";
            let (id, events) = run(&reg, "a", &format!("{} >nul", marker), None, 1 << 20).await;
            tokio::time::sleep(Duration::from_millis(800)).await;
            assert!(ping_processes("-n 61").ge(&1), "孫の ping が動いている");
            let (was_running, killer) = reg.request_kill("a", &id).unwrap();
            assert!(was_running);
            killer.unwrap().kill();
            let exit = wait_exit(&events, 10).await.expect("kill で exit が届く");
            assert_eq!(exit["signal"], "killed");
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert_eq!(ping_processes("-n 61"), 0, "ツリーが残っていない");
        }

        #[tokio::test]
        async fn exit_arrives_even_when_a_grandchild_holds_the_pipe() {
            // cmd は ping を `start /b` で残して先に終わる。ping が stdout を握ったままでも exit は届き、
            // ping も回収される
            let reg = AppProcessRegistry::default();
            let started = Instant::now();
            let (_id, events) = run(&reg, "a", "echo hi&& start /b ping -n 63 127.0.0.1", None, 1 << 20).await;
            let exit = wait_exit(&events, 15).await.expect("孫がパイプを握っていても exit が届く");
            assert!(started.elapsed() < Duration::from_secs(15));
            assert!(exit["code"].is_number());
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert_eq!(ping_processes("-n 63"), 0, "孫が残っていない");
            assert!(lines_of(&events).contains(&"hi".to_string()));
        }

        #[tokio::test]
        async fn timeout_kills_the_tree_and_reports_it() {
            let reg = AppProcessRegistry::default();
            let (_id, events) = run(&reg, "a", "ping -n 65 127.0.0.1 >nul", Some(Duration::from_millis(800)), 1 << 20).await;
            let exit = wait_exit(&events, 10).await.expect("timeout で exit が届く");
            assert_eq!(exit["signal"], "timeout");
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert_eq!(ping_processes("-n 65"), 0);
        }

        #[tokio::test]
        async fn output_limit_kills_the_tree() {
            let reg = AppProcessRegistry::default();
            let (id, events) = run(&reg, "a", "for /l %i in (1,1,200000) do @echo line%i", None, 2048).await;
            let exit = wait_exit(&events, 20).await.expect("上限超過で exit が届く");
            assert_eq!(exit["signal"], "output-limit");
            // 上限を超えた分は捨てる（リングの総量は上限付近で止まる）
            let r = reg.read("a", &id, 0).unwrap();
            assert!(r.lines.iter().map(|l| l.text.len()).sum::<usize>() < 8192);
        }

        #[tokio::test]
        async fn carriage_return_progress_arrives_as_lines() {
            let reg = AppProcessRegistry::default();
            // `\r` だけで更新する出力（改行なし）。powershell を使わず cmd の set /p で `\r` を出すのは難しいので、
            // 行分割のテスト（splits_on_lf_cr_and_crlf）に加えて、cmd の出力が行として届くことだけを見る
            let (_id, events) = run(&reg, "a", "echo 10%&& echo 20%", None, 1 << 20).await;
            wait_exit(&events, 10).await.expect("exit");
            assert_eq!(lines_of(&events), vec!["10%", "20%"]);
        }

        #[tokio::test]
        async fn stop_of_the_instance_kills_running_processes_and_blocks_new_ones() {
            let reg = AppProcessRegistry::default();
            let (_id, events) = run(&reg, "a", "ping -n 67 127.0.0.1 >nul", None, 1 << 20).await;
            tokio::time::sleep(Duration::from_millis(600)).await;
            for k in reg.abort_all("a", true) {
                k.kill();
            }
            // エントリは外れるので exit の push は無いが、プロセスは回収される
            tokio::time::sleep(Duration::from_millis(1500)).await;
            assert_eq!(ping_processes("-n 67"), 0, "停止でツリーが残らない");
            assert!(reg.begin("a", "c", &spec(1)).is_err());
            drop(events);
        }
    }
}
