//! アプリ形式アーティファクト向けのファイルシステム API と、パス解決の基盤。
//!
//! # パススキーム
//!
//! アプリが指定できるのは `worktree://<相対>`（インスタンスの置き場所のワークツリー）と
//! `artifact://<相対>`（インスタンス同梱領域 `app_data/artifact-files/<worktreeId>/<artifactId>/`）だけ。
//! 生の絶対パスは受け付けない。DB（#374）や wasm も、ファイルの場所は [`resolve_scoped_path`] を通す。
//!
//! # 関門
//!
//! 見るのは host 側に保存した grant の [`ResolvedFs`]（承認時に解決した絶対パス）だけ。
//! アーティファクト JSON 内の manifest は見ない（AI が書き換えられるため）。
//!
//! # パス検査
//!
//! 1. 相対部を字句的に検査する（`..`・`:`・予約デバイス名・末尾のドット/空白など）。
//! 2. ルートに結合し、**存在する最も深い祖先を `canonicalize`** する（junction・symlink は解決後の
//!    実パスで判定される）。`symlink_metadata` で在るのに `canonicalize` できないもの（壊れたリンク）は拒否。
//! 3. 実パスが、承認されたスコープ（Read は `write: false`、Write は `write: true`）の配下か。
//! 4. `worktree://` への Write は、既定で [`SENSITIVE_NAMES`] を含むパスを拒否する。
//!
//! 検査と I/O の間の競合（TOCTOU）と、ハードリンクは防げない。これは境界ではなくソフトガードで、
//! アプリのコードがファイルシステムへ直接アクセスできないこと（iframe のサンドボックス）が本体。

use crate::app_manifest::{RequestedFs, ResolvedFs, ResolvedFsScope};
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// 一時ファイル名の接尾辞。同じパスへの並行書き込み（`Promise.all` や並列の `artifact_file put`）で
/// 衝突しないよう、プロセス内連番を混ぜる。
pub fn unique_tmp_suffix() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("oretachi-{}-{}", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// 1 回の read / write の上限。
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// 1 回の list が返す件数の上限。
pub const MAX_LIST_ENTRIES: usize = 1000;
/// `artifact_file put` で同梱できる 1 ファイルの上限（wasm や初期 DB を想定）。
pub const MAX_BUNDLE_FILE_BYTES: u64 = 32 * 1024 * 1024;

const MAX_COMPONENTS: usize = 64;
const MAX_COMPONENT_CHARS: usize = 255;

/// `worktree://` への書き込みを既定で拒否する名前（**任意の階層**の成分。大文字小文字は無視）。
///
/// 書き込みが次の実行を引き起こす経路になる場所:
/// `.git`（hooks・config。ワークツリーでは `.git` はファイルのこともある）、`.claude`（settings の hooks）、
/// `.vscode` / `.idea`（tasks）、`.github`（workflows・composite action）、`.husky`、`.devcontainer`、
/// `.mcp.json`（MCP サーバの起動コマンド）、`.envrc`（direnv）、`CLAUDE.md` / `CLAUDE.local.md` /
/// `AGENTS.md`（ユーザー権限で動くエージェントへの指示）。
pub const SENSITIVE_NAMES: &[&str] = &[
    ".git",
    ".github",
    ".claude",
    ".mcp.json",
    ".vscode",
    ".idea",
    ".husky",
    ".devcontainer",
    ".envrc",
    // oretachi が起動するエージェント・ツールの設定と、フック・タスクに準ずるもの
    ".codex",
    ".gemini",
    ".cursor",
    ".githooks",
    ".gitmodules",
    ".cargo",
    ".npmrc",
    ".yarnrc.yml",
    ".pre-commit-config.yaml",
    "CLAUDE.md",
    "CLAUDE.local.md",
    "AGENTS.md",
];

pub fn is_sensitive_name(name: &str) -> bool {
    SENSITIVE_NAMES.iter().any(|s| s.eq_ignore_ascii_case(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Worktree,
    Artifact,
}

impl Scheme {
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Worktree => "worktree",
            Scheme::Artifact => "artifact",
        }
    }

    fn uri_prefix(self) -> &'static str {
        match self {
            Scheme::Worktree => "worktree://",
            Scheme::Artifact => "artifact://",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

// ─── 字句的な検査（I/O なし） ─────────────────────────────────────────

/// Windows が予約しているデバイス名か（拡張子付き `CON.txt` も同じ）。`stem` は最初の `.` より前。
fn is_reserved_device(stem: &str) -> bool {
    let up = stem.trim_end_matches(' ').to_uppercase();
    if matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let mut chars = up.chars();
    let head: String = chars.by_ref().take(3).collect();
    if head == "COM" || head == "LPT" {
        // COM1〜9 に加え、上付き数字 ¹²³ もデバイスとして扱われる
        if let (Some(d), None) = (chars.next(), chars.next()) {
            return d.is_ascii_digit() || matches!(d, '¹' | '²' | '³');
        }
    }
    false
}

fn validate_component(c: &str) -> Result<(), String> {
    if c.is_empty() {
        return Err("空のパス成分があります".to_string());
    }
    if c == "." || c == ".." {
        return Err(format!("'{}' は使えません（相対移動は許可しません）", c));
    }
    if c.chars().count() > MAX_COMPONENT_CHARS {
        return Err(format!("パス成分が長すぎます（{} 文字まで）", MAX_COMPONENT_CHARS));
    }
    if let Some(bad) = c.chars().find(|ch| (*ch as u32) < 0x20 || *ch == '\u{7f}' || ":*?\"<>|".contains(*ch)) {
        return Err(format!("パス成分 '{}' に使えない文字 {:?} が含まれます", c, bad));
    }
    // Windows は末尾のドット・空白を黙って剥がす（`.git.` が `.git` になる）
    if c.ends_with('.') || c.ends_with(' ') {
        return Err(format!("パス成分 '{}' は末尾がドットまたは空白です", c));
    }
    if is_reserved_device(c.split('.').next().unwrap_or(c)) {
        return Err(format!("パス成分 '{}' は予約されたデバイス名です", c));
    }
    Ok(())
}

/// スキーム以降の相対部を成分へ分解して検査する。空文字はルート自身（成分なし）。
/// 区切りは `/` と `\`。末尾の区切りは許す。先頭の区切り・連続する区切りは拒否する。
pub fn validate_relative(rel: &str) -> Result<Vec<String>, String> {
    if rel.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts: Vec<&str> = rel.split(['/', '\\']).collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.len() > MAX_COMPONENTS {
        return Err(format!("パスが深すぎます（{} 階層まで）", MAX_COMPONENTS));
    }
    for p in &parts {
        validate_component(p)?;
    }
    Ok(parts.into_iter().map(str::to_string).collect())
}

/// `worktree://<相対>` / `artifact://<相対>` を分解する。それ以外（生の絶対パス・UNC・未知のスキーム）は拒否。
pub fn split_uri(uri: &str) -> Result<(Scheme, Vec<String>), String> {
    let (scheme, rest) = if let Some(r) = uri.strip_prefix(Scheme::Worktree.uri_prefix()) {
        (Scheme::Worktree, r)
    } else if let Some(r) = uri.strip_prefix(Scheme::Artifact.uri_prefix()) {
        (Scheme::Artifact, r)
    } else {
        return Err(format!(
            "パス '{}' は worktree://<相対> または artifact://<相対> の形で指定してください（絶対パスは受け付けません）",
            uri
        ));
    };
    let comps = validate_relative(rest).map_err(|e| format!("パス '{}' が不正です: {}", uri, e))?;
    Ok((scheme, comps))
}

fn prefix_of(comps: &[String]) -> String {
    if comps.is_empty() {
        String::new()
    } else {
        format!("{}/", comps.join("/"))
    }
}

/// manifest のスコープ 1 件を正規化する（`worktree://src\out` → `worktree://src/out/`）。
pub fn normalize_scope(s: &str) -> Result<String, String> {
    let (scheme, comps) = split_uri(s.trim())?;
    Ok(format!("{}{}", scheme.uri_prefix(), prefix_of(&comps)))
}

/// 正規化済みのスコープが、既定拒否パスの成分を含む `worktree://` か。
pub fn scope_hits_sensitive(scope: &str) -> bool {
    match split_uri(scope) {
        Ok((Scheme::Worktree, comps)) => comps.iter().any(|c| is_sensitive_name(c)),
        _ => false,
    }
}

// ─── 承認時の解決 ─────────────────────────────────────────────────────

/// ID をディレクトリ名に使えるか。`validate_path_component`（`..` と区切り文字）に加えて、
/// ディレクトリ名として**自分自身や別の ID と同じ場所を指してしまうもの**を拒否する:
/// `.`（`artifact-files/<wt>/.` は `<wt>` 自身になり、他アーティファクトの領域の親になる）、
/// 末尾のドット・空白（Windows が剥がすので `foo.` が `foo` と同じ領域になる）、予約デバイス名。
fn validate_storage_id(id: &str) -> Result<(), String> {
    crate::validate_path_component(id)?;
    validate_component(id).map_err(|e| format!("ID '{}' は同梱領域の名前に使えません: {}", id, e))
}

/// `artifact://` の置き場所。`validate_path_component` で両 ID を検査する。
pub fn artifact_files_dir(data_dir: &Path, worktree_id: &str, artifact_id: &str) -> Result<PathBuf, String> {
    validate_storage_id(worktree_id)?;
    validate_storage_id(artifact_id)?;
    Ok(data_dir.join("artifact-files").join(worktree_id).join(artifact_id))
}

/// manifest の `fs` を、承認時の形（解決済みの絶対パス）へ解決する。
///
/// `worktree_path` は**インスタンスの置き場所のワークツリー**の `path`。ホーム / リポジトリ擬似
/// ワークツリーは全ワークツリーの祖先・メインクローンになりうるので、呼び出し側が `None` を渡して拒否させる。
pub fn resolve_requested_fs(
    requested: &RequestedFs,
    worktree_path: Option<&Path>,
    artifact_dir: &Path,
) -> Result<ResolvedFs, String> {
    let mut entries: Vec<(Scheme, Vec<String>, bool)> = Vec::new();
    for (list, write) in [(&requested.read, false), (&requested.write, true)] {
        for s in list {
            let (scheme, comps) = split_uri(s)?;
            entries.push((scheme, comps, write));
        }
    }
    let uses = |sc: Scheme| entries.iter().any(|(s, _, _)| *s == sc);

    let worktree_root = if uses(Scheme::Worktree) {
        let p = worktree_path.ok_or_else(|| {
            "worktree:// はこのワークツリー（ホーム / リポジトリ）では使えません。通常のワークツリーに置いたアプリだけが申請できます".to_string()
        })?;
        Some(std::fs::canonicalize(p).map_err(|e| format!("ワークツリー '{}' を解決できません: {}", p.display(), e))?)
    } else {
        None
    };
    let artifact_root = if uses(Scheme::Artifact) {
        std::fs::create_dir_all(artifact_dir)
            .map_err(|e| format!("artifact:// の領域を作れません: {}", e))?;
        Some(
            std::fs::canonicalize(artifact_dir)
                .map_err(|e| format!("artifact:// の領域を解決できません: {}", e))?,
        )
    } else {
        None
    };

    let scopes = entries
        .into_iter()
        .map(|(scheme, comps, write)| {
            let root = match scheme {
                Scheme::Worktree => worktree_root.clone(),
                Scheme::Artifact => artifact_root.clone(),
            }
            .expect("root is resolved for every used scheme");
            let mut lexical = root.clone();
            for c in &comps {
                lexical.push(c);
            }
            // スコープも実パスへ解決して保存する。字句結合のままだと、ディスク上の綴り（大文字小文字）や
            // ディレクトリ内のリンクの先と食い違い、ダイアログに出した絶対パスと実際に許される範囲がずれる
            let path = canonicalize_allow_missing_tail(&lexical)
                .map_err(|e| format!("{}://{} を解決できません: {}", scheme.name(), prefix_of(&comps), e))?;
            if !path.starts_with(&root) {
                return Err(format!(
                    "{}://{} は {}:// の外を指しています（リンクの先がルートの外です）",
                    scheme.name(),
                    prefix_of(&comps),
                    scheme.name()
                ));
            }
            Ok(ResolvedFsScope {
                scheme: scheme.name().to_string(),
                prefix: prefix_of(&comps),
                path: path.to_string_lossy().into_owned(),
                write,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(ResolvedFs {
        worktree_root: worktree_root.map(|p| p.to_string_lossy().into_owned()),
        artifact_root: artifact_root.map(|p| p.to_string_lossy().into_owned()),
        scopes,
        allow_sensitive_paths: requested.allow_sensitive_paths,
    })
}

// ─── 呼び出し時のパス解決 ─────────────────────────────────────────────

/// 存在する最も深い祖先を `canonicalize` し、まだ無い成分を後ろへ足す。
///
/// 存在判定は `symlink_metadata`（リンクを辿らない）。在るのに `canonicalize` できないもの
/// （壊れた symlink / junction）は、まだ無いファイルとして扱わず**拒否**する。字句結合に落とすと、
/// 後でリンクが辿られて範囲外へ出る。
fn canonicalize_allow_missing_tail(path: &Path) -> Result<PathBuf, String> {
    let mut tail: Vec<OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&cur) {
            Ok(_) => {
                let mut base = std::fs::canonicalize(&cur).map_err(|e| {
                    format!("パスを解決できません（壊れたリンクの可能性があります）: {}", e)
                })?;
                for t in tail.iter().rev() {
                    base.push(t);
                }
                return Ok(base);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = cur
                    .file_name()
                    .ok_or_else(|| "パスを解決できません（存在する祖先がありません）".to_string())?
                    .to_owned();
                tail.push(name);
                if !cur.pop() {
                    return Err("パスを解決できません（存在する祖先がありません）".to_string());
                }
            }
            Err(e) => return Err(format!("パスを確認できません: {}", e)),
        }
    }
}

/// `uri` を、承認されたスコープの範囲内の**実パス**へ解決する。範囲外・不正は `Err`。
///
/// 返すパスは `canonicalize` 済みの形で、呼び出し側はこれに対して I/O する（生の結合パスを使わない）。
pub(crate) fn resolve_scoped_path(fs: &ResolvedFs, uri: &str, access: Access) -> Result<PathBuf, String> {
    let (scheme, comps) = split_uri(uri)?;
    let root = match scheme {
        Scheme::Worktree => fs.worktree_root.as_deref(),
        Scheme::Artifact => fs.artifact_root.as_deref(),
    }
    .ok_or_else(|| format!("このアプリには {}// の権限が承認されていません", scheme.name()))?;
    let root = Path::new(root);

    // ルートが承認時から動いていない（移動・削除・差し替えされていない）こと。
    // 無いルートの下へ書くと、ワークツリーのディレクトリごと作り直してしまう
    let live_root = std::fs::canonicalize(root)
        .map_err(|e| format!("{}// のルートを解決できません（移動または削除された可能性があります）: {}", scheme.name(), e))?;
    if live_root != root {
        return Err(format!(
            "{}// のルートが承認時から変わっています。アプリを再承認してください",
            scheme.name()
        ));
    }

    let mut lexical = root.to_path_buf();
    for c in &comps {
        lexical.push(c);
    }
    let canonical = canonicalize_allow_missing_tail(&lexical)?;

    let want_write = access == Access::Write;
    let in_scope = fs
        .scopes
        .iter()
        .any(|s| s.scheme == scheme.name() && s.write == want_write && canonical.starts_with(&s.path));
    if !in_scope {
        return Err(format!(
            "'{}' は承認された{}範囲の外です",
            uri,
            if want_write { "書き込み" } else { "読み取り" }
        ));
    }

    if want_write && scheme == Scheme::Worktree && !fs.allow_sensitive_paths {
        let rel = canonical
            .strip_prefix(root)
            .map_err(|_| format!("'{}' は承認された書き込み範囲の外です", uri))?;
        if let Some(hit) = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .find(|c| is_sensitive_name(c))
        {
            return Err(format!(
                "'{}' は既定で書き込みを拒否するパス（{}）を含みます。manifest の fs.allowSensitivePaths を承認された場合だけ書けます",
                uri, hit
            ));
        }
    }
    Ok(canonical)
}

// ─── I/O（検査済みの実パスに対して行う） ───────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Base64,
}

pub fn parse_encoding(s: &str) -> Result<Encoding, String> {
    match s {
        "utf8" => Ok(Encoding::Utf8),
        "base64" => Ok(Encoding::Base64),
        other => Err(format!("encoding は utf8 / base64 のどちらかです（指定: {}）", other)),
    }
}

pub fn read_file(path: &Path, enc: Encoding) -> Result<String, String> {
    use base64::Engine;
    let meta = std::fs::metadata(path).map_err(|e| format!("読み取れません: {}", e))?;
    if !meta.is_file() {
        return Err("ファイルではありません".to_string());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("ファイルが大きすぎます（{} バイト。上限 {}）", meta.len(), MAX_FILE_BYTES));
    }
    // 検査のあとに大きくなっても上限を超えて読まない
    let mut bytes = Vec::new();
    {
        use std::io::Read;
        let f = std::fs::File::open(path).map_err(|e| format!("読み取れません: {}", e))?;
        f.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("読み取れません: {}", e))?;
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("ファイルが大きすぎます（上限 {}）", MAX_FILE_BYTES));
    }
    match enc {
        Encoding::Utf8 => String::from_utf8(bytes)
            .map_err(|_| "UTF-8 として読めません（バイナリは encoding: \"base64\" で読んでください）".to_string()),
        Encoding::Base64 => Ok(base64::engine::general_purpose::STANDARD.encode(bytes)),
    }
}

pub fn write_file(path: &Path, data: &str, enc: Encoding, create_dirs: bool) -> Result<u64, String> {
    use base64::Engine;
    let bytes = match enc {
        Encoding::Utf8 => data.as_bytes().to_vec(),
        Encoding::Base64 => {
            if data.len() as u64 > MAX_FILE_BYTES / 3 * 4 + 8 {
                return Err(format!("データが大きすぎます（上限 {} バイト）", MAX_FILE_BYTES));
            }
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("base64 を解釈できません: {}", e))?
        }
    };
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("データが大きすぎます（{} バイト。上限 {}）", bytes.len(), MAX_FILE_BYTES));
    }
    if let Ok(m) = std::fs::symlink_metadata(path) {
        if m.is_dir() {
            return Err("ディレクトリには書き込めません".to_string());
        }
    }
    let parent = path.parent().ok_or_else(|| "親ディレクトリがありません".to_string())?;
    if create_dirs {
        std::fs::create_dir_all(parent).map_err(|e| format!("ディレクトリを作れません: {}", e))?;
    } else if !parent.is_dir() {
        return Err("親ディレクトリがありません（createDirs: true で作成できます）".to_string());
    }
    // 途中で落ちても壊れたファイルを残さない: 同じディレクトリの一時ファイルへ書いてから置き換える
    let name = path.file_name().ok_or_else(|| "ファイル名がありません".to_string())?;
    let tmp = parent.join(format!(".{}.{}.tmp", name.to_string_lossy(), unique_tmp_suffix()));
    std::fs::write(&tmp, &bytes).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("書き込めません: {}", e)
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("書き込めません: {}", e)
    })?;
    Ok(bytes.len() as u64)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListEntry {
    pub name: String,
    /// "file" | "dir" | "other"（symlink を含む）
    pub kind: &'static str,
    pub size: u64,
    pub modified_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListResult {
    pub entries: Vec<ListEntry>,
    /// [`MAX_LIST_ENTRIES`] を超えて打ち切ったか
    pub truncated: bool,
}

pub fn list_dir(path: &Path) -> Result<ListResult, String> {
    let rd = std::fs::read_dir(path).map_err(|e| format!("一覧できません: {}", e))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for ent in rd {
        let ent = ent.map_err(|e| format!("一覧できません: {}", e))?;
        if entries.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
        // リンクは辿らない（kind は "other"）
        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let meta = ent.metadata().ok();
        entries.push(ListEntry {
            name: ent.file_name().to_string_lossy().into_owned(),
            kind: if ft.is_symlink() {
                "other"
            } else if ft.is_dir() {
                "dir"
            } else if ft.is_file() {
                "file"
            } else {
                "other"
            },
            size: if ft.is_file() { meta.as_ref().map(|m| m.len()).unwrap_or(0) } else { 0 },
            modified_ms: meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ListResult { entries, truncated })
}

pub fn remove_file(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("削除できません: {}", e))?;
    if meta.is_dir() {
        return Err("ディレクトリは削除できません（ファイルだけ）".to_string());
    }
    std::fs::remove_file(path).map_err(|e| format!("削除できません: {}", e))
}

// ─── artifact:// 領域の寿命 ───────────────────────────────────────────

/// アーティファクトの同梱領域を消す。無ければ何もしない。
pub fn remove_artifact_files(data_dir: &Path, worktree_id: &str, artifact_id: &str) -> Result<(), String> {
    remove_dir_if_exists(&artifact_files_dir(data_dir, worktree_id, artifact_id)?)
}

/// ワークツリー丸ごとの同梱領域を消す。
pub fn remove_worktree_artifact_files(data_dir: &Path, worktree_id: &str) -> Result<(), String> {
    crate::validate_path_component(worktree_id)?;
    remove_dir_if_exists(&data_dir.join("artifact-files").join(worktree_id))
}

fn remove_dir_if_exists(dir: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("'{}' を削除できません: {}", dir.display(), e)),
    }
}

// ─── Tauri コマンド（appinst-* ウィンドウからだけ） ────────────────────

/// 共通の前置き: 呼び出し元ウィンドウ → インスタンス → 保存済み grant の fs。
fn caller_fs(
    window: &tauri::Window,
    app: &AppHandle,
) -> Result<(crate::app_instance::InstanceContext, ResolvedFs), String> {
    crate::app_instance::require_instance_window(window)?;
    let ctx = crate::app_instance::context_for_label(app, window.label())?;
    let grant = app
        .state::<crate::app_grant::AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .ok_or_else(|| "このアプリには承認がありません".to_string())?;
    let fs = grant
        .resolved
        .fs
        .ok_or_else(|| "このアプリにはファイルシステムの権限が承認されていません".to_string())?;
    Ok((ctx, fs))
}

/// 拒否・失敗はデバッグログ（アプリのログとアプリのログファイル）へ残す。成功は変更系だけ残す。
fn log_outcome<T>(
    app: &AppHandle,
    ctx: &crate::app_instance::InstanceContext,
    op: &str,
    path: &str,
    result: &Result<T, String>,
    log_success: bool,
) {
    match result {
        Ok(_) if log_success => {
            crate::app_instance::add_log(app, &ctx.worktree_id, &ctx.artifact_id, "info", format!("fs.{}: {}", op, path));
        }
        Ok(_) => {}
        Err(e) => {
            // 再承認待ちの間に新しい権限が要る呼び出しをしたときは、理由が分かるよう付記する
            let note = if ctx.reapproval_pending {
                "（再承認待ちのため、新しい権限はまだ使えません）"
            } else {
                ""
            };
            log::debug!("[app-fs] 拒否 {} {}/{} {} — {}", op, ctx.worktree_id, ctx.artifact_id, path, e);
            crate::app_instance::add_log(
                app,
                &ctx.worktree_id,
                &ctx.artifact_id,
                "warn",
                format!("fs.{} を拒否/失敗: {} — {}{}", op, path, e, note),
            );
        }
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("task join error: {}", e))?
}

#[tauri::command]
pub async fn app_fs_read(
    window: tauri::Window,
    app: AppHandle,
    path: String,
    encoding: String,
) -> Result<String, String> {
    let (ctx, fs) = caller_fs(&window, &app)?;
    let p = path.clone();
    let result = blocking(move || {
        let enc = parse_encoding(&encoding)?;
        let real = resolve_scoped_path(&fs, &p, Access::Read)?;
        read_file(&real, enc)
    })
    .await;
    log_outcome(&app, &ctx, "read", &path, &result, false);
    result
}

#[tauri::command]
pub async fn app_fs_write(
    window: tauri::Window,
    app: AppHandle,
    path: String,
    data: String,
    encoding: String,
    create_dirs: bool,
) -> Result<u64, String> {
    let (ctx, fs) = caller_fs(&window, &app)?;
    let p = path.clone();
    let result = blocking(move || {
        let enc = parse_encoding(&encoding)?;
        let real = resolve_scoped_path(&fs, &p, Access::Write)?;
        write_file(&real, &data, enc, create_dirs)
    })
    .await;
    log_outcome(&app, &ctx, "write", &path, &result, true);
    result
}

#[tauri::command]
pub async fn app_fs_list(window: tauri::Window, app: AppHandle, path: String) -> Result<ListResult, String> {
    let (ctx, fs) = caller_fs(&window, &app)?;
    let p = path.clone();
    let result = blocking(move || {
        let real = resolve_scoped_path(&fs, &p, Access::Read)?;
        list_dir(&real)
    })
    .await;
    log_outcome(&app, &ctx, "list", &path, &result, false);
    result
}

#[tauri::command]
pub async fn app_fs_remove(window: tauri::Window, app: AppHandle, path: String) -> Result<(), String> {
    let (ctx, fs) = caller_fs(&window, &app)?;
    let p = path.clone();
    let result = blocking(move || {
        let real = resolve_scoped_path(&fs, &p, Access::Write)?;
        remove_file(&real)
    })
    .await;
    log_outcome(&app, &ctx, "remove", &path, &result, true);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oretachi-appfs-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn requested(read: &[&str], write: &[&str], allow: bool) -> RequestedFs {
        RequestedFs {
            read: read.iter().map(|s| normalize_scope(s).unwrap()).collect(),
            write: write.iter().map(|s| normalize_scope(s).unwrap()).collect(),
            allow_sensitive_paths: allow,
        }
    }

    /// `<tmp>/wt`（ワークツリー）と `<tmp>/art`（同梱領域）を用意して解決する。
    fn setup(tag: &str, read: &[&str], write: &[&str], allow: bool) -> (PathBuf, ResolvedFs) {
        let base = temp_dir(tag);
        let wt = base.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let fs = resolve_requested_fs(&requested(read, write, allow), Some(&wt), &base.join("art")).unwrap();
        (base, fs)
    }

    fn denied(r: Result<PathBuf, String>, needle: &str) {
        let e = r.expect_err("denied");
        assert!(e.contains(needle), "期待するメッセージ '{}' を含まない: {}", needle, e);
    }

    // ── 字句 ──

    #[test]
    fn uri_requires_known_scheme() {
        assert!(split_uri("worktree://a/b").is_ok());
        assert!(split_uri("artifact://a").is_ok());
        for bad in [
            "a/b",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "C:/x",
            "\\\\server\\share\\x",
            "\\\\?\\C:\\x",
            "file://x",
            "http://x",
            "WORKTREE://x",
            "",
        ] {
            assert!(split_uri(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn relative_part_rejects_escapes_and_windows_tricks() {
        for bad in [
            "..",
            "a/../b",
            "a\\..\\b",
            "./a",
            "a/./b",
            "/abs",
            "\\abs",
            "a//b",
            "a:b",
            "C:",
            "a.txt:stream",
            "name.",
            "name ",
            ".git.",
            "a/b*",
            "q?",
            "x|y",
            "CON",
            "con.txt",
            "a/NUL",
            "Nul.tar.gz",
            "COM1",
            "lpt9.log",
            "COM¹",
            "LPT³",
            "CONIN$",
            "CONOUT$",
            "bad\u{0}nul",
            "tab\there",
        ] {
            assert!(validate_relative(bad).is_err(), "{bad:?}");
        }
        for ok in ["a", "a/b", "a\\b/c", "dir/", "src/main.rs", "comet", "COM10", "console.log", ".hidden", "a b"] {
            assert!(validate_relative(ok).is_ok(), "{ok:?}");
        }
        assert_eq!(validate_relative("").unwrap(), Vec::<String>::new());
        assert_eq!(validate_relative("a\\b/c/").unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn scope_normalization() {
        assert_eq!(normalize_scope(" worktree://src\\out ").unwrap(), "worktree://src/out/");
        assert_eq!(normalize_scope("worktree://").unwrap(), "worktree://");
        assert_eq!(normalize_scope("artifact://data/").unwrap(), "artifact://data/");
        assert!(normalize_scope("src/out").is_err());
    }

    #[test]
    fn sensitive_scope_detection() {
        assert!(scope_hits_sensitive("worktree://.git/hooks/"));
        assert!(scope_hits_sensitive("worktree://sub/.GitHub/workflows/"));
        assert!(!scope_hits_sensitive("worktree://src/"));
        assert!(!scope_hits_sensitive("worktree://"));
        // artifact:// は app_data 配下で、ワークツリーのような実行経路を持たない
        assert!(!scope_hits_sensitive("artifact://.git/"));
    }

    #[test]
    fn sensitive_names_are_pinned() {
        // 一覧を変えるときはこのテストと SKILL.md を一緒に直す
        let mut names: Vec<&str> = SENSITIVE_NAMES.to_vec();
        names.sort();
        let mut expected = vec![
            ".claude", ".devcontainer", ".envrc", ".git", ".github", ".husky", ".idea", ".mcp.json",
            ".vscode", "AGENTS.md", "CLAUDE.local.md", "CLAUDE.md", ".codex", ".gemini", ".cursor",
            ".githooks", ".gitmodules", ".cargo", ".npmrc", ".yarnrc.yml", ".pre-commit-config.yaml",
        ];
        expected.sort();
        assert_eq!(names, expected);
    }

    // ── 解決 ──

    #[test]
    fn resolve_requires_a_normal_worktree_for_worktree_scheme() {
        let base = temp_dir("pseudo");
        let r = requested(&["worktree://"], &[], false);
        let e = resolve_requested_fs(&r, None, &base.join("art")).unwrap_err();
        assert!(e.contains("worktree://"));
        // artifact:// だけならワークツリー無しでも解決できる
        let r = requested(&["artifact://"], &[], false);
        assert!(resolve_requested_fs(&r, None, &base.join("art")).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn scopes_are_stored_as_real_paths() {
        // 申請時の綴りではなく、ディスク上の綴りで保存する（ダイアログの絶対パスと実際の範囲を一致させる）
        let base = temp_dir("realpath");
        let wt = base.join("wt");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        let fs = resolve_requested_fs(&requested(&["worktree://SRC/"], &[], false), Some(&wt), &base.join("art")).unwrap();
        let real = std::fs::canonicalize(wt.join("src")).unwrap();
        assert_eq!(Path::new(&fs.scopes[0].path), real.as_path());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn scope_through_a_link_that_leaves_the_root_is_rejected_at_approval() {
        let base = temp_dir("scope-link");
        let wt = base.join("wt");
        let outside = base.join("outside");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        if !make_dir_link(&wt.join("escape"), &outside) || std::fs::canonicalize(wt.join("escape")).is_err() {
            eprintln!("SKIP: リンクを作れない / 辿れない環境");
            return;
        }
        let e = resolve_requested_fs(&requested(&["worktree://escape/"], &[], false), Some(&wt), &base.join("art")).unwrap_err();
        assert!(e.contains("外を指しています"), "{e}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn tmp_suffixes_do_not_collide() {
        assert_ne!(unique_tmp_suffix(), unique_tmp_suffix());
    }

    #[test]
    fn grant_json_roundtrip_keeps_resolved_paths() {
        let (base, fs) = setup("json", &["worktree://src/"], &["artifact://data/"], false);
        let s = serde_json::to_string(&fs).unwrap();
        assert!(s.contains("worktreeRoot") && s.contains("allowSensitivePaths"));
        let back: ResolvedFs = serde_json::from_str(&s).unwrap();
        assert_eq!(back, fs);
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── 範囲内 / 範囲外 ──

    #[test]
    fn in_scope_read_write_list_remove() {
        let (base, fs) = setup("basic", &["worktree://src/", "artifact://"], &["worktree://out/", "artifact://data/"], false);
        let wt = base.join("wt");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join("src/a.txt"), "hello").unwrap();

        let p = resolve_scoped_path(&fs, "worktree://src/a.txt", Access::Read).unwrap();
        assert_eq!(read_file(&p, Encoding::Utf8).unwrap(), "hello");
        assert_eq!(read_file(&p, Encoding::Base64).unwrap(), "aGVsbG8=");

        // 新規ファイル（親ディレクトリも無い）への書き込み
        let w = resolve_scoped_path(&fs, "worktree://out/deep/new.bin", Access::Write).unwrap();
        assert_eq!(write_file(&w, "AAEC", Encoding::Base64, true).unwrap(), 3);
        assert_eq!(std::fs::read(wt.join("out/deep/new.bin")).unwrap(), vec![0, 1, 2]);
        assert!(write_file(&w, "x", Encoding::Utf8, false).is_ok(), "親があれば createDirs 無しでも書ける");

        // 親が無く createDirs も無ければ失敗
        let w2 = resolve_scoped_path(&fs, "worktree://out/none/x.txt", Access::Write).unwrap();
        assert!(write_file(&w2, "x", Encoding::Utf8, false).unwrap_err().contains("親ディレクトリ"));

        // 一覧（書き込み範囲は読めない。読み取りスコープ側で見る）
        let l = resolve_scoped_path(&fs, "worktree://src", Access::Read).unwrap();
        let list = list_dir(&l).unwrap();
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].name, "a.txt");
        assert_eq!(list.entries[0].kind, "file");
        assert_eq!(list.entries[0].size, 5);
        assert!(!list.truncated);

        // 削除
        let rm = resolve_scoped_path(&fs, "worktree://out/deep/new.bin", Access::Write).unwrap();
        remove_file(&rm).unwrap();
        assert!(!wt.join("out/deep/new.bin").exists());
        let dir = resolve_scoped_path(&fs, "worktree://out/deep", Access::Write).unwrap();
        assert!(remove_file(&dir).unwrap_err().contains("ディレクトリ"));
        assert!(write_file(&dir, "x", Encoding::Utf8, false).unwrap_err().contains("ディレクトリ"));

        // artifact:// も同じ
        let aw = resolve_scoped_path(&fs, "artifact://data/db.bin", Access::Write).unwrap();
        write_file(&aw, "abc", Encoding::Utf8, true).unwrap();
        let ar = resolve_scoped_path(&fs, "artifact://data/db.bin", Access::Read).unwrap();
        assert_eq!(read_file(&ar, Encoding::Utf8).unwrap(), "abc");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn out_of_scope_and_wrong_access_are_denied() {
        let (base, fs) = setup("scope", &["worktree://src/"], &["worktree://out/"], false);
        // 範囲外
        denied(resolve_scoped_path(&fs, "worktree://other/a.txt", Access::Read), "範囲の外");
        denied(resolve_scoped_path(&fs, "worktree://srcx/a.txt", Access::Read), "範囲の外");
        denied(resolve_scoped_path(&fs, "worktree://", Access::Read), "範囲の外");
        // write は read を含意しない / read は write を含意しない
        denied(resolve_scoped_path(&fs, "worktree://out/a.txt", Access::Read), "範囲の外");
        denied(resolve_scoped_path(&fs, "worktree://src/a.txt", Access::Write), "範囲の外");
        // 承認されていないスキーム
        denied(resolve_scoped_path(&fs, "artifact://a", Access::Read), "承認されていません");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn escapes_are_rejected_before_touching_the_disk() {
        let (base, fs) = setup("escape", &["worktree://"], &["worktree://"], false);
        std::fs::write(base.join("secret.txt"), "s").unwrap();
        for bad in [
            "worktree://../secret.txt",
            "worktree://a/../../secret.txt",
            "worktree://..\\secret.txt",
            "worktree:///etc/passwd",
            "worktree://C:/Windows/win.ini",
            "worktree://\\\\server\\share\\x",
            "worktree://x:y",
            "worktree://CON",
            "worktree://NUL.txt",
            "C:\\x",
            "/etc/passwd",
            "\\\\?\\C:\\x",
            "\\\\.\\PHYSICALDRIVE0",
        ] {
            assert!(resolve_scoped_path(&fs, bad, Access::Read).is_err(), "{bad}");
            assert!(resolve_scoped_path(&fs, bad, Access::Write).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn moved_or_deleted_root_is_rejected() {
        let (base, fs) = setup("root", &["worktree://"], &["worktree://"], false);
        std::fs::remove_dir_all(base.join("wt")).unwrap();
        // 無いルートの下へ新規ファイルを書いて、ワークツリーのディレクトリごと作り直してはいけない
        denied(resolve_scoped_path(&fs, "worktree://new.txt", Access::Write), "ルート");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── 既定拒否パス ──

    #[test]
    fn sensitive_paths_are_denied_for_write_by_default() {
        let (base, fs) = setup("sens", &["worktree://"], &["worktree://"], false);
        let wt = base.join("wt");
        std::fs::create_dir_all(wt.join(".git/hooks")).unwrap();
        std::fs::write(wt.join(".git/config"), "[core]").unwrap();
        for p in [
            ".git/hooks/pre-commit",
            ".git/config",
            ".GIT/config",
            ".git",
            "sub/.git/hooks/x",
            "sub/.Git",
            ".github/workflows/ci.yml",
            ".github/actions/a/action.yml",
            ".claude/settings.json",
            ".claude/settings.local.json",
            ".vscode/tasks.json",
            ".idea/workspace.xml",
            ".husky/pre-commit",
            ".devcontainer/devcontainer.json",
            ".mcp.json",
            ".envrc",
            "CLAUDE.md",
            "claude.local.md",
            "pkg/AGENTS.md",
        ] {
            let uri = format!("worktree://{}", p);
            denied(resolve_scoped_path(&fs, &uri, Access::Write), "書き込みを拒否");
            // 読み取りは常に通る（存在しなくても解決はできる）
            assert!(resolve_scoped_path(&fs, &uri, Access::Read).is_ok(), "read {p}");
        }
        // 似た名前は拒否しない
        for p in [".gitignore", ".gitattributes", "git/x", "github/x", "claude.md.bak", ".claude-notes/x", "my.git/x"] {
            let uri = format!("worktree://{}", p);
            assert!(resolve_scoped_path(&fs, &uri, Access::Write).is_ok(), "write {p}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn allow_sensitive_paths_lifts_the_denylist_for_write() {
        let (base, fs) = setup("sens-ok", &[], &["worktree://"], true);
        for p in [".git/hooks/pre-commit", ".github/workflows/ci.yml", ".claude/settings.json", "CLAUDE.md"] {
            assert!(resolve_scoped_path(&fs, &format!("worktree://{}", p), Access::Write).is_ok(), "{p}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn artifact_scheme_has_no_sensitive_denylist() {
        let (base, fs) = setup("art-sens", &[], &["artifact://"], false);
        assert!(resolve_scoped_path(&fs, "artifact://.git/x", Access::Write).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sensitive_check_uses_the_resolved_path_not_the_requested_one() {
        // `link` → `.git` の junction / symlink 経由でも、解決後に .git が現れるので拒否される
        let (base, fs) = setup("sens-link", &["worktree://"], &["worktree://"], false);
        let wt = base.join("wt");
        std::fs::create_dir_all(wt.join(".git")).unwrap();
        if !make_dir_link(&wt.join("link"), &wt.join(".git")) {
            eprintln!("SKIP: ディレクトリのリンクを作れない環境");
            return;
        }
        if std::fs::canonicalize(wt.join("link")).is_err() {
            eprintln!("SKIP: リンクを辿れない環境（RedirectionGuard など）");
            return;
        }
        denied(resolve_scoped_path(&fs, "worktree://link/hooks/x", Access::Write), "書き込みを拒否");
        denied(resolve_scoped_path(&fs, "worktree://link/config", Access::Write), "書き込みを拒否");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── リンク ──

    /// `link` から `target`（ディレクトリ）へのリンクを作る。Windows は junction（管理者権限が要らない）。
    fn make_dir_link(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    /// リンクが実際に辿れるか（陽性対照）。辿れない環境では、拒否の検査が偶然通ってしまうので、
    /// 呼び出し側は明示的にスキップする。
    fn link_is_traversable(link: &Path, probe: &str) -> bool {
        link.join(probe).exists()
    }

    #[test]
    fn junction_or_symlink_escape_is_rejected_and_inside_link_is_allowed() {
        let (base, fs) = setup("link", &["worktree://"], &["worktree://"], false);
        let wt = base.join("wt");
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "s").unwrap();
        std::fs::create_dir_all(wt.join("real")).unwrap();
        std::fs::write(wt.join("real/ok.txt"), "ok").unwrap();

        let inside_link = wt.join("inside");
        let escape_link = wt.join("escape");
        if !make_dir_link(&inside_link, &wt.join("real")) || !make_dir_link(&escape_link, &outside) {
            eprintln!("SKIP: ディレクトリのリンクを作れない環境");
            return;
        }
        // 陽性対照: 範囲内を指すリンクは辿れて、通る
        if !link_is_traversable(&inside_link, "ok.txt") || !link_is_traversable(&escape_link, "secret.txt") {
            eprintln!("SKIP: リンクを辿れない環境（RedirectionGuard など）。拒否の検査が偶然通らないよう、ここで打ち切る");
            return;
        }
        let ok = resolve_scoped_path(&fs, "worktree://inside/ok.txt", Access::Read).unwrap();
        assert_eq!(read_file(&ok, Encoding::Utf8).unwrap(), "ok");
        let okw = resolve_scoped_path(&fs, "worktree://inside/new.txt", Access::Write).unwrap();
        write_file(&okw, "w", Encoding::Utf8, false).unwrap();
        assert!(wt.join("real/new.txt").exists());

        // 範囲外を指すリンク経由は、既存ファイル・新規ファイル・ディレクトリのどれも拒否
        denied(resolve_scoped_path(&fs, "worktree://escape/secret.txt", Access::Read), "範囲の外");
        denied(resolve_scoped_path(&fs, "worktree://escape/new.txt", Access::Write), "範囲の外");
        denied(resolve_scoped_path(&fs, "worktree://escape/deep/new.txt", Access::Write), "範囲の外");
        denied(resolve_scoped_path(&fs, "worktree://escape", Access::Read), "範囲の外");
        assert!(!outside.join("new.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dangling_link_is_rejected_at_leaf_and_in_the_middle() {
        let (base, fs) = setup("dangling", &["worktree://"], &["worktree://"], false);
        let wt = base.join("wt");
        let gone = base.join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        let link = wt.join("dangling");
        if !make_dir_link(&link, &gone) {
            eprintln!("SKIP: ディレクトリのリンクを作れない環境");
            return;
        }
        std::fs::remove_dir_all(&gone).unwrap();
        if std::fs::symlink_metadata(&link).is_err() {
            eprintln!("SKIP: 壊れたリンクを作れない環境");
            return;
        }
        if std::fs::canonicalize(&link).is_ok() {
            eprintln!("SKIP: 壊れたリンクにならない環境");
            return;
        }
        // 葉がそのもの / 中間ディレクトリ
        denied(resolve_scoped_path(&fs, "worktree://dangling", Access::Write), "壊れたリンク");
        denied(resolve_scoped_path(&fs, "worktree://dangling/x.txt", Access::Write), "壊れたリンク");
        denied(resolve_scoped_path(&fs, "worktree://dangling/a/b.txt", Access::Read), "壊れたリンク");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── 上限 ──

    #[test]
    fn size_and_count_limits() {
        let base = temp_dir("limits");
        let f = base.join("big.txt");
        let big = "x".repeat(MAX_FILE_BYTES as usize + 1);
        assert!(write_file(&f, &big, Encoding::Utf8, false).unwrap_err().contains("大きすぎ"));
        assert!(!f.exists());
        std::fs::write(&f, &big).unwrap();
        assert!(read_file(&f, Encoding::Utf8).unwrap_err().contains("大きすぎ"));
        let exact = "x".repeat(MAX_FILE_BYTES as usize);
        assert_eq!(write_file(&f, &exact, Encoding::Utf8, false).unwrap(), MAX_FILE_BYTES);
        // base64 でも上限を超えたらデコード前に拒否
        let huge_b64 = "A".repeat((MAX_FILE_BYTES as usize / 3 * 4) + 100);
        assert!(write_file(&base.join("b.bin"), &huge_b64, Encoding::Base64, false).unwrap_err().contains("大きすぎ"));
        assert!(write_file(&base.join("c.bin"), "!!!", Encoding::Base64, false).unwrap_err().contains("base64"));

        let many = base.join("many");
        std::fs::create_dir_all(&many).unwrap();
        for i in 0..(MAX_LIST_ENTRIES + 5) {
            std::fs::write(many.join(format!("f{:05}", i)), "").unwrap();
        }
        let l = list_dir(&many).unwrap();
        assert_eq!(l.entries.len(), MAX_LIST_ENTRIES);
        assert!(l.truncated);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn non_utf8_text_read_points_to_base64() {
        let base = temp_dir("utf8");
        let f = base.join("b.bin");
        std::fs::write(&f, [0xff, 0xfe, 0x00]).unwrap();
        assert!(read_file(&f, Encoding::Utf8).unwrap_err().contains("base64"));
        assert_eq!(read_file(&f, Encoding::Base64).unwrap(), "//4A");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_leaves_no_temp_file() {
        let base = temp_dir("tmp");
        let f = base.join("a.txt");
        write_file(&f, "1", Encoding::Utf8, false).unwrap();
        write_file(&f, "22", Encoding::Utf8, false).unwrap();
        let names: Vec<String> = list_dir(&base).unwrap().entries.into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["a.txt"]);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "22");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── artifact:// の寿命 ──

    #[test]
    fn artifact_files_dir_rejects_unsafe_ids() {
        let d = Path::new("data");
        assert!(artifact_files_dir(d, "wt", "a").is_ok());
        // `.` は `artifact-files/<wt>` 自身（全アーティファクトの領域の親）、`foo.` / `foo ` は Windows が剥がして
        // `foo` と同じ領域になる
        for bad in [".", "..", "a/b", "a\\b", "c:", "", "foo.", "foo ", "CON", "nul.txt"] {
            if bad.is_empty() {
                continue;
            }
            assert!(artifact_files_dir(d, bad, "a").is_err(), "{bad}");
            assert!(artifact_files_dir(d, "wt", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn removing_artifact_files_is_scoped_and_idempotent() {
        let base = temp_dir("remove-area");
        for (w, a) in [("wt1", "a1"), ("wt1", "a2"), ("wt10", "a1")] {
            let d = artifact_files_dir(&base, w, a).unwrap();
            std::fs::create_dir_all(d.join("sub")).unwrap();
            std::fs::write(d.join("sub/f.bin"), "x").unwrap();
        }
        remove_artifact_files(&base, "wt1", "a1").unwrap();
        remove_artifact_files(&base, "wt1", "a1").unwrap();
        assert!(!artifact_files_dir(&base, "wt1", "a1").unwrap().exists());
        assert!(artifact_files_dir(&base, "wt1", "a2").unwrap().exists());
        remove_worktree_artifact_files(&base, "wt1").unwrap();
        assert!(!artifact_files_dir(&base, "wt1", "a2").unwrap().exists());
        // 前方一致する別ワークツリーは消えない
        assert!(artifact_files_dir(&base, "wt10", "a1").unwrap().exists());
        remove_worktree_artifact_files(&base, "never-existed").unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }
}
