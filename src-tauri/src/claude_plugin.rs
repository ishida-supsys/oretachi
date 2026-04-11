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

/// 起動時にプラグインファイル群を生成・更新する
/// - ディレクトリ構造の作成
/// - plugin.json: env.ORETACHI_APP_PATH を現在のexeパスで更新
/// - hooks/hooks.json: 全イベントのフック定義
/// - .mcp.json はポート確定後に update_mcp_config で書き込むため、ここでは生成しない
pub fn generate_plugin_files(app_handle: &AppHandle) -> Result<(), String> {
    let plugin_dir = plugin_dir(app_handle)?;
    let claude_plugin_dir = plugin_dir.join(".claude-plugin");
    let hooks_dir = plugin_dir.join("hooks");

    std::fs::create_dir_all(&claude_plugin_dir)
        .map_err(|e| format!("Failed to create .claude-plugin dir: {}", e))?;
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Failed to create hooks dir: {}", e))?;

    let exe_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "oretachi".to_string());

    // plugin.json
    let plugin_json = build_plugin_json(&exe_path);
    let plugin_json_path = claude_plugin_dir.join("plugin.json");
    std::fs::write(
        &plugin_json_path,
        serde_json::to_string_pretty(&plugin_json)
            .map_err(|e| format!("Failed to serialize plugin.json: {}", e))?,
    )
    .map_err(|e| format!("Failed to write plugin.json: {}", e))?;

    // hooks/hooks.json
    let hooks_json = build_hooks_json();
    let hooks_json_path = hooks_dir.join("hooks.json");
    std::fs::write(
        &hooks_json_path,
        serde_json::to_string_pretty(&hooks_json)
            .map_err(|e| format!("Failed to serialize hooks.json: {}", e))?,
    )
    .map_err(|e| format!("Failed to write hooks.json: {}", e))?;

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
        serde_json::json!({ "description": "Worktree name for notifications" }),
    );
    for (_, key) in EVENT_CONFIG_KEYS {
        user_config.insert(
            key.to_string(),
            serde_json::json!({ "description": format!("Notification kind for {} event", key) }),
        );
    }

    serde_json::json!({
        "name": PLUGIN_NAME,
        "description": "oretachi worktree notification hooks & MCP server",
        "hooks": "./hooks/hooks.json",
        "mcpServers": "./.mcp.json",
        "env": {
            "ORETACHI_APP_PATH": exe_path
        },
        "userConfig": user_config
    })
}

fn build_hooks_json() -> serde_json::Value {
    // ${ORETACHI_APP_PATH} は plugin.json の env フィールドで定義された環境変数。
    // ${user_config.XXX} は Claude Code プラグイン仕様のユーザー設定値展開構文。
    // 各ワークツリーの pluginConfigs.oretachi@oretachi.options から値が注入される。
    let mut hooks = serde_json::Map::new();
    for (event, key) in EVENT_CONFIG_KEYS {
        let command = format!(
            "\"${{ORETACHI_APP_PATH}}\" --notify \"${{user_config.worktree_name}}\" --kind ${{user_config.{}}} --agent cc",
            key
        );
        hooks.insert(
            event.to_string(),
            serde_json::json!([{
                "matcher": "",
                "hooks": [{ "type": "command", "command": command }]
            }]),
        );
    }
    serde_json::json!({ "hooks": hooks })
}

fn build_mcp_json(port: u16, api_key: &str) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            PLUGIN_NAME: {
                "type": "streamableHttp",
                "url": format!("http://127.0.0.1:{}/mcp", port),
                "headers": {
                    "x-api-key": api_key
                }
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
