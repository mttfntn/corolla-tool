//! A tiny PDF writer: one JPEG per page, or pages of monospaced text.

use std::io::{self, Write};

const A4: (f32, f32) = (595.0, 842.0);
const MARGIN: f32 = 56.0;
const FONT_SIZE: f32 = 10.0;
const LINE_HEIGHT: f32 = 14.0;
/// Courier is exactly 0.6 em wide, which makes wrapping exact.
const CHAR_WIDTH: f32 = FONT_SIZE * 0.6;

struct Builder<W: Write> {
    out: W,
    written: usize,
    /// Byte offset of each object, by number minus one. Zero until it's written.
    offsets: Vec<usize>,
}

impl Builder<Vec<u8>> {
    fn new() -> Self {
        Self::on(Vec::new()).expect("writing to memory can't fail")
    }

    fn object(&mut self, head: &str, stream: Option<&[u8]>) {
        self.add(head, stream).expect("writing to memory can't fail");
    }

    fn finish(self) -> Vec<u8> {
        self.close().expect("writing to memory can't fail")
    }
}

impl<W: Write> Builder<W> {
    fn on(out: W) -> io::Result<Self> {
        let mut builder = Self { out, written: 0, offsets: Vec::new() };
        builder.put(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n")?;
        Ok(builder)
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.written += bytes.len();
        self.out.write_all(bytes)
    }

    /// Takes the next object number, to be written later with `write`.
    fn reserve(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len()
    }

    fn write(&mut self, number: usize, head: &str, stream: Option<&[u8]>) -> io::Result<()> {
        self.offsets[number - 1] = self.written;
        self.put(format!("{number} 0 obj\n{head}\n").as_bytes())?;
        if let Some(data) = stream {
            self.put(b"stream\n")?;
            self.put(data)?;
            self.put(b"\nendstream\n")?;
        }
        self.put(b"endobj\n")
    }

    /// Writes the next object number.
    fn add(&mut self, head: &str, stream: Option<&[u8]>) -> io::Result<usize> {
        let number = self.reserve();
        self.write(number, head, stream)?;
        Ok(number)
    }

    fn close(mut self) -> io::Result<W> {
        let table = self.written;
        let count = self.offsets.len() + 1;
        self.put(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes())?;
        for offset in self.offsets.clone() {
            self.put(format!("{offset:010} 00000 n \n").as_bytes())?;
        }
        self.put(format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n").as_bytes())?;
        Ok(self.out)
    }
}

/// A PDF with one JPEG per page, written as it goes so the images never have to
/// fit in memory together. Each page is exactly its image's size, at 150 pixels
/// per inch.
pub struct JpegPages<W: Write> {
    pdf: Builder<W>,
    pages_object: usize,
    kids: Vec<String>,
}

impl<W: Write> JpegPages<W> {
    pub fn new(out: W) -> io::Result<Self> {
        let mut pdf = Builder::on(out)?;
        let catalog = pdf.reserve();
        let pages_object = pdf.reserve();
        pdf.write(catalog, &format!("<< /Type /Catalog /Pages {pages_object} 0 R >>"), None)?;
        Ok(Self { pdf, pages_object, kids: Vec::new() })
    }

    pub fn page_count(&self) -> usize {
        self.kids.len()
    }

    pub fn add(&mut self, jpeg: &[u8], width: u32, height: u32, components: u8) -> io::Result<()> {
        let page_w = width as f32 * 72.0 / 150.0;
        let page_h = height as f32 * 72.0 / 150.0;
        let colour = match components {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let content = format!("q {page_w} 0 0 {page_h} 0 0 cm /Im0 Do Q");
        let page = self.pdf.reserve();
        let contents = self.pdf.add(&format!("<< /Length {} >>", content.len()), Some(content.as_bytes()))?;
        let image = self.pdf.add(
            &format!(
                "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {colour} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>",
                jpeg.len()
            ),
            Some(jpeg),
        )?;
        self.pdf.write(
            page,
            &format!(
                "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {page_w} {page_h}] /Contents {contents} 0 R /Resources << /XObject << /Im0 {image} 0 R >> >> >>",
                self.pages_object
            ),
            None,
        )?;
        self.kids.push(format!("{page} 0 R"));
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        let head = format!("<< /Type /Pages /Kids [{}] /Count {} >>", self.kids.join(" "), self.kids.len());
        let number = self.pages_object;
        self.pdf.write(number, &head, None)?;
        self.pdf.close()
    }
}

/// One page exactly the size of the image.
pub fn single_jpeg(jpeg: &[u8], width: u32, height: u32, components: u8) -> Vec<u8> {
    let mut pdf = JpegPages::new(Vec::new()).expect("writing to memory can't fail");
    pdf.add(jpeg, width, height, components).expect("writing to memory can't fail");
    pdf.finish().expect("writing to memory can't fail")
}

/// Wraps to a column count, keeping blank lines, breaking at spaces if it can.
pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest: Vec<char> = raw.replace('\t', "    ").chars().collect();
        while rest.len() > columns {
            let cut = rest[..=columns].iter().rposition(|c| *c == ' ').filter(|i| *i > 0).unwrap_or(columns);
            lines.push(rest[..cut].iter().collect());
            let skip = if rest[cut] == ' ' { cut + 1 } else { cut };
            rest.drain(..skip);
        }
        lines.push(rest.into_iter().collect());
    }
    lines
}

/// Encodes a char for the standard fonts' WinAnsi encoding, as PDF string bytes.
fn push_escaped(out: &mut Vec<u8>, c: char) {
    let byte = match c as u32 {
        0x20..=0x7E | 0xA0..=0xFF => c as u32 as u8,
        0x20AC => 0x80,
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201C => 0x93,
        0x201D => 0x94,
        0x2022 => 0x95,
        0x2013 => 0x96,
        0x2014 => 0x97,
        0x2026 => 0x85,
        _ => b'?', // The standard fonts can't draw it, for instance Cyrillic or CJK.
    };
    if matches!(byte, b'(' | b')' | b'\\') {
        out.push(b'\\');
    }
    out.push(byte);
}

/// True if every character can be drawn by `text_pages`.
pub fn fully_encodable(text: &str) -> bool {
    text.chars().all(|c| {
        c.is_whitespace() || {
            let mut out = Vec::new();
            push_escaped(&mut out, c);
            out != *b"?" || c == '?'
        }
    })
}

pub fn text_pages(text: &str) -> Vec<u8> {
    let columns = ((A4.0 - 2.0 * MARGIN) / CHAR_WIDTH) as usize;
    let per_page = ((A4.1 - 2.0 * MARGIN) / LINE_HEIGHT) as usize;
    let lines = wrap(text, columns);
    let pages: Vec<&[String]> = if lines.is_empty() {
        vec![&[]]
    } else {
        lines.chunks(per_page).collect()
    };

    let mut pdf = Builder::new();
    pdf.object("<< /Type /Catalog /Pages 2 0 R >>", None);
    // Objects: 1 catalog, 2 pages, 3 font, then a page and its content for each page.
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
    pdf.object(
        &format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
        None,
    );
    pdf.object(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>",
        None,
    );
    for (i, page) in pages.iter().enumerate() {
        pdf.object(
            &format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {} 0 R /Resources << /Font << /F0 3 0 R >> >> >>",
                A4.0,
                A4.1,
                5 + i * 2
            ),
            None,
        );
        let mut content =
            format!("BT /F0 {FONT_SIZE} Tf {LINE_HEIGHT} TL {MARGIN} {} Td\n", A4.1 - MARGIN).into_bytes();
        for line in page.iter() {
            content.push(b'(');
            for c in line.chars() {
                push_escaped(&mut content, c);
            }
            content.extend(b") Tj T*\n");
        }
        content.extend(b"ET");
        pdf.object(&format!("<< /Length {} >>", content.len()), Some(&content));
    }
    pdf.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_spaces_and_keeps_blank_lines() {
        assert_eq!(wrap("aaa bbb ccc\n\nd", 7), vec!["aaa bbb", "ccc", "", "d"]);
    }

    #[test]
    fn wrap_splits_long_words() {
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn long_text_gets_several_pages() {
        let text = "line\n".repeat(200);
        let pdf = String::from_utf8_lossy(&text_pages(&text)).to_string();
        assert!(pdf.starts_with("%PDF-1.4"));
        assert!(pdf.contains("/Count 4"), "200 lines at 52 per page");
        assert!(pdf.trim_end().ends_with("%%EOF"));
    }

    #[test]
    fn xref_offsets_point_at_their_objects() {
        let pdf = text_pages("hello");
        let text = String::from_utf8_lossy(&pdf).to_string();
        let table = text.find("xref\n").unwrap();
        let entries: Vec<&str> = text[table..].lines().skip(3).take_while(|l| l.ends_with(" n ")).collect();
        assert!(!entries.is_empty());
        for (i, entry) in entries.iter().enumerate() {
            let offset: usize = entry[..10].parse().unwrap();
            let expected = format!("{} 0 obj", i + 1);
            assert!(pdf[offset..].starts_with(expected.as_bytes()), "object {}", i + 1);
        }
    }

    #[test]
    fn italian_accents_are_drawable_and_parens_escaped() {
        assert!(fully_encodable("perché è più città €"));
        assert!(!fully_encodable("привет"));
        let mut out = Vec::new();
        push_escaped(&mut out, '(');
        assert_eq!(out, b"\\(");
    }

    #[test]
    fn several_jpeg_pages_share_one_page_tree() {
        let mut pdf = JpegPages::new(Vec::new()).unwrap();
        pdf.add(b"ONE", 150, 150, 3).unwrap();
        pdf.add(b"TWO", 300, 150, 1).unwrap();
        assert_eq!(pdf.page_count(), 2);
        let bytes = pdf.finish().unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        assert!(text.contains("/Count 2"));
        assert!(text.contains("ONE") && text.contains("TWO"));
        assert!(text.contains("/MediaBox [0 0 144 72]"));
        // Objects can be written out of order, so check each offset points at its object.
        let table = text.find("xref\n").unwrap();
        let entries: Vec<&str> = text[table..].lines().skip(3).take_while(|l| l.ends_with(" n ")).collect();
        for (i, entry) in entries.iter().enumerate() {
            let offset: usize = entry[..10].parse().unwrap();
            assert!(bytes[offset..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {}", i + 1);
        }
    }

    #[test]
    fn jpeg_page_embeds_the_data_untouched() {
        let pdf = single_jpeg(b"JPEGDATA", 150, 300, 3);
        let text = String::from_utf8_lossy(&pdf).to_string();
        assert!(text.contains("JPEGDATA"));
        assert!(text.contains("/MediaBox [0 0 72 144]"));
        assert!(text.contains("/DeviceRGB"));
    }
}
