#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engines;
mod formats;
mod gesture;
mod logfile;
mod output;
mod queue;
mod settings;
mod tools;
mod tray;
mod wheel;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tools::{Action, Op};
use tauri::{AppHandle, DragDropEvent, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// Where the engines live: inside the installed app, or the dev folder.
fn engine_folders(app: &AppHandle) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Ok(resources) = app.path().resource_dir() {
        folders.push(resources.join("binaries"));
    }
    if cfg!(debug_assertions) {
        let os = if cfg!(target_os = "macos") { "macos" } else { "windows" };
        folders.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(os));
    }
    folders
}

/// "Converted 3 files to JPG", "Compressed 3 files", and so on.
fn outcome(action: &Action, files: &str) -> String {
    match action {
        Action::Convert(target) => format!("Converted {files} to {}", target.to_uppercase()),
        Action::CombinePdf => format!("Combined {files} into one PDF"),
        Action::Process { op: Op::Compress { .. }, .. } => format!("Compressed {files}"),
        Action::Process { .. } => format!("Resized {files}"),
    }
}

fn summary(report: &queue::BatchReport) -> (String, String) {
    let done = outcome(&report.action, &plural(report.converted));
    let title = if report.failures.is_empty() && report.cancelled == 0 {
        if report.converted == 0 && report.not_smaller > 0 {
            format!("{} already small", plural(report.not_smaller))
        } else {
            done
        }
    } else if report.cancelled > 0 {
        format!("Cancelled: {done}")
    } else {
        format!("{} failed of {}", report.failures.len(), plural(report.converted + report.failures.len()))
    };
    let mut body = String::new();
    if report.skipped > 0 {
        body += &format!("{} already {}, skipped. ", plural(report.skipped), report.action.label());
    }
    if report.not_smaller > 0 && report.converted + report.failures.len() > 0 {
        body += &format!("{} already small, left alone. ", plural(report.not_smaller));
    }
    if let Some((path, error)) = report.failures.first() {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let first_line = error.lines().next().unwrap_or_default();
        body += &format!("{name}: {first_line}. Details in the log (tray menu).");
    } else if let Some(output) = &report.last_output {
        body += &output.parent().map(|p| p.display().to_string()).unwrap_or_default();
    }
    (title, body)
}

fn plural(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

fn main() {
    tauri::Builder::default()
        // Opening the app again while it's running is how you'd look for its settings.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| tray::open_settings(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::default_settings,
            settings::save_settings,
            settings::catalog,
            settings::ring_for,
            settings::get_login,
            settings::set_login,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if let Ok(directory) = app.path().app_log_dir() {
                logfile::init(directory);
            }
            logfile::line("started");

            let settings_path = settings::path_in(&app.path().app_config_dir()?);
            let saved = settings::load(&settings_path);

            let tools = Arc::new(engines::Tools::locate(&engine_folders(app.handle())));
            // The encoder list runs to hundreds of names: log only how many.
            let encoders = tools.ffmpeg_encoders.as_ref().map(Vec::len);
            let listed = engines::Tools { ffmpeg_encoders: None, ..(*tools).clone() };
            logfile::line(format!("engines: {listed:?}, FFmpeg encoders: {encoders:?}"));
            let quality = Arc::new(engines::QualitySetting::default());
            quality.set(engines::Quality::from_u8(saved.quality));
            app.manage(tools.clone());
            app.manage(quality.clone());
            app.manage(tray::LastOutput::default());
            app.manage(Arc::new(settings::Store::new(settings_path, saved.clone())));

            let queue = queue::Queue::start(tools, quality);
            app.manage(queue.clone());
            let controller = wheel::Controller::new(app.handle().clone());
            controller.enabled.store(saved.enabled, Ordering::Relaxed);
            app.manage(controller.clone());

            let on_change = controller.clone();
            let handle = app.handle().clone();
            queue.set_listeners(
                Box::new(move || on_change.on_queue_changed()),
                Box::new(move |report| {
                    for (path, error) in &report.failures {
                        logfile::line(format!("failed: {}: {error}", path.display()));
                    }
                    if report.last_output.is_some() {
                        *handle.state::<tray::LastOutput>().0.lock().unwrap() = report.last_output.clone();
                    }
                    let (title, body) = summary(&report);
                    logfile::line(format!("{title} {body}"));
                    if let Err(error) = handle.notification().builder().title(title).body(body).show() {
                        logfile::line(format!("Can't notify: {error}"));
                    }
                }),
            );

            tray::build(app.handle(), &saved)?;

            let window = app.get_webview_window("wheel").expect("wheel window in config");
            let _ = window.set_visible_on_all_workspaces(true);
            // The window starts visible but off-screen: on Windows the drop
            // handler only attaches properly to a webview created while shown.
            let _ = window.hide();
            let events = controller.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::DragDrop(drag) = event {
                    match drag {
                        DragDropEvent::Enter { paths, .. } => events.on_drag_enter(paths.clone()),
                        DragDropEvent::Over { position } => events.on_drag_over(position.x, position.y),
                        DragDropEvent::Drop { paths, position } => {
                            events.on_drop(paths.clone(), position.x, position.y)
                        }
                        DragDropEvent::Leave => events.on_drag_leave(),
                        _ => {}
                    }
                }
            });

            let signals = controller.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let mut detector = gesture::Detector::new();
                loop {
                    let sample = gesture::sample();
                    if let Some(signal) = detector.step(sample, started.elapsed().as_millis() as u64) {
                        signals.on_signal(signal);
                    }
                    // Check often during a press, rarely otherwise, to stay near 0% CPU.
                    std::thread::sleep(Duration::from_millis(if sample.left { 8 } else { 25 }));
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start Corolla")
        .run(|_, _| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    use queue::BatchReport;

    fn report() -> BatchReport {
        BatchReport { action: Action::Convert("jpg".into()), ..Default::default() }
    }

    #[test]
    fn success_summary_counts_files() {
        let r = BatchReport { converted: 3, last_output: Some("/a/b/c.jpg".into()), ..report() };
        let (title, body) = summary(&r);
        assert_eq!(title, "Converted 3 files to JPG");
        assert!(body.contains('b'));
    }

    #[test]
    fn failure_summary_names_the_first_bad_file() {
        let r = BatchReport {
            converted: 1,
            failures: vec![("/x/bad.png".into(), "broken\nmore detail".into())],
            ..report()
        };
        let (title, body) = summary(&r);
        assert_eq!(title, "1 failed of 2 files");
        assert!(body.contains("bad.png: broken."));
        assert!(!body.contains("more detail"));
    }

    #[test]
    fn skipped_files_are_mentioned() {
        let r = BatchReport { skipped: 1, ..report() };
        assert!(summary(&r).1.contains("1 file already JPG"));
    }

    /// The version is written in three places; a release tag must match all of them.
    #[test]
    fn every_file_states_the_same_version() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |file: &str| std::fs::read_to_string(root.join(file)).unwrap();
        let json_version = |file: &str| {
            let value: serde_json::Value = serde_json::from_str(&read(file)).unwrap();
            value["version"].as_str().unwrap().to_string()
        };
        let cargo = env!("CARGO_PKG_VERSION");
        assert_eq!(json_version("tauri.conf.json"), cargo, "src-tauri/tauri.conf.json");
        assert_eq!(json_version("../package.json"), cargo, "package.json");
        let lock: serde_json::Value = serde_json::from_str(&read("../package-lock.json")).unwrap();
        assert_eq!(lock["version"].as_str(), Some(cargo), "package-lock.json");
    }

    #[test]
    fn tools_are_described_in_words() {
        let compress = Action::Process {
            op: Op::Compress { quality: 70, max_edge: None },
            label: "CMPRSS".into(),
            suffix: "_cmprss".into(),
        };
        let r = BatchReport { converted: 2, action: compress.clone(), ..report() };
        assert_eq!(summary(&r).0, "Compressed 2 files");
        let r = BatchReport { not_smaller: 2, action: compress, ..report() };
        assert_eq!(summary(&r).0, "2 files already small");
        let r = BatchReport { converted: 5, action: Action::CombinePdf, ..report() };
        assert_eq!(summary(&r).0, "Combined 5 files into one PDF");
    }
}
