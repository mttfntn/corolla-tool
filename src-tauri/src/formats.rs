//! The conversion matrix from formats.json, and what the wheel should offer
//! for a given set of dragged files.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub struct Format {
    pub family: String,
    pub targets: Vec<String>,
}

#[derive(Deserialize)]
pub struct Matrix {
    pub aliases: HashMap<String, String>,
    pub formats: HashMap<String, Format>,
}

/// formats.json, compiled into the app and parsed once.
pub fn matrix() -> &'static Matrix {
    static MATRIX: OnceLock<Matrix> = OnceLock::new();
    MATRIX.get_or_init(|| {
        serde_json::from_str(include_str!("../formats.json")).expect("formats.json is valid")
    })
}

/// The canonical format of a path, or None if it isn't one we convert.
/// Folders have no format, so they make a drag unconvertible.
pub fn format_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.to_lowercase();
    let ext = if name.ends_with(".tar.gz") {
        "gzip".to_string()
    } else {
        Path::new(&name).extension()?.to_str()?.to_string()
    };
    let m = matrix();
    let canonical = m.aliases.get(&ext).cloned().unwrap_or(ext);
    m.formats.contains_key(&canonical).then_some(canonical)
}

/// File extension to write for a target format.
pub fn extension_for(target: &str) -> &str {
    match target {
        "gzip" => "tar.gz",
        other => other,
    }
}

/// Targets valid for every file in the set, in the first file's order, each with
/// whether it's greyed out.
///
/// A file's own format counts as valid for it, because that file is simply
/// skipped. A target every file already has is pointless, so it stays on the
/// wheel but greyed out. `can_convert` removes pairs this machine can't do.
pub fn common_targets(formats: &[String], can_convert: &dyn Fn(&str, &str) -> bool) -> Vec<(String, bool)> {
    let m = matrix();
    let Some(first) = formats.first() else { return Vec::new() };
    let Some(first_format) = m.formats.get(first) else { return Vec::new() };

    let mut candidates: Vec<&String> = first_format.targets.iter().collect();
    // A file's own format never appears in its target list, so add it back:
    // other files in the batch may be able to become it.
    candidates.push(first);

    candidates
        .into_iter()
        .filter(|target| {
            formats.iter().all(|f| {
                f == *target
                    || (m.formats.get(f).is_some_and(|x| x.targets.contains(*target))
                        && can_convert(f, target))
            })
        })
        .map(|target| (target.clone(), formats.iter().all(|f| f == target)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &str, _: &str) -> bool {
        true
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn matrix_targets_all_exist_and_never_self_reference() {
        let m = matrix();
        for (name, f) in &m.formats {
            assert!(!f.targets.is_empty(), "{name} has no targets");
            assert!(!f.targets.contains(name), "{name} targets itself");
            for t in &f.targets {
                let known = m.formats.contains_key(t) || t == "docx";
                assert!(known, "{name} -> unknown target {t}");
            }
        }
    }

    #[test]
    fn aliases_and_compound_extensions() {
        assert_eq!(format_of(Path::new("a/B.JPEG")).as_deref(), Some("jpg"));
        assert_eq!(format_of(Path::new("x.tif")).as_deref(), Some("tiff"));
        assert_eq!(format_of(Path::new("backup.tar.gz")).as_deref(), Some("gzip"));
        assert_eq!(format_of(Path::new("model.GLB")).as_deref(), Some("glb"));
        assert_eq!(format_of(Path::new("part.sldprt")), None);
        assert_eq!(format_of(Path::new("no_extension")), None);
    }

    /// The active targets, leaving out the greyed-out own format.
    fn active(formats: &[&str], can: &dyn Fn(&str, &str) -> bool) -> Vec<String> {
        common_targets(&v(formats), can)
            .into_iter()
            .filter(|(_, disabled)| !disabled)
            .map(|(target, _)| target)
            .collect()
    }

    #[test]
    fn single_model_offers_the_other_five() {
        assert_eq!(active(&["fbx"], &all), v(&["glb", "gltf", "dae", "obj", "stl"]));
    }

    #[test]
    fn single_jpg_matches_reference_list() {
        assert_eq!(active(&["jpg"], &all), v(&["png", "webp", "heic", "tiff", "avif", "bmp", "exr", "hdr", "ico", "pdf", "docx"]));
    }

    #[test]
    fn own_format_is_listed_greyed_out() {
        assert_eq!(common_targets(&v(&["zip"]), &all).last(), Some(&("zip".to_string(), true)));
        assert_eq!(common_targets(&v(&["wav", "wav"]), &all).last(), Some(&("wav".to_string(), true)));
    }

    #[test]
    fn same_format_batch_is_like_single() {
        assert_eq!(active(&["wav", "wav", "wav"], &all), active(&["wav"], &all));
    }

    #[test]
    fn mixed_family_members_can_target_each_other() {
        let t = active(&["png", "jpg"], &all);
        assert!(t.contains(&"jpg".to_string()), "png can become jpg, jpg is skipped: {t:?}");
        assert!(t.contains(&"png".to_string()), "jpg can become png, png is skipped: {t:?}");
        assert!(t.contains(&"webp".to_string()));
    }

    #[test]
    fn incompatible_families_offer_nothing() {
        assert!(common_targets(&v(&["png", "stl"]), &all).is_empty());
        assert!(common_targets(&v(&["wav", "zip"]), &all).is_empty());
    }

    #[test]
    fn video_and_audio_share_only_mp3() {
        assert_eq!(active(&["mp4", "wav"], &all), v(&["mp3"]));
    }

    #[test]
    fn unavailable_engine_hides_the_wedge() {
        let no_rar = |_: &str, t: &str| t != "rar";
        assert_eq!(active(&["zip"], &no_rar), v(&["tar", "gzip"]));
    }

    #[test]
    fn empty_and_unknown_inputs() {
        assert!(common_targets(&[], &all).is_empty());
        assert!(common_targets(&v(&["nope"]), &all).is_empty());
    }

    #[test]
    fn step_converts_to_meshes_only() {
        assert_eq!(format_of(Path::new("part.STP")).as_deref(), Some("step"));
        assert_eq!(active(&["step"], &all), v(&["stl", "obj", "glb", "gltf", "fbx", "dae"]));
        assert!(!active(&["stl"], &all).contains(&"step".to_string()));
    }
}
