//! The tray icon and its menu.

use crate::engines::{Quality, QualitySetting};
use crate::queue::Queue;
use crate::settings::{Settings, Store};
use crate::{logfile, wheel::Controller};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

/// The folder of the most recent output, for "Open last output".
#[derive(Default)]
pub struct LastOutput(pub Mutex<Option<PathBuf>>);

/// The check marks, so changes made in the Settings window show in the menu too.
pub struct TrayItems {
    enabled: CheckMenuItem<Wry>,
    login: CheckMenuItem<Wry>,
    quality: [CheckMenuItem<Wry>; 3],
}

impl TrayItems {
    pub fn show(&self, enabled: bool, quality: u8) {
        let _ = self.enabled.set_checked(enabled);
        for (i, item) in self.quality.iter().enumerate() {
            let _ = item.set_checked(i as u8 == quality);
        }
    }

    pub fn show_login(&self, on: bool) {
        let _ = self.login.set_checked(on);
    }
}

/// Shows the Settings window, creating it the first time. On its own thread,
/// because creating a window can wait on the thread a menu event runs on.
pub fn open_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
            .title("Corolla Settings")
            .inner_size(920.0, 680.0)
            .min_inner_size(760.0, 560.0)
            .disable_drag_drop_handler()
            .build();
        if let Err(error) = built {
            logfile::line(format!("Can't open Settings: {error}"));
        }
    });
}

pub fn build(app: &AppHandle, initial: &Settings) -> tauri::Result<()> {
    let enabled = CheckMenuItem::with_id(app, "enabled", "Enabled", true, initial.enabled, None::<&str>)?;
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let login = CheckMenuItem::with_id(app, "login", "Start at login", true, autostart_on, None::<&str>)?;
    let high = CheckMenuItem::with_id(app, "q0", "High", true, initial.quality == 0, None::<&str>)?;
    let balanced = CheckMenuItem::with_id(app, "q1", "Balanced", true, initial.quality == 1, None::<&str>)?;
    let small = CheckMenuItem::with_id(app, "q2", "Small files", true, initial.quality == 2, None::<&str>)?;
    let quality = Submenu::with_items(app, "Quality", true, &[&high, &balanced, &small])?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let cancel = MenuItem::with_id(app, "cancel", "Cancel conversions", true, None::<&str>)?;
    let last = MenuItem::with_id(app, "last", "Open last output", true, None::<&str>)?;
    let logs = MenuItem::with_id(app, "logs", "Open log folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Corolla", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let separator_2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&settings, &enabled, &login, &quality, &separator, &cancel, &last, &logs, &separator_2, &quit],
    )?;

    let quality_items = [high.clone(), balanced.clone(), small.clone()];
    let enabled_item = enabled.clone();
    let login_item = login.clone();
    app.manage(TrayItems { enabled, login, quality: [high, balanced, small] });

    let tray = TrayIconBuilder::with_id("main")
        .tooltip("Corolla: Ctrl+drag files to convert")
        .menu(&menu);
    let tray = match app.default_window_icon() {
        Some(icon) => tray.icon(icon.clone()),
        None => tray,
    };
    tray.on_menu_event(move |app, event| {
        let store = app.state::<Arc<Store>>();
        match event.id().as_ref() {
            "settings" => open_settings(app),
            "enabled" => {
                let on = enabled_item.is_checked().unwrap_or(true);
                app.state::<Arc<Controller>>().enabled.store(on, Ordering::Relaxed);
                store.update(|s| s.enabled = on);
            }
            "login" => {
                let want = login_item.is_checked().unwrap_or(false);
                let autolaunch = app.autolaunch();
                let result = if want { autolaunch.enable() } else { autolaunch.disable() };
                if let Err(error) = result {
                    logfile::line(format!("Can't change start at login: {error}"));
                    let _ = login_item.set_checked(!want);
                }
            }
            id @ ("q0" | "q1" | "q2") => {
                let value = id[1..].parse::<u8>().unwrap_or(1);
                for (i, item) in quality_items.iter().enumerate() {
                    let _ = item.set_checked(i as u8 == value);
                }
                app.state::<Arc<QualitySetting>>().set(Quality::from_u8(value));
                store.update(|s| s.quality = value);
            }
            "cancel" => app.state::<Arc<Queue>>().cancel(),
            "last" => {
                let path = app.state::<LastOutput>().0.lock().unwrap().clone();
                if let Some(path) = path {
                    let _ = app.opener().reveal_item_in_dir(path);
                }
            }
            "logs" => {
                if let Some(directory) = logfile::directory() {
                    let _ = app.opener().open_path(directory.to_string_lossy(), None::<&str>);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        }
    })
    .build(app)?;
    Ok(())
}
