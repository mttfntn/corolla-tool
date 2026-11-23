//! 3D models: Assimp by default, Blender for FBX when it's installed.

use super::{require_output, run_tool, Run};
use crate::logfile;
use std::path::Path;
use std::process::Command;

/// Assimp's exporter id for a target.
pub fn assimp_id(target: &str) -> Option<&'static str> {
    Some(match target {
        "fbx" => "fbx",
        "glb" => "glb2",
        "gltf" => "gltf2",
        "dae" => "collada",
        "obj" => "obj",
        "stl" => "stlb", // binary STL: smaller, and what slicers expect
        _ => return None,
    })
}

/// Blender keeps FBX rigs and animation far better. Recent Blender versions
/// dropped COLLADA, so those conversions always stay with Assimp.
pub fn blender_handles(from: &str, to: &str) -> bool {
    const BLENDER: [&str; 5] = ["fbx", "glb", "gltf", "obj", "stl"];
    (from == "fbx" || to == "fbx") && BLENDER.contains(&from) && BLENDER.contains(&to)
}

const BLENDER_SCRIPT: &str = r#"
import sys, bpy
source, output, source_kind, target_kind = sys.argv[sys.argv.index("--") + 1:]
bpy.ops.wm.read_factory_settings(use_empty=True)
if source_kind == "fbx": bpy.ops.import_scene.fbx(filepath=source)
elif source_kind in ("glb", "gltf"): bpy.ops.import_scene.gltf(filepath=source)
elif source_kind == "obj": bpy.ops.wm.obj_import(filepath=source)
elif source_kind == "stl": bpy.ops.wm.stl_import(filepath=source)
if target_kind == "fbx": bpy.ops.export_scene.fbx(filepath=output, path_mode="COPY", embed_textures=True)
elif target_kind == "glb": bpy.ops.export_scene.gltf(filepath=output, export_format="GLB")
elif target_kind == "gltf": bpy.ops.export_scene.gltf(filepath=output, export_format="GLTF_SEPARATE")
elif target_kind == "obj": bpy.ops.wm.obj_export(filepath=output)
elif target_kind == "stl": bpy.ops.wm.stl_export(filepath=output)
"#;

fn with_blender(run: &Run) -> Result<(), String> {
    let blender = run.tools.blender.as_ref().ok_or("Blender is missing")?;
    // In the system temp folder, because `work` is delivered as output.
    let scripts = super::WorkDir::new()?;
    let script = scripts.0.join("convert.py");
    std::fs::write(&script, BLENDER_SCRIPT).map_err(|e| e.to_string())?;
    let mut command = Command::new(blender);
    command
        .args(["--background", "--factory-startup", "--python-exit-code", "1", "--python"])
        .arg(&script)
        .arg("--")
        .arg(run.source)
        .arg(run.output)
        .args([run.from, run.target]);
    run_tool(run, &mut command)?;
    require_output(run.output)
}

/// Converts any file Assimp can read into `run.target`, at `run.output`.
pub fn assimp_export(run: &Run, source: &Path) -> Result<(), String> {
    let assimp = run.tools.assimp.as_ref().ok_or("Assimp is missing")?;
    let id = assimp_id(run.target).ok_or("Unknown 3D target")?;
    let mut command = Command::new(assimp);
    command.arg("export").arg(source).arg(run.output).arg(format!("-f{id}"));
    run_tool(run, &mut command)?;
    require_output(run.output)
}

fn with_assimp(run: &Run) -> Result<(), String> {
    assimp_export(run, run.source)
}

fn clear_work(run: &Run) {
    if let Ok(entries) = std::fs::read_dir(run.work) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub fn convert(run: &Run) -> Result<(), String> {
    if run.tools.blender.is_some() && blender_handles(run.from, run.target) {
        match with_blender(run) {
            Ok(()) => return Ok(()),
            Err(error) => {
                logfile::line(format!("Blender failed, falling back to Assimp: {error}"));
                if run.tools.assimp.is_none() {
                    return Err(error);
                }
                clear_work(run); // A half-written export must not be delivered.
            }
        }
    }
    with_assimp(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_3d_target_has_an_exporter() {
        for (name, format) in &crate::formats::matrix().formats {
            if format.family == "model" {
                for target in &format.targets {
                    assert!(assimp_id(target).is_some(), "{name} -> {target}");
                }
            }
        }
    }

    #[test]
    fn blender_only_takes_fbx_conversions_it_can_do() {
        assert!(blender_handles("fbx", "glb"));
        assert!(blender_handles("obj", "fbx"));
        assert!(!blender_handles("glb", "obj"), "no FBX involved");
        assert!(!blender_handles("fbx", "dae"), "COLLADA removed from Blender");
        assert!(!blender_handles("dae", "fbx"), "COLLADA removed from Blender");
    }
}
