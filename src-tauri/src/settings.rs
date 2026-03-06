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

fn default_focus_main_window() -> HotkeyBinding {
    HotkeyBinding { ctrl: false, meta: false, shift: false, alt: true, key: "m".to_string() }
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
    #[serde(default = "default_focus_main_window", rename = "focusMainWindow")]
    pub focus_main_window: HotkeyBinding,
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
            focus_main_window: default_focus_main_window(),
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
