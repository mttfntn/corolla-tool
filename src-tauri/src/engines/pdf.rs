//! PDF to images, text or DOCX, through PDFium.

use super::{docx, require_output, run_tool, Run};
use pdfium_render::prelude::*;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

/// PDFium is not safe to use from two threads at once.
static PDFIUM_LOCK: Mutex<()> = Mutex::new(());

/// Longest edge for a rendered page: about 200 DPI on A4.
const RENDER_EDGE: i32 = 2400;

fn open_pdfium(library: &Path) -> Result<Pdfium, String> {
    Pdfium::bind_to_library(library)
        .map(Pdfium::new)
        .map_err(|e| format!("Can't load PDFium: {e}"))
}

fn page_text(document: &PdfDocument) -> Result<String, String> {
    let mut text = String::new();
    for page in document.pages().iter() {
        let content = page.text().map_err(|e| format!("Can't read page text: {e}"))?;
        text.push_str(&content.all());
        text.push('\n');
    }
    Ok(text)
}

fn render_pages(run: &Run, document: &PdfDocument) -> Result<(), String> {
    let count = document.pages().len();
    let config = PdfRenderConfig::new().set_target_width(RENDER_EDGE).set_maximum_height(RENDER_EDGE);
    for (i, page) in document.pages().iter().enumerate() {
        let picture = page
            .render_with_config(&config)
            .map_err(|e| format!("Can't render page {}: {e}", i + 1))?
            .as_image();
        let path = if count == 1 {
            run.output.to_path_buf()
        } else {
            run.work.join(format!("{}-p{}.{}", run.stem, i + 1, run.target))
        };
        // JPEG has no alpha channel, so flatten both formats to plain RGB.
        picture
            .into_rgb8()
            .save(&path)
            .map_err(|e| format!("Can't save page {}: {e}", i + 1))?;
    }
    Ok(())
}

/// LibreOffice keeps layout far better, so prefer it when it's installed.
fn with_libreoffice(run: &Run) -> Result<(), String> {
    let soffice = run.tools.soffice.as_ref().ok_or("LibreOffice is missing")?;
    let mut command = Command::new(soffice);
    command
        .args(["--headless", "--infilter=writer_pdf_import", "--convert-to", "docx", "--outdir"])
        .arg(run.work)
        .arg(run.source);
    run_tool(run, &mut command)?;
    let produced = run.work.join(format!(
        "{}.docx",
        run.source.file_stem().unwrap_or_default().to_string_lossy()
    ));
    if produced != run.output {
        std::fs::rename(&produced, run.output).map_err(|e| e.to_string())?;
    }
    require_output(run.output)
}

pub fn convert(run: &Run) -> Result<(), String> {
    if run.target == "docx" && run.tools.soffice.is_some() && with_libreoffice(run).is_ok() {
        return Ok(());
    }
    let library = run.tools.pdfium.as_ref().ok_or("PDFium is missing")?;
    let _guard = PDFIUM_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let pdfium = open_pdfium(library)?;
    let document = pdfium
        .load_pdf_from_file(run.source, None)
        .map_err(|e| format!("Can't open PDF: {e}"))?;
    match run.target {
        "txt" => std::fs::write(run.output, page_text(&document)?).map_err(|e| e.to_string()),
        "docx" => docx::write_text(run.output, &page_text(&document)?),
        "jpg" | "png" => render_pages(run, &document),
        other => Err(format!("Unknown PDF target {other}")),
    }
}
