//! End-to-end: every pair in formats.json through the real engines. Ignored by
//! default because it needs them; run after scripts/fetch-engines:
//!   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture

use super::*;
use super::fonts::Outlines as fonts_outlines;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

fn perform(source: PathBuf, action: Action, tools: &Tools, cancel: &AtomicBool) -> Result<Done, String> {
    run_all(vec![source], action, tools, cancel)
}

fn run_all(sources: Vec<PathBuf>, action: Action, tools: &Tools, cancel: &AtomicBool) -> Result<Done, String> {
    let fraction = AtomicU32::new(0);
    let reporter = Reporter { fraction: &fraction, notify: &|| {} };
    convert(&Job { sources, action }, tools, Quality::Balanced, cancel, &reporter)
}

fn convert_to(source: PathBuf, target: &str, tools: &Tools, cancel: &AtomicBool) -> Result<PathBuf, String> {
    perform(source, Action::Convert(target.to_string()), tools, cancel)?.path()
}

fn bundled_dir() -> PathBuf {
    let os = if cfg!(target_os = "macos") { "macos" } else { "windows" };
    Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries").join(os)
}

fn quiet(program: &Path, args: &[&str]) -> bool {
    let mut command = Command::new(program);
    command.args(args).stdout(Stdio::null()).stderr(Stdio::null());
    command.status().is_ok_and(|s| s.success())
}

const OBJ: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\nf 1 2 4\nf 1 3 4\nf 2 3 4\n";
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><circle cx="32" cy="32" r="30" fill="orange"/></svg>"#;
const SRT: &str = "1\n00:00:01,000 --> 00:00:02,000\nHello\n";

/// Makes one sample of every format, using the conversions verified before it.
fn make_samples(tools: &Tools, dir: &Path) -> HashMap<String, PathBuf> {
    let mut samples = HashMap::new();
    let path = |name: &str| dir.join(name);

    std::fs::write(path("sample.obj"), OBJ).unwrap();
    std::fs::write(path("sample.svg"), SVG).unwrap();
    std::fs::write(path("sample.srt"), SRT).unwrap();
    std::fs::write(path("sample.vtt"), "WEBVTT\n\n00:01.000 --> 00:02.000\nHello\n").unwrap();
    std::fs::write(path("sample.txt"), "Hello there\nperché è così\n").unwrap();
    std::fs::write(path("payload.txt"), "archive contents").unwrap();
    for name in ["obj", "svg", "srt", "vtt", "txt"] {
        samples.insert(name.to_string(), path(&format!("sample.{name}")));
    }

    let ffmpeg = tools.ffmpeg.as_ref().expect("FFmpeg");
    let generate = |input: &[&str], name: &str, extra: &[&str]| {
        let out = path(name);
        let mut args = vec!["-hide_banner", "-loglevel", "error", "-y"];
        args.extend_from_slice(input);
        args.extend_from_slice(extra);
        args.push(out.to_str().unwrap());
        assert!(quiet(ffmpeg, &args), "generating {name}");
        out
    };
    let tone = ["-f", "lavfi", "-i", "sine=frequency=440:duration=1"];
    samples.insert("wav".into(), generate(&tone, "sample.wav", &[]));
    let clip = [
        "-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=1",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
    ];
    samples.insert("mp4".into(), generate(&clip, "sample.mp4", &["-pix_fmt", "yuv420p", "-shortest"]));
    let frame = ["-f", "lavfi", "-i", "testsrc=size=160x120:rate=1:duration=1"];
    samples.insert("png".into(), generate(&frame, "sample.png", &["-frames:v", "1", "-update", "1"]));

    if let Some(seven) = &tools.sevenzip {
        let payload = path("payload.txt");
        let zip = path("sample.zip");
        assert!(quiet(seven, &["a", "-tzip", zip.to_str().unwrap(), payload.to_str().unwrap()]));
        samples.insert("zip".into(), zip);
    }
    samples
}

/// Faces in the sample OBJ, which every model conversion must preserve.
const SAMPLE_FACES: u64 = 4;
const STEP_FACES: u64 = 12;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(name)
}

fn model_faces(assimp: &Path, model: &Path) -> Option<u64> {
    let result = Command::new(assimp).arg("info").arg(model).output().ok()?;
    let text = String::from_utf8_lossy(&result.stdout).to_string();
    text.lines()
        .find_map(|line| line.trim().strip_prefix("Faces:"))
        .and_then(|count| count.trim().parse().ok())
}

fn is_readable(tools: &Tools, from: &str, target: &str, output: &Path) -> Result<(), String> {
    signature::check_file(target, output)?;
    let file = output.to_str().unwrap();
    let family = super::family(target);
    let readable = match (target, family) {
        (_, Some("audio" | "video")) => {
            let probe = tools.ffmpeg.as_ref().unwrap().with_file_name(format!("ffprobe{EXE}"));
            quiet(&probe, &["-v", "error", file])
        }
        (_, Some("model")) => {
            let faces = model_faces(tools.assimp.as_ref().expect("Assimp"), output);
            // The sample STEP is a cube: six sides of two triangles.
            let expected = if from == "step" { STEP_FACES } else { SAMPLE_FACES };
            if faces != Some(expected) {
                return Err(format!("expected {expected} faces, found {faces:?}: {file}"));
            }
            true
        }
        (_, Some("image")) if target != "pdf" && target != "svg" => match &tools.magick {
            Some(magick) => quiet(magick, &["identify", file]),
            None => true,
        },
        (_, Some("font")) => {
            let bytes = std::fs::read(output).unwrap();
            let bare = match target {
                "woff" => wuff::decompress_woff1(&bytes).ok(),
                "woff2" => wuff::decompress_woff2(&bytes).ok(),
                _ => Some(bytes),
            };
            bare.is_some_and(|sfnt| ttf_parser::Face::parse(&sfnt, 0).is_ok())
        }
        ("docx", _) => zip::ZipArchive::new(std::fs::File::open(output).unwrap())
            .is_ok_and(|mut a| a.by_name("word/document.xml").is_ok()),
        ("pdf", _) => std::fs::read(output).is_ok_and(|b| b.starts_with(b"%PDF")),
        (_, Some("archive")) => match &tools.sevenzip {
            Some(seven) => quiet(seven, &["t", file]),
            None => true,
        },
        _ => std::fs::metadata(output).is_ok_and(|m| m.len() > 0),
    };
    if readable {
        Ok(())
    } else {
        Err(format!("output isn't readable: {file}"))
    }
}

/// Every source format, derived from an already verified sample when needed.
fn sample_for(
    format: &str,
    samples: &mut HashMap<String, PathBuf>,
    tools: &Tools,
    dir: &Path,
) -> Option<PathBuf> {
    if let Some(existing) = samples.get(format) {
        return Some(existing.clone());
    }
    let matrix = formats::matrix();
    let family = &matrix.formats.get(format)?.family;
    let seed = match family.as_str() {
        "audio" => "wav",
        "video" => "mp4",
        "image" | "pdf" => "png",
        "archive" => "zip",
        "model" => "obj",
        "font" => "ttf",
        _ => return None,
    };
    let seed_path = samples.get(seed)?.clone();
    let folder = dir.join(format!("seed-{format}"));
    std::fs::create_dir_all(&folder).unwrap();
    let copy = folder.join(seed_path.file_name()?);
    std::fs::copy(&seed_path, &copy).ok()?;
    let made = convert_to(copy, format, tools, &AtomicBool::new(false)).ok()?;
    samples.insert(format.to_string(), made.clone());
    Some(made)
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn every_pair_in_the_matrix_converts() {
    let tools = Tools::locate(&[bundled_dir()]);
    println!("{tools:#?}");
    let dir = std::env::temp_dir().join(format!("corolla-matrix-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut samples = make_samples(&tools, &dir);
    // No engine here can write HEIC, so reading it needs a real photo.
    let heic = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/sample.heic");
    let has_heic = heic.is_file();
    if has_heic {
        samples.insert("heic".into(), heic);
    }
    samples.insert("step".into(), fixture("sample.step"));
    samples.insert("ttf".into(), fixture("font-static.ttf"));
    samples.insert("otf".into(), fixture("font-cff.otf"));
    let mut untested = Vec::new();

    let matrix = formats::matrix();
    let mut names: Vec<&String> = matrix.formats.keys().collect();
    names.sort();
    let (mut passed, mut failed, mut hidden) = (0, Vec::new(), Vec::new());
    for from in names {
        let mut targets = matrix.formats[from].targets.clone();
        targets.sort();
        for target in targets {
            if !can_convert(&tools, from, &target) {
                hidden.push(format!("{from} -> {target}"));
                continue;
            }
            let label = format!("{from} -> {target}");
            let changes_outlines = matches!(from.as_str(), "woff" | "woff2") && target == "otf";
            if changes_outlines && tools.fontforge.is_none() {
                hidden.push(format!("{label} (needs FontForge)"));
                continue;
            }
            if from == "heic" && !has_heic && !can_convert(&tools, "png", "heic") {
                untested.push(label);
                continue;
            }
            let Some(sample) = sample_for(from, &mut samples, &tools, &dir) else {
                failed.push(format!("{label}: couldn't make a sample of {from}"));
                continue;
            };
            let folder = dir.join(format!("{from}-to-{target}"));
            std::fs::create_dir_all(&folder).unwrap();
            let source = folder.join(sample.file_name().unwrap());
            std::fs::copy(&sample, &source).unwrap();
            // A .gltf or .obj is unusable without the files written beside it.
            for extension in ["bin", "mtl"] {
                let companion = sample.with_extension(extension);
                if companion.exists() {
                    std::fs::copy(&companion, folder.join(companion.file_name().unwrap())).unwrap();
                }
            }
            let before = std::fs::read(&source).unwrap();
            let outcome = convert_to(source.clone(), &target, &tools, &AtomicBool::new(false))
                .and_then(|out| is_readable(&tools, from, &target, &out));
            assert_eq!(std::fs::read(&source).unwrap(), before, "{label} changed its source");
            match outcome {
                Ok(()) => passed += 1,
                Err(error) => failed.push(format!("{label}: {error}")),
            }
        }
    }
    println!("passed {passed}, hidden for missing engines: {hidden:?}");
    println!("untested, no sample available: {untested:?}");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failed.is_empty(), "{} failed:\n{}", failed.len(), failed.join("\n"));
}

#[test]
fn builtin_conversions_work_without_any_engine() {
    let tools = Tools::default();
    let dir = WorkDir::new().unwrap();
    let txt = dir.0.join("note.txt");
    std::fs::write(&txt, "Hello\nWorld\n").unwrap();
    let srt = dir.0.join("subs.srt");
    std::fs::write(&srt, SRT).unwrap();
    let svg = dir.0.join("logo.svg");
    std::fs::write(&svg, SVG).unwrap();
    let off = AtomicBool::new(false);
    let cases = [
        (&txt, "pdf", "note.pdf"),
        (&txt, "png", "note.png"),
        (&txt, "jpg", "note.jpg"),
        (&txt, "srt", "note.srt"),
        (&txt, "vtt", "note.vtt"),
        (&srt, "vtt", "subs.vtt"),
        (&srt, "txt", "subs.txt"),
        (&svg, "png", "logo.png"),
        (&svg, "pdf", "logo.pdf"),
    ];
    for (source, target, expected) in cases {
        let out = convert_to(source.clone(), target, &tools, &off)
            .unwrap_or_else(|e| panic!("{} -> {target}: {e}", source.display()));
        assert_eq!(out, dir.0.join(expected));
        assert!(std::fs::metadata(&out).unwrap().len() > 0, "{expected} is empty");
    }
    assert!(std::fs::read(dir.0.join("note.pdf")).unwrap().starts_with(b"%PDF"));
    assert!(std::fs::read(dir.0.join("logo.pdf")).unwrap().starts_with(b"%PDF"));
    assert!(std::fs::read_to_string(dir.0.join("subs.vtt")).unwrap().starts_with("WEBVTT"));
    let (w, h) = {
        let size = imagesize::size(dir.0.join("note.jpg")).unwrap();
        (size.width, size.height)
    };
    assert!(w >= 1024 && h > 0);
}

#[test]
fn failure_leaves_nothing_behind() {
    let tools = Tools::default();
    let dir = WorkDir::new().unwrap();
    let srt = dir.0.join("empty.srt");
    std::fs::write(&srt, "not subtitles").unwrap();
    assert!(convert_to(srt.clone(), "vtt", &tools, &AtomicBool::new(false)).is_err());
    let names: Vec<_> = std::fs::read_dir(&dir.0).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names.len(), 1, "only the source should remain: {names:?}");
}

#[test]
fn nothing_is_offered_without_its_engine() {
    let tools = Tools::default();
    assert!(!can_convert(&tools, "mp3", "wav"));
    assert!(!can_convert(&tools, "zip", "tar"));
    assert!(!can_convert(&tools, "fbx", "glb"));
    assert!(!can_convert(&tools, "pdf", "txt"));
    assert!(can_convert(&tools, "srt", "vtt"));
    let magick_only = Tools { magick: Some("magick".into()), ..Default::default() };
    assert!(can_convert(&magick_only, "png", "webp"));
    assert!(can_convert(&magick_only, "heic", "png"), "reading HEIC is fine");
    assert!(!can_convert(&magick_only, "png", "heic"), "this ImageMagick can't write HEIC");
    let no_rar = Tools { sevenzip: Some("7z".into()), ..Default::default() };
    assert!(can_convert(&no_rar, "zip", "tar"));
    assert!(!can_convert(&no_rar, "zip", "rar"));
    assert!(can_convert(&no_rar, "rar", "zip"), "7-Zip can read RAR");
}

#[test]
fn cancelled_job_does_nothing() {
    let dir = WorkDir::new().unwrap();
    let srt = dir.0.join("a.srt");
    std::fs::write(&srt, SRT).unwrap();
    assert!(convert_to(srt, "vtt", &Tools::default(), &AtomicBool::new(true)).is_err());
    assert!(!dir.0.join("a.vtt").exists());
}

fn engines_or_skip() -> Tools {
    Tools::locate(&[bundled_dir()])
}

fn tool(op: Op, suffix: &str) -> Action {
    Action::Process { label: "T".into(), suffix: suffix.into(), op }
}

/// A photo-like picture at high quality, so compressing it has something to gain.
fn noisy_image(tools: &Tools, dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    let out = dir.join(name);
    let master = dir.join(format!("master-{name}.png"));
    let source = format!("gradients=size={width}x{height}:nb_colors=6,noise=alls=25:allf=t");
    let ok = quiet(tools.ffmpeg.as_ref().expect("FFmpeg"), &[
        "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &source,
        "-frames:v", "1", "-update", "1", master.to_str().unwrap(),
    ]);
    assert!(ok, "generating {name}");
    let magick = tools.magick.as_ref().expect("ImageMagick");
    assert!(quiet(magick, &[master.to_str().unwrap(), "-quality", "95", out.to_str().unwrap()]), "encoding {name}");
    std::fs::remove_file(&master).unwrap();
    out
}

/// Marks a JPEG as "rotate 90 degrees to view", the way a phone held upright does.
fn mark_sideways(jpeg: &Path) {
    let data = std::fs::read(jpeg).unwrap();
    let tiff = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
    let mut marked = data[..2].to_vec();
    marked.extend([0xFF, 0xE1]);
    marked.extend(((tiff.len() + 8) as u16).to_be_bytes());
    marked.extend(b"Exif  ");
    marked.extend(tiff);
    marked.extend(&data[2..]);
    std::fs::write(jpeg, marked).unwrap();
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn resize_halves_and_quarters_exactly_and_keeps_the_format() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    for extension in ["jpg", "png", "webp", "tiff", "bmp"] {
        let source = noisy_image(&tools, &dir.0, &format!("r.{extension}"), 400, 200);
        for (by, suffix, want) in [(2, "_rsz2", (200, 100)), (4, "_rsz4", (100, 50))] {
            let out = perform(source.clone(), tool(Op::ResizeDivide { by }, suffix), &tools, &off).unwrap().path().unwrap();
            assert_eq!(out.file_name().unwrap().to_str().unwrap(), format!("r{suffix}.{extension}"));
            signature::check_file(extension, &out).unwrap();
            let size = imagesize::size(&out).unwrap();
            assert_eq!((size.width, size.height), want, "{extension} /{by}");
        }
    }
    let source = noisy_image(&tools, &dir.0, "edge.jpg", 400, 200);
    let out = perform(source.clone(), tool(Op::ResizeEdge { edge: 100 }, "_e"), &tools, &off).unwrap().path().unwrap();
    assert_eq!(imagesize::size(&out).unwrap().width, 100);
    let out = perform(source, tool(Op::ResizeEdge { edge: 4000 }, "_big"), &tools, &off).unwrap().path().unwrap();
    assert_eq!(imagesize::size(&out).unwrap().width, 400, "never enlarged");
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn resize_applies_a_photos_rotation() {
    let tools = engines_or_skip();
    let dir = WorkDir::new().unwrap();
    let source = noisy_image(&tools, &dir.0, "rotated.jpg", 400, 200);
    mark_sideways(&source);
    let off = AtomicBool::new(false);
    let out = perform(source.clone(), tool(Op::ResizeDivide { by: 2 }, "_rsz2"), &tools, &off).unwrap().path().unwrap();
    let size = imagesize::size(&out).unwrap();
    assert_eq!((size.width, size.height), (100, 200), "sideways photo stood up");

    // The same photo placed in a PDF must not come out sideways either.
    let pdf = perform(source, Action::Convert("pdf".into()), &tools, &off).unwrap().path().unwrap();
    let pdfium = pdfium_render::prelude::Pdfium::bind_to_library(tools.pdfium.as_ref().expect("PDFium"))
        .map(pdfium_render::prelude::Pdfium::new)
        .unwrap();
    let document = pdfium.load_pdf_from_file(&pdf, None).unwrap();
    let page = document.pages().get(0).unwrap();
    assert!(page.height().value > page.width().value, "PDF page is upright");
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn compress_shrinks_images_and_leaves_their_size_alone() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    for extension in ["jpg", "png", "webp", "tiff", "bmp", "avif"] {
        let source = noisy_image(&tools, &dir.0, &format!("c.{extension}"), 600, 400);
        let before = std::fs::read(&source).unwrap();
        let result = perform(source.clone(), tool(Op::Compress { quality: 60, max_edge: None }, "_cmprss"), &tools, &off).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), before, "{extension} source changed");
        let Done::Output { path, .. } = result else { panic!("{extension} didn't shrink") };
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), format!("c_cmprss.{extension}"));
        signature::check_file(extension, &path).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() < before.len() as u64, "{extension}");
        let size = imagesize::size(&path).unwrap();
        assert_eq!((size.width, size.height), (600, 400), "{extension}");
    }
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn compress_that_gains_nothing_writes_nothing() {
    let tools = engines_or_skip();
    let dir = WorkDir::new().unwrap();
    let source = noisy_image(&tools, &dir.0, "small.png", 16, 16);
    let tiny = dir.0.join("tiny.jpg");
    assert!(quiet(tools.magick.as_ref().unwrap(), &[source.to_str().unwrap(), "-strip", "-quality", "1", tiny.to_str().unwrap()]));
    let result = perform(tiny, tool(Op::Compress { quality: 100, max_edge: None }, "_c"), &tools, &AtomicBool::new(false)).unwrap();
    assert_eq!(result, Done::NotSmaller);
    assert!(!dir.0.join("tiny_c.jpg").exists(), "the placeholder must be gone too");
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn compress_shrinks_video_and_keeps_its_container() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    let ffmpeg = tools.ffmpeg.as_ref().expect("FFmpeg");
    let probe = ffmpeg.with_file_name(format!("ffprobe{EXE}"));
    for extension in ["mp4", "mov", "mkv", "webm", "avi", "wmv", "gif"] {
        let source = dir.0.join(format!("v.{extension}"));
        assert!(quiet(ffmpeg, &[
            "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
            "testsrc2=size=640x360:rate=15:duration=2,noise=alls=30:allf=t", "-q:v", "1", "-qscale:v", "1",
            source.to_str().unwrap(),
        ]), "generating {extension}");
        let fraction = AtomicU32::new(0);
        let reporter = Reporter { fraction: &fraction, notify: &|| {} };
        let job = Job::one(source.clone(), tool(Op::Compress { quality: 40, max_edge: Some(320) }, "_cmprss"));
        let result = convert(&job, &tools, Quality::Balanced, &off, &reporter).unwrap();
        let Done::Output { path, .. } = result else { panic!("{extension} didn't shrink") };
        signature::check_file(extension, &path).unwrap();
        assert!(quiet(&probe, &["-v", "error", path.to_str().unwrap()]), "{extension} unreadable");
        assert!(fraction.load(Ordering::Relaxed) > 500, "{extension} reported no progress");
        let dims = Command::new(&probe)
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width", "-of", "csv=p=0"])
            .arg(&path)
            .output()
            .unwrap();
        let width: u32 = String::from_utf8_lossy(&dims.stdout).trim().trim_end_matches(',').parse().unwrap();
        assert_eq!(width, 320, "{extension} wasn't limited");
    }
}

fn pdf_pages_and_text(tools: &Tools, path: &Path) -> (usize, String) {
    let pdfium = pdfium_render::prelude::Pdfium::bind_to_library(tools.pdfium.as_ref().expect("PDFium"))
        .map(pdfium_render::prelude::Pdfium::new)
        .unwrap();
    let document = pdfium.load_pdf_from_file(path, None).unwrap();
    let mut text = String::new();
    for page in document.pages().iter() {
        text += &page.text().unwrap().all();
    }
    (document.pages().len() as usize, text)
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn images_combine_into_one_pdf_in_natural_order() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let album = WorkDir::new().unwrap().0.clone();
    let dir = album.join("Holiday");
    std::fs::create_dir_all(&dir).unwrap();
    let mut sources = Vec::new();
    for (name, width) in [("img10.jpg", 300), ("img2.png", 200), ("img1.webp", 100)] {
        sources.push(noisy_image(&tools, &dir, name, width, 100));
    }
    let broken = dir.join("img3.jpg");
    std::fs::write(&broken, b"this is not a picture").unwrap();
    sources.push(broken.clone());

    let result = run_all(sources, Action::CombinePdf, &tools, &off).unwrap();
    let Done::Output { path, skipped } = result else { panic!("nothing written") };
    assert_eq!(path, dir.join("Holiday.pdf"));
    assert_eq!(skipped.len(), 1, "the broken file is reported");
    assert_eq!(skipped[0].0, broken);
    signature::check_file("pdf", &path).unwrap();
    let pdfium = pdfium_render::prelude::Pdfium::bind_to_library(tools.pdfium.as_ref().expect("PDFium"))
        .map(pdfium_render::prelude::Pdfium::new)
        .unwrap();
    let document = pdfium.load_pdf_from_file(&path, None).unwrap();
    let widths: Vec<i32> = document.pages().iter().map(|p| (p.width().value * 150.0 / 72.0).round() as i32).collect();
    assert_eq!(widths, [100, 200, 300], "img1, img2, img10");
    drop(document);
    drop(pdfium);
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names.len(), 5, "no temporary files left: {names:?}");
    let _ = std::fs::remove_dir_all(&album);
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn pdf_compress_shrinks_pictures_and_keeps_the_text() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    // A page of real text, then big pictures: PDFium can't merge, so lopdf joins them.
    let note = dir.0.join("note.txt");
    std::fs::write(&note, "The quick brown fox\n").unwrap();
    let text_pdf = convert_to(note, "pdf", &tools, &off).unwrap();
    let raw = dir.0.join("big.png");
    assert!(quiet(tools.ffmpeg.as_ref().unwrap(), &[
        "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
        "gradients=size=3600x2400:nb_colors=4,noise=alls=12:allf=t",
        "-frames:v", "1", "-update", "1", raw.to_str().unwrap(),
    ]));
    let picture_pdf = convert_to(raw, "pdf", &tools, &off).unwrap();

    // Text plus picture on separate pages of one document.
    let mut text_doc = lopdf::Document::load(&text_pdf).unwrap();
    let picture_doc = lopdf::Document::load(&picture_pdf).unwrap();
    let merged = dir.0.join("merged.pdf");
    merge_for_test(&mut text_doc, picture_doc);
    text_doc.save(&merged).unwrap();
    assert_eq!(pdf_pages_and_text(&tools, &merged).0, 2);

    let before = std::fs::metadata(&merged).unwrap().len();
    let result = perform(merged.clone(), tool(Op::Compress { quality: 50, max_edge: None }, "_cmprss"), &tools, &off).unwrap();
    let Done::Output { path, .. } = result else { panic!("didn't shrink") };
    assert_eq!(path, dir.0.join("merged_cmprss.pdf"));
    let after = std::fs::metadata(&path).unwrap().len();
    assert!(after < before, "{before} -> {after}");
    let (pages, text) = pdf_pages_and_text(&tools, &path);
    assert_eq!(pages, 2);
    assert!(text.contains("quick brown fox"), "text survived: {text:?}");
    println!("pdf compress: {before} -> {after} bytes");
}

/// Appends another document's pages, enough to build a mixed test PDF.
fn merge_for_test(into: &mut lopdf::Document, mut other: lopdf::Document) {
    use lopdf::Object;
    other.renumber_objects_with(into.max_id + 1);
    into.max_id = other.max_id;
    let other_pages: Vec<lopdf::ObjectId> = other.get_pages().into_values().collect();
    let root = into.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let pages_id = into.get_object(root).unwrap().as_dict().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
    for (id, object) in other.objects {
        into.objects.insert(id, object);
    }
    for page in &other_pages {
        into.get_object_mut(*page).unwrap().as_dict_mut().unwrap().set("Parent", Object::Reference(pages_id));
    }
    let pages = into.get_object_mut(pages_id).unwrap().as_dict_mut().unwrap();
    let mut kids = pages.get(b"Kids").unwrap().as_array().unwrap().clone();
    kids.extend(other_pages.iter().map(|p| Object::Reference(*p)));
    let count = kids.len() as i64;
    pages.set("Kids", kids);
    pages.set("Count", count);
}

#[test]
fn compress_is_never_offered_where_it_cant_work() {
    let compress = Op::Compress { quality: 70, max_edge: None };
    let none = Tools::default();
    assert!(can_process(&none, "pdf", &compress), "built in");
    assert!(!can_process(&none, "mp4", &compress));
    assert!(!can_process(&none, "png", &compress));
    let magick = Tools { magick: Some("m".into()), ..Default::default() };
    assert!(can_process(&magick, "png", &compress));
    assert!(can_process(&magick, "png", &Op::ResizeDivide { by: 2 }));
    assert!(!can_process(&magick, "svg", &compress));
    assert!(!can_process(&magick, "heic", &compress), "this ImageMagick can't write HEIC");
    assert!(!can_process(&magick, "pdf", &Op::ResizeDivide { by: 2 }));
    assert!(!can_process(&magick, "mp3", &compress));
    assert!(can_process(&magick, "exr", &compress), "ZIP loses nothing");
    assert!(!can_process(&magick, "ico", &Op::ResizeDivide { by: 2 }));
    assert!(!can_convert(&none, "png", "ico") && can_convert(&magick, "png", "ico"));
    assert!(!can_convert(&magick, "ico", "heic"), "still no HEIC writer");
    assert!(!can_process(&magick, "hdr", &compress), "HDR has nothing more to offer");
    assert!(can_process(&magick, "hdr", &Op::ResizeDivide { by: 2 }));
    assert!(can_combine(&none, &["jpg".to_string()]));
    assert!(!can_combine(&none, &["jpg".to_string(), "png".to_string()]));
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn step_mesh_detail_follows_the_quality_setting() {
    let tools = engines_or_skip();
    let assimp = tools.assimp.as_ref().expect("Assimp");
    let off = AtomicBool::new(false);
    let fraction = AtomicU32::new(0);
    let reporter = Reporter { fraction: &fraction, notify: &|| {} };
    let mut faces = Vec::new();
    for quality in [Quality::Small, Quality::Balanced, Quality::High] {
        let dir = WorkDir::new().unwrap();
        let source = dir.0.join("curved.stp");
        std::fs::copy(fixture("curved.stp"), &source).unwrap();
        let job = Job::one(source, Action::Convert("stl".into()));
        let out = convert(&job, &tools, quality, &off, &reporter).unwrap().path().unwrap();
        faces.push(model_faces(assimp, &out).expect("readable mesh"));
    }
    println!("STEP triangles small, balanced, high: {faces:?}");
    assert!(faces[0] < faces[1] && faces[1] < faces[2], "{faces:?}");
}

/// Asks ImageMagick for a number about an image, such as its brightest value.
fn image_number(tools: &Tools, path: &Path, format: &str) -> f64 {
    let result = Command::new(tools.magick.as_ref().expect("ImageMagick"))
        .args(["identify", "-format", format])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&result.stdout).trim().parse().unwrap_or_else(|_| panic!("{format} of {path:?}"))
}

fn near(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 0.02, "{what}: expected {expected}, got {actual}");
}

/// A flat image of one linear brightness, as an EXR.
fn linear_exr(tools: &Tools, dir: &Path, name: &str, percent: u32) -> PathBuf {
    let out = dir.join(name);
    let colour = format!("xc:rgb({percent}%,{percent}%,{percent}%)");
    assert!(quiet(tools.magick.as_ref().expect("ImageMagick"), &[
        "-size", "64x64", &colour, "-set", "colorspace", "RGB", out.to_str().unwrap(),
    ]));
    out
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn bright_values_survive_between_the_high_dynamic_range_formats() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    let source = linear_exr(&tools, &dir.0, "bright.exr", 200);
    near(image_number(&tools, &source, "%[fx:maxima]"), 2.0, "the sample itself");

    let hdr = convert_to(source.clone(), "hdr", &tools, &off).unwrap();
    signature::check_file("hdr", &hdr).unwrap();
    near(image_number(&tools, &hdr, "%[fx:maxima]"), 2.0, "EXR to HDR");

    let tiff = convert_to(source.clone(), "tiff", &tools, &off).unwrap();
    near(image_number(&tools, &tiff, "%[fx:maxima]"), 2.0, "EXR to TIFF");
    near(image_number(&tools, &tiff, "%z"), 32.0, "TIFF bit depth");

    std::fs::remove_file(&source).unwrap();
    let back = convert_to(hdr.clone(), "exr", &tools, &off).unwrap();
    signature::check_file("exr", &back).unwrap();
    near(image_number(&tools, &back, "%[fx:maxima]"), 2.0, "HDR to EXR");

    // A 32-bit TIFF counts as linear too, so nothing shifts on the way back.
    std::fs::remove_file(&back).unwrap();
    let from_tiff = convert_to(tiff, "exr", &tools, &off).unwrap();
    near(image_number(&tools, &from_tiff, "%[fx:maxima]"), 2.0, "TIFF to EXR");
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn high_dynamic_range_images_look_right_on_ordinary_screens() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    // Half of full light is about 0.735 once it's encoded for a screen.
    let source = linear_exr(&tools, &dir.0, "grey.exr", 50);
    for target in ["jpg", "png", "webp", "bmp"] {
        let out = convert_to(source.clone(), target, &tools, &off).unwrap();
        near(image_number(&tools, &out, "%[fx:mean]"), 0.735, &format!("EXR to {target}"));
    }
    let hdr = convert_to(source.clone(), "hdr", &tools, &off).unwrap();
    let png_dir = dir.0.join("from-hdr");
    std::fs::create_dir_all(&png_dir).unwrap();
    let hdr_copy = png_dir.join("grey.hdr");
    std::fs::copy(&hdr, &hdr_copy).unwrap();
    let png = convert_to(hdr_copy, "png", &tools, &off).unwrap();
    near(image_number(&tools, &png, "%[fx:mean]"), 0.735, "HDR to PNG");

    // And an ordinary image going in and out again ends up where it started.
    let out_dir = dir.0.join("round");
    std::fs::create_dir_all(&out_dir).unwrap();
    let start = out_dir.join("flat.png");
    assert!(quiet(tools.magick.as_ref().unwrap(), &["-size", "64x64", "xc:rgb(50%,50%,50%)", start.to_str().unwrap()]));
    let exr = convert_to(start.clone(), "exr", &tools, &off).unwrap();
    near(image_number(&tools, &exr, "%[fx:mean]"), 0.214, "stored as linear light");
    std::fs::remove_file(&start).unwrap();
    let again = convert_to(exr, "png", &tools, &off).unwrap();
    near(image_number(&tools, &again, "%[fx:mean]"), 0.5, "PNG to EXR to PNG");
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn resizing_high_dynamic_range_images_keeps_their_values() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    let source = linear_exr(&tools, &dir.0, "big.exr", 200);
    let half = tool(Op::ResizeDivide { by: 2 }, "_rsz2");
    let exr = perform(source.clone(), half.clone(), &tools, &off).unwrap().path().unwrap();
    signature::check_file("exr", &exr).unwrap();
    near(image_number(&tools, &exr, "%w"), 32.0, "EXR width");
    near(image_number(&tools, &exr, "%[fx:maxima]"), 2.0, "EXR values");
    let hdr = convert_to(source.clone(), "hdr", &tools, &off).unwrap();
    let small = perform(hdr, half.clone(), &tools, &off).unwrap().path().unwrap();
    near(image_number(&tools, &small, "%w"), 32.0, "HDR width");
    near(image_number(&tools, &small, "%[fx:maxima]"), 2.0, "HDR values");
    let tiff = convert_to(source, "tiff", &tools, &off).unwrap();
    let small = perform(tiff, half, &tools, &off).unwrap().path().unwrap();
    near(image_number(&tools, &small, "%[fx:maxima]"), 2.0, "float TIFF values");
    near(image_number(&tools, &small, "%z"), 32.0, "float TIFF depth");
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn compressing_an_uncompressed_exr_loses_nothing() {
    let tools = engines_or_skip();
    let dir = WorkDir::new().unwrap();
    let start = noisy_image(&tools, &dir.0, "master.png", 400, 300);
    let source = dir.0.join("raw.exr");
    assert!(quiet(tools.magick.as_ref().unwrap(), &[
        start.to_str().unwrap(), "-colorspace", "RGB", "-compress", "None", source.to_str().unwrap(),
    ]));
    let action = tool(Op::Compress { quality: 70, max_edge: None }, "_cmprss");
    let result = perform(source.clone(), action, &tools, &AtomicBool::new(false)).unwrap();
    let Done::Output { path, .. } = result else { panic!("didn't shrink") };
    signature::check_file("exr", &path).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() < std::fs::metadata(&source).unwrap().len());
    let (a, b) = (image_number(&tools, &source, "%[fx:mean]"), image_number(&tools, &path, "%[fx:mean]"));
    assert!((a - b).abs() < 0.0005, "{a} vs {b}");
}

/// The bare font inside any of the four formats.
fn bare_font(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    match path.extension().and_then(|e| e.to_str()) {
        Some("woff") => wuff::decompress_woff1(&bytes).unwrap(),
        Some("woff2") => wuff::decompress_woff2(&bytes).unwrap(),
        _ => bytes,
    }
}

/// Every character a font can draw.
fn characters(path: &Path) -> std::collections::BTreeSet<u32> {
    let sfnt = bare_font(path);
    let face = ttf_parser::Face::parse(&sfnt, 0).unwrap();
    let mut found = std::collections::BTreeSet::new();
    if let Some(cmap) = face.tables().cmap {
        for subtable in cmap.subtables {
            if subtable.is_unicode() {
                subtable.codepoints(|c| {
                    if subtable.glyph_index(c).is_some_and(|g| g.0 != 0) {
                        found.insert(c);
                    }
                });
            }
        }
    }
    found
}

/// Copyright notice and weight, which converters have been known to drop.
fn font_details(path: &Path) -> (String, u16) {
    let sfnt = bare_font(path);
    let face = ttf_parser::Face::parse(&sfnt, 0).unwrap();
    let copyright = face
        .names()
        .into_iter()
        .find(|n| n.name_id == 0 && n.is_unicode())
        .and_then(|n| n.to_string())
        .unwrap_or_default();
    (copyright, face.weight().to_number())
}

/// Glyph count, family, style name and whether it's variable.
fn font_identity(path: &Path) -> (u16, String, String, bool) {
    let sfnt = bare_font(path);
    let face = ttf_parser::Face::parse(&sfnt, 0).unwrap();
    let name = |ids: &[u16]| {
        ids.iter()
            .find_map(|id| {
                face.names()
                    .into_iter()
                    .find(|n| n.name_id == *id && n.is_unicode())
                    .and_then(|n| n.to_string())
            })
            .unwrap_or_default()
    };
    (face.number_of_glyphs(), name(&[16, 1]), name(&[17, 2]), face.is_variable())
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn fonts_keep_their_glyphs_and_names_in_every_format() {
    let tools = engines_or_skip();
    assert!(tools.fontforge.is_some(), "FontForge is needed for this test");
    let off = AtomicBool::new(false);
    for sample in ["font-static.ttf", "font-cff.otf"] {
        let expected = font_identity(&fixture(sample));
        println!("{sample}: {expected:?}");
        for target in ["ttf", "otf", "woff", "woff2"] {
            if sample.ends_with(target) {
                continue;
            }
            let dir = WorkDir::new().unwrap();
            let source = dir.0.join(sample);
            std::fs::copy(fixture(sample), &source).unwrap();
            let out = convert_to(source, target, &tools, &off)
                .unwrap_or_else(|e| panic!("{sample} -> {target}: {e}"));
            signature::check_file(target, &out).unwrap();
            let got = font_identity(&out);
            println!("  -> {target}: {got:?}");
            assert_eq!((&got.1, &got.2), (&expected.1, &expected.2), "{sample} -> {target}: names");
            // TrueType requires a couple of technical glyphs that CFF fonts don't
            // have, so the count may grow, but every character must still be there.
            assert!(got.0 >= expected.0 && got.0 <= expected.0 + 2, "{sample} -> {target}: glyphs {} vs {}", got.0, expected.0);
            assert_eq!(characters(&out), characters(&fixture(sample)), "{sample} -> {target}: characters");
            assert_eq!(font_details(&out), font_details(&fixture(sample)), "{sample} -> {target}: copyright and weight");
            let wanted = match target {
                "ttf" => Some(fonts_outlines::TrueType),
                "otf" => Some(fonts_outlines::Cff),
                _ => None,
            };
            if let Some(wanted) = wanted {
                assert_eq!(fonts::outlines_of(&bare_font(&out)), Some(wanted), "{sample} -> {target}: curves");
            }
        }
    }
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn variable_fonts_stay_whole_or_are_refused() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    let source = dir.0.join("variable.ttf");
    std::fs::copy(fixture("font-variable.ttf"), &source).unwrap();
    let woff2 = convert_to(source.clone(), "woff2", &tools, &off).unwrap();
    assert!(font_identity(&woff2).3, "WOFF2 keeps it variable");
    let refused = convert_to(source, "otf", &tools, &off).unwrap_err();
    assert!(refused.contains("variable"), "{refused}");
    assert!(!dir.0.join("variable.otf").exists(), "nothing left behind");
}

#[test]
#[ignore = "needs the engines from scripts/fetch-engines"]
fn favicons_hold_every_size_square_and_small() {
    let tools = engines_or_skip();
    let off = AtomicBool::new(false);
    let dir = WorkDir::new().unwrap();
    // An opaque picture, like most logos without a transparent background.
    let wide = dir.0.join("logo.png");
    assert!(quiet(tools.ffmpeg.as_ref().unwrap(), &[
        "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=600x300:rate=1:duration=1",
        "-frames:v", "1", "-update", "1", wide.to_str().unwrap(),
    ]));
    let ico = convert_to(wide, "ico", &tools, &off).unwrap();
    signature::check_file("ico", &ico).unwrap();
    let size = std::fs::metadata(&ico).unwrap().len();
    println!("favicon from a 600x300 picture: {size} bytes");
    let magick = tools.magick.as_ref().unwrap();
    let listing = Command::new(magick).args(["identify", "-format", "%wx%h "]).arg(&ico).output().unwrap();
    let mut sizes: Vec<String> = String::from_utf8_lossy(&listing.stdout).split_whitespace().map(String::from).collect();
    sizes.sort_by_key(|s| s.split('x').next().unwrap().parse::<u32>().unwrap());
    assert_eq!(sizes, ["16x16", "32x32", "48x48", "256x256"]);
    // The wide picture is centred: the top and bottom bands stay see-through.
    let corner = Command::new(magick).arg(format!("{}[3]", ico.display())).args(["-format", "%[fx:p{0,0}.a]", "info:"]).output();
    let alpha: String = String::from_utf8_lossy(&corner.unwrap().stdout).trim().to_string();
    let frames = Command::new(magick).args(["identify", "-format", "%w,"]).arg(&ico).output().unwrap();
    let biggest = String::from_utf8_lossy(&frames.stdout).split(',').position(|w| w == "256").unwrap();
    let corner = Command::new(magick).arg(format!("{}[{biggest}]", ico.display())).args(["-format", "%[fx:p{0,0}.a] %[fx:p{128,128}.a]", "info:"]).output().unwrap();
    let corner = String::from_utf8_lossy(&corner.stdout).to_string();
    assert_eq!(corner.trim(), "0 1", "transparent corner, solid middle (first read {alpha})");

    // Back again: the largest size, whichever order the icon stores them in.
    let out = dir.0.join("again");
    std::fs::create_dir_all(&out).unwrap();
    let copy = out.join("logo.ico");
    std::fs::copy(&ico, &copy).unwrap();
    let png = convert_to(copy, "png", &tools, &off).unwrap();
    let back = imagesize::size(&png).unwrap();
    assert_eq!((back.width, back.height), (256, 256));

    let svg = dir.0.join("mark.svg");
    std::fs::write(&svg, SVG).unwrap();
    let from_svg = convert_to(svg, "ico", &tools, &off).unwrap();
    signature::check_file("ico", &from_svg).unwrap();
}
