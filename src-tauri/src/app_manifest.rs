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
const RESERVED_PERMISSION_KEYS: &[&str] =
    &["fs", "db", "network", "subWindow", "wasm", "aiTerminal"];

const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_VERSION_CHARS: usize = 50;
const MAX_TARGETS: usize = 20;
const MAX_TARGET_CHARS: usize = 200;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestedPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<RequestedMcp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryPermission>,
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
/// `permissions` 内の未知キーは**拒否**する（fail closed）。#370 が受理するのは `mcp` と `memory` だけで、
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
            k if RESERVED_PERMISSION_KEYS.contains(&k) => {
                return Err(format!(
                    "権限 '{}' はこの版ではまだ使えません（対応する機能が未実装です）。manifest から外してください",
                    k
                ));
            }
            other => {
                return Err(format!(
                    "未知の権限 '{}' です。指定できるのは mcp / memory だけです",
                    other
                ));
            }
        }
    }
    Ok(out)
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
            },
            resolved: resolved(&["notify_worktree"], &["wt-b"]),
        };
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains("grantedAt") && s.contains("maxBytes"));
        let back: AppGrant = serde_json::from_str(&s).unwrap();
        assert_eq!(back, g);
    }
}
