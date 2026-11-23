//! STEP (CAD solids) to meshes. Mayo does the meshing where it's bundled, which is
//! Windows; FreeCAD is the fallback if it's installed. FBX and DAE go on through Assimp.
//! Mesh to STEP isn't offered: it would be a heavy shell of triangles, not a solid.

use super::{mesh3d, require_output, run_tool, Quality, Run, Tools, WorkDir};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn find_freecad() -> Option<PathBuf> {
    let known = [
        "/Applications/FreeCAD.app/Contents/Resources/bin/freecadcmd",
        "/Applications/FreeCAD.app/Contents/MacOS/FreeCADCmd",
    ];
    if let Some(found) = known.iter().map(PathBuf::from).find(|p| p.is_file()) {
        return Some(found);
    }
    let root = Path::new("C:\\Program Files");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("FreeCAD"))
        .map(|entry| entry.path().join("bin").join("FreeCADCmd.exe"))
        .filter(|path| path.is_file())
        .collect();
    versions.sort();
    versions.pop()
}

/// What each meshing engine can write by itself.
fn mayo_writes(target: &str) -> bool {
    matches!(target, "stl" | "obj" | "glb" | "gltf")
}

pub fn available(tools: &Tools, target: &str) -> bool {
    if tools.mayo.is_some() && mayo_writes(target) {
        return true;
    }
    let meshes_at_all = tools.mayo.is_some() || tools.freecad.is_some();
    let direct = tools.freecad.is_some() && target == "stl";
    direct || (meshes_at_all && tools.assimp.is_some() && mesh3d::assimp_id(target).is_some())
}

fn mayo_quality(quality: Quality) -> &'static str {
    match quality {
        Quality::High => "Precise",
        Quality::Balanced => "Normal",
        Quality::Small => "Coarse",
    }
}

/// Mayo's settings file. Its glTF writer defaults to binary whatever the file is
/// called, so a `.gltf` has to ask for JSON.
fn mayo_settings(quality: Quality, output: &Path) -> String {
    let json = output.extension().is_some_and(|extension| extension == "gltf");
    let format = if json { "Json" } else { "Binary" };
    format!("[meshing]\nmeshingQuality={}\n[export]\nGLTF\\format={format}\n", mayo_quality(quality))
}

fn with_mayo(run: &Run, mayo: &Path, output: &Path) -> Result<(), String> {
    // In the system temp folder, because `work` is delivered as output.
    let settings = WorkDir::new()?;
    let ini = settings.0.join("mayo.ini");
    std::fs::write(&ini, mayo_settings(run.quality, output)).map_err(|e| e.to_string())?;
    let mut command = Command::new(mayo);
    command.arg("--no-progress").arg("-u").arg(&ini).arg("-e").arg(output).arg(run.source);
    run_tool(run, &mut command)?;
    require_output(output)
}

const FREECAD_SCRIPT: &str = r#"
import sys, Part, MeshPart
source, output, linear, angular = sys.argv[-4:]
shape = Part.read(source)
mesh = MeshPart.meshFromShape(Shape=shape, LinearDeflection=float(linear), AngularDeflection=float(angular))
mesh.write(output)
"#;

fn with_freecad(run: &Run, freecad: &Path, stl: &Path) -> Result<(), String> {
    let scripts = WorkDir::new()?;
    let script = scripts.0.join("step_to_stl.py");
    std::fs::write(&script, FREECAD_SCRIPT).map_err(|e| e.to_string())?;
    let (linear, angular) = match run.quality {
        Quality::High => ("0.1", "0.2"),
        Quality::Balanced => ("0.5", "0.35"),
        Quality::Small => ("1.5", "0.6"),
    };
    let mut command = Command::new(freecad);
    command.arg(&script).arg(run.source).arg(stl).args([linear, angular]);
    run_tool(run, &mut command)?;
    require_output(stl)
}

pub fn convert(run: &Run) -> Result<(), String> {
    if let Some(mayo) = &run.tools.mayo {
        if mayo_writes(run.target) {
            return with_mayo(run, mayo, run.output);
        }
        // GLB keeps the parts and colours on the way to FBX or DAE.
        let middle = run.work.join("intermediate-step.glb");
        with_mayo(run, mayo, &middle)?;
        let result = mesh3d::assimp_export(run, &middle);
        let _ = std::fs::remove_file(&middle);
        return result;
    }
    let freecad = run.tools.freecad.as_ref().ok_or("No STEP engine is available")?;
    if run.target == "stl" {
        return with_freecad(run, freecad, run.output);
    }
    let middle = run.work.join("intermediate-step.stl");
    with_freecad(run, freecad, &middle)?;
    let result = mesh3d::assimp_export(run, &middle);
    let _ = std::fs::remove_file(&middle);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_ask_for_the_format_the_file_name_promises() {
        let gltf = mayo_settings(Quality::High, Path::new("a.gltf"));
        assert!(gltf.contains("meshingQuality=Precise"));
        assert!(gltf.contains("GLTF\\format=Json"));
        assert!(mayo_settings(Quality::Small, Path::new("a.glb")).contains("GLTF\\format=Binary"));
    }

    #[test]
    fn step_targets_depend_on_what_is_installed() {
        assert!(!available(&Tools::default(), "stl"));
        let mayo = Tools { mayo: Some("m".into()), ..Default::default() };
        assert!(available(&mayo, "glb"));
        assert!(!available(&mayo, "fbx"), "FBX needs Assimp too");
        let both = Tools { assimp: Some("a".into()), ..mayo.clone() };
        assert!(available(&both, "fbx") && available(&both, "dae"));
        let freecad = Tools { freecad: Some("f".into()), ..Default::default() };
        assert!(available(&freecad, "stl"));
        assert!(!available(&freecad, "glb"), "FreeCAD only gives STL here");
    }
}
