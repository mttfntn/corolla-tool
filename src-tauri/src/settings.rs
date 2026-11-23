//! What's remembered between launches, and the commands the Settings window uses.

use crate::engines::{self, Quality, QualitySetting};
use crate::tools::{self, CustomTool, Item, Layout, Op};
use crate::tray::TrayItems;
use crate::wheel::{self, Controller, Ring};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// 0 high, 1 balanced, 2 small; see engines::Quality.
    pub quality: u8,
    pub tools: Vec<CustomTool>,
    /// By the keys of `tools::layout_key`.
    pub layout: Layout,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true, quality: 1, tools: tools::default_tools(), layout: Layout::new() }
    }
}

const COMPRESS_FAMILIES: [&str; 3] = ["image", "pdf", "video"];
const LABEL_LIMIT: usize = 8;
/// Smaller than this and a picture isn't useful, and it protects against typos.
const MIN_EDGE: u32 = 16;
const MAX_EDGE: u32 = 65_535;

fn clean_label(text: &str) -> String {
    text.trim().to_uppercase().chars().take(LABEL_LIMIT).collect()
}

impl Settings {
    /// Whatever the settings file or window says, the rest of the app can trust this.
    pub fn sanitized(mut self) -> Self {
        self.quality = self.quality.min(2);
        let mut seen: Vec<String> = Vec::new();
        let mut kept = Vec::new();
        for mut tool in std::mem::take(&mut self.tools) {
            tool.line1 = clean_label(&tool.line1);
            tool.line2 = clean_label(&tool.line2);
            let allowed: &[&str] = match tool.op {
                Op::Compress { .. } => &COMPRESS_FAMILIES,
                _ => &["image"],
            };
            tool.families.retain(|family| allowed.contains(&family.as_str()));
            tool.families.dedup();
            if tool.line1.is_empty() || tool.families.is_empty() {
                continue;
            }
            tool.op = match tool.op {
                Op::Compress { quality, max_edge } => Op::Compress {
                    quality: quality.clamp(1, 100),
                    max_edge: max_edge.filter(|edge| *edge > 0).map(|edge| edge.clamp(MIN_EDGE, MAX_EDGE)),
                },
                Op::ResizeDivide { by } => Op::ResizeDivide { by: by.clamp(2, 64) },
                Op::ResizeEdge { edge } => Op::ResizeEdge { edge: edge.clamp(MIN_EDGE, MAX_EDGE) },
            };
            // An id doubles as the name of a circle, so it can't be a format's.
            let taken = |id: &str, seen: &[String]| {
                id.is_empty()
                    || id == "combine"
                    || crate::formats::matrix().formats.contains_key(id)
                    || seen.iter().any(|s| s == id)
            };
            let mut number = kept.len();
            while taken(&tool.id, &seen) {
                number += 1;
                tool.id = format!("custom-{number}");
            }
            seen.push(tool.id.clone());
            kept.push(tool);
        }
        self.tools = kept;
        self
    }
}

/// Where settings live: `settings.json` in the app's config folder.
pub fn path_in(directory: &Path) -> PathBuf {
    directory.join("settings.json")
}

/// A missing file just means defaults. A damaged one means defaults too, but its
/// presets are kept aside rather than overwritten by the next save.
pub fn load(path: &Path) -> Settings {
    let Ok(bytes) = std::fs::read(path) else { return Settings::default() };
    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(settings) => settings.sanitized(),
        Err(_) => {
            let _ = std::fs::copy(path, path.with_extension("damaged.json"));
            Settings::default()
        }
    }
}

/// Goes through a temporary file, so a crash mid-save can't wipe the presets.
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
    std::fs::rename(&temporary, path)
}

/// The settings in use, and where they're saved.
pub struct Store {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl Store {
    /// Holds `initial` in memory; `update` writes every change to `path`.
    pub fn new(path: PathBuf, initial: Settings) -> Self {
        Self { path, current: Mutex::new(initial) }
    }

    /// A copy of the current settings.
    pub fn get(&self) -> Settings {
        self.current.lock().unwrap().clone()
    }

    /// Changes the settings and saves them at once. A failed save is logged,
    /// and the change still applies until Corolla quits.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) {
        let mut current = self.current.lock().unwrap();
        change(&mut current);
        if let Err(error) = save(&self.path, &current) {
            crate::logfile::line(format!("Can't save settings: {error}"));
        }
    }
}

/// Settings window: the current settings.
#[tauri::command]
pub fn get_settings(store: State<Arc<Store>>) -> Settings {
    store.get()
}

/// Settings window: the defaults, for "Reset".
#[tauri::command]
pub fn default_settings() -> Settings {
    Settings::default()
}

/// Applies at once: the next drag uses it. Returns what was actually kept.
#[tauri::command]
pub fn save_settings(app: AppHandle, settings: Settings) -> Settings {
    let settings = settings.sanitized();
    app.state::<Arc<Store>>().update(|current| *current = settings.clone());
    app.state::<Arc<QualitySetting>>().set(Quality::from_u8(settings.quality));
    app.state::<Arc<Controller>>().enabled.store(settings.enabled, Ordering::Relaxed);
    app.state::<TrayItems>().show(settings.enabled, settings.quality);
    settings
}

/// Settings window: every circle of a family, in the user's order. HEIC is
/// left out when nothing on this machine can write it.
#[tauri::command]
pub fn catalog(family: String, store: State<Arc<Store>>, tools_found: State<Arc<engines::Tools>>) -> Vec<Item> {
    let settings = store.get();
    let mut items = tools::catalog(&family, &settings.tools, &settings.layout);
    // Nothing on this machine can write HEIC if `heic` isn't a possible target of `png`.
    if family == "image" && !engines::can_convert(&tools_found, "png", "heic") {
        items.retain(|item| item.id != "heic");
    }
    items
}

/// The circle layout for a number of circles, so the preview matches the wheel.
#[tauri::command]
pub fn ring_for(count: usize) -> Ring {
    wheel::ring(count)
}

/// Settings window: whether Corolla starts at login.
#[tauri::command]
pub fn get_login(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Settings window: switches start at login on or off, and updates the tray.
#[tauri::command]
pub fn set_login(app: AppHandle, on: bool) -> Result<bool, String> {
    let autolaunch = app.autolaunch();
    let result = if on { autolaunch.enable() } else { autolaunch.disable() };
    result.map_err(|e| e.to_string())?;
    app.state::<TrayItems>().show_login(on);
    Ok(on)
}

#[cfg(test)]
fn layout_with(family: &str, order: &[&str]) -> Layout {
    let mut layout = Layout::new();
    layout.insert(
        family.to_string(),
        tools::FamilyLayout { order: order.iter().map(|s| s.to_string()).collect(), hidden: Vec::new() },
    );
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compress(id: &str, quality: u8) -> CustomTool {
        CustomTool {
            id: id.into(),
            line1: "cmprss".into(),
            line2: "60".into(),
            op: Op::Compress { quality, max_edge: Some(0) },
            families: vec!["image".into(), "audio".into()],
        }
    }

    #[test]
    fn round_trips_and_survives_bad_files() {
        let dir = crate::engines::WorkDir::new().unwrap();
        let path = path_in(&dir.0);
        assert_eq!(load(&path), Settings::default());
        let custom = Settings {
            enabled: false,
            quality: 2,
            tools: vec![tools::default_tools().remove(0)],
            layout: layout_with("image", &["rsz2"]),
        };
        save(&path, &custom).unwrap();
        assert_eq!(load(&path), custom);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, b"{ broken").unwrap();
        assert_eq!(load(&path), Settings::default());
        assert!(path.with_extension("damaged.json").exists(), "bad file kept aside");
    }

    #[test]
    fn settings_from_the_old_version_get_the_new_tools() {
        let dir = crate::engines::WorkDir::new().unwrap();
        let path = path_in(&dir.0);
        std::fs::write(&path, br#"{"enabled":true,"quality":0}"#).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.quality, 0);
        assert_eq!(loaded.tools, tools::default_tools());
    }

    #[test]
    fn nonsense_is_cleaned_up() {
        let settings = Settings {
            quality: 9,
            tools: vec![
                compress("a", 200),
                compress("a", 0),
                compress("png", 50),
                CustomTool {
                    id: "b".into(),
                    line1: "  ".into(),
                    line2: String::new(),
                    op: Op::ResizeDivide { by: 2 },
                    families: vec!["image".into()],
                },
                CustomTool {
                    id: "c".into(),
                    line1: "rsz".into(),
                    line2: String::new(),
                    op: Op::ResizeDivide { by: 1 },
                    families: vec!["pdf".into()],
                },
                CustomTool {
                    id: "d".into(),
                    line1: "averylonglabel".into(),
                    line2: String::new(),
                    op: Op::ResizeEdge { edge: 1 },
                    families: vec!["image".into()],
                },
            ],
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(settings.quality, 2);
        assert_eq!(settings.tools.len(), 4, "the blank label and PDF resize are dropped");
        let first = &settings.tools[0];
        assert_eq!((first.line1.as_str(), first.line2.as_str()), ("CMPRSS", "60"));
        assert_eq!(first.op, Op::Compress { quality: 100, max_edge: None });
        assert_eq!(first.families, ["image"], "compress can't apply to audio");
        assert_eq!(settings.tools[1].op, Op::Compress { quality: 1, max_edge: None });
        let ids: Vec<&str> = settings.tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids[0], "a");
        assert_ne!(ids[1], "a", "ids stay unique");
        assert_ne!(ids[2], "png", "an id can't be a format");
        assert_eq!(settings.tools[3].line1, "AVERYLON");
        assert_eq!(settings.tools[3].op, Op::ResizeEdge { edge: MIN_EDGE });
    }
}
