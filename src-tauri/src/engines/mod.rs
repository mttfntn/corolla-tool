//! Routes a conversion to the engine that can do it, and runs it so that a
//! failure leaves nothing behind and nothing existing is ever overwritten.

mod archives;
mod docx;
pub(crate) mod fonts;
mod images;
mod media;
mod mesh3d;
mod pdf;
mod pdf_compress;
mod pdf_writer;
mod signature;
mod step;
mod subtitles;
mod textpages;

use crate::tools::{self, Action, Op};
use crate::{formats, output};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// One unit of work. Only `Action::CombinePdf` has more than one source.
pub struct Job {
    pub sources: Vec<PathBuf>,
    pub action: Action,
}

impl Job {
    pub fn one(source: PathBuf, action: Action) -> Self {
        Self { sources: vec![source], action }
    }

    /// How many files this job counts for in the wheel's X/Y.
    pub fn units(&self) -> usize {
        self.sources.len()
    }
}

/// What a job ended with.
#[derive(Debug, PartialEq)]
pub enum Done {
    Output {
        path: PathBuf,
        /// Files left out of a combined PDF, with why.
        skipped: Vec<(PathBuf, String)>,
    },
    /// Compressing wouldn't have made it smaller, so no file was written.
    NotSmaller,
}

impl Done {
    #[cfg(test)]
    pub fn path(self) -> Result<PathBuf, String> {
        match self {
            Done::Output { path, .. } => Ok(path),
            Done::NotSmaller => Err("not smaller".into()),
        }
    }
}

/// Lets an engine report how far along a job is. Fractions are stored as 1/1000s
/// and the listener only hears about steps of half a percent.
pub struct Reporter<'a> {
    pub fraction: &'a AtomicU32,
    pub notify: &'a (dyn Fn() + Sync),
}

impl Reporter<'_> {
    pub fn set(&self, fraction: f64) {
        let value = (fraction.clamp(0.0, 1.0) * 1000.0) as u32;
        let old = self.fraction.swap(value, Ordering::Relaxed);
        if value / 5 != old / 5 {
            (self.notify)();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Quality {
    High,
    Balanced,
    Small,
}

impl Quality {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Quality::High,
            2 => Quality::Small,
            _ => Quality::Balanced,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Quality::High => 0,
            Quality::Balanced => 1,
            Quality::Small => 2,
        }
    }
}

pub struct QualitySetting(AtomicU8);

impl Default for QualitySetting {
    fn default() -> Self {
        Self(AtomicU8::new(Quality::Balanced.as_u8()))
    }
}

impl QualitySetting {
    pub fn get(&self) -> Quality {
        Quality::from_u8(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, quality: Quality) {
        self.0.store(quality.as_u8(), Ordering::Relaxed);
    }
}

/// Engines found on this machine: bundled first, then whatever is installed.
#[derive(Default, Clone, Debug)]
pub struct Tools {
    pub ffmpeg: Option<PathBuf>,
    pub magick: Option<PathBuf>,
    pub sips: Option<PathBuf>,
    pub sevenzip: Option<PathBuf>,
    pub rar: Option<PathBuf>,
    pub assimp: Option<PathBuf>,
    pub blender: Option<PathBuf>,
    pub soffice: Option<PathBuf>,
    pub pdfium: Option<PathBuf>,
    pub mayo: Option<PathBuf>,
    pub freecad: Option<PathBuf>,
    pub fontforge: Option<PathBuf>,
    /// The encoders FFmpeg reports. None if that couldn't be asked: then every
    /// FFmpeg target is offered, as if it had them all.
    pub ffmpeg_encoders: Option<Vec<String>>,
}

const EXE: &str = std::env::consts::EXE_SUFFIX;

/// Homebrew's folders (Apple silicon, Intel). An app opened from the Finder
/// doesn't get the shell's PATH, so tools installed with Homebrew would
/// otherwise never be found.
#[cfg(target_os = "macos")]
const EXTRA_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];
#[cfg(not(target_os = "macos"))]
const EXTRA_DIRS: &[&str] = &[];

fn on_path(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{EXE}");
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(EXTRA_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// Asks FFmpeg for its encoders. Builds differ: Homebrew's, for one, has no
/// Vorbis, WebP or AOM AV1 encoder.
fn ffmpeg_encoders(ffmpeg: &Path) -> Option<Vec<String>> {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-encoders"]).stdin(Stdio::null()).stderr(Stdio::null());
    hide_console(&mut command);
    let output = command.output().ok().filter(|o| o.status.success())?;
    let encoders = parse_encoders(&String::from_utf8_lossy(&output.stdout));
    (!encoders.is_empty()).then_some(encoders)
}

/// Lines look like " A....D libmp3lame   libmp3lame MP3 (MPEG audio layer 3)".
fn parse_encoders(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let flags = words.next()?;
            let name = words.next()?;
            let is_entry = flags.len() == 6 && matches!(flags.as_bytes()[0], b'V' | b'A' | b'S') && name != "=";
            is_entry.then(|| name.to_string())
        })
        .collect()
}

/// The FFmpeg encoders a target needs beyond the ones every build has.
/// Kept in step with media.rs (audio and video) and images.rs (WebP, AVIF).
fn encoders_needed(target: &str) -> &'static [&'static str] {
    match target {
        "mp3" | "avi" => &["libmp3lame"],
        "ogg" => &["libvorbis"],
        "opus" => &["libopus"],
        "mp4" | "mov" | "mkv" => &["libx264"],
        "webm" => &["libvpx-vp9", "libopus"],
        "webp" => &["libwebp"],
        "avif" => &["libaom-av1"],
        _ => &[],
    }
}

/// Blender installs into a versioned folder and isn't normally on PATH.
fn find_blender() -> Option<PathBuf> {
    if let Some(found) = on_path("blender") {
        return Some(found);
    }
    #[cfg(target_os = "macos")]
    {
        let app = PathBuf::from("/Applications/Blender.app/Contents/MacOS/Blender");
        app.is_file().then_some(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let root = Path::new("C:\\Program Files\\Blender Foundation");
        let mut versions: Vec<PathBuf> = std::fs::read_dir(root)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join("blender.exe"))
            .filter(|path| path.is_file())
            .collect();
        versions.sort();
        versions.pop()
    }
}

fn find_known(paths: &[&str]) -> Option<PathBuf> {
    paths.iter().map(PathBuf::from).find(|p| p.is_file())
}

impl Tools {
    /// `bundled` holds the engines shipped with the app.
    pub fn locate(bundled: &[PathBuf]) -> Self {
        let find = |name: &str| {
            let file = format!("{name}{EXE}");
            bundled
                .iter()
                .map(|dir| dir.join(&file))
                .find(|path| path.is_file())
                .or_else(|| on_path(name))
        };
        let pdfium_file = if cfg!(target_os = "macos") { "libpdfium.dylib" } else { "pdfium.dll" };
        let ffmpeg = find("ffmpeg");
        Self {
            ffmpeg_encoders: ffmpeg.as_deref().and_then(ffmpeg_encoders),
            ffmpeg,
            magick: find("magick"),
            sips: if cfg!(target_os = "macos") { on_path("sips") } else { None },
            sevenzip: find("7z").or_else(|| find("7zz")),
            rar: find("rar").or_else(|| {
                find_known(&["C:\\Program Files\\WinRAR\\Rar.exe", "/usr/local/bin/rar", "/opt/homebrew/bin/rar"])
            }),
            assimp: find("assimp"),
            blender: find_blender(),
            soffice: find("soffice").or_else(|| {
                find_known(&[
                    "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
                    "/Applications/LibreOffice.app/Contents/MacOS/soffice",
                ])
            }),
            pdfium: bundled.iter().map(|dir| dir.join(pdfium_file)).find(|path| path.is_file()),
            // In its own folder, because it needs its own versions of common DLLs.
            mayo: bundled
                .iter()
                .map(|dir| dir.join("mayo").join(format!("mayo-conv{EXE}")))
                .find(|path| path.is_file())
                .or_else(|| on_path("mayo-conv")),
            freecad: step::find_freecad(),
            fontforge: bundled
                .iter()
                .map(|dir| dir.join("fontforge").join("bin").join(format!("fontforge{EXE}")))
                .find(|path| path.is_file())
                .or_else(fonts::find_fontforge),
        }
    }

    pub fn has_image_tool(&self) -> bool {
        self.magick.is_some() || (self.sips.is_some() && self.ffmpeg.is_some())
    }

    /// Whether FFmpeg is here and has the encoders `target` needs.
    pub fn ffmpeg_writes(&self, target: &str) -> bool {
        self.ffmpeg.is_some()
            && self.ffmpeg_encoders.as_ref().is_none_or(|have| {
                encoders_needed(target).iter().all(|need| have.iter().any(|name| name == need))
            })
    }

    /// Without ImageMagick, FFmpeg writes WebP and AVIF images (images.rs).
    fn image_target_ok(&self, target: &str) -> bool {
        self.magick.is_some() || !matches!(target, "webp" | "avif") || self.ffmpeg_writes(target)
    }
}

fn family(format: &str) -> Option<&'static str> {
    formats::matrix().formats.get(format).map(|f| f.family.as_str())
}

/// Whether this machine has what `from` -> `to` needs. Assumes the pair is in the matrix.
pub fn can_convert(tools: &Tools, from: &str, to: &str) -> bool {
    match family(from) {
        Some("audio" | "video") => tools.ffmpeg_writes(to),
        Some("subtitle") => true,
        Some("text") => true,
        Some("archive") => {
            tools.sevenzip.is_some() && (to != "rar" || tools.rar.is_some())
        }
        Some("cad") => step::available(tools, to),
        Some("font") => fonts::available(tools.fontforge.is_some(), from, to),
        Some("model") => tools.assimp.is_some() || (mesh3d::blender_handles(from, to) && tools.blender.is_some()),
        Some("pdf") => tools.pdfium.is_some(),
        Some("image") => match (from, to) {
            ("svg", "png" | "pdf") => true,
            ("jpg" | "png", "docx") | ("jpg", "pdf") => true,
            // Only macOS's sips can write HEIC: the ImageMagick build has no encoder.
            (_, "heic") => tools.sips.is_some() && tools.has_image_tool(),
            // Making icons and picking the largest size in one needs ImageMagick.
            ("ico", _) | (_, "ico") => tools.magick.is_some(),
            _ => tools.has_image_tool() && tools.image_target_ok(to),
        },
        _ => false,
    }
}

/// Whether this machine can compress or resize a format in place.
pub fn can_process(tools: &Tools, from: &str, op: &Op) -> bool {
    let compress = matches!(op, Op::Compress { .. });
    match family(from) {
        Some("pdf") => compress,
        Some("video") => compress && tools.ffmpeg_writes(from),
        // The result has to be written back as HEIC, which only sips can do.
        Some("image") => match from {
            // An icon is several sizes already; resizing or squeezing it makes no sense.
            "svg" | "ico" => false,
            // Radiance files have only one kind of compression, always on.
            "hdr" if compress => false,
            "heic" => tools.sips.is_some() && tools.has_image_tool(),
            _ => tools.has_image_tool() && tools.image_target_ok(from),
        },
        _ => false,
    }
}

/// JPEGs go in as they are; anything else needs converting first.
pub fn can_combine(tools: &Tools, formats_in_drag: &[String]) -> bool {
    tools.has_image_tool() || formats_in_drag.iter().all(|f| f == "jpg")
}

/// Everything an engine needs for one job. Engines write into `work`, a private
/// folder; on success its contents are moved next to the source file.
pub struct Run<'a> {
    pub tools: &'a Tools,
    pub quality: Quality,
    pub cancel: &'a AtomicBool,
    pub source: &'a Path,
    pub sources: &'a [PathBuf],
    pub action: &'a Action,
    pub from: &'a str,
    pub target: &'a str,
    pub work: &'a Path,
    /// `work` joined with the final file name, for engines with a single output.
    pub output: &'a Path,
    pub stem: &'a str,
    pub reporter: &'a Reporter<'a>,
    /// Sources an engine left out while still producing a result.
    pub skipped: &'a Mutex<Vec<(PathBuf, String)>>,
}

static WORK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A private temp folder, removed when dropped.
pub struct WorkDir(pub PathBuf);

impl WorkDir {
    pub fn new() -> Result<Self, String> {
        let id = WORK_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("corolla-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|e| format!("Can't create temp folder: {e}"))?;
        Ok(Self(path))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A combined PDF is named after the folder its images are in.
fn combined_name(first: &Path) -> String {
    first
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "Combined".to_string())
}

pub fn convert(
    job: &Job,
    tools: &Tools,
    quality: Quality,
    cancel: &AtomicBool,
    reporter: &Reporter,
) -> Result<Done, String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".into());
    }
    let source = job.sources.first().ok_or("Nothing to convert")?;
    let from = formats::format_of(source).ok_or("Unsupported file type")?;
    let target = match &job.action {
        Action::Convert(target) => target.clone(),
        Action::CombinePdf => "pdf".to_string(),
        Action::Process { .. } => from.clone(),
    };
    let available = match &job.action {
        Action::Convert(target) => can_convert(tools, &from, target),
        Action::CombinePdf => {
            let all: Vec<String> = job.sources.iter().filter_map(|p| formats::format_of(p)).collect();
            can_combine(tools, &all)
        }
        Action::Process { op, .. } => can_process(tools, &from, op),
    };
    if !available {
        return Err(format!("No engine available for {from} to {}", job.action.label()));
    }

    let extension = formats::extension_for(&target);
    let reserved = match &job.action {
        Action::Convert(_) => output::reserve(source, "", extension, companions(&target)),
        Action::Process { suffix, .. } => output::reserve(source, suffix, extension, &[]),
        Action::CombinePdf => {
            let directory = source.parent().unwrap_or(Path::new("."));
            output::reserve_in(directory, &combined_name(source), extension, &[])
        }
    }
    .map_err(|e| format!("Can't create the output file: {e}"))?;

    let result = convert_reserved(job, tools, quality, cancel, reporter, &from, &target, &reserved, extension);
    if !matches!(result, Ok(Done::Output { .. })) {
        let _ = std::fs::remove_file(&reserved.path);
    }
    result
}

/// Extra files a format writes beside its main file, which must not overwrite anything.
fn companions(target: &str) -> &'static [&'static str] {
    match target {
        "gltf" => &["bin"],
        "obj" => &["mtl"],
        _ => &[],
    }
}

#[allow(clippy::too_many_arguments)]
fn convert_reserved(
    job: &Job,
    tools: &Tools,
    quality: Quality,
    cancel: &AtomicBool,
    reporter: &Reporter,
    from: &str,
    target: &str,
    reserved: &output::Reserved,
    extension: &str,
) -> Result<Done, String> {
    let work = WorkDir::new()?;
    let output_in_work = work.0.join(format!("{}.{extension}", reserved.stem));
    let skipped = Mutex::new(Vec::new());
    let run = Run {
        tools,
        quality,
        cancel,
        source: &job.sources[0],
        sources: &job.sources,
        action: &job.action,
        from,
        target,
        work: &work.0,
        output: &output_in_work,
        stem: &reserved.stem,
        reporter,
        skipped: &skipped,
    };
    dispatch(&run)?;
    if cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".into());
    }
    // A PDF split into page images has no single main file: those are checked per file.
    if output_in_work.exists() {
        signature::check_file(target, &output_in_work)?;
    } else {
        for entry in std::fs::read_dir(&work.0).map_err(|e| e.to_string())?.flatten() {
            signature::check_file(target, &entry.path())?;
        }
    }
    if let Action::Process { op: Op::Compress { .. }, .. } = &job.action {
        let size = |path: &Path| std::fs::metadata(path).map(|m| m.len()).map_err(|e| e.to_string());
        if size(&output_in_work)? >= size(&job.sources[0])? {
            return Ok(Done::NotSmaller);
        }
    }
    let path = output::deliver(&work.0, reserved).map_err(|e| format!("Can't save the output: {e}"))?;
    Ok(Done::Output { path, skipped: skipped.into_inner().unwrap_or_default() })
}

fn dispatch(run: &Run) -> Result<(), String> {
    match run.action {
        Action::CombinePdf => images::combine(run, tools::natural_cmp),
        Action::Process { op, .. } => match family(run.from) {
            Some("pdf") => pdf_compress::run(run, op),
            Some("video") => media::compress(run, op),
            Some("image") => images::process(run, op),
            _ => Err("This file can't be compressed or resized".into()),
        },
        Action::Convert(_) => match family(run.from) {
            Some("audio" | "video") => media::convert(run),
            Some("subtitle") => subtitles::convert_file(run),
            Some("text") => textpages::convert(run),
            Some("archive") => archives::convert(run),
            Some("cad") => step::convert(run),
            Some("font") => fonts::convert(run),
            Some("model") => mesh3d::convert(run),
            Some("pdf") => pdf::convert(run),
            Some("image") => images::convert(run),
            _ => Err("Unsupported file type".into()),
        },
    }
}

const ERROR_TAIL: usize = 600;

/// Runs a tool without a console window, and kills it on cancel.
pub fn run_tool(run: &Run, command: &mut Command) -> Result<(), String> {
    run_tool_watching(run, command, &|_| {})
}

/// Like `run_tool`, handing each line of the tool's output to `on_line` as it arrives.
pub fn run_tool_watching(
    run: &Run,
    command: &mut Command,
    on_line: &(dyn Fn(&str) + Sync),
) -> Result<(), String> {
    let name = Path::new(command.get_program())
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    hide_console(command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start {name}: {e}"))?;
    // Read both on their own threads, or a chatty tool blocks once a pipe fills.
    // Both matter: Assimp and Blender report errors on stdout.
    let stdout = child.stdout.take().ok_or("No stdout pipe")?;
    let stderr_reader = drain(child.stderr.take().ok_or("No stderr pipe")?);
    let (status, printed) = std::thread::scope(|scope| {
        let stdout_reader = scope.spawn(move || {
            let mut text = String::new();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                on_line(&line);
                text.push_str(&line);
                text.push('\n');
                // Only the end is ever shown, and FFmpeg's progress output never stops.
                if text.len() > ERROR_TAIL * 16 {
                    let cut = text.char_indices().rev().nth(ERROR_TAIL).map_or(0, |(i, _)| i);
                    text.drain(..cut);
                }
            }
            text
        });
        let status = loop {
            if run.cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                break Err("Cancelled".to_string());
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!("{name} failed: {e}"));
                }
            }
        };
        (status, stdout_reader.join().unwrap_or_default())
    });
    let status = status?;
    let errors = stderr_reader.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let detail = if errors.trim().is_empty() { printed } else { errors };
    let start = detail.char_indices().rev().nth(ERROR_TAIL).map_or(0, |(i, _)| i);
    Err(format!("{name} failed: {}", detail[start..].trim()))
}

fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).to_string()
    })
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// Requires that a tool actually produced a non-empty file.
pub fn require_output(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > 0 => Ok(()),
        _ => Err("The engine finished but produced no output".into()),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod abuse_tests;

#[cfg(test)]
mod encoder_tests {
    use super::*;

    /// The start of `ffmpeg -hide_banner -encoders` from Homebrew's FFmpeg 9.
    const LISTING: &str = "Encoders:
 V..... = Video
 A..... = Audio
 S..... = Subtitle
 .F.... = Frame-level multithreading
 ------
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V....D libvpx-vp9           libvpx VP9 (codec vp9)
 A....D aac                  AAC (Advanced Audio Coding)
 A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3) (codec mp3)
 A..X.D vorbis               Vorbis
 A....D libopus              libopus Opus (codec opus)
 S..... srt                  SubRip subtitle
";

    #[test]
    fn encoder_listing_is_parsed_without_the_legend() {
        let names = parse_encoders(LISTING);
        assert_eq!(names, ["libx264", "libvpx-vp9", "aac", "libmp3lame", "vorbis", "libopus", "srt"]);
    }

    #[test]
    fn targets_without_their_encoder_are_not_offered() {
        let tools = Tools {
            ffmpeg: Some("ffmpeg".into()),
            sips: Some("sips".into()),
            ffmpeg_encoders: Some(parse_encoders(LISTING)),
            ..Default::default()
        };
        assert!(can_convert(&tools, "wav", "mp3"));
        assert!(can_convert(&tools, "mp4", "webm"));
        assert!(!can_convert(&tools, "wav", "ogg"), "the native vorbis encoder isn't libvorbis");
        assert!(!can_convert(&tools, "png", "webp"), "no libwebp, and no ImageMagick");
        assert!(!can_convert(&tools, "png", "avif"));
        assert!(can_convert(&tools, "png", "jpg"), "sips");
        let with_magick = Tools { magick: Some("magick".into()), ..tools.clone() };
        assert!(can_convert(&with_magick, "png", "webp"), "ImageMagick writes WebP itself");
    }

    #[test]
    fn an_unknown_ffmpeg_offers_everything_as_before() {
        let tools = Tools { ffmpeg: Some("ffmpeg".into()), ..Default::default() };
        assert!(can_convert(&tools, "wav", "ogg"));
        assert!(can_convert(&tools, "mp4", "webm"));
    }
}
