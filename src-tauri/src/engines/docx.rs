//! A minimal valid DOCX: either one image, or paragraphs of text.

use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpg" ContentType="image/jpeg"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const DOC_OPEN: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>"#;
const DOC_CLOSE: &str = "<w:sectPr/></w:body></w:document>";

/// The text area of a default page: 6 by 9 inches, in OOXML's EMU unit.
const MAX_WIDTH_EMU: u64 = 5_486_400;
const MAX_HEIGHT_EMU: u64 = 8_229_600;
const EMU_PER_PIXEL: u64 = 9_525;

pub fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| matches!(*c, '\t' | '\n' | '\r') || (*c >= ' ' && *c != '\u{FFFE}' && *c != '\u{FFFF}'))
        .fold(String::with_capacity(text.len()), |mut out, c| {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                other => out.push(other),
            }
            out
        })
}

fn write_package(
    output: &Path,
    document: &str,
    media: Option<(&str, &[u8])>,
) -> Result<(), String> {
    let run = || -> zip::result::ZipResult<()> {
        let file = std::fs::File::create(output)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", options)?;
        zip.write_all(CONTENT_TYPES.as_bytes())?;
        zip.start_file("_rels/.rels", options)?;
        zip.write_all(ROOT_RELS.as_bytes())?;
        zip.start_file("word/document.xml", options)?;
        zip.write_all(document.as_bytes())?;
        if let Some((name, data)) = media {
            zip.start_file("word/_rels/document.xml.rels", options)?;
            let rels = format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{name}"/></Relationships>"#
            );
            zip.write_all(rels.as_bytes())?;
            // Images are already compressed; don't try again.
            let stored = options.compression_method(zip::CompressionMethod::Stored);
            zip.start_file(format!("word/media/{name}"), stored)?;
            zip.write_all(data)?;
        }
        zip.finish()?;
        Ok(())
    };
    run().map_err(|e| format!("Can't write DOCX: {e}"))
}

/// The image's size in EMU, shrunk to fit the page without changing its shape.
pub fn fit_emu(width: u32, height: u32) -> (u64, u64) {
    let w = width.max(1) as u64 * EMU_PER_PIXEL;
    let h = height.max(1) as u64 * EMU_PER_PIXEL;
    let shrink = (MAX_WIDTH_EMU as f64 / w as f64).min(MAX_HEIGHT_EMU as f64 / h as f64).min(1.0);
    ((w as f64 * shrink) as u64, (h as f64 * shrink) as u64)
}

pub fn write_image(output: &Path, name: &str, data: &[u8], width: u32, height: u32) -> Result<(), String> {
    let (cx, cy) = fit_emu(width, height);
    let document = format!(
        r#"{DOC_OPEN}<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="1" name="Image"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="Image"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>{DOC_CLOSE}"#
    );
    write_package(output, &document, Some((name, data)))
}

pub fn write_text(output: &Path, text: &str) -> Result<(), String> {
    let mut document = String::from(DOC_OPEN);
    for line in text.lines() {
        document.push_str(r#"<w:p><w:r><w:t xml:space="preserve">"#);
        document.push_str(&escape(line));
        document.push_str("</w:t></w:r></w:p>");
    }
    document.push_str(DOC_CLOSE);
    write_package(output, &document, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn entry(path: &Path, name: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut text = String::new();
        archive.by_name(name).unwrap().read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn large_images_shrink_to_the_page_and_keep_their_shape() {
        let (cx, cy) = fit_emu(4000, 2000);
        assert_eq!(cx, MAX_WIDTH_EMU);
        assert_eq!(cy, MAX_WIDTH_EMU / 2);
        let (cx, cy) = fit_emu(1000, 4000);
        assert_eq!(cy, MAX_HEIGHT_EMU);
        assert_eq!(cx, MAX_HEIGHT_EMU / 4);
    }

    #[test]
    fn small_images_are_not_enlarged() {
        assert_eq!(fit_emu(100, 50), (100 * EMU_PER_PIXEL, 50 * EMU_PER_PIXEL));
    }

    #[test]
    fn text_docx_escapes_markup() {
        let dir = crate::engines::WorkDir::new().unwrap();
        let out = dir.0.join("a.docx");
        write_text(&out, "a < b & c\nsecond").unwrap();
        let doc = entry(&out, "word/document.xml");
        assert!(doc.contains("a &lt; b &amp; c"));
        assert_eq!(doc.matches("<w:p>").count(), 2);
        assert!(entry(&out, "[Content_Types].xml").contains("wordprocessingml.document.main"));
    }

    #[test]
    fn image_docx_links_its_media() {
        let dir = crate::engines::WorkDir::new().unwrap();
        let out = dir.0.join("a.docx");
        write_image(&out, "image1.png", b"png-bytes", 10, 10).unwrap();
        assert!(entry(&out, "word/_rels/document.xml.rels").contains("media/image1.png"));
        assert!(entry(&out, "word/document.xml").contains(r#"r:embed="rId1""#));
    }

    #[test]
    fn control_characters_are_removed_as_xml_forbids_them() {
        assert_eq!(escape("a\u{1}b"), "ab");
    }
}
