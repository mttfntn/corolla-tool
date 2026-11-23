//! ZIP, TAR, GZIP and RAR: unpack everything, then pack it again.

use super::{require_output, run_tool, Run, WorkDir};
use std::path::{Path, PathBuf};
use std::process::Command;

fn extract(run: &Run, tool: &Path, archive: &Path, into: &Path) -> Result<(), String> {
    let mut command = Command::new(tool);
    command.args(["x", "-y", "-bd"]).arg(archive).arg(format!("-o{}", into.display()));
    run_tool(run, &mut command)
}

/// `.tar.gz` comes apart in two steps: if unpacking left a lone tar, open that too.
fn lone_tar(folder: &Path) -> Option<PathBuf> {
    let entries: Vec<PathBuf> = std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).collect();
    match entries.as_slice() {
        [only] if only.extension().is_some_and(|e| e.eq_ignore_ascii_case("tar")) => Some(only.clone()),
        _ => None,
    }
}

fn pack_with_7z(run: &Run, tool: &Path, kind: &str, contents: &Path, output: &Path) -> Result<(), String> {
    let mut command = Command::new(tool);
    command
        .args(["a", "-bd", "-y", &format!("-t{kind}")])
        .arg(output)
        .arg(".")
        .current_dir(contents);
    run_tool(run, &mut command)
}

pub fn convert(run: &Run) -> Result<(), String> {
    let tool = run.tools.sevenzip.as_ref().ok_or("7-Zip is missing")?;
    // Inside `work`'s sibling: `work` must end up holding only the final file.
    let unpacked = WorkDir::new()?;
    extract(run, tool, run.source, &unpacked.0)?;
    if let Some(tar) = lone_tar(&unpacked.0) {
        // A source that is itself a tar holding one tar stays as it is: only
        // the compressed forms wrap a tar as part of their format.
        if run.from == "gzip" {
            extract(run, tool, &tar, &unpacked.0)?;
            std::fs::remove_file(&tar).map_err(|e| e.to_string())?;
        }
    }

    match run.target {
        "zip" | "tar" => pack_with_7z(run, tool, run.target, &unpacked.0, run.output)?,
        "gzip" => {
            let staging = WorkDir::new()?;
            let tar = staging.0.join(format!("{}.tar", run.stem));
            pack_with_7z(run, tool, "tar", &unpacked.0, &tar)?;
            let mut command = Command::new(tool);
            command.args(["a", "-bd", "-y", "-tgzip"]).arg(run.output).arg(&tar);
            run_tool(run, &mut command)?;
        }
        "rar" => {
            let rar = run.tools.rar.as_ref().ok_or("Creating RAR needs WinRAR or rar installed")?;
            let mut command = Command::new(rar);
            command.args(["a", "-r", "-idq", "-ep1"]).arg(run.output).arg(".").current_dir(&unpacked.0);
            run_tool(run, &mut command)?;
        }
        other => return Err(format!("Unknown archive target {other}")),
    }
    require_output(run.output)
}
