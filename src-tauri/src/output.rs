//! Picks where results go: next to the source, never over an existing file.

use std::fs::OpenOptions;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

/// An empty placeholder holding the final name, so parallel jobs can't collide.
#[derive(Debug)]
pub struct Reserved {
    pub path: PathBuf,
    pub stem: String,
}

const MAX_ATTEMPTS: u32 = 10_000;

fn source_stem(source: &Path) -> String {
    let name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let lower = name.to_lowercase();
    if lower.ends_with(".tar.gz") {
        return name[..name.len() - ".tar.gz".len()].to_string();
    }
    source.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name)
}

/// Reserves `stem<suffix>.extension` beside the source.
pub fn reserve(source: &Path, suffix: &str, extension: &str, companions: &[&str]) -> io::Result<Reserved> {
    let directory = source.parent().unwrap_or(Path::new("."));
    reserve_in(directory, &format!("{}{suffix}", source_stem(source)), extension, companions)
}

/// Reserves `base.extension` in a folder, or `base (1).extension` and so on.
/// A name is skipped if any file in `companions` would also clash.
pub fn reserve_in(directory: &Path, base: &str, extension: &str, companions: &[&str]) -> io::Result<Reserved> {
    for attempt in 0..MAX_ATTEMPTS {
        let stem = if attempt == 0 { base.to_string() } else { format!("{base} ({attempt})") };
        if companions.iter().any(|ext| directory.join(format!("{stem}.{ext}")).exists()) {
            continue;
        }
        let path = directory.join(format!("{stem}.{extension}"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(Reserved { path, stem }),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(ErrorKind::AlreadyExists, "too many files with this name"))
}

fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Rename fails across drives, for instance from the temp folder.
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

fn free_name(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let directory = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let extension = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..MAX_ATTEMPTS)
        .map(|n| directory.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

/// Moves everything an engine wrote into the source's folder. Returns the main
/// output: the reserved file, or if there isn't one (a PDF split into page
/// images), the first file delivered, with the unused placeholder removed.
pub fn deliver(work: &Path, reserved: &Reserved) -> io::Result<PathBuf> {
    let directory = reserved.path.parent().unwrap_or(Path::new("."));
    let main_name = reserved.path.file_name();
    let mut produced: Vec<PathBuf> = std::fs::read_dir(work)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    produced.sort();
    if produced.is_empty() {
        return Err(io::Error::new(ErrorKind::NotFound, "the engine produced nothing"));
    }

    let has_main = produced.iter().any(|path| path.file_name() == main_name);
    let mut first = None;
    if has_main {
        // The placeholder is ours, so replacing it can't lose anyone's data.
        let _ = std::fs::remove_file(&reserved.path);
    }
    for path in &produced {
        let name = path.file_name().unwrap_or_default();
        let destination = if name == main_name.unwrap_or_default() {
            reserved.path.clone()
        } else {
            free_name(&directory.join(name))
        };
        move_file(path, &destination)?;
        first.get_or_insert(destination);
    }
    if has_main {
        return Ok(reserved.path.clone());
    }
    let _ = std::fs::remove_file(&reserved.path);
    first.ok_or_else(|| io::Error::new(ErrorKind::NotFound, "nothing delivered"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("corolla-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn free_name_is_used_as_is() {
        let dir = folder("free");
        let r = reserve(&dir.join("photo.png"), "", "jpg", &[]).unwrap();
        assert_eq!(r.path, dir.join("photo.jpg"));
        assert_eq!(r.stem, "photo");
        assert!(r.path.exists());
    }

    #[test]
    fn suffix_goes_before_the_extension_and_clashes_still_count_up() {
        let dir = folder("suffix");
        let first = reserve(&dir.join("photo.jpg"), "_rsz2", "jpg", &[]).unwrap();
        let second = reserve(&dir.join("photo.jpg"), "_rsz2", "jpg", &[]).unwrap();
        assert_eq!(first.path, dir.join("photo_rsz2.jpg"));
        assert_eq!(second.path, dir.join("photo_rsz2 (1).jpg"));
    }

    #[test]
    fn existing_files_are_never_reused() {
        let dir = folder("clash");
        std::fs::write(dir.join("photo.jpg"), b"precious").unwrap();
        let first = reserve(&dir.join("photo.png"), "", "jpg", &[]).unwrap();
        let second = reserve(&dir.join("photo.png"), "", "jpg", &[]).unwrap();
        assert_eq!(first.path, dir.join("photo (1).jpg"));
        assert_eq!(second.path, dir.join("photo (2).jpg"));
        assert_eq!(std::fs::read(dir.join("photo.jpg")).unwrap(), b"precious");
    }

    #[test]
    fn companion_clash_moves_the_whole_name() {
        let dir = folder("companion");
        std::fs::write(dir.join("model.bin"), b"precious").unwrap();
        let r = reserve(&dir.join("model.fbx"), "", "gltf", &["bin"]).unwrap();
        assert_eq!(r.stem, "model (1)");
        assert_eq!(r.path, dir.join("model (1).gltf"));
    }

    #[test]
    fn tar_gz_stem_drops_both_extensions() {
        let dir = folder("targz");
        let r = reserve(&dir.join("backup.tar.gz"), "", "zip", &[]).unwrap();
        assert_eq!(r.path, dir.join("backup.zip"));
    }

    #[test]
    fn deliver_moves_main_output_and_companions() {
        let dir = folder("deliver");
        let work = folder("deliver-work");
        let r = reserve(&dir.join("m.fbx"), "", "gltf", &["bin"]).unwrap();
        std::fs::write(work.join("m.gltf"), b"json").unwrap();
        std::fs::write(work.join("m.bin"), b"data").unwrap();
        assert_eq!(deliver(&work, &r).unwrap(), dir.join("m.gltf"));
        assert_eq!(std::fs::read(dir.join("m.gltf")).unwrap(), b"json");
        assert_eq!(std::fs::read(dir.join("m.bin")).unwrap(), b"data");
    }

    #[test]
    fn deliver_without_main_output_removes_placeholder() {
        let dir = folder("pages");
        let work = folder("pages-work");
        let r = reserve(&dir.join("doc.pdf"), "", "png", &[]).unwrap();
        std::fs::write(work.join("doc-p1.png"), b"1").unwrap();
        std::fs::write(work.join("doc-p2.png"), b"2").unwrap();
        assert_eq!(deliver(&work, &r).unwrap(), dir.join("doc-p1.png"));
        assert!(dir.join("doc-p2.png").exists());
        assert!(!dir.join("doc.png").exists());
    }

    #[test]
    fn deliver_never_overwrites_an_extra_file() {
        let dir = folder("extra");
        let work = folder("extra-work");
        std::fs::write(dir.join("doc-p1.png"), b"precious").unwrap();
        let r = reserve(&dir.join("doc.pdf"), "", "png", &[]).unwrap();
        std::fs::write(work.join("doc-p1.png"), b"new").unwrap();
        assert_eq!(deliver(&work, &r).unwrap(), dir.join("doc-p1 (1).png"));
        assert_eq!(std::fs::read(dir.join("doc-p1.png")).unwrap(), b"precious");
    }
}
