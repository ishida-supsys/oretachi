//! アーティファクトへのコメント（人が付ける修正指示と、AI の返信）。
//!
//! 保存先は状態サイドカー `<id>.state` の `comments` キー。本体 `<id>.json` は AI が
//! 同じ ID で作り直すと丸ごと置き換わるが、サイドカーは別ファイルなのでコメントが消えない
//! （「コメントを付けて AI に直させる」流れで、直した結果コメントが消えては困る）。
//!
//! 書き込みは `update_artifact_state` のロック内で行う（ピン止め・メモリーと同じファイルを
//! 触るため、read-modify-write を直列化しないと互いを踏み潰す）。
//!
//! 変更を知らせるイベントは `artifact-state-changed` を**使わない**。そちらは
//! 「メモリーが外から書き換わった」意味で、React アーティファクトの iframe へメモリーを
//! 押し込む購読者がいる。コメントが付くたびにフォーム入力を巻き戻させないよう、
//! 専用の `artifact-comments-changed` を飛ばす。
//!
//! `locked_while_open` はコメントの書き込みを止めない。ロックは本体（AI の成果物）を
//! ユーザーが見ている間に裏で書き換えさせないためのもので、コメントは本体に触らない。
//! しかも AI が返信したいのは、まさにユーザーが開いているとき。

use serde_json::{json, Map, Value};
use tauri::Emitter;

/// サイドカー内のキー
pub(crate) const COMMENTS_KEY: &str = "comments";
/// コメント配列全体の上限。サイドカーは毎回全体を書き直すので、書き込みコストの上限でもある
const COMMENTS_MAX_BYTES: usize = 512 * 1024;
/// コメント 1 件・返信 1 件の本文の上限（文字数）
const BODY_MAX_CHARS: usize = 4000;
/// アンカー（位置情報）の上限（シリアライズ後のバイト数）
const ANCHOR_MAX_BYTES: usize = 8 * 1024;

/// 書き手。ビューアは "user"、MCP は "ai" を固定で渡す（呼び手が名乗るものではない）
pub(crate) const AUTHOR_USER: &str = "user";
pub(crate) const AUTHOR_AI: &str = "ai";

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn validate_author(author: &str) -> Result<(), String> {
    match author {
        AUTHOR_USER | AUTHOR_AI => Ok(()),
        other => Err(format!("author は user / ai のいずれかです: {}", other)),
    }
}

fn validate_body(body: &str) -> Result<String, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("コメント本文が空です".to_string());
    }
    if body.chars().count() > BODY_MAX_CHARS {
        return Err(format!("コメント本文が長すぎます（上限 {} 文字）", BODY_MAX_CHARS));
    }
    Ok(body.to_string())
}

/// アンカーは JSON オブジェクトに限る。形（座標・セレクタ・抜粋）は表示側が決めるので、
/// ここでは「オブジェクトであること」と大きさだけ見る
fn validate_anchor(anchor: Value) -> Result<Value, String> {
    if !anchor.is_object() {
        return Err("anchor は JSON オブジェクトである必要があります".to_string());
    }
    let size = serde_json::to_string(&anchor).map_err(|e| e.to_string())?.len();
    if size > ANCHOR_MAX_BYTES {
        return Err(format!(
            "anchor が上限を超えています: {} バイト（上限 {} バイト）",
            size, ANCHOR_MAX_BYTES
        ));
    }
    Ok(anchor)
}

/// サイドカーから `comments` を取り出す（無い・配列でないなら空）
fn comments_mut(obj: &mut Map<String, Value>) -> &mut Vec<Value> {
    let entry = obj
        .entry(COMMENTS_KEY.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    entry.as_array_mut().expect("just ensured array")
}

fn check_total_size(obj: &Map<String, Value>) -> Result<(), String> {
    let size = obj
        .get(COMMENTS_KEY)
        .map(|v| serde_json::to_string(v).map(|s| s.len()).unwrap_or(0))
        .unwrap_or(0);
    if size > COMMENTS_MAX_BYTES {
        return Err(format!(
            "コメントが上限を超えています: {} バイト（上限 {} バイト）。解決済みのコメントを削除してください",
            size, COMMENTS_MAX_BYTES
        ));
    }
    Ok(())
}

/// 空になった `comments` は残さない（既定値だけのサイドカーをファイルごと消すため）
fn drop_if_empty(obj: &mut Map<String, Value>) {
    let empty = obj
        .get(COMMENTS_KEY)
        .and_then(|v| v.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(true);
    if empty {
        obj.remove(COMMENTS_KEY);
    }
}

fn find_comment<'a>(list: &'a mut [Value], comment_id: &str) -> Result<&'a mut Value, String> {
    list.iter_mut()
        .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(comment_id))
        .ok_or_else(|| format!("コメント '{}' が見つかりません", comment_id))
}

// ─── サイドカーのオブジェクトに対する純粋な操作（テスト対象） ───────────────

/// コメントを追加して ID を返す
pub(crate) fn add_comment(
    obj: &mut Map<String, Value>,
    author: &str,
    body: &str,
    anchor: Value,
) -> Result<String, String> {
    validate_author(author)?;
    let body = validate_body(body)?;
    let anchor = validate_anchor(anchor)?;
    let id = new_id();
    comments_mut(obj).push(json!({
        "id": id,
        "author": author,
        "body": body,
        "anchor": anchor,
        "createdAt": now_ms(),
        "resolved": false,
        "replies": [],
    }));
    check_total_size(obj)?;
    Ok(id)
}

/// 返信を追加して ID を返す。解決済みへの返信は未解決へ戻す
/// （解決済みは非表示なので、返信が付いたことに誰も気づけなくなるのを避ける）
pub(crate) fn reply_comment(
    obj: &mut Map<String, Value>,
    comment_id: &str,
    author: &str,
    body: &str,
) -> Result<String, String> {
    validate_author(author)?;
    let body = validate_body(body)?;
    let id = new_id();
    {
        let comment = find_comment(comments_mut(obj), comment_id)?;
        let reply = json!({ "id": id, "author": author, "body": body, "createdAt": now_ms() });
        let replies = comment
            .as_object_mut()
            .ok_or_else(|| "コメントの形式が不正です".to_string())?
            .entry("replies".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !replies.is_array() {
            *replies = Value::Array(Vec::new());
        }
        replies.as_array_mut().expect("just ensured array").push(reply);
        comment["resolved"] = Value::Bool(false);
    }
    check_total_size(obj)?;
    Ok(id)
}

pub(crate) fn set_resolved(
    obj: &mut Map<String, Value>,
    comment_id: &str,
    resolved: bool,
) -> Result<(), String> {
    let comment = find_comment(comments_mut(obj), comment_id)?;
    comment["resolved"] = Value::Bool(resolved);
    drop_if_empty(obj);
    Ok(())
}

/// 未解決のコメントをすべて解決済みにして、その件数を返す
pub(crate) fn resolve_all(obj: &mut Map<String, Value>) -> usize {
    let mut count = 0;
    if let Some(list) = obj.get_mut(COMMENTS_KEY).and_then(|v| v.as_array_mut()) {
        for c in list.iter_mut() {
            if c.get("resolved").and_then(|v| v.as_bool()) != Some(true) {
                c["resolved"] = Value::Bool(true);
                count += 1;
            }
        }
    }
    count
}

pub(crate) fn delete_comment(obj: &mut Map<String, Value>, comment_id: &str) -> Result<(), String> {
    let list = comments_mut(obj);
    let before = list.len();
    list.retain(|c| c.get("id").and_then(|v| v.as_str()) != Some(comment_id));
    let removed = list.len() != before;
    drop_if_empty(obj);
    if removed {
        Ok(())
    } else {
        Err(format!("コメント '{}' が見つかりません", comment_id))
    }
}

/// サイドカーのオブジェクトからコメント一覧を読む。`include_resolved=false` は未解決だけ
pub(crate) fn list_comments(obj: &Map<String, Value>, include_resolved: bool) -> Vec<Value> {
    obj.get(COMMENTS_KEY)
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter(|c| include_resolved || c.get("resolved").and_then(|v| v.as_bool()) != Some(true))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 状態サイドカーを読む（無い・壊れている場合は空）
pub(crate) fn read_state_object(dir: &std::path::Path, artifact_id: &str) -> Map<String, Value> {
    std::fs::read_to_string(crate::artifact_state_path(dir, artifact_id))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

/// 変更を開いているビューア・アプリウィンドウへ知らせる
pub(crate) fn emit_changed(
    app_handle: &tauri::AppHandle,
    scope: &str,
    scope_id: &str,
    artifact_id: &str,
) {
    if let Err(e) = app_handle.emit(
        "artifact-comments-changed",
        json!({ "scope": scope, "scopeId": scope_id, "artifactId": artifact_id }),
    ) {
        log::warn!("Failed to emit artifact-comments-changed: {}", e);
    }
}

/// 1 件の変更操作。`Add` / `Reply` は採番した ID、`ResolveAll` は件数を返す
pub(crate) enum Op {
    Add { author: String, body: String, anchor: Value },
    Reply { comment_id: String, author: String, body: String },
    SetResolved { comment_id: String, resolved: bool },
    ResolveAll,
    Delete { comment_id: String },
}

/// 変更を 1 回の read-modify-write で適用する。戻り値は `Add` / `Reply` なら新しい ID、
/// `ResolveAll` なら件数の文字列、それ以外は空文字
pub(crate) async fn apply(
    dir: std::path::PathBuf,
    artifact_id: &str,
    op: Op,
) -> Result<String, String> {
    // 本体が無い ID のサイドカーは一覧走査に出てくる孤児になり、以後どこからも消せない
    if !dir.join(format!("{}.json", artifact_id)).exists() {
        return Err(format!("アーティファクト '{}' が存在しません", artifact_id));
    }
    let out = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sink = out.clone();
    crate::update_artifact_state(dir, artifact_id, move |obj| {
        let result = match op {
            Op::Add { author, body, anchor } => add_comment(obj, &author, &body, anchor)?,
            Op::Reply { comment_id, author, body } => reply_comment(obj, &comment_id, &author, &body)?,
            Op::SetResolved { comment_id, resolved } => {
                set_resolved(obj, &comment_id, resolved)?;
                String::new()
            }
            Op::ResolveAll => resolve_all(obj).to_string(),
            Op::Delete { comment_id } => {
                delete_comment(obj, &comment_id)?;
                String::new()
            }
        };
        if let Ok(mut s) = sink.lock() {
            *s = result;
        }
        Ok(())
    })
    .await?;
    let result = out.lock().map(|s| s.clone()).unwrap_or_default();
    Ok(result)
}

// ─── ビューア（Tauri コマンド）。書き手は常に user ───────────────────────────

fn validate_ids(artifact_id: &str) -> Result<(), String> {
    crate::validate_path_component(artifact_id)
}

#[tauri::command]
pub async fn add_artifact_comment(
    app_handle: tauri::AppHandle,
    scope: String,
    scope_id: String,
    artifact_id: String,
    body: String,
    anchor: Value,
) -> Result<String, String> {
    validate_ids(&artifact_id)?;
    let dir = crate::artifact_scope_dir(&app_handle, &scope, &scope_id)?;
    let id = apply(dir, &artifact_id, Op::Add { author: AUTHOR_USER.into(), body, anchor }).await?;
    emit_changed(&app_handle, &scope, &scope_id, &artifact_id);
    Ok(id)
}

#[tauri::command]
pub async fn reply_artifact_comment(
    app_handle: tauri::AppHandle,
    scope: String,
    scope_id: String,
    artifact_id: String,
    comment_id: String,
    body: String,
) -> Result<String, String> {
    validate_ids(&artifact_id)?;
    let dir = crate::artifact_scope_dir(&app_handle, &scope, &scope_id)?;
    let id = apply(
        dir,
        &artifact_id,
        Op::Reply { comment_id, author: AUTHOR_USER.into(), body },
    )
    .await?;
    emit_changed(&app_handle, &scope, &scope_id, &artifact_id);
    Ok(id)
}

#[tauri::command]
pub async fn set_artifact_comment_resolved(
    app_handle: tauri::AppHandle,
    scope: String,
    scope_id: String,
    artifact_id: String,
    comment_id: String,
    resolved: bool,
) -> Result<(), String> {
    validate_ids(&artifact_id)?;
    let dir = crate::artifact_scope_dir(&app_handle, &scope, &scope_id)?;
    apply(dir, &artifact_id, Op::SetResolved { comment_id, resolved }).await?;
    emit_changed(&app_handle, &scope, &scope_id, &artifact_id);
    Ok(())
}

/// 解決した件数を返す
#[tauri::command]
pub async fn resolve_all_artifact_comments(
    app_handle: tauri::AppHandle,
    scope: String,
    scope_id: String,
    artifact_id: String,
) -> Result<usize, String> {
    validate_ids(&artifact_id)?;
    let dir = crate::artifact_scope_dir(&app_handle, &scope, &scope_id)?;
    let n = apply(dir, &artifact_id, Op::ResolveAll).await?;
    emit_changed(&app_handle, &scope, &scope_id, &artifact_id);
    Ok(n.parse().unwrap_or(0))
}

#[tauri::command]
pub async fn delete_artifact_comment(
    app_handle: tauri::AppHandle,
    scope: String,
    scope_id: String,
    artifact_id: String,
    comment_id: String,
) -> Result<(), String> {
    validate_ids(&artifact_id)?;
    let dir = crate::artifact_scope_dir(&app_handle, &scope, &scope_id)?;
    apply(dir, &artifact_id, Op::Delete { comment_id }).await?;
    emit_changed(&app_handle, &scope, &scope_id, &artifact_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> Value {
        json!({ "kind": "point", "x": 0.5, "y": 0.25 })
    }

    #[test]
    fn add_then_list_unresolved_only() {
        let mut obj = Map::new();
        let a = add_comment(&mut obj, AUTHOR_USER, "  直して  ", anchor()).unwrap();
        let b = add_comment(&mut obj, AUTHOR_USER, "二つ目", anchor()).unwrap();
        set_resolved(&mut obj, &a, true).unwrap();
        let open = list_comments(&obj, false);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0]["id"], b);
        assert_eq!(list_comments(&obj, true).len(), 2);
        // 本文は trim される
        assert_eq!(list_comments(&obj, true)[0]["body"], "直して");
    }

    #[test]
    fn rejects_bad_input() {
        let mut obj = Map::new();
        assert!(add_comment(&mut obj, "bot", "x", anchor()).is_err());
        assert!(add_comment(&mut obj, AUTHOR_USER, "   ", anchor()).is_err());
        assert!(add_comment(&mut obj, AUTHOR_USER, "x", json!("str")).is_err());
        assert!(add_comment(&mut obj, AUTHOR_USER, &"あ".repeat(BODY_MAX_CHARS + 1), anchor()).is_err());
        let huge = json!({ "selector": "a".repeat(ANCHOR_MAX_BYTES) });
        assert!(add_comment(&mut obj, AUTHOR_USER, "x", huge).is_err());
        // 失敗した追加は何も残さない
        assert!(list_comments(&obj, true).is_empty());
    }

    #[test]
    fn reply_reopens_resolved_comment() {
        let mut obj = Map::new();
        let id = add_comment(&mut obj, AUTHOR_USER, "直して", anchor()).unwrap();
        set_resolved(&mut obj, &id, true).unwrap();
        reply_comment(&mut obj, &id, AUTHOR_AI, "直しました").unwrap();
        let c = &list_comments(&obj, false)[0];
        assert_eq!(c["resolved"], false);
        assert_eq!(c["replies"].as_array().unwrap().len(), 1);
        assert_eq!(c["replies"][0]["author"], AUTHOR_AI);
        assert!(reply_comment(&mut obj, "nope", AUTHOR_AI, "x").is_err());
    }

    #[test]
    fn resolve_all_counts_only_open_ones() {
        let mut obj = Map::new();
        let a = add_comment(&mut obj, AUTHOR_USER, "1", anchor()).unwrap();
        add_comment(&mut obj, AUTHOR_USER, "2", anchor()).unwrap();
        add_comment(&mut obj, AUTHOR_USER, "3", anchor()).unwrap();
        set_resolved(&mut obj, &a, true).unwrap();
        assert_eq!(resolve_all(&mut obj), 2);
        assert_eq!(resolve_all(&mut obj), 0);
        assert!(list_comments(&obj, false).is_empty());
    }

    #[test]
    fn delete_removes_key_when_last() {
        let mut obj = Map::new();
        let id = add_comment(&mut obj, AUTHOR_USER, "1", anchor()).unwrap();
        assert!(delete_comment(&mut obj, "nope").is_err());
        delete_comment(&mut obj, &id).unwrap();
        // 空のサイドカーはファイルごと消えるよう、キーを残さない
        assert!(!obj.contains_key(COMMENTS_KEY));
    }

    #[test]
    fn total_size_is_capped() {
        let mut obj = Map::new();
        let body = "あ".repeat(BODY_MAX_CHARS);
        let mut failed = false;
        for _ in 0..200 {
            if add_comment(&mut obj, AUTHOR_USER, &body, anchor()).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "合計サイズの上限で止まること");
    }

    #[test]
    fn corrupt_comments_value_is_replaced() {
        let mut obj = Map::new();
        obj.insert(COMMENTS_KEY.to_string(), json!("garbage"));
        assert!(list_comments(&obj, true).is_empty());
        add_comment(&mut obj, AUTHOR_USER, "x", anchor()).unwrap();
        assert_eq!(list_comments(&obj, true).len(), 1);
    }
}
