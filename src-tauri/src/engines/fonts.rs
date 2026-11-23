//! Fonts: TTF and OTF, and WOFF and WOFF2, which are compressed wrappers around
//! one of them. Wrapping and unwrapping happen here, losslessly. Changing the
//! outlines between TrueType (TTF) and CFF (OTF) needs FontForge.

use super::{require_output, run_tool, Run, WorkDir};
use flate2::{write::ZlibEncoder, Compression};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outlines {
    TrueType,
    Cff,
}

impl Outlines {
    fn extension(self) -> &'static str {
        match self {
            Outlines::TrueType => "ttf",
            Outlines::Cff => "otf",
        }
    }
}

/// The kind of curves in a bare font file, from its first four bytes.
pub fn outlines_of(sfnt: &[u8]) -> Option<Outlines> {
    match sfnt.get(..4)? {
        [0, 1, 0, 0] | b"true" => Some(Outlines::TrueType),
        b"OTTO" => Some(Outlines::Cff),
        _ => None,
    }
}

/// A variable font holds a range of styles. FontForge would quietly keep only one.
pub fn is_variable(sfnt: &[u8]) -> bool {
    ttf_parser::Face::parse(sfnt, 0).is_ok_and(|face| face.is_variable())
}

pub fn find_fontforge() -> Option<PathBuf> {
    let known = [
        "C:\\Program Files\\FontForgeBuilds\\bin\\fontforge.exe",
        "/Applications/FontForge.app/Contents/Resources/opt/local/bin/fontforge",
        "/Applications/FontForge.app/Contents/MacOS/FontForge",
        "/opt/homebrew/bin/fontforge",
        "/usr/local/bin/fontforge",
    ];
    known.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// The bare TTF or OTF inside a font file.
fn unwrap(format: &str, bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match format {
        "woff" => wuff::decompress_woff1(&bytes).map_err(|e| format!("Can't read the WOFF: {e:?}")),
        "woff2" => wuff::decompress_woff2(&bytes).map_err(|e| format!("Can't read the WOFF2: {e:?}")),
        _ => Ok(bytes),
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// One table of a bare font: its tag, checksum and contents.
struct Table<'a> {
    tag: [u8; 4],
    checksum: u32,
    data: &'a [u8],
}

fn tables(sfnt: &[u8]) -> Option<Vec<Table<'_>>> {
    let count = usize::from(u16_at(sfnt, 4)?);
    let mut list = Vec::with_capacity(count);
    for i in 0..count {
        let entry = 12 + i * 16;
        let tag: [u8; 4] = sfnt.get(entry..entry + 4)?.try_into().ok()?;
        let offset = u32_at(sfnt, entry + 8)? as usize;
        let length = u32_at(sfnt, entry + 12)? as usize;
        list.push(Table { tag, checksum: u32_at(sfnt, entry + 4)?, data: sfnt.get(offset..offset + length)? });
    }
    list.sort_by_key(|table| table.tag);
    Some(list)
}

fn padded(length: usize) -> usize {
    (length + 3) & !3
}

/// WOFF 1.0: the same tables, each compressed with zlib when that helps.
pub fn wrap_woff(sfnt: &[u8]) -> Result<Vec<u8>, String> {
    let list = tables(sfnt).ok_or("The font's table directory is damaged")?;
    let flavour = u32_at(sfnt, 0).ok_or("Not a font")?;
    let header = 44;
    let directory = 20 * list.len();
    let mut body = Vec::new();
    let mut entries = Vec::new();
    for table in &list {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(table.data).map_err(|e| e.to_string())?;
        let squeezed = encoder.finish().map_err(|e| e.to_string())?;
        // The format stores a table as it is if compressing wouldn't make it smaller.
        let stored = if squeezed.len() < table.data.len() { squeezed } else { table.data.to_vec() };
        entries.push((table.tag, header + directory + body.len(), stored.len(), table.data.len(), table.checksum));
        body.extend_from_slice(&stored);
        body.resize(padded(body.len()), 0);
    }
    let original_size = 12 + 16 * list.len() + list.iter().map(|t| padded(t.data.len())).sum::<usize>();
    let total = header + directory + body.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"wOFF");
    out.extend_from_slice(&flavour.to_be_bytes());
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(list.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // reserved
    out.extend_from_slice(&(original_size as u32).to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // font version, major
    out.extend_from_slice(&0u16.to_be_bytes()); // and minor
    out.extend_from_slice(&[0; 20]); // no metadata and no private data
    for (tag, offset, stored, original, checksum) in entries {
        out.extend_from_slice(&tag);
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(stored as u32).to_be_bytes());
        out.extend_from_slice(&(original as u32).to_be_bytes());
        out.extend_from_slice(&checksum.to_be_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}

fn wrap_woff2(sfnt: &[u8]) -> Result<Vec<u8>, String> {
    ttf2woff2::encode(sfnt, ttf2woff2::BrotliQuality::default()).map_err(|e| format!("Can't make the WOFF2: {e}"))
}

const FONTFORGE_SCRIPT: &str = "Open($1); Generate($2); Close()";

/// The Windows build carries its own Python. Pointing at it, as FontForge's own
/// launcher does, stops it picking up a different Python installed on the PC.
fn use_own_python(command: &mut Command, fontforge: &std::path::Path) {
    let Some(root) = fontforge.parent().and_then(std::path::Path::parent) else { return };
    let library = std::fs::read_dir(root.join("lib")).ok().and_then(|entries| {
        entries.flatten().map(|e| e.path()).find(|p| {
            p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("python"))
        })
    });
    if let Some(library) = library {
        command.env("PYTHONHOME", root).env("PYTHONPATH", library);
    }
}

/// Redraws the letters with the other kind of curves.
fn with_fontforge(run: &Run, sfnt: &[u8], from: Outlines, to: Outlines) -> Result<Vec<u8>, String> {
    let fontforge = run.tools.fontforge.as_ref().ok_or(
        "Changing between TTF and OTF outlines needs FontForge, which isn't installed",
    )?;
    if is_variable(sfnt) {
        return Err("This is a variable font: converting between TTF and OTF would keep only one of its styles. \
                    WOFF and WOFF2 keep it whole."
            .into());
    }
    // In the system temp folder, because `work` is delivered as output.
    let scratch = WorkDir::new()?;
    let input = scratch.0.join(format!("in.{}", from.extension()));
    let output = scratch.0.join(format!("out.{}", to.extension()));
    std::fs::write(&input, sfnt).map_err(|e| e.to_string())?;
    let mut command = Command::new(fontforge);
    command.args(["-quiet", "-lang=ff", "-c", FONTFORGE_SCRIPT]).arg(&input).arg(&output);
    use_own_python(&mut command, fontforge);
    run_tool(run, &mut command)?;
    require_output(&output)?;
    let converted = std::fs::read(&output).map_err(|e| e.to_string())?;
    if outlines_of(&converted) != Some(to) {
        return Err("FontForge didn't produce the expected kind of font".into());
    }
    Ok(converted)
}

pub fn convert(run: &Run) -> Result<(), String> {
    let bytes = std::fs::read(run.source).map_err(|e| format!("Can't read the font: {e}"))?;
    let sfnt = unwrap(run.from, bytes)?;
    let has = outlines_of(&sfnt).ok_or("This isn't a TrueType or OpenType font")?;
    let wanted = match run.target {
        "ttf" => Some(Outlines::TrueType),
        "otf" => Some(Outlines::Cff),
        _ => None,
    };
    let sfnt = match wanted {
        Some(want) if want != has => with_fontforge(run, &sfnt, has, want)?,
        _ => sfnt,
    };
    let finished = match run.target {
        "woff" => wrap_woff(&sfnt)?,
        "woff2" => wrap_woff2(&sfnt)?,
        _ => sfnt,
    };
    ttf_parser::Face::parse(&unwrap(run.target, finished.clone())?, 0)
        .map_err(|e| format!("The converted font doesn't open: {e}"))?;
    std::fs::write(run.output, finished).map_err(|e| e.to_string())?;
    require_output(run.output)
}

/// What a dropped font can become on this machine. Wrapping is always possible;
/// a known TTF ↔ OTF change needs FontForge. A WOFF's contents aren't known until
/// it's read, so a mismatch there is reported then.
pub fn available(fontforge: bool, from: &str, to: &str) -> bool {
    match (from, to) {
        ("ttf", "otf") | ("otf", "ttf") => fontforge,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(name)).unwrap()
    }

    #[test]
    fn outlines_are_told_apart() {
        assert_eq!(outlines_of(&fixture("font-static.ttf")), Some(Outlines::TrueType));
        assert_eq!(outlines_of(&fixture("font-cff.otf")), Some(Outlines::Cff));
        assert_eq!(outlines_of(b"wOFF"), None);
    }

    #[test]
    fn variable_fonts_are_spotted() {
        assert!(is_variable(&fixture("font-variable.ttf")));
        assert!(!is_variable(&fixture("font-static.ttf")));
    }

    /// Every table, keyed by tag, so two fonts can be compared whatever their layout.
    fn contents(sfnt: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        tables(sfnt).unwrap().into_iter().map(|t| (t.tag, t.data.to_vec())).collect()
    }

    #[test]
    fn woff_round_trip_keeps_every_table_exactly() {
        for name in ["font-static.ttf", "font-cff.otf", "font-variable.ttf"] {
            let original = fixture(name);
            let woff = wrap_woff(&original).unwrap();
            assert!(woff.starts_with(b"wOFF"));
            assert!(woff.len() < original.len(), "{name} got smaller");
            let back = unwrap("woff", woff).unwrap();
            assert_eq!(contents(&back), contents(&original), "{name}");
        }
    }

    #[test]
    fn woff2_round_trip_keeps_the_glyphs() {
        for name in ["font-static.ttf", "font-cff.otf", "font-variable.ttf"] {
            let original = fixture(name);
            let woff2 = wrap_woff2(&original).unwrap();
            assert!(woff2.starts_with(b"wOF2"));
            assert!(woff2.len() < wrap_woff(&original).unwrap().len(), "{name}: WOFF2 beats WOFF");
            let back = unwrap("woff2", woff2).unwrap();
            let (a, b) = (ttf_parser::Face::parse(&original, 0).unwrap(), ttf_parser::Face::parse(&back, 0).unwrap());
            assert_eq!(a.number_of_glyphs(), b.number_of_glyphs(), "{name}");
            assert_eq!(a.is_variable(), b.is_variable(), "{name}");
            assert_eq!(outlines_of(&back), outlines_of(&original), "{name}");
        }
    }

    #[test]
    fn outline_changes_need_fontforge() {
        assert!(available(false, "ttf", "woff2"));
        assert!(available(false, "woff", "ttf"));
        assert!(!available(false, "ttf", "otf"));
        assert!(!available(false, "otf", "ttf"));
        assert!(available(true, "otf", "ttf"));
    }
}
