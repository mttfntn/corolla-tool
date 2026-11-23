//! Raster images, SVG, and images placed into PDF or DOCX.

use super::{docx, hide_console, pdf_writer, require_output, run_tool, Quality, Run};
use crate::formats;
use crate::tools::Op;
use std::cmp::Ordering;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

fn jpeg_quality(quality: Quality) -> &'static str {
    match quality {
        Quality::High => "95",
        Quality::Balanced => "85",
        Quality::Small => "65",
    }
}

/// Fonts are slow to load, and SVG text needs them, so load them once.
pub fn font_database() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
}

fn svg_tree(path: &Path) -> Result<usvg::Tree, String> {
    let data = std::fs::read(path).map_err(|e| format!("Can't read SVG: {e}"))?;
    parse_svg(&data, path.parent())
}

/// Parses SVG with the system's fonts; `base` is where relative image links
/// are looked up.
pub fn parse_svg(data: &[u8], base: Option<&Path>) -> Result<usvg::Tree, String> {
    let options = usvg::Options {
        resources_dir: base.map(PathBuf::from),
        fontdb: font_database(),
        ..Default::default()
    };
    usvg::Tree::from_data(data, &options).map_err(|e| format!("Invalid SVG: {e}"))
}

/// Small SVG icons would rasterise uselessly tiny, so scale them up to this.
const MIN_RASTER_EDGE: f32 = 1024.0;
const MAX_RASTER_EDGE: f32 = 16384.0;

/// Renders SVG to PNG, scaling small drawings up so the longest edge is at
/// least 1024 px (and at most 16384 px).
pub fn render_tree_to_png(tree: &usvg::Tree, output: &Path) -> Result<(), String> {
    let size = tree.size();
    let longest = size.width().max(size.height()).max(1.0);
    let scale = (MIN_RASTER_EDGE / longest).max(1.0).min(MAX_RASTER_EDGE / longest);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("SVG size is not usable")?;
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.save_png(output).map_err(|e| format!("Can't write PNG: {e}"))
}

/// Whether a file holds linear light rather than display-ready sRGB. EXR and HDR
/// always do, and so does a 32-bit TIFF by convention. ImageMagick labels EXR as
/// sRGB, so without saying this outright, colours and bright values come out wrong.
fn holds_linear_light(run: &Run, source: &Path, format: &str) -> bool {
    match format {
        "exr" | "hdr" => true,
        "tiff" => run.tools.magick.as_ref().is_some_and(|magick| {
            let mut command = Command::new(magick);
            command.args(["identify", "-format", "%z"]).arg(format!("{}[0]", source.display()));
            hide_console(&mut command);
            command.output().is_ok_and(|result| String::from_utf8_lossy(&result.stdout).trim() == "32")
        }),
        _ => false,
    }
}

const FLOAT_TIFF: [&str; 4] = ["-depth", "32", "-define", "quantum:format=floating-point"];
/// ImageMagick leaves EXR uncompressed unless told; ZIP loses nothing.
const EXR_ZIP: [&str; 2] = ["-compress", "Zip"];

/// Raster-to-raster through whichever engine this machine has.
pub fn raster_to(run: &Run, source: &Path, source_format: &str, target: &str, output: &Path) -> Result<(), String> {
    if let Some(magick) = &run.tools.magick {
        let linear = holds_linear_light(run, source, source_format);
        let mut command = Command::new(magick);
        // One frame only: the first page of a TIFF, the largest size of an icon.
        let frame = frame_index(run, source, source_format);
        command.arg(format!("{}[{frame}]", source.display())).arg("-auto-orient");
        if linear {
            command.args(["-set", "colorspace", "RGB"]);
        }
        match target {
            "exr" | "hdr" if !linear => {
                command.args(["-colorspace", "RGB"]);
            }
            "exr" | "hdr" => {}
            // Keeps values above 1.0, which only a floating-point TIFF can hold.
            "tiff" if linear => {
                command.args(FLOAT_TIFF);
            }
            _ if linear => {
                command.args(["-colorspace", "sRGB"]);
            }
            _ => {}
        }
        if target == "exr" {
            command.args(EXR_ZIP);
        }
        if matches!(target, "jpg" | "bmp") {
            command.args(["-background", "white", "-alpha", "remove", "-alpha", "off"]);
        }
        command.args(["-quality", jpeg_quality(run.quality)]).arg(output);
        run_tool(run, &mut command)?;
        return require_output(output);
    }
    native_raster_to(run, source, source_format, target, output)
}

/// macOS without ImageMagick: `sips` does the Apple-friendly formats, FFmpeg the rest.
fn native_raster_to(run: &Run, source: &Path, source_format: &str, target: &str, output: &Path) -> Result<(), String> {
    let sips_reads = matches!(source_format, "jpg" | "png" | "tiff" | "bmp" | "heic");
    let sips_writes = matches!(target, "jpg" | "png" | "tiff" | "bmp" | "heic");
    if sips_reads && sips_writes {
        return sips(run, source, target, output);
    }
    if !sips_reads && target != "heic" {
        return ffmpeg_image(run, source, output);
    }
    // Needs both engines: go through a PNG they can both handle.
    let middle = run.work.join("intermediate-raster.png");
    if sips_reads {
        sips(run, source, "png", &middle)?;
        ffmpeg_image(run, &middle, output)?;
    } else {
        ffmpeg_image(run, source, &middle)?;
        sips(run, &middle, target, output)?;
    }
    let _ = std::fs::remove_file(&middle);
    Ok(())
}

fn sips(run: &Run, source: &Path, target: &str, output: &Path) -> Result<(), String> {
    let tool = run.tools.sips.as_ref().ok_or("sips is missing")?;
    let format = match target {
        "jpg" => "jpeg",
        "tiff" => "tiff",
        other => other,
    };
    let mut command = Command::new(tool);
    command.args(["-s", "format", format, "-s", "formatOptions", jpeg_quality(run.quality)]);
    command.arg(source).arg("--out").arg(output);
    run_tool(run, &mut command)?;
    require_output(output)
}

fn ffmpeg_image(run: &Run, source: &Path, output: &Path) -> Result<(), String> {
    let tool = run.tools.ffmpeg.as_ref().ok_or("FFmpeg is missing")?;
    let mut command = Command::new(tool);
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]);
    command.arg(source).args(["-frames:v", "1", "-update", "1"]);
    let crf = match run.quality {
        Quality::High => "20",
        Quality::Balanced => "30",
        Quality::Small => "42",
    };
    match output.extension().and_then(|e| e.to_str()) {
        Some("avif") => {
            command.args(["-c:v", "libaom-av1", "-still-picture", "1", "-crf", crf]);
        }
        Some("webp") => {
            command.args(["-quality", jpeg_quality(run.quality)]);
        }
        _ => {}
    }
    command.arg(output);
    run_tool(run, &mut command)?;
    require_output(output)
}

fn to_pdf(run: &Run) -> Result<(), String> {
    if run.from == "svg" {
        let tree = svg_tree(run.source)?;
        let pdf = svg2pdf::to_pdf(&tree, svg2pdf::ConversionOptions::default(), svg2pdf::PageOptions::default())
            .map_err(|e| format!("Can't convert SVG to PDF: {e}"))?;
        return std::fs::write(run.output, pdf).map_err(|e| e.to_string());
    }
    let (jpeg, _cleanup) = as_jpeg(run)?;
    let data = std::fs::read(&jpeg).map_err(|e| e.to_string())?;
    let size = imagesize::blob_size(&data).map_err(|e| format!("Can't read image size: {e}"))?;
    let pdf = pdf_writer::single_jpeg(&data, size.width as u32, size.height as u32, jpeg_components(&data));
    std::fs::write(run.output, pdf).map_err(|e| e.to_string())
}

/// PDF must declare a JPEG's colour model: grey (1), RGB (3) or CMYK (4).
fn jpeg_components(data: &[u8]) -> u8 {
    let mut i = 2;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        let is_frame = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_frame {
            return data[i + 9];
        }
        let length = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        i += 2 + length.max(2);
    }
    3
}

/// The EXIF rotation of a JPEG, where 1 means upright. PDF ignores EXIF, so a
/// phone photo embedded as it is could come out sideways.
fn jpeg_orientation(data: &[u8]) -> u16 {
    let mut i = 2;
    while i + 4 < data.len() && data[i] == 0xFF {
        let marker = data[i + 1];
        let length = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if marker == 0xE1 {
            let segment = data.get(i + 4..i + 2 + length).unwrap_or_default();
            if let Some(tiff) = segment.strip_prefix(b"Exif\0\0") {
                return exif_orientation(tiff).unwrap_or(1);
            }
        }
        if marker == 0xDA {
            break; // The picture data has started: no more metadata.
        }
        i += 2 + length.max(2);
    }
    1
}

fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let b = tiff.get(at..at + 2)?;
        Some(if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let b = tiff.get(4..8)?;
    let first = if little {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    } as usize;
    for entry in 0..usize::from(u16_at(first)?) {
        let at = first + 2 + entry * 12;
        if u16_at(at)? == 0x0112 {
            return u16_at(at + 8);
        }
    }
    None
}

/// A file as a JPEG that's fit to place on a PDF page, either the original or a
/// temporary made in the work folder.
fn page_jpeg(run: &Run, source: &Path, format: &str) -> Result<PathBuf, String> {
    let jpeg = run.work.join("intermediate-page.jpg");
    if format == "jpg" {
        let data = std::fs::read(source).map_err(|e| format!("Can't read image: {e}"))?;
        if jpeg_orientation(&data) == 1 || !run.tools.has_image_tool() {
            return Ok(source.to_path_buf());
        }
    } else if format == "svg" {
        let middle = run.work.join("intermediate-svg.png");
        render_tree_to_png(&svg_tree(source)?, &middle)?;
        raster_to(run, &middle, "png", "jpg", &jpeg)?;
        let _ = std::fs::remove_file(&middle);
        return Ok(jpeg);
    }
    raster_to(run, source, format, "jpg", &jpeg)?;
    Ok(jpeg)
}

/// The source as a JPEG on disk. The bool says it's a temporary we made.
fn as_jpeg(run: &Run) -> Result<(PathBuf, bool), String> {
    let jpeg = page_jpeg(run, run.source, run.from)?;
    let made = jpeg != run.source;
    Ok((jpeg, made))
}

/// All the sources as pages of one PDF. A file that can't be read is left out and
/// reported, unless nothing at all could be.
pub fn combine(run: &Run, order: fn(&Path, &Path) -> Ordering) -> Result<(), String> {
    let mut sources: Vec<&PathBuf> = run.sources.iter().collect();
    sources.sort_by(|a, b| order(a, b));
    let file = std::fs::File::create(run.output).map_err(|e| e.to_string())?;
    let mut pdf = pdf_writer::JpegPages::new(BufWriter::new(file)).map_err(|e| e.to_string())?;
    let temporary = run.work.join("intermediate-page.jpg");
    let total = sources.len() as f64;
    let mut first_error = None;
    for (i, source) in sources.iter().enumerate() {
        if run.cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("Cancelled".into());
        }
        let page = formats::format_of(source)
            .ok_or_else(|| "Unsupported file type".to_string())
            .and_then(|format| page_jpeg(run, source, &format))
            .and_then(|jpeg| {
                let data = std::fs::read(&jpeg).map_err(|e| format!("Can't read image: {e}"))?;
                let size = imagesize::blob_size(&data).map_err(|e| format!("Can't read image size: {e}"))?;
                pdf.add(&data, size.width as u32, size.height as u32, jpeg_components(&data))
                    .map_err(|e| format!("Can't write the PDF: {e}"))
            });
        let _ = std::fs::remove_file(&temporary);
        if let Err(error) = page {
            first_error.get_or_insert_with(|| error.clone());
            run.skipped.lock().unwrap().push(((*source).clone(), error));
        }
        run.reporter.set((i + 1) as f64 / total);
    }
    if pdf.page_count() == 0 {
        run.skipped.lock().unwrap().clear();
        return Err(first_error.unwrap_or_else(|| "No images could be read".to_string()));
    }
    pdf.finish().map_err(|e| format!("Can't write the PDF: {e}"))?;
    require_output(run.output)
}

/// `-resize` for ImageMagick; the longest edge is never enlarged.
fn resize_geometry(op: &Op) -> Option<String> {
    match *op {
        Op::Compress { max_edge: Some(edge), .. } | Op::ResizeEdge { edge } => Some(format!("{edge}x{edge}>")),
        Op::Compress { max_edge: None, .. } => None,
        Op::ResizeDivide { by } => Some(format!("{}%", 100.0 / by.max(1) as f32)),
    }
}

/// Compress or resize, keeping the format.
pub fn process(run: &Run, op: &Op) -> Result<(), String> {
    let compress_quality = match *op {
        Op::Compress { quality, .. } => Some(quality.clamp(1, 100)),
        _ => None,
    };
    if run.tools.magick.is_none() {
        return native_process(run, op, compress_quality);
    }
    let magick = run.tools.magick.as_ref().ok_or("ImageMagick is missing")?;
    let mut command = Command::new(magick);
    // Rotating first, because removing the metadata removes the rotation tag.
    command.arg(format!("{}[0]", run.source.display())).arg("-auto-orient");
    let linear = holds_linear_light(run, run.source, run.from);
    if linear {
        command.args(["-set", "colorspace", "RGB"]);
    }
    if let Some(geometry) = resize_geometry(op) {
        command.arg("-resize").arg(geometry);
    }
    if run.target == "exr" {
        command.args(EXR_ZIP);
    }
    if run.target == "tiff" && linear {
        command.args(FLOAT_TIFF);
    }
    match compress_quality {
        Some(quality) => {
            command.args(["+profile", "!icc,*"]);
            command.arg("-quality").arg(quality.to_string());
            // These two ignore -quality, so reducing the colours is what shrinks them.
            match run.target {
                "png" => {
                    command.args(["-define", "png:compression-level=9"]);
                    if quality < 90 {
                        command.args(["-colors", "256", "-define", "png:format=png8"]);
                    }
                }
                "bmp" if quality < 90 => {
                    command.args(["-colors", "256", "-type", "Palette"]);
                }
                "tiff" => {
                    command.args(["-compress", "Zip"]);
                }
                _ => {}
            }
        }
        None => {
            command.arg("-quality").arg(jpeg_quality(run.quality));
        }
    }
    command.arg(run.output);
    run_tool(run, &mut command)?;
    require_output(run.output)
}

/// macOS has no ImageMagick: `sips` where it can, FFmpeg for the rest.
fn native_process(run: &Run, op: &Op, compress_quality: Option<u8>) -> Result<(), String> {
    let size = imagesize::size(run.source).map_err(|e| format!("Can't read image size: {e}"))?;
    let longest = size.width.max(size.height) as u32;
    let edge = match *op {
        Op::Compress { max_edge, .. } => max_edge.map(|edge| edge.min(longest)),
        Op::ResizeEdge { edge } => Some(edge.min(longest)),
        Op::ResizeDivide { by } => Some((longest / by.max(1)).max(1)),
    };
    let quality = compress_quality.map_or_else(|| jpeg_quality(run.quality).to_string(), |q| q.to_string());
    if matches!(run.from, "jpg" | "png" | "tiff" | "bmp" | "heic") {
        let tool = run.tools.sips.as_ref().ok_or("sips is missing")?;
        let mut command = Command::new(tool);
        command.args(["-s", "formatOptions", &quality]);
        if let Some(edge) = edge {
            command.arg("-Z").arg(edge.to_string());
        }
        command.arg(run.source).arg("--out").arg(run.output);
        run_tool(run, &mut command)?;
        return require_output(run.output);
    }
    let tool = run.tools.ffmpeg.as_ref().ok_or("FFmpeg is missing")?;
    let mut command = Command::new(tool);
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]);
    command.arg(run.source).args(["-frames:v", "1", "-update", "1"]);
    if let Some(edge) = edge {
        command.arg("-vf").arg(format!(
            "scale='if(gt(iw,ih),{edge},-2)':'if(gt(iw,ih),-2,{edge})'"
        ));
    }
    if run.from == "avif" {
        let percent: f32 = quality.parse().unwrap_or(70.0);
        let crf = ((100.0 - percent) * 0.5 + 12.0) as u8;
        command.args(["-c:v", "libaom-av1", "-still-picture", "1", "-crf", &crf.to_string()]);
    } else {
        command.args(["-quality", &quality]);
    }
    command.arg(run.output);
    run_tool(run, &mut command)?;
    require_output(run.output)
}

fn to_docx(run: &Run) -> Result<(), String> {
    let name = if run.from == "jpg" { "image1.jpg" } else { "image1.png" };
    let data = std::fs::read(run.source).map_err(|e| format!("Can't read image: {e}"))?;
    let size = imagesize::blob_size(&data).map_err(|e| format!("Can't read image size: {e}"))?;
    docx::write_image(run.output, name, &data, size.width as u32, size.height as u32)
}

/// The sizes in a favicon: what browsers use (16, 32, 48), and 256 so the same
/// file also works as a Windows icon.
pub const ICON_SIZES: [u32; 4] = [16, 32, 48, 256];

/// An ICO file holding the given PNGs, which every browser and Windows since
/// Vista accept. Far smaller than ImageMagick's, which stores them uncompressed.
pub fn assemble_ico(frames: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * frames.len();
    for (size, png) in frames {
        // A byte holds up to 255, so 256 is written as 0.
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in frames {
        out.extend_from_slice(png);
    }
    out
}

/// Each icon size made from the picture, centred on a transparent square so a
/// wide picture isn't squashed.
fn to_ico(run: &Run, source: &Path, format: &str) -> Result<(), String> {
    let magick = run.tools.magick.as_ref().ok_or("ImageMagick is missing")?;
    let linear = holds_linear_light(run, source, format);
    let mut frames = Vec::new();
    for size in ICON_SIZES {
        let png = run.work.join(format!("intermediate-icon-{size}.png"));
        let mut command = Command::new(magick);
        command.arg(format!("{}[{}]", source.display(), frame_index(run, source, format))).arg("-auto-orient");
        if linear {
            command.args(["-set", "colorspace", "RGB", "-colorspace", "sRGB"]);
        }
        let square = format!("{size}x{size}");
        command.args(["-resize", &square, "-background", "none", "-gravity", "center", "-extent", &square]);
        command.arg(format!("PNG32:{}", png.display()));
        run_tool(run, &mut command)?;
        frames.push((size, std::fs::read(&png).map_err(|e| e.to_string())?));
        let _ = std::fs::remove_file(&png);
    }
    std::fs::write(run.output, assemble_ico(&frames)).map_err(|e| e.to_string())
}

/// Which picture to read from a file. An icon holds several sizes in no fixed
/// order, so it's the largest; anything else, the first.
fn frame_index(run: &Run, source: &Path, format: &str) -> usize {
    if format != "ico" {
        return 0;
    }
    let Some(magick) = &run.tools.magick else { return 0 };
    let mut command = Command::new(magick);
    command.args(["identify", "-format", "%w\\n"]).arg(source);
    hide_console(&mut command);
    let Ok(result) = command.output() else { return 0 };
    String::from_utf8_lossy(&result.stdout)
        .lines()
        .enumerate()
        .max_by_key(|(_, width)| width.trim().parse::<u32>().unwrap_or(0))
        .map_or(0, |(index, _)| index)
}

pub fn convert(run: &Run) -> Result<(), String> {
    let result = match run.target {
        "pdf" => to_pdf(run),
        "docx" => to_docx(run),
        "ico" if run.from == "svg" => {
            let middle = run.work.join("intermediate-svg.png");
            render_tree_to_png(&svg_tree(run.source)?, &middle)?;
            to_ico(run, &middle, "png")
        }
        "ico" => to_ico(run, run.source, run.from),
        target if run.from == "svg" => {
            let tree = svg_tree(run.source)?;
            if target == "png" {
                render_tree_to_png(&tree, run.output)
            } else {
                let middle = run.work.join("intermediate-svg.png");
                render_tree_to_png(&tree, &middle)?;
                raster_to(run, &middle, "png", target, run.output)
            }
        }
        target => raster_to(run, run.source, run.from, target, run.output),
    };
    result?;
    // Only the finished file may be delivered, not the steps on the way to it.
    for leftover in ["intermediate-page.jpg", "intermediate-svg.png", "intermediate-raster.png"] {
        let _ = std::fs::remove_file(run.work.join(leftover));
    }
    require_output(run.output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_component_count_comes_from_the_frame_header() {
        // SOI, then an APP0 segment to skip, then SOF0 with 3 components.
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0];
        data.extend([0xFF, 0xC0, 0x00, 0x11, 8, 0, 16, 0, 16, 3, 1, 1, 0, 2, 1, 1, 3, 1, 1]);
        assert_eq!(jpeg_components(&data), 3);
        let len = data.len();
        data[len - 10] = 1;
        assert_eq!(jpeg_components(&data), 1);
    }

    #[test]
    fn exif_rotation_is_read_in_both_byte_orders() {
        let wrap = |tiff: &[u8]| {
            let mut data = vec![0xFF, 0xD8, 0xFF, 0xE1];
            data.extend(((tiff.len() + 8) as u16).to_be_bytes());
            data.extend(b"Exif\0\0");
            data.extend(tiff);
            data.extend([0xFF, 0xDA, 0, 2]);
            data
        };
        let little = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0];
        let big = [b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 8, 0, 0];
        assert_eq!(jpeg_orientation(&wrap(&little)), 6);
        assert_eq!(jpeg_orientation(&wrap(&big)), 8);
        assert_eq!(jpeg_orientation(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]), 1);
    }

    #[test]
    fn resize_never_enlarges_by_longest_edge() {
        assert_eq!(resize_geometry(&Op::ResizeDivide { by: 2 }).unwrap(), "50%");
        assert_eq!(resize_geometry(&Op::ResizeDivide { by: 4 }).unwrap(), "25%");
        assert_eq!(resize_geometry(&Op::ResizeEdge { edge: 1080 }).unwrap(), "1080x1080>");
        assert_eq!(resize_geometry(&Op::Compress { quality: 70, max_edge: None }), None);
    }

    #[test]
    fn icon_directory_points_at_each_picture() {
        let ico = assemble_ico(&[(16, vec![1; 10]), (256, vec![2; 30])]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[22], 0, "256 is written as 0");
        let offset = |entry: usize| u32::from_le_bytes(ico[6 + entry * 16 + 12..6 + entry * 16 + 16].try_into().unwrap()) as usize;
        assert_eq!(&ico[offset(0)..offset(0) + 10], &[1; 10]);
        assert_eq!(&ico[offset(1)..offset(1) + 30], &[2; 30]);
        assert_eq!(ico.len(), offset(1) + 30);
    }

    #[test]
    fn small_svg_is_scaled_up_when_rendered() {
        let tree = parse_svg(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect width="32" height="16" fill="red"/></svg>"#,
            None,
        )
        .unwrap();
        let dir = super::super::WorkDir::new().unwrap();
        let out = dir.0.join("o.png");
        render_tree_to_png(&tree, &out).unwrap();
        let size = imagesize::size(&out).unwrap();
        assert_eq!((size.width, size.height), (1024, 512));
    }
}
