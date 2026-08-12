use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::settings::NotificationHookEntry;

const PLUGIN_NAME: &str = "oretachi";
const PLUGIN_ID: &str = "oretachi@oretachi";
const MARKETPLACE_DIR: &str = "claude-plugins";

/// Claude Code プラグイン: 管理対象イベントとそのuserConfigキー
const EVENT_CONFIG_KEYS: &[(&str, &str)] = &[
    ("Stop", "stop_kind"),
    ("Notification", "notification_kind"),
    ("SubagentStop", "subagent_stop_kind"),
    ("PreToolUse", "pre_tool_use_kind"),
    ("PostToolUse", "post_tool_use_kind"),
    ("PermissionRequest", "permission_request_kind"),
];

/// マーケットプレイスディレクトリ（extraKnownMarketplacesで指定するパス）を返す
/// Windows: %APPDATA%/com.ia.oretachi/claude-plugins
pub fn marketplace_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_data_dir()
        .map(|d| d.join(MARKETPLACE_DIR))
        .map_err(|e| format!("Failed to get app_data_dir: {}", e))
}

/// プラグイン本体ディレクトリを返す
/// marketplace_dir/oretachi/
fn plugin_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    marketplace_dir(app_handle).map(|d| d.join(PLUGIN_NAME))
}

/// グローバルプラグイン（marketplace / .mcp.json）を上書きするか。
/// 未設定は true（本番インストール済みアプリには .env* が無いため常に上書き＝従来挙動）。
/// dev では .env.development の ORETACHI_PLUGIN_OVERWRITE=false で抑止できる。
pub fn overwrite_enabled() -> bool {
    std::env::var("ORETACHI_PLUGIN_OVERWRITE")
        .map(|v| v != "false")
        .unwrap_or(true)
}

/// 起動時にプラグインファイル群を生成・更新する
/// - ディレクトリ構造の作成
/// - marketplace.json: マーケットプレイスルートのカタログ
/// - plugin.json: env.ORETACHI_APP_PATH を現在のexeパスで更新
/// - hooks/hooks.json: 全イベントのフック定義
/// - .mcp.json はポート確定後に update_mcp_config で書き込むため、ここでは生成しない
pub fn generate_plugin_files(app_handle: &AppHandle) -> Result<(), String> {
    let mkt_dir = marketplace_dir(app_handle)?;
    let plugin_dir = plugin_dir(app_handle)?;
    let mkt_claude_plugin_dir = mkt_dir.join(".claude-plugin");
    let claude_plugin_dir = plugin_dir.join(".claude-plugin");
    let hooks_dir = plugin_dir.join("hooks");

    std::fs::create_dir_all(&mkt_claude_plugin_dir)
        .map_err(|e| format!("Failed to create marketplace .claude-plugin dir: {}", e))?;
    std::fs::create_dir_all(&claude_plugin_dir)
        .map_err(|e| format!("Failed to create .claude-plugin dir: {}", e))?;
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Failed to create hooks dir: {}", e))?;

    // marketplace.json
    let marketplace_json = build_marketplace_json();
    let marketplace_json_path = mkt_claude_plugin_dir.join("marketplace.json");
    std::fs::write(
        &marketplace_json_path,
        serde_json::to_string_pretty(&marketplace_json)
            .map_err(|e| format!("Failed to serialize marketplace.json: {}", e))?,
    )
    .map_err(|e| format!("Failed to write marketplace.json: {}", e))?;

    let exe_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "oretachi".to_string());
    let sidecar_path = sidecar_path();

    // plugin.json
    let plugin_json = build_plugin_json(&exe_path);
    let plugin_json_path = claude_plugin_dir.join("plugin.json");
    std::fs::write(
        &plugin_json_path,
        serde_json::to_string_pretty(&plugin_json)
            .map_err(|e| format!("Failed to serialize plugin.json: {}", e))?,
    )
    .map_err(|e| format!("Failed to write plugin.json: {}", e))?;

    // hooks/hooks.json （通知は GUI 本体ではなく oretachi-notify サイドカーが受ける）
    let hooks_json = build_hooks_json(&sidecar_path);
    let hooks_json_path = hooks_dir.join("hooks.json");
    std::fs::write(
        &hooks_json_path,
        serde_json::to_string_pretty(&hooks_json)
            .map_err(|e| format!("Failed to serialize hooks.json: {}", e))?,
    )
    .map_err(|e| format!("Failed to write hooks.json: {}", e))?;

    // skills/
    write_skill_files(&plugin_dir)?;

    Ok(())
}

fn write_skill_files(plugin_dir: &std::path::Path) -> Result<(), String> {
    let skills_dir = plugin_dir.join("skills");
    for (rel_path, content) in crate::claude_plugin_skills::SKILL_FILES {
        let dest = skills_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("Failed to create skill dir {}: {}", parent.display(), e)
            })?;
        }
        std::fs::write(&dest, content)
            .map_err(|e| format!("Failed to write skill file {}: {}", dest.display(), e))?;
    }
    Ok(())
}

/// .mcp.json のみ更新する（MCP サーバー起動後にポート確定値で呼ばれる）
pub fn update_mcp_config(app_handle: &AppHandle, port: u16, api_key: &str) -> Result<(), String> {
    let plugin_dir = plugin_dir(app_handle)?;
    // プラグインディレクトリが存在しない場合は初回生成前なのでスキップ
    if !plugin_dir.exists() {
        return Ok(());
    }
    update_mcp_json(&plugin_dir, port, api_key)
}

fn update_mcp_json(plugin_dir: &std::path::Path, port: u16, api_key: &str) -> Result<(), String> {
    let mcp_json = build_mcp_json(port, api_key);
    let mcp_json_path = plugin_dir.join(".mcp.json");
    std::fs::write(
        &mcp_json_path,
        serde_json::to_string_pretty(&mcp_json)
            .map_err(|e| format!("Failed to serialize .mcp.json: {}", e))?,
    )
    .map_err(|e| format!("Failed to write .mcp.json: {}", e))
}

fn build_plugin_json(exe_path: &str) -> serde_json::Value {
    let mut user_config = serde_json::Map::new();
    user_config.insert(
        "worktree_name".to_string(),
        serde_json::json!({
            "type": "string",
            "title": "Worktree name",
            "description": "Worktree name for notifications"
        }),
    );
    for (_, key) in EVENT_CONFIG_KEYS {
        user_config.insert(
            key.to_string(),
            serde_json::json!({
                "type": "string",
                "title": *key,
                "description": format!("Notification kind for {} event", key)
            }),
        );
    }

    serde_json::json!({
        "name": PLUGIN_NAME,
        "description": "oretachi worktree notification hooks & MCP server",
        "mcpServers": "./.mcp.json",
        "skills": "./skills/",
        "env": {
            "ORETACHI_APP_PATH": exe_path
        },
        "userConfig": user_config
    })
}

/// hooks.json の command が参照する通知サイドカー (oretachi-notify) の絶対パスを返す。
/// externalBin としてバンドルされたサイドカーは GUI 本体と同じディレクトリに配置される
/// （dev 時も target/debug 隣に複製される）ため、current_exe と同階層を指す。
fn sidecar_path() -> String {
    const SIDECAR_BIN: &str = if cfg!(target_os = "windows") {
        "oretachi-notify.exe"
    } else {
        "oretachi-notify"
    };
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(SIDECAR_BIN)))
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "oretachi-notify".to_string())
}

fn build_hooks_json(notifier_path: &str) -> serde_json::Value {
    // notifier_path は通知サイドカー (oretachi-notify) の絶対パスを直接ハードコードする。
    //
    // userConfig 非依存・exec-form で生成する。Claude Code 2.1.207 は
    //   (a) shell-form の command 内で ${user_config.*} を参照すると拒否し、
    //   (b) pluginConfigs (${user_config.*} の値) を settings.local.json から読まなくなった
    // ため、userConfig ベースの旧方式は成立しない。そこで hook からは userConfig ではなく
    // CC 組み込み変数 ${CLAUDE_PROJECT_DIR}（全バージョンで有効・userConfig ではない）と
    // イベント名リテラルのみを渡し、「パス→ワークツリー」「イベント→kind」の解決は
    // oretachi サーバー側 (/notify ハンドラ) で行う。exec-form (args 配列) は shell-form の
    // ${user_config.*} 拒否チェックの対象外であり、${CLAUDE_PROJECT_DIR} は args 内でも置換される。
    let mut hooks = serde_json::Map::new();
    for (event, _key) in EVENT_CONFIG_KEYS {
        hooks.insert(
            event.to_string(),
            serde_json::json!([{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": notifier_path,
                    "args": [
                        "--notify",
                        "--project-dir", "${CLAUDE_PROJECT_DIR}",
                        "--event", event,
                        "--agent", "cc"
                    ]
                }]
            }]),
        );
    }

    // Stop フック（2つ目のグループ）: ターン境界で購読イベントの未読を additionalContext として
    // 注入する（#124）。Stop は「at the end of the turn (conversation continues so Claude can act
    // on feedback)」なので、注入すればそのまま会話が継続してエージェントが着手する。
    //
    // 上の EVENT_CONFIG_KEYS ループが張る `--notify --event Stop` とは**別グループ**にする:
    //   - `--notify` は6イベント共有で post_json（read 500ms・body を読まない・失敗時 exit 1）。
    //     body を読むために post_json_read_body（read 2s）へ切り替えると Stop 以外まで遅くなる
    //   - additionalContext を返す経路は「失敗しても無出力で必ず exit 0」に倒す必要があり
    //     （claude 側に警告を出させない）、exit 1 しうる --notify とは方針が逆
    // PermissionRequest に ExitPlanMode グループを追記しているのと同じ構造。
    let turn_context_group = serde_json::json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": notifier_path,
            "args": ["--turn-context", "--project-dir", "${CLAUDE_PROJECT_DIR}"]
        }]
    });
    if let Some(arr) = hooks.get_mut("Stop").and_then(|v| v.as_array_mut()) {
        arr.push(turn_context_group);
    } else {
        hooks.insert("Stop".to_string(), serde_json::json!([turn_context_group]));
    }

    // ExitPlanMode フック: プラン確定時に通知サイドカー (oretachi-notify) を起動し、
    // プランを AI 要約してワークツリーの description にセットさせる。
    // ExitPlanMode はユーザー操作を伴うツールのため PreToolUse/PostToolUse は発火せず、
    // PermissionRequest イベントで発火する（matcher "ExitPlanMode"、プラン本文は tool_input.plan）。
    // 既存の通知用 PermissionRequest グループ(matcher "")と共存させるため配列に追記する。
    // decision は返さず観測のみ（exit 0）なので通常の承認フローはブロックしない。
    let exit_plan_group = serde_json::json!({
        "matcher": "ExitPlanMode",
        "hooks": [{
            "type": "command",
            "command": notifier_path,
            "args": ["--set-description", "--project-dir", "${CLAUDE_PROJECT_DIR}"]
        }]
    });
    if let Some(arr) = hooks
        .get_mut("PermissionRequest")
        .and_then(|v| v.as_array_mut())
    {
        arr.push(exit_plan_group);
    } else {
        hooks.insert("PermissionRequest".to_string(), serde_json::json!([exit_plan_group]));
    }

    // SessionStart フック: セッション開始（startup/resume/clear/compact）のたびにサイドカーを起動し、
    // /session-context からワークツリー所属グループの systemPrompt を取得してコンテキストに注入する。
    // 解決は毎回サーバー側で行うため、グループ設定の変更は次の SessionStart から自動反映される。
    // アプリ非稼働時はサイドカーが何も出力せず exit 0 するため、単体起動の claude には影響しない。
    hooks.insert(
        "SessionStart".to_string(),
        serde_json::json!([{
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": notifier_path,
                "args": ["--session-context", "--project-dir", "${CLAUDE_PROJECT_DIR}"]
            }]
        }]),
    );

    // UserPromptSubmit フック: プロンプト送信時にサイドカーを起動し、現在の description を
    // additionalContext として注入する（逸脱していたら oretachi_set_description で更新させる）。
    // 頻度はサーバー側 (/prompt-context) のワークツリー単位スロットルで抑制する。
    // 通知は不要なので EVENT_CONFIG_KEYS には含めない（あちらは通知 kind の設定用）。
    hooks.insert(
        "UserPromptSubmit".to_string(),
        serde_json::json!([{
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": notifier_path,
                "args": ["--prompt-context", "--project-dir", "${CLAUDE_PROJECT_DIR}"]
            }]
        }]),
    );

    serde_json::json!({ "hooks": hooks })
}

fn build_marketplace_json() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://anthropic.com/claude-code/marketplace.schema.json",
        "name": PLUGIN_NAME,
        "description": "oretachi worktree notification hooks & MCP server",
        "owner": {
            "name": "oretachi"
        },
        "plugins": [
            {
                "name": PLUGIN_NAME,
                "description": "oretachi worktree notification hooks & MCP server",
                "source": format!("./{}", PLUGIN_NAME)
            }
        ]
    })
}

fn build_mcp_json(port: u16, api_key: &str) -> serde_json::Value {
    serde_json::json!({
        PLUGIN_NAME: {
            "type": "http",
            "url": format!("http://127.0.0.1:{}/mcp", port),
            "headers": {
                "Authorization": format!("Bearer {}", api_key)
            }
        }
    })
}

/// ワークツリーの .claude/settings.local.json にプラグイン設定を書き込む
/// - extraKnownMarketplaces: プラグインのマーケットプレイスディレクトリ
/// - enabledPlugins: oretachi プラグインを有効化
/// - pluginConfigs: ワークツリー名と各イベントの通知 kind
/// - 旧形式の hooks キー内 oretachi フックを削除（マイグレーション）
pub fn write_plugin_config(
    worktree_path: &str,
    worktree_name: &str,
    hooks: Vec<NotificationHookEntry>,
    marketplace_dir_path: &str,
) -> Result<(), String> {
    use std::path::Path;

    let claude_dir = Path::new(worktree_path).join(".claude");
    std::fs::create_dir_all(&claude_dir)
        .map_err(|e| format!("Failed to create .claude dir: {}", e))?;

    let settings_path = claude_dir.join("settings.local.json");
    let mut json: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)
            .map_err(|e| format!("Failed to read settings.local.json: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse settings.local.json: {}", e))?
    } else {
        serde_json::json!({})
    };

    let root = json
        .as_object_mut()
        .ok_or_else(|| "settings.local.json is not a JSON object".to_string())?;

    // extraKnownMarketplaces に oretachi マーケットプレイスを追加
    {
        let marketplaces = root
            .entry("extraKnownMarketplaces")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = marketplaces.as_object_mut() {
            obj.insert(
                PLUGIN_NAME.to_string(),
                serde_json::json!({
                    "source": {
                        "source": "directory",
                        "path": marketplace_dir_path
                    }
                }),
            );
        }
    }

    // enabledPlugins に oretachi を追加
    {
        let enabled = root
            .entry("enabledPlugins")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = enabled.as_object_mut() {
            obj.insert(PLUGIN_ID.to_string(), serde_json::Value::Bool(true));
        }
    }

    // pluginConfigs に oretachi の設定を追加
    {
        let user_events: std::collections::HashMap<&str, &str> = hooks
            .iter()
            .map(|h| (h.event.as_str(), h.kind.as_str()))
            .collect();

        let mut options = serde_json::Map::new();
        options.insert(
            "worktree_name".to_string(),
            serde_json::Value::String(worktree_name.to_string()),
        );
        for (event, key) in EVENT_CONFIG_KEYS {
            let kind = user_events.get(event).copied().unwrap_or("hook");
            options.insert(
                key.to_string(),
                serde_json::Value::String(kind.to_string()),
            );
        }

        let plugin_configs = root
            .entry("pluginConfigs")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = plugin_configs.as_object_mut() {
            obj.insert(
                PLUGIN_ID.to_string(),
                serde_json::json!({ "options": options }),
            );
        }
    }

    // マイグレーション: 旧形式の hooks キー内 oretachi フックを削除
    migrate_remove_oretachi_hooks(&mut json);

    let content = serde_json::to_string_pretty(&json)
        .map_err(|e| format!("Failed to serialize settings.local.json: {}", e))?;
    std::fs::write(&settings_path, content)
        .map_err(|e| format!("Failed to write settings.local.json: {}", e))?;

    Ok(())
}

/// 旧形式の hooks オブジェクト内から oretachi が注入したフックを削除する。
/// oretachi フックの識別: --notify と --agent cc の両方を含むコマンド。
/// 各イベント配列から oretachi フックを除き、空になったイベントキーは削除する。
fn migrate_remove_oretachi_hooks(json: &mut serde_json::Value) {
    let Some(hooks_val) = json.get_mut("hooks") else {
        return;
    };
    let Some(hooks_obj) = hooks_val.as_object_mut() else {
        return;
    };

    let events_to_check: Vec<String> = hooks_obj.keys().cloned().collect();
    for event in events_to_check {
        let Some(groups) = hooks_obj.get_mut(&event).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        // oretachi フックを含むグループを除去（--notify と --agent cc の組み合わせで識別）
        groups.retain(|group| {
            let has_oretachi = group
                .get("hooks")
                .and_then(|hs| hs.as_array())
                .map_or(false, |hs| {
                    hs.iter().any(|h| {
                        h.get("command")
                            .and_then(|c| c.as_str())
                            .map_or(false, |c| c.contains("--notify") && c.contains("--agent cc"))
                    })
                });
            !has_oretachi
        });
    }

    // 空配列になったイベントキーを削除
    hooks_obj.retain(|_, v| v.as_array().map_or(true, |arr| !arr.is_empty()));

    // hooks オブジェクト自体が空になったら削除
    if hooks_obj.is_empty() {
        if let Some(obj) = json.as_object_mut() {
            obj.remove("hooks");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Stop` には通知用 (`--notify`) とターン境界配送用 (`--turn-context`) の
    /// **2つのグループ**が並ぶ（#124）。片方に統合すると、6イベント共有の `--notify` が
    /// レスポンス body を読む必要が出て全イベントの hook が遅くなる。
    #[test]
    fn build_hooks_json_registers_turn_context_alongside_notify() {
        let v = build_hooks_json("X:/bin/oretachi-notify.exe");
        let stop = v["hooks"]["Stop"].as_array().expect("Stop グループ");
        assert_eq!(stop.len(), 2, "通知とターン境界配送で2グループ");

        let args_of = |i: usize| -> Vec<String> {
            stop[i]["hooks"][0]["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_string())
                .collect()
        };
        assert!(args_of(0).contains(&"--notify".to_string()));
        assert!(args_of(0).contains(&"Stop".to_string()));
        let turn = args_of(1);
        assert_eq!(turn[0], "--turn-context");
        assert!(turn.contains(&"${CLAUDE_PROJECT_DIR}".to_string()));
        // --turn-context は event / agent を取らない（--notify とは別系統）
        assert!(!turn.contains(&"--event".to_string()));

        // 他イベントには生えていない（Stop 限定であること）
        for ev in ["Notification", "PreToolUse", "PostToolUse", "SubagentStop"] {
            assert_eq!(v["hooks"][ev].as_array().unwrap().len(), 1, "{} は1グループのまま", ev);
        }
    }

    /// SessionStart / UserPromptSubmit / PermissionRequest の既存構成を壊していないこと。
    #[test]
    fn build_hooks_json_keeps_existing_groups() {
        let v = build_hooks_json("X:/bin/oretachi-notify.exe");
        assert_eq!(v["hooks"]["SessionStart"][0]["hooks"][0]["args"][0], "--session-context");
        assert_eq!(v["hooks"]["UserPromptSubmit"][0]["hooks"][0]["args"][0], "--prompt-context");
        // PermissionRequest は通知 + ExitPlanMode の2グループ
        assert_eq!(v["hooks"]["PermissionRequest"].as_array().unwrap().len(), 2);
    }
}
