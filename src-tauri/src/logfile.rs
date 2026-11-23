//! A plain append-only log, so failures can be read after the fact.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static PATH: OnceLock<PathBuf> = OnceLock::new();
static WRITE: Mutex<()> = Mutex::new(());

/// Sets the log folder (the app's log directory). Until this runs, `line` does nothing.
pub fn init(directory: PathBuf) {
    let _ = std::fs::create_dir_all(&directory);
    let _ = PATH.set(directory.join("corolla.log"));
}

/// The log folder, for the tray's "Open log folder".
pub fn directory() -> Option<PathBuf> {
    PATH.get()?.parent().map(PathBuf::from)
}

/// Appends one line, prefixed with Unix time in seconds. Errors are ignored:
/// logging must never stop a conversion.
pub fn line(message: impl AsRef<str>) {
    let Some(path) = PATH.get() else { return };
    let _guard = WRITE.lock();
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = writeln!(file, "{secs:.3} {}", message.as_ref());
    }
}
