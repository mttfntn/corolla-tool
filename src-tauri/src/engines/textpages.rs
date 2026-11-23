//! TXT to PDF, images, or subtitles.

use super::{docx, images, pdf_writer, subtitles, Run};
use crate::logfile;

const IMAGE_WIDTH: usize = 1200;
const IMAGE_PADDING: usize = 60;
const IMAGE_FONT: usize = 20;
const IMAGE_LINE: usize = 30;
const IMAGE_COLUMNS: usize = 90;
/// Beyond this a picture of text is unusable, so it's better to say so.
const MAX_IMAGE_LINES: usize = 500;

fn text_svg(text: &str) -> Result<String, String> {
    let lines = pdf_writer::wrap(text, IMAGE_COLUMNS);
    if lines.len() > MAX_IMAGE_LINES {
        return Err(format!(
            "This text is too long for one image ({} lines, limit {MAX_IMAGE_LINES}). Try PDF instead.",
            lines.len()
        ));
    }
    let height = IMAGE_PADDING * 2 + lines.len().max(1) * IMAGE_LINE;
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{IMAGE_WIDTH}" height="{height}"><rect width="100%" height="100%" fill="#ffffff"/><g font-family="Consolas, Menlo, Courier New, monospace" font-size="{IMAGE_FONT}" fill="#111111">"##
    );
    for (i, line) in lines.iter().enumerate() {
        let y = IMAGE_PADDING + IMAGE_FONT + i * IMAGE_LINE;
        svg += &format!(
            r#"<text x="{IMAGE_PADDING}" y="{y}" xml:space="preserve">{}</text>"#,
            docx::escape(line)
        );
    }
    svg += "</g></svg>";
    Ok(svg)
}

pub fn convert(run: &Run) -> Result<(), String> {
    let text = subtitles::read_text(run.source)?;
    match run.target {
        "srt" | "vtt" => {
            let cues = subtitles::from_text(&text);
            if cues.is_empty() {
                return Err("This text file is empty".into());
            }
            let result = if run.target == "srt" { subtitles::to_srt(&cues) } else { subtitles::to_vtt(&cues) };
            std::fs::write(run.output, result).map_err(|e| e.to_string())
        }
        "pdf" => {
            if !pdf_writer::fully_encodable(&text) {
                logfile::line(format!(
                    "{}: some characters can't be drawn in the PDF font and became '?'",
                    run.source.display()
                ));
            }
            std::fs::write(run.output, pdf_writer::text_pages(&text)).map_err(|e| e.to_string())
        }
        "png" | "jpg" => {
            let svg = text_svg(&text)?;
            let tree = images::parse_svg(svg.as_bytes(), None)?;
            if run.target == "png" {
                return render(&tree, run.output);
            }
            let middle = run.work.join("intermediate-text.png");
            render(&tree, &middle)?;
            let picture = image::open(&middle).map_err(|e| e.to_string())?.into_rgb8();
            let _ = std::fs::remove_file(&middle);
            picture.save(run.output).map_err(|e| format!("Can't write JPG: {e}"))
        }
        other => Err(format!("Unknown text target {other}")),
    }
}

fn render(tree: &usvg::Tree, output: &std::path::Path) -> Result<(), String> {
    images::render_tree_to_png(tree, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_in_the_svg_and_it_parses() {
        let svg = text_svg("a < b & c").unwrap();
        assert!(svg.contains("a &lt; b &amp; c"));
        images::parse_svg(svg.as_bytes(), None).unwrap();
    }

    #[test]
    fn huge_text_is_refused_with_a_reason() {
        let error = text_svg(&"line\n".repeat(MAX_IMAGE_LINES + 1)).unwrap_err();
        assert!(error.contains("too long"));
    }
}
