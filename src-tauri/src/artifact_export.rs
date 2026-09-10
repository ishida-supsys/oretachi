//! アーティファクトの zip エクスポート / インポート。
//!
//! 用途は「レポート（アーティファクト）の出力に不備があったとき、それを issue へ添付して
//! 報告する」こと。GitHub の issue 添付は `.html` を受け付けないため、レンダリング結果と
//! 再現材料をまとめて 1 つの zip にする。受け取った側は同じ zip をインポートすれば、
//! 別の oretachi 上に同じアーティファクトを復元できる。
//!
//! zip の中身:
//!
//! ```text
//! manifest.json   このファイルが何か（format / version / メタ）
//! artifact.json   本体（content + modules + メタ）= インポートで読む唯一の正
//! state.json      状態サイドカー（memory）。無いこともある
//! view.html       レンダリング結果（自己完結・外部通信なし）。無いこともある
//! content.<ext>   素のソース（.md / .csv / .jsx など）。人が読む用
//! modules/<name>  React アーティファクトのモジュール。人が読む用
//! README.txt      開き方
//! ```
//!
//! `artifact.json` 以外はすべて人が読むための添え物で、インポートは
//! `manifest.json` と `artifact.json` しか見ない。

use std::io::{Read, Write};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{artifact_scope_dir, artifact_state_path, validate_path_component};

/// zip が oretachi のアーティファクトであることを示す識別子（`manifest.json` の `format`）
const EXPORT_FORMAT: &str = "oretachi-artifact-export";
/// 書き出す manifest のバージョン。読む側は「これ以下なら読める」で判定する
const EXPORT_FORMAT_VERSION: u32 = 1;

/// 読み込む 1 エントリあたりの上限。壊れた zip や zip bomb でメモリを
/// 食い潰さないための保険（32MiB）
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportArtifactResult {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportArtifactResult {
    pub artifact_id: String,
    pub title: String,
    /// 既存 ID と衝突したため別 ID で取り込んだ場合の、zip に入っていた元の ID
    pub renamed_from: Option<String>,
}

/// content_type から「素のソース」の拡張子を決める。
pub fn source_extension(content_type: &str, language: Option<&str>) -> &'static str {
    match content_type {
        "text/markdown" => "md",
        "text/html" => "html",
        "image/svg+xml" => "svg",
        "application/vnd.ant.mermaid" => "mmd",
        "application/vnd.ant.react" => "jsx",
        "text/csv" => "csv",
        "text/tab-separated-values" => "tsv",
        "text/uri-list" => "uri",
        "application/vnd.ant.code" => match language {
            Some("typescript") | Some("ts") => "ts",
            Some("javascript") | Some("js") => "js",
            Some("python") | Some("py") => "py",
            Some("rust") | Some("rs") => "rs",
            Some("json") => "json",
            _ => "txt",
        },
        _ => "txt",
    }
}

/// zip 内のパスとして安全な名前へ均す。`..` やドライブレターを持ち込ませない
/// （受け取った側が展開したときにディレクトリの外へ書かせないため）。
fn sanitize_entry_name(name: &str) -> String {
    let mut out = String::new();
    for part in name.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." {
            continue;
        }
        let cleaned: String = part
            .chars()
            .map(|c| if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(&cleaned);
    }
    out
}

/// アーティファクト ID として使える文字だけを残す。空になったら `None`。
fn sanitize_artifact_id(id: &str) -> Option<String> {
    let cleaned: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() || validate_path_component(&trimmed).is_err() {
        None
    } else {
        Some(trimmed)
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn readme_text(title: &str, content_type: &str, has_view: bool) -> String {
    let view_line = if has_view {
        // text/html だけは外部画像 (https:) を読める CSP なので「外部通信ゼロ」とは書かない
        "view.html      レンダリング結果。ブラウザで開いてください（スクリプトからの外部通信はブロックされます）\n"
    } else {
        "               ※ この種類のアーティファクトは view.html を含みません（素のソースを見てください）\n"
    };
    format!(
        "oretachi アーティファクトのエクスポート\n\
         \n\
         タイトル: {title}\n\
         種類: {content_type}\n\
         \n\
         中身:\n\
         {view_line}\
         artifact.json  本体。oretachi のアーティファクトビューアからこの zip をインポートすると復元できます\n\
         state.json     メモリー（フォーム入力などの保存内容）。無い場合もあります\n\
         content.*      素のソース\n\
         modules/       React アーティファクトのモジュール（あれば）\n\
         \n\
         PDF が要る場合は view.html をブラウザで開いて印刷（Ctrl+P）してください。\n"
    )
}

/// アーティファクト 1 件を zip として書き出す。
///
/// `view_html` はフロント側が組み立てたレンダリング結果（React の srcdoc など）。
/// レンダリングはブラウザ側の資産（Babel / Tailwind / mermaid）に依存するので Rust では作らない。
#[tauri::command]
pub async fn export_artifact(
    app_handle: AppHandle,
    scope: String,
    scope_id: String,
    artifact_id: String,
    dest_path: String,
    view_html: Option<String>,
) -> Result<ExportArtifactResult, String> {
    validate_path_component(&artifact_id)?;
    let dir = artifact_scope_dir(&app_handle, &scope, &scope_id)?;
    let artifact_path = dir.join(format!("{}.json", artifact_id));
    let state_path = artifact_state_path(&dir, &artifact_id);
    let app_version = app_handle.package_info().version.to_string();

    tokio::task::spawn_blocking(move || -> Result<ExportArtifactResult, String> {
        let raw = std::fs::read_to_string(&artifact_path)
            .map_err(|e| format!("アーティファクトの読み込みに失敗しました: {}", e))?;
        let body: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| format!("アーティファクトの JSON 解析に失敗しました: {}", e))?;

        let title = body.get("title").and_then(|v| v.as_str()).unwrap_or(&artifact_id);
        let content_type = body
            .get("type")
            .or_else(|| body.get("content_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("text/plain");
        let language = body.get("language").and_then(|v| v.as_str());
        let content = body.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let manifest = serde_json::json!({
            "format": EXPORT_FORMAT,
            "version": EXPORT_FORMAT_VERSION,
            "exportedAt": now_millis(),
            "appVersion": app_version,
            "artifactId": artifact_id,
            "title": title,
            "contentType": content_type,
        });

        let file = std::fs::File::create(&dest_path)
            .map_err(|e| format!("エクスポート先を作成できませんでした: {}", e))?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let put = |zip: &mut zip::ZipWriter<std::io::BufWriter<std::fs::File>>,
                       name: &str,
                       body: &str|
         -> Result<(), String> {
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            zip.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
            Ok(())
        };

        put(
            &mut zip,
            "manifest.json",
            &serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )?;
        put(&mut zip, "artifact.json", &raw)?;
        if let Some(html) = view_html.as_deref() {
            put(&mut zip, "view.html", html)?;
        }
        put(
            &mut zip,
            &format!("content.{}", source_extension(content_type, language)),
            content,
        )?;
        if let Some(modules) = body.get("modules").and_then(|v| v.as_object()) {
            // `a:b` と `a_b` のように、別名が sanitize で同名へ潰れることがある。
            // 同じ名前で start_file すると zip 側がエラーになりエクスポート全体が落ちるので、
            // 衝突した方に連番を振る（モジュールの原本は artifact.json 側に残っている）
            let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (name, src) in modules {
                let safe = sanitize_entry_name(name);
                if safe.is_empty() {
                    continue;
                }
                let mut entry = safe.clone();
                let mut suffix = 2;
                while !used.insert(entry.clone()) {
                    entry = format!("{}-{}", safe, suffix);
                    suffix += 1;
                }
                put(&mut zip, &format!("modules/{}", entry), src.as_str().unwrap_or(""))?;
            }
        }
        if let Ok(state) = std::fs::read_to_string(&state_path) {
            put(&mut zip, "state.json", &state)?;
        }
        put(&mut zip, "README.txt", &readme_text(title, content_type, view_html.is_some()))?;

        zip.finish().map_err(|e| format!("zip の書き出しに失敗しました: {}", e))?;

        let bytes = std::fs::metadata(&dest_path).map(|m| m.len()).unwrap_or(0);
        Ok(ExportArtifactResult { path: dest_path, bytes })
    })
    .await
    .map_err(|e| format!("task join error: {}", e))?
}

/// zip から 1 エントリを文字列で読む。無ければ `None`。
fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<String>, String> {
    let file = match archive.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("zip の読み込みに失敗しました: {}", e)),
    };
    if file.size() > MAX_ENTRY_BYTES {
        return Err(format!("{} が大きすぎます", name));
    }
    // ヘッダの申告値は嘘をつけるので、実際の読み取り側でも上限で切る
    let mut buf = String::new();
    let read = file
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_string(&mut buf)
        .map_err(|e| format!("{} の読み込みに失敗しました: {}", name, e))?;
    if read as u64 > MAX_ENTRY_BYTES {
        return Err(format!("{} が大きすぎます", name));
    }
    Ok(Some(buf))
}

/// エクスポートした zip をスコープへ取り込む。
///
/// ID が既に埋まっている場合は連番を足した別 ID で取り込む（既存のアーティファクトを
/// 黙って上書きしない）。取り込んだ本体は「このスコープで今できたもの」として扱うため、
/// `updated_at` を現在時刻に、転送元マーカー (`source_worktree_id`) は落とす。
#[tauri::command]
pub async fn import_artifact(
    app_handle: AppHandle,
    scope: String,
    scope_id: String,
    zip_path: String,
) -> Result<ImportArtifactResult, String> {
    let dir = artifact_scope_dir(&app_handle, &scope, &scope_id)?;

    let result = tokio::task::spawn_blocking(move || -> Result<ImportArtifactResult, String> {
        let file = std::fs::File::open(&zip_path)
            .map_err(|e| format!("zip を開けませんでした: {}", e))?;
        let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|e| format!("zip として読めませんでした: {}", e))?;

        let manifest_raw = read_zip_entry(&mut archive, "manifest.json")?
            .ok_or_else(|| "oretachi のアーティファクト zip ではありません (manifest.json がありません)".to_string())?;
        let manifest: serde_json::Value = serde_json::from_str(&manifest_raw)
            .map_err(|e| format!("manifest.json を解析できませんでした: {}", e))?;
        if manifest.get("format").and_then(|v| v.as_str()) != Some(EXPORT_FORMAT) {
            return Err("oretachi のアーティファクト zip ではありません".to_string());
        }
        let version = manifest.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
        if version > EXPORT_FORMAT_VERSION as u64 {
            return Err(format!(
                "この zip は新しい形式です (version {})。oretachi を更新してください",
                version
            ));
        }

        let body_raw = read_zip_entry(&mut archive, "artifact.json")?
            .ok_or_else(|| "zip に artifact.json がありません".to_string())?;
        let mut body: serde_json::Value = serde_json::from_str(&body_raw)
            .map_err(|e| format!("artifact.json を解析できませんでした: {}", e))?;
        let obj = body
            .as_object_mut()
            .ok_or_else(|| "artifact.json の形式が不正です".to_string())?;

        // 本体の id が sanitize で落ちたときに manifest 側へ落ちられるよう、順に試す
        let base_id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(sanitize_artifact_id)
            .or_else(|| {
                manifest
                    .get("artifactId")
                    .and_then(|v| v.as_str())
                    .and_then(sanitize_artifact_id)
            })
            .unwrap_or_else(|| format!("imported-{}", now_secs()));

        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut artifact_id = base_id.clone();
        let mut suffix = 2;
        while dir.join(format!("{}.json", artifact_id)).exists() {
            artifact_id = format!("{}-{}", base_id, suffix);
            suffix += 1;
            if suffix > 1000 {
                return Err("同名のアーティファクトが多すぎます".to_string());
            }
        }
        let renamed_from = if artifact_id == base_id { None } else { Some(base_id) };

        obj.insert("id".to_string(), serde_json::Value::String(artifact_id.clone()));
        // 転送元マーカーは「このリポジトリ保管庫へ転送された」ことを表す値なので、
        // 別マシンから持ち込んだものに残っていても意味を持たない
        obj.remove("source_worktree_id");
        // 一覧は updated_at 降順。取り込んだものが埋もれないよう現在時刻にする
        // （いつ作られたかは created_at と manifest.exportedAt に残る）
        obj.insert("updated_at".to_string(), serde_json::json!(now_secs()));
        if !obj.contains_key("created_at") {
            obj.insert("created_at".to_string(), serde_json::json!(now_secs()));
        }
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(&artifact_id)
            .to_string();

        let json = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        let dest_path = dir.join(format!("{}.json", artifact_id));
        let tmp_path = dir.join(format!(".{}.import{}.tmp", artifact_id, std::process::id()));
        if let Err(e) = std::fs::write(&tmp_path, json) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("アーティファクトの書き込みに失敗しました: {}", e));
        }
        if let Err(e) = std::fs::rename(&tmp_path, &dest_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("アーティファクトの書き込みに失敗しました: {}", e));
        }

        // メモリーはアーティファクトの中身に属する状態なので引き継ぐ。
        // ピン止めは取り込み先ローカルの並び順なので持ち込まない。
        if let Some(state_raw) = read_zip_entry(&mut archive, "state.json")? {
            if let Ok(serde_json::Value::Object(state)) = serde_json::from_str(&state_raw) {
                if let Some(memory) = state.get("memory").filter(|v| v.is_object()) {
                    let sidecar = serde_json::json!({
                        "memory": memory,
                        "memoryUpdatedAt": now_millis(),
                    });
                    // サイドカーは補助情報。失敗しても本体の取り込みは成功として扱う
                    let _ = std::fs::write(
                        artifact_state_path(&dir, &artifact_id),
                        serde_json::to_string(&sidecar).unwrap_or_default(),
                    );
                }
            }
        }

        Ok(ImportArtifactResult { artifact_id, title, renamed_from })
    })
    .await
    .map_err(|e| format!("task join error: {}", e))??;

    // 一覧の更新はビューアの既存の変更ハンドラ (command=create) に任せる
    if scope == "repository" {
        let _ = app_handle.emit(
            "repo-artifact-changed",
            serde_json::json!({
                "repositoryId": scope_id,
                "artifactId": result.artifact_id,
                "command": "create",
            }),
        );
    } else {
        let _ = app_handle.emit(
            "artifact-changed",
            serde_json::json!({
                "worktreeId": scope_id,
                "artifactId": result.artifact_id,
                "command": "create",
                // 取り込みはユーザー自身の操作なので、表示中のものを奪わずトーストで導線を出す
                "autoOpen": true,
            }),
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_entry_name_drops_traversal() {
        assert_eq!(sanitize_entry_name("data/flow"), "data/flow");
        assert_eq!(sanitize_entry_name("../../etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_entry_name("C:\\tmp\\x"), "C_/tmp/x");
        assert_eq!(sanitize_entry_name("../.."), "");
    }

    #[test]
    fn sanitize_artifact_id_rejects_unsafe() {
        assert_eq!(sanitize_artifact_id("report-1"), Some("report-1".to_string()));
        assert_eq!(sanitize_artifact_id("../evil"), Some("evil".to_string()));
        assert_eq!(sanitize_artifact_id("///"), None);
        assert_eq!(sanitize_artifact_id(".."), None);
    }

    #[test]
    fn source_extension_maps_known_types() {
        assert_eq!(source_extension("text/markdown", None), "md");
        assert_eq!(source_extension("application/vnd.ant.react", None), "jsx");
        assert_eq!(source_extension("application/vnd.ant.code", Some("python")), "py");
        assert_eq!(source_extension("application/vnd.ant.code", None), "txt");
        assert_eq!(source_extension("application/x-unknown", None), "txt");
    }
}
