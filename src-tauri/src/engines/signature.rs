//! Checks a file really is the format its name claims. Engines can't be trusted
//! on this alone: ImageMagick without a HEIC encoder writes a PNG named .heic
//! and still exits successfully.

use std::io::Read;
use std::path::Path;

const HEADER: usize = 300;

fn at(bytes: &[u8], offset: usize, expected: &[u8]) -> bool {
    bytes.get(offset..offset + expected.len()) == Some(expected)
}

/// True if the bytes fit the format. Formats with no fixed header (STL, OBJ and
/// plain text) only need to be non-empty.
pub fn matches(target: &str, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let riff = |kind: &[u8]| at(bytes, 0, b"RIFF") && at(bytes, 8, kind);
    let iso_media = at(bytes, 4, b"ftyp");
    let asf = at(bytes, 0, &[0x30, 0x26, 0xB2, 0x75]);
    let matroska = at(bytes, 0, &[0x1A, 0x45, 0xDF, 0xA3]);
    match target {
        "jpg" => at(bytes, 0, &[0xFF, 0xD8, 0xFF]),
        "png" => at(bytes, 0, &[0x89, b'P', b'N', b'G']),
        "gif" => at(bytes, 0, b"GIF8"),
        "bmp" => at(bytes, 0, b"BM"),
        "webp" => riff(b"WEBP"),
        "tiff" => at(bytes, 0, b"II*\0") || at(bytes, 0, b"MM\0*"),
        "avif" | "heic" => iso_media,
        "pdf" => at(bytes, 0, b"%PDF"),
        "docx" | "zip" => at(bytes, 0, b"PK"),
        "mp3" => at(bytes, 0, b"ID3") || (bytes[0] == 0xFF && bytes.get(1).is_some_and(|b| b & 0xE0 == 0xE0)),
        "wav" => riff(b"WAVE"),
        "avi" => riff(b"AVI "),
        "flac" => at(bytes, 0, b"fLaC"),
        "ogg" | "opus" => at(bytes, 0, b"OggS"),
        "aiff" => at(bytes, 0, b"FORM"),
        "wma" | "wmv" => asf,
        "m4a" | "mp4" => iso_media,
        // Older QuickTime files may start with another atom.
        "mov" => iso_media || at(bytes, 4, b"moov") || at(bytes, 4, b"wide") || at(bytes, 4, b"mdat"),
        "mkv" | "webm" => matroska,
        "gzip" => at(bytes, 0, &[0x1F, 0x8B]),
        "tar" => at(bytes, 257, b"ustar"),
        "rar" => at(bytes, 0, b"Rar!"),
        "ttf" => at(bytes, 0, &[0, 1, 0, 0]) || at(bytes, 0, b"true"),
        "otf" => at(bytes, 0, b"OTTO"),
        "woff" => at(bytes, 0, b"wOFF"),
        "woff2" => at(bytes, 0, b"wOF2"),
        "ico" => at(bytes, 0, &[0, 0, 1, 0]),
        "exr" => at(bytes, 0, &[0x76, 0x2F, 0x31, 0x01]),
        "hdr" => at(bytes, 0, b"#?"),
        "step" => at(bytes, 0, b"ISO-10303-21"),
        "glb" => at(bytes, 0, b"glTF"),
        "gltf" => bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{'),
        "fbx" => at(bytes, 0, b"Kaydara FBX") || at(bytes, 0, b"; FBX"),
        "dae" => {
            let start = String::from_utf8_lossy(bytes);
            start.trim_start_matches('\u{FEFF}').trim_start().starts_with('<')
        }
        _ => true,
    }
}

pub fn check_file(target: &str, path: &Path) -> Result<(), String> {
    let mut header = Vec::with_capacity(HEADER);
    std::fs::File::open(path)
        .map_err(|e| format!("Can't read the output: {e}"))?
        .take(HEADER as u64)
        .read_to_end(&mut header)
        .map_err(|e| format!("Can't read the output: {e}"))?;
    if matches(target, &header) {
        Ok(())
    } else {
        Err(format!("The engine's output isn't really {}", target.to_uppercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn a_png_named_heic_is_caught() {
        assert!(!matches("heic", PNG));
        assert!(matches("png", PNG));
    }

    #[test]
    fn empty_is_never_valid() {
        assert!(!matches("txt", b""));
        assert!(!matches("stl", b""));
    }

    #[test]
    fn riff_containers_are_told_apart() {
        assert!(matches("wav", b"RIFF\0\0\0\0WAVEfmt "));
        assert!(!matches("avi", b"RIFF\0\0\0\0WAVEfmt "));
        assert!(!matches("webp", b"RIFF\0\0\0\0AVI LIST"));
    }

    #[test]
    fn tar_marker_is_found_past_the_first_block_header() {
        let mut tar = vec![0u8; 300];
        tar[257..262].copy_from_slice(b"ustar");
        assert!(matches("tar", &tar));
        assert!(!matches("tar", &[0u8; 300]));
    }

    #[test]
    fn formats_without_a_header_only_need_content() {
        assert!(matches("obj", b"v 0 0 0"));
        assert!(matches("srt", b"1"));
    }

    #[test]
    fn text_based_3d_formats_are_recognised() {
        assert!(matches("gltf", b"  {\"asset\":{}}"));
        assert!(!matches("gltf", b"PK"));
        assert!(matches("dae", b"<?xml version=\"1.0\"?>"));
        assert!(!matches("dae", b"Kaydara FBX Binary"));
    }
}
