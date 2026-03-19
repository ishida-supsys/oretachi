use crate::ai_provider::AiAgentKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default, rename = "execScript")]
    pub exec_script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "repositoryId")]
    pub repository_id: String,
    #[serde(rename = "repositoryName")]
    pub repository_name: String,
    pub path: String,
    #[serde(rename = "branchName")]
    pub branch_name: String,
    #[serde(default, rename = "hotkeyChar")]
    pub hotkey_char: Option<String>,
    #[serde(default, rename = "autoApproval")]
    pub auto_approval: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyBinding {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub meta: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    pub key: String,
}

fn default_terminal_next() -> HotkeyBinding {
    HotkeyBinding { ctrl: true, meta: false, shift: false, alt: false, key: "Tab".to_string() }
}

fn default_terminal_prev() -> HotkeyBinding {
    HotkeyBinding { ctrl: true, meta: false, shift: true, alt: false, key: "Tab".to_string() }
}

fn default_terminal_add() -> HotkeyBinding {
    if cfg!(target_os = "macos") {
        HotkeyBinding { ctrl: false, meta: true, shift: false, alt: false, key: "t".to_string() }
    } else {
        HotkeyBinding { ctrl: true, meta: false, shift: false, alt: false, key: "t".to_string() }
    }
}

fn default_terminal_close() -> HotkeyBinding {
    if cfg!(target_os = "macos") {
        HotkeyBinding { ctrl: false, meta: true, shift: false, alt: false, key: "w".to_string() }
    } else {
        HotkeyBinding { ctrl: true, meta: false, shift: false, alt: false, key: "w".to_string() }
    }
}

fn default_tray_next() -> HotkeyBinding {
    if cfg!(target_os = "macos") {
        HotkeyBinding { ctrl: false, meta: true, shift: false, alt: false, key: "n".to_string() }
    } else {
        HotkeyBinding { ctrl: true, meta: false, shift: false, alt: false, key: "n".to_string() }
    }
}

fn default_home_tab() -> HotkeyBinding {
    HotkeyBinding { ctrl: false, meta: false, shift: false, alt: true, key: "0".to_string() }
}

fn default_add_task() -> HotkeyBinding {
    if cfg!(target_os = "macos") {
        HotkeyBinding { ctrl: false, meta: true, shift: true, alt: false, key: "n".to_string() }
    } else {
        HotkeyBinding { ctrl: true, meta: false, shift: true, alt: false, key: "n".to_string() }
    }
}

fn default_global_tray_popup() -> HotkeyBinding {
    if cfg!(target_os = "macos") {
        HotkeyBinding { ctrl: false, meta: true, shift: true, alt: false, key: "o".to_string() }
    } else {
        HotkeyBinding { ctrl: true, meta: false, shift: true, alt: false, key: "o".to_string() }
    }
}

// 旧フォーマット (string) と新フォーマット (HotkeyBinding object) の両方を受け入れる
fn deserialize_global_tray_popup<'de, D>(deserializer: D) -> Result<HotkeyBinding, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(_) | serde_json::Value::Null => Ok(default_global_tray_popup()),
        serde_json::Value::Object(_) => {
            serde_json::from_value(value).map_err(D::Error::custom)
        }
        _ => Ok(default_global_tray_popup()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeySettings {
    #[serde(
        default = "default_global_tray_popup",
        rename = "globalTrayPopup",
        deserialize_with = "deserialize_global_tray_popup"
    )]
    pub global_tray_popup: HotkeyBinding,
    #[serde(default = "default_terminal_next", rename = "terminalNext")]
    pub terminal_next: HotkeyBinding,
    #[serde(default = "default_terminal_prev", rename = "terminalPrev")]
    pub terminal_prev: HotkeyBinding,
    #[serde(default = "default_terminal_add", rename = "terminalAdd")]
    pub terminal_add: HotkeyBinding,
    #[serde(default = "default_terminal_close", rename = "terminalClose")]
    pub terminal_close: HotkeyBinding,
    #[serde(default = "default_tray_next", rename = "trayNext")]
    pub tray_next: HotkeyBinding,
    #[serde(default = "default_home_tab", rename = "homeTab", alias = "focusMainWindow")]
    pub home_tab: HotkeyBinding,
    #[serde(default = "default_add_task", rename = "addTask")]
    pub add_task: HotkeyBinding,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        HotkeySettings {
            global_tray_popup: default_global_tray_popup(),
            terminal_next: default_terminal_next(),
            terminal_prev: default_terminal_prev(),
            terminal_add: default_terminal_add(),
            terminal_close: default_terminal_close(),
            tray_next: default_tray_next(),
            home_tab: default_home_tab(),
            add_task: default_add_task(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSettings {
    #[serde(default = "default_font_size", rename = "fontSize")]
    pub font_size: u32,
    #[serde(default)]
    pub shell: Option<String>,
}

fn default_font_size() -> u32 {
    14
}

impl Default for TerminalSettings {
    fn default() -> Self {
        TerminalSettings {
            font_size: default_font_size(),
            shell: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiAgentSettings {
    #[serde(default)]
    pub approval_agent: Option<AiAgentKind>,
    #[serde(default)]
    pub task_add_agent: Option<AiAgentKind>,
    #[serde(default)]
    pub remote_exec: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDefaults {
    #[serde(default)]
    pub open_in_sub_window: bool,
    #[serde(default)]
    pub auto_approval: bool,
}

fn default_monaco_font_size() -> u32 { 13 }
fn default_monaco_minimap() -> bool { true }
fn default_monaco_word_wrap() -> String { "off".to_string() }
fn default_monaco_line_numbers() -> String { "on".to_string() }
fn default_chat_hotkey() -> HotkeyBinding {
    HotkeyBinding { ctrl: true, meta: false, shift: false, alt: false, key: "l".to_string() }
}
fn default_auto_open_review_on_diff() -> bool { true }
fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    #[serde(default = "default_true", rename = "enableAcrylic")]
    pub enable_acrylic: bool,
    #[serde(default, rename = "acrylicOpacity")]
    pub acrylic_opacity: Option<u8>,
    #[serde(default, rename = "acrylicColor")]
    pub acrylic_color: Option<String>,
    #[serde(default, rename = "enableGamingBorder")]
    pub enable_gaming_border: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        AppearanceSettings { enable_acrylic: true, acrylic_opacity: None, acrylic_color: None, enable_gaming_border: false }
    }
}

fn default_notification_volume() -> u32 { 80 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSoundSettings {
    #[serde(default = "default_notification_volume")]
    pub volume: u32, // 0-100
    #[serde(default)]
    pub approval: Option<String>, // None=音なし, "system:<filename>", "custom:<filename>"
    #[serde(default)]
    pub completed: Option<String>,
    #[serde(default)]
    pub general: Option<String>,
}

impl Default for NotificationSoundSettings {
    fn default() -> Self {
        NotificationSoundSettings {
            volume: default_notification_volume(),
            approval: None,
            completed: None,
            general: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeReviewSettings {
    #[serde(default = "default_monaco_font_size", rename = "monacoFontSize")]
    pub monaco_font_size: u32,
    #[serde(default = "default_monaco_minimap", rename = "monacoMinimap")]
    pub monaco_minimap: bool,
    #[serde(default = "default_monaco_word_wrap", rename = "monacoWordWrap")]
    pub monaco_word_wrap: String,
    #[serde(default = "default_monaco_line_numbers", rename = "monacoLineNumbers")]
    pub monaco_line_numbers: String,
    #[serde(default = "default_chat_hotkey", rename = "chatHotkey")]
    pub chat_hotkey: HotkeyBinding,
    #[serde(default = "default_auto_open_review_on_diff", rename = "autoOpenReviewOnDiff")]
    pub auto_open_review_on_diff: bool,
}

impl Default for CodeReviewSettings {
    fn default() -> Self {
        CodeReviewSettings {
            monaco_font_size: default_monaco_font_size(),
            monaco_minimap: default_monaco_minimap(),
            monaco_word_wrap: default_monaco_word_wrap(),
            monaco_line_numbers: default_monaco_line_numbers(),
            chat_hotkey: default_chat_hotkey(),
            auto_open_review_on_diff: default_auto_open_review_on_diff(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub repositories: Vec<Repository>,
    #[serde(rename = "worktreeBaseDir")]
    pub worktree_base_dir: String,
    pub worktrees: Vec<WorktreeEntry>,
    #[serde(default)]
    pub terminal: TerminalSettings,
    #[serde(default)]
    pub hotkeys: HotkeySettings,
    #[serde(default, rename = "alwaysOnTop")]
    pub always_on_top: bool,
    #[serde(default, rename = "enableOsNotification")]
    pub enable_os_notification: bool,
    #[serde(default, rename = "autoAssignHotkey")]
    pub auto_assign_hotkey: bool,
    #[serde(default, rename = "detachedWorktreeIds")]
    pub detached_worktree_ids: Vec<String>,
    #[serde(default, rename = "focusMainOnEmptyTray")]
    pub focus_main_on_empty_tray: bool,
    #[serde(default, rename = "aiAgent")]
    pub ai_agent: Option<AiAgentSettings>,
    #[serde(default, rename = "worktreeDefaults")]
    pub worktree_defaults: Option<WorktreeDefaults>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default, rename = "codeReview")]
    pub code_review: Option<CodeReviewSettings>,
    #[serde(default)]
    pub appearance: Option<AppearanceSettings>,
    #[serde(default, rename = "notificationSound")]
    pub notification_sound: Option<NotificationSoundSettings>,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            repositories: Vec::new(),
            worktree_base_dir: String::new(),
            worktrees: Vec::new(),
            terminal: TerminalSettings::default(),
            hotkeys: HotkeySettings::default(),
            always_on_top: false,
            enable_os_notification: false,
            auto_assign_hotkey: false,
            detached_worktree_ids: Vec::new(),
            focus_main_on_empty_tray: false,
            ai_agent: None,
            worktree_defaults: None,
            locale: None,
            code_review: None,
            appearance: None,
            notification_sound: None,
        }
    }
}

pub struct SettingsManager {
    settings: Mutex<AppSettings>,
    file_path: Mutex<Option<PathBuf>>,
}

impl SettingsManager {
    pub fn new() -> Self {
        SettingsManager {
            settings: Mutex::new(AppSettings::default()),
            file_path: Mutex::new(None),
        }
    }

    pub fn init(&self, app_handle: &AppHandle) {
        let path = app_handle
            .path()
            .app_data_dir()
            .expect("app_data_dir not available")
            .join("settings.json");

        let settings = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
                Err(_) => AppSettings::default(),
            }
        } else {
            AppSettings::default()
        };

        *self.settings.lock().unwrap() = settings;
        *self.file_path.lock().unwrap() = Some(path);
    }

    pub fn get(&self) -> AppSettings {
        self.settings.lock().unwrap().clone()
    }

    pub fn save(&self, settings: AppSettings) -> Result<(), String> {
        let path_guard = self.file_path.lock().unwrap();
        let path = path_guard.as_ref().ok_or("Settings not initialized")?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Dir create error: {}", e))?;
        }

        let json =
            serde_json::to_string_pretty(&settings).map_err(|e| format!("JSON error: {}", e))?;

        std::fs::write(path, json).map_err(|e| format!("Write error: {}", e))?;

        *self.settings.lock().unwrap() = settings;
        Ok(())
    }
}

/// Windowsのシステムサウンドファイル一覧を返す (C:\Windows\Media\*.wav)
#[tauri::command]
pub fn list_system_sounds() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        let media_dir = std::path::Path::new(r"C:\Windows\Media");
        if let Ok(entries) = std::fs::read_dir(media_dir) {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let path = e.path();
                    if path.extension().and_then(|x| x.to_str()) == Some("wav") {
                        path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect();
            names.sort();
            return names;
        }
    }
    Vec::new()
}

/// カスタム通知音ファイルをアプリデータディレクトリにコピーし、コピー後のファイル名を返す
#[tauri::command]
pub async fn copy_custom_sound(app_handle: tauri::AppHandle, source_path: String) -> Result<String, String> {
    let dest_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("notification-sounds");
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;

    let ext = std::path::Path::new(&source_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("wav")
        .to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = format!("{}.{}", ts, ext);
    let dest = dest_dir.join(&filename);
    std::fs::copy(&source_path, &dest).map_err(|e| e.to_string())?;
    Ok(filename)
}

/// 音声ファイルをバイト列として読み込み、base64エンコードして返す
#[tauri::command]
pub async fn read_audio_file(app_handle: tauri::AppHandle, sound: String) -> Result<String, String> {
    let path = if let Some(filename) = sound.strip_prefix("system:") {
        std::path::PathBuf::from(format!(r"C:\Windows\Media\{}", filename))
    } else if let Some(filename) = sound.strip_prefix("custom:") {
        app_handle
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("notification-sounds")
            .join(filename)
    } else {
        return Err("Invalid sound prefix".to_string());
    };

    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_settings_default_round_trip() {
        let settings = AppSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.repositories.len(), settings.repositories.len());
        assert_eq!(restored.worktrees.len(), settings.worktrees.len());
        assert_eq!(restored.always_on_top, settings.always_on_top);
        assert_eq!(restored.terminal.font_size, settings.terminal.font_size);
    }

    #[test]
    fn test_settings_missing_optional_fields_use_defaults() {
        let json = r#"{"repositories": [], "worktreeBaseDir": "", "worktrees": []}"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert!(!settings.always_on_top);
        assert!(!settings.enable_os_notification);
        assert_eq!(settings.terminal.font_size, 14);
        assert!(settings.terminal.shell.is_none());
    }

    #[test]
    fn test_deserialize_old_global_tray_popup_string() {
        let json = r#"{
            "repositories": [], "worktreeBaseDir": "", "worktrees": [],
            "hotkeys": {"globalTrayPopup": "Ctrl+Shift+O"}
        }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        let expected = default_global_tray_popup();
        assert_eq!(settings.hotkeys.global_tray_popup.key, expected.key);
    }

    #[test]
    fn test_deserialize_global_tray_popup_null() {
        let json = r#"{
            "repositories": [], "worktreeBaseDir": "", "worktrees": [],
            "hotkeys": {"globalTrayPopup": null}
        }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        let expected = default_global_tray_popup();
        assert_eq!(settings.hotkeys.global_tray_popup.key, expected.key);
    }

    #[test]
    fn test_deserialize_global_tray_popup_object() {
        let json = r#"{
            "repositories": [], "worktreeBaseDir": "", "worktrees": [],
            "hotkeys": {"globalTrayPopup": {"ctrl": true, "meta": false, "shift": false, "alt": false, "key": "F1"}}
        }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.hotkeys.global_tray_popup.key, "F1");
        assert!(settings.hotkeys.global_tray_popup.ctrl);
    }

    #[test]
    fn test_hotkey_settings_default_terminal_next() {
        let hotkeys = HotkeySettings::default();
        assert_eq!(hotkeys.terminal_next.key, "Tab");
        assert!(hotkeys.terminal_next.ctrl);
        assert!(!hotkeys.terminal_next.shift);
    }

    #[test]
    fn test_hotkey_settings_default_terminal_prev() {
        let hotkeys = HotkeySettings::default();
        assert_eq!(hotkeys.terminal_prev.key, "Tab");
        assert!(hotkeys.terminal_prev.ctrl);
        assert!(hotkeys.terminal_prev.shift);
    }

    #[test]
    fn test_repository_exec_script_defaults_to_none() {
        let json = r#"{"id": "1", "name": "repo", "path": "/path"}"#;
        let repo: Repository = serde_json::from_str(json).unwrap();
        assert!(repo.exec_script.is_none());
    }
}
