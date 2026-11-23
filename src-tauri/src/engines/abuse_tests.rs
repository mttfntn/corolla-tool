//! Hostile input: odd file names, broken files, files whose extension lies, and
//! archives that try to write outside their folder. Ignored by default because
//! they need the engines; run after scripts/fetch-engines:
//!   cargo test --manifest-path src-tauri/Cargo.toml abuse -- --ignored --nocapture

use super::*;
use std::sync::atomic::AtomicBool;

fn tools() -> Tools {
    let os = if cfg!(target_os = "macos") { "macos" } else { "windows" };
    Tools::locate(&[Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries").join(os)])
}

fn convert_to(source: &Path, target: &str, tools: &Tools) -> Result<PathBuf, String> {
    let fraction = AtomicU32::new(0);
    let reporter = Reporter { fraction: &fraction, notify: &|| {} };
    let job = Job::one(source.to_path_buf(), Action::Convert(target.to_string()));
    convert(&job, tools, Quality::Balanced, &AtomicBool::new(false), &reporter)?.path()
}

/// A fresh folder in the temp directory, removed afterwards.
fn scratch(name: &str) -> WorkDir {
    let dir = WorkDir::new().unwrap();
    let inner = dir.0.join(name);
    std::fs::create_dir_all(&inner).unwrap();
    dir
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

fn wav(tools: &Tools, path: &Path) {
    let ffmpeg = tools.ffmpeg.as_ref().expect("FFmpeg");
    let ok = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=duration=0.2"])
        .arg(path)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "making {}", path.display());
}

/// Names that a shell or a tool's option parser could misread still convert,
/// and the result lands beside the source with the same stem.
#[test]
#[ignore]
fn abuse_odd_file_names_convert_beside_the_source() {
    let tools = tools();
    let dir = scratch("names");
    let folder = dir.0.join("names");
    let mut names = vec![
        "-y.wav".to_string(),
        "-i x.wav".to_string(),
        "quote ' and $(touch pwned) and %PATH%.wav".to_string(),
        "semi;colon & ampersand.wav".to_string(),
        "unicode perché 日本 🙂.wav".to_string(),
        "trailing dots....wav".to_string(),
    ];
    if cfg!(windows) {
        // Windows forbids " | < > in names, and its 260-character path limit (temp
        // folder included) is reached before Corolla is involved.
        names.push(format!("{}.wav", "long".repeat(25)));
    } else {
        names.push("double \" quote and | pipe.wav".to_string());
        names.push(format!("{}.wav", "long".repeat(55)));
    }
    for name in &names {
        let source = folder.join(name);
        wav(&tools, &source);
        let output = convert_to(&source, "flac", &tools).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(output.parent(), Some(folder.as_path()), "{name}");
        assert_eq!(output.file_stem(), source.file_stem(), "{name}");
    }
    assert!(!folder.join("pwned").exists() && !Path::new("pwned").exists());
    println!("{:#?}", files_in(&folder));
}

/// A name already at the file system's 255-byte limit can't take a longer
/// extension: the conversion fails with a message and leaves nothing behind.
/// Not on Windows, where the 260-character path limit is reached first.
#[cfg(not(windows))]
#[test]
#[ignore]
fn abuse_name_at_the_length_limit_fails_cleanly() {
    let tools = tools();
    let dir = scratch("long");
    let folder = dir.0.join("long");
    let source = folder.join(format!("{}.mp3", "x".repeat(251)));
    let ffmpeg = tools.ffmpeg.as_ref().expect("FFmpeg");
    assert!(Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=duration=0.2"])
        .arg(&source)
        .status()
        .is_ok_and(|s| s.success()));
    let result = convert_to(&source, "flac", &tools);
    println!("{result:?}");
    assert!(result.is_err());
    assert_eq!(files_in(&folder).len(), 1, "only the source is left");
}

/// Broken files and files whose extension lies fail with a message, and
/// leave no empty or half-written output behind.
#[test]
#[ignore]
fn abuse_broken_and_mislabelled_files_leave_nothing_behind() {
    let tools = tools();
    let dir = scratch("broken");
    let folder = dir.0.join("broken");
    let garbage: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let cases: &[(&str, &[u8], &str)] = &[
        ("empty.png", b"", "jpg"),
        ("garbage.png", &garbage, "jpg"),
        ("garbage.mp4", &garbage, "mp3"),
        ("garbage.pdf", &garbage, "png"),
        ("garbage.zip", &garbage, "tar"),
        ("garbage.ttf", &garbage, "woff2"),
        ("garbage.obj", &garbage, "stl"),
        ("garbage.srt", &garbage, "vtt"),
        ("truncated.pdf", b"%PDF-1.7\n1 0 obj\n<<", "png"),
        ("html.svg", b"<html><script>alert(1)</script></html>", "png"),
    ];
    for (name, bytes, target) in cases {
        std::fs::write(folder.join(name), bytes).unwrap();
        let result = convert_to(&folder.join(name), target, &tools);
        let first_line = match &result {
            Ok(path) => format!("OK {}", path.display()),
            Err(e) => e.lines().next().unwrap_or("").to_string(),
        };
        println!("{name} -> {target}: {first_line}");
        assert!(result.is_err(), "{name} -> {target} claimed success");
    }
    let left = files_in(&folder);
    let originals: Vec<String> = cases.iter().map(|c| c.0.to_string()).collect();
    let extra: Vec<&String> = left.iter().filter(|n| !originals.contains(n)).collect();
    assert!(extra.is_empty(), "left behind: {extra:?}");
}

/// An archive whose entries try to leave the folder: `../`, absolute paths and
/// a symlink to a file outside. Nothing may be written outside the temp
/// folder, and nothing from outside may end up inside the result.
#[test]
#[ignore]
fn abuse_archives_cannot_escape_their_folder() {
    let tools = tools();
    let dir = scratch("escape");
    let folder = dir.0.join("escape");
    let secret = dir.0.join("secret.txt");
    std::fs::write(&secret, "outside contents").unwrap();
    let before_temp: Vec<String> = files_in(&std::env::temp_dir());

    // Built by hand with the zip crate, as no normal tool writes such names.
    let zip_path = folder.join("evil.zip");
    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for name in ["../../escaped-up.txt", "/tmp/escaped-abs.txt", "ok/normal.txt"] {
            zip.start_file(name, options).unwrap();
            std::io::Write::write_all(&mut zip, b"payload").unwrap();
        }
        zip.finish().unwrap();
    }
    let link_zip = folder.join("link.zip");
    {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&link_zip).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("normal.txt", options).unwrap();
        std::io::Write::write_all(&mut zip, b"payload").unwrap();
        zip.add_symlink("link-to-secret", secret.to_str().unwrap(), options).unwrap();
        zip.finish().unwrap();
    }

    for (archive, target) in [(&zip_path, "tar"), (&zip_path, "gzip"), (&link_zip, "tar"), (&link_zip, "zip")] {
        let result = convert_to(archive, target, &tools);
        let name = archive.file_name().unwrap().to_string_lossy();
        println!("{name} -> {target}: {:?}", result.as_ref().map(|p| p.display().to_string()));
        if let Ok(output) = result {
            // List what the result holds, and check the outside file's text isn't in it.
            let seven = tools.sevenzip.as_ref().unwrap();
            let listing = Command::new(seven).args(["l", "-ba"]).arg(&output).output().unwrap();
            println!("{}", String::from_utf8_lossy(&listing.stdout));
            let bytes = std::fs::read(&output).unwrap();
            if target != "gzip" {
                assert!(
                    !bytes.windows(16).any(|w| w == b"outside contents"),
                    "the symlink's target was copied into the archive"
                );
            }
        }
    }
    for escaped in [dir.0.join("escaped-up.txt"), std::env::temp_dir().join("escaped-up.txt"), PathBuf::from("/tmp/escaped-abs.txt")] {
        assert!(!escaped.exists(), "{} was written outside", escaped.display());
    }
    let after_temp: Vec<String> = files_in(&std::env::temp_dir());
    let new_in_temp: Vec<&String> = after_temp.iter().filter(|n| !before_temp.contains(n)).collect();
    println!("new in temp after the run: {new_in_temp:?}");
}

/// A large text file converts in reasonable time and memory.
#[test]
#[ignore]
fn abuse_large_text_to_pdf() {
    let tools = tools();
    let dir = scratch("large");
    let source = dir.0.join("large").join("big.txt");
    let line = "The quick brown fox jumps over the lazy dog, perché è così. 0123456789\n";
    std::fs::write(&source, line.repeat(80_000)).unwrap(); // about 5.6 MB
    let started = std::time::Instant::now();
    let result = convert_to(&source, "pdf", &tools);
    println!(
        "5.6 MB txt -> pdf: {:?} in {:.1} s",
        result.as_ref().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)),
        started.elapsed().as_secs_f64()
    );
    assert!(result.is_ok());
}
