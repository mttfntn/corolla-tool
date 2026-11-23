//! What the wheel offers for a set of dragged files: format conversions plus the
//! compress and resize tools, arranged the way the user chose in Settings.

use crate::formats;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Op {
    /// Keeps the format. `quality` is 1 to 100; `max_edge` also limits the longest side.
    Compress {
        quality: u8,
        #[serde(default)]
        max_edge: Option<u32>,
    },
    ResizeDivide { by: u32 },
    ResizeEdge { edge: u32 },
}

/// A circle the user can add, edit or delete in Settings.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CustomTool {
    pub id: String,
    pub line1: String,
    #[serde(default)]
    pub line2: String,
    pub op: Op,
    pub families: Vec<String>,
}

impl CustomTool {
    /// Added to the output's name: `photo_rsz2.jpg`.
    pub fn suffix(&self) -> String {
        let slug: String = format!("{}{}", self.line1, self.line2)
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase();
        format!("_{slug}")
    }

    pub fn label(&self) -> String {
        format!("{} {}", self.line1, self.line2).trim().to_string()
    }
}

pub fn default_tools() -> Vec<CustomTool> {
    let resize = |id: &str, line2: &str, by| CustomTool {
        id: id.into(),
        line1: "RSZ".into(),
        line2: line2.into(),
        op: Op::ResizeDivide { by },
        families: vec!["image".into()],
    };
    vec![
        CustomTool {
            id: "cmprss".into(),
            line1: "CMPRSS".into(),
            line2: String::new(),
            op: Op::Compress { quality: 70, max_edge: None },
            families: vec!["image".into(), "pdf".into(), "video".into()],
        },
        resize("rsz2", "/2", 2),
        resize("rsz4", "/4", 4),
    ]
}

/// The user's arrangement for one kind of file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct FamilyLayout {
    pub order: Vec<String>,
    pub hidden: Vec<String>,
}

pub type Layout = HashMap<String, FamilyLayout>;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Convert(String),
    /// All the dragged images become one PDF.
    CombinePdf,
    Process { op: Op, label: String, suffix: String },
}

impl Action {
    /// Short name for the wheel and the summary.
    pub fn label(&self) -> String {
        match self {
            Action::Convert(target) => target.to_uppercase(),
            Action::CombinePdf => "PDF".into(),
            Action::Process { label, .. } => label.clone(),
        }
    }
}

/// One circle on the wheel.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub line1: String,
    pub line2: String,
    /// Every dragged file already has this format.
    pub disabled: bool,
    #[serde(skip)]
    pub action: Option<Action>,
}

fn convert_item(target: &str, disabled: bool) -> Item {
    Item {
        id: target.to_string(),
        line1: target.to_uppercase(),
        line2: String::new(),
        disabled,
        action: (!disabled).then(|| Action::Convert(target.to_string())),
    }
}

fn tool_item(tool: &CustomTool) -> Item {
    Item {
        id: tool.id.clone(),
        line1: tool.line1.clone(),
        line2: tool.line2.clone(),
        disabled: false,
        action: Some(Action::Process {
            op: tool.op.clone(),
            label: tool.label(),
            suffix: tool.suffix(),
        }),
    }
}

/// STEP and mesh formats share one arrangement in Settings.
pub fn layout_key(family: &str) -> &str {
    match family {
        "cad" => "model",
        other => other,
    }
}

fn family_of(format: &str) -> Option<&'static str> {
    formats::matrix().formats.get(format).map(|f| f.family.as_str())
}

fn arrange(items: &mut Vec<Item>, layout: Option<&FamilyLayout>, keep_hidden: bool) {
    let Some(layout) = layout else { return };
    let rank = |id: &str| layout.order.iter().position(|o| o == id).unwrap_or(layout.order.len());
    items.sort_by_key(|item| rank(&item.id));
    if !keep_hidden {
        items.retain(|item| !layout.hidden.contains(&item.id));
    }
}

/// Everything to show for these files, in display order.
pub fn offer(
    formats_in_drag: &[String],
    custom: &[CustomTool],
    layout: &Layout,
    can_convert: &dyn Fn(&str, &str) -> bool,
    can_process: &dyn Fn(&str, &Op) -> bool,
    can_combine: &dyn Fn(&[String]) -> bool,
) -> Vec<Item> {
    let Some(first) = formats_in_drag.first() else { return Vec::new() };
    let mut items: Vec<Item> = formats::common_targets(formats_in_drag, can_convert)
        .into_iter()
        .map(|(target, disabled)| convert_item(&target, disabled))
        .collect();

    let all_images = formats_in_drag.iter().all(|f| family_of(f) == Some("image"));
    if formats_in_drag.len() > 1 && all_images && can_combine(formats_in_drag) {
        if let Some(position) = items.iter().position(|i| i.id == "pdf") {
            items[position].line2 = "EACH".into();
            items.insert(position, combine_item());
        }
    }

    for tool in custom {
        let fits = formats_in_drag.iter().all(|f| {
            family_of(f).is_some_and(|family| tool.families.iter().any(|t| t == family))
                && can_process(f, &tool.op)
        });
        if fits {
            items.push(tool_item(tool));
        }
    }

    let key = family_of(first).map(layout_key).unwrap_or_default();
    arrange(&mut items, layout.get(key), false);
    items
}

fn combine_item() -> Item {
    Item {
        id: "combine".into(),
        line1: "PDF".into(),
        line2: String::new(),
        disabled: false,
        action: Some(Action::CombinePdf),
    }
}

/// Every circle a kind of file can show, for arranging in Settings.
pub fn catalog(family: &str, custom: &[CustomTool], layout: &Layout) -> Vec<Item> {
    let matrix = formats::matrix();
    let mut names: Vec<&String> = matrix
        .formats
        .iter()
        .filter(|(_, format)| layout_key(&format.family) == family)
        .map(|(name, _)| name)
        .collect();
    names.sort();
    // Starting from a typical file's list, so this order matches a real drag's.
    let typical = match family {
        "image" => "jpg",
        "pdf" => "pdf",
        "video" => "mp4",
        "audio" => "mp3",
        "model" => "fbx",
        "archive" => "zip",
        "subtitle" => "srt",
        "text" => "txt",
        "font" => "ttf",
        _ => "",
    };
    let mut items = Vec::new();
    let mut add = |id: &str| {
        if !items.iter().any(|item: &Item| item.id == id) {
            items.push(convert_item(id, false));
        }
    };
    if let Some(format) = matrix.formats.get(typical) {
        format.targets.iter().for_each(|target| add(target));
        add(typical);
    }
    for name in &names {
        matrix.formats[*name].targets.iter().for_each(|target| add(target));
        add(name);
    }
    if family == "image" {
        if let Some(position) = items.iter().position(|item| item.id == "pdf") {
            items[position].line2 = "EACH".into();
            items.insert(position, combine_item());
        }
    }
    for tool in custom {
        if tool.families.iter().any(|f| f == family) {
            items.push(tool_item(tool));
        }
    }
    arrange(&mut items, layout.get(family), true);
    items
}

/// `img2` before `img10`, ignoring case. Runs of digits compare as numbers.
pub fn natural_cmp(a: &Path, b: &Path) -> Ordering {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (a, b) = (name(a), name(b));
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        digits.push(c);
                    }
                    digits
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(p), Some(q)) => {
                if p != q {
                    return p.cmp(&q);
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn show(items: &[&str], layout: &Layout) -> Vec<Item> {
        offer(&v(items), &default_tools(), layout, &|_, _| true, &|_, _| true, &|_| true)
    }

    fn ids(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn own_format_is_grey_and_has_no_action() {
        let items = show(&["jpg"], &Layout::new());
        let own = items.iter().find(|i| i.id == "jpg").unwrap();
        assert!(own.disabled);
        assert_eq!(own.action, None);
        assert!(!items.iter().find(|i| i.id == "png").unwrap().disabled);
    }

    #[test]
    fn a_format_only_some_files_have_stays_active() {
        let items = show(&["jpg", "png"], &Layout::new());
        assert!(!items.iter().find(|i| i.id == "jpg").unwrap().disabled);
        assert!(!items.iter().find(|i| i.id == "png").unwrap().disabled);
    }

    #[test]
    fn several_images_get_both_pdf_circles() {
        let items = show(&["jpg", "jpg"], &Layout::new());
        let combine = items.iter().find(|i| i.id == "combine").unwrap();
        assert_eq!((combine.line1.as_str(), combine.line2.as_str()), ("PDF", ""));
        assert_eq!(combine.action, Some(Action::CombinePdf));
        let each = items.iter().find(|i| i.id == "pdf").unwrap();
        assert_eq!(each.line2, "EACH");
    }

    #[test]
    fn one_image_only_gets_plain_pdf() {
        let items = show(&["jpg"], &Layout::new());
        assert!(!ids(&items).contains(&"combine"));
        assert_eq!(items.iter().find(|i| i.id == "pdf").unwrap().line2, "");
    }

    #[test]
    fn tools_only_appear_for_their_families() {
        let image = show(&["png"], &Layout::new());
        assert!(ids(&image).contains(&"cmprss") && ids(&image).contains(&"rsz2"));
        let pdf = show(&["pdf"], &Layout::new());
        assert!(ids(&pdf).contains(&"cmprss") && !ids(&pdf).contains(&"rsz2"));
        let video = show(&["mp4"], &Layout::new());
        assert!(ids(&video).contains(&"cmprss") && !ids(&video).contains(&"rsz2"));
        let audio = show(&["mp3"], &Layout::new());
        assert!(!ids(&audio).contains(&"cmprss"));
        let mixed = show(&["png", "pdf"], &Layout::new());
        assert!(ids(&mixed).contains(&"cmprss"), "every file can be compressed");
        assert!(!ids(&mixed).contains(&"rsz2"), "a PDF can't be resized");
    }

    #[test]
    fn unsupported_format_loses_the_tool() {
        let items = offer(
            &v(&["svg"]),
            &default_tools(),
            &Layout::new(),
            &|_, _| true,
            &|format, _| format != "svg",
            &|_| true,
        );
        assert!(!ids(&items).contains(&"cmprss"));
    }

    #[test]
    fn layout_reorders_and_hides() {
        let mut layout = Layout::new();
        layout.insert(
            "image".into(),
            FamilyLayout { order: v(&["rsz2", "cmprss"]), hidden: v(&["docx", "rsz4"]) },
        );
        let items = show(&["jpg"], &layout);
        assert_eq!(&ids(&items)[..2], ["rsz2", "cmprss"]);
        assert!(!ids(&items).contains(&"docx") && !ids(&items).contains(&"rsz4"));
        assert!(ids(&items).contains(&"png"), "circles not in the order still appear");
    }

    #[test]
    fn suffix_comes_from_the_label() {
        let tools = default_tools();
        assert_eq!(tools[0].suffix(), "_cmprss");
        assert_eq!(tools[1].suffix(), "_rsz2");
        assert_eq!(tools[1].label(), "RSZ /2");
    }

    #[test]
    fn catalog_lists_formats_and_tools_for_settings() {
        let images = catalog("image", &default_tools(), &Layout::new());
        for id in ["combine", "jpg", "heic", "docx", "pdf", "cmprss", "rsz4"] {
            assert!(ids(&images).contains(&id), "{id}");
        }
        let models = catalog("model", &default_tools(), &Layout::new());
        assert!(ids(&models).contains(&"step") && ids(&models).contains(&"fbx"));
        assert!(!ids(&models).contains(&"cmprss"));
    }

    #[test]
    fn catalog_starts_in_the_same_order_as_a_real_drag() {
        let real = show(&["jpg", "jpg"], &Layout::new());
        let listed = catalog("image", &default_tools(), &Layout::new());
        let real_formats: Vec<&str> = ids(&real).into_iter().filter(|id| *id != "cmprss" && !id.starts_with("rsz")).collect();
        assert_eq!(&ids(&listed)[..real_formats.len()], &real_formats[..]);
        let each = listed.iter().find(|i| i.id == "pdf").unwrap();
        assert_eq!(each.line2, "EACH", "the two PDF circles can be told apart");
    }

    #[test]
    fn natural_order_counts_numbers() {
        let mut names: Vec<&Path> = ["img10.jpg", "IMG2.jpg", "a.jpg", "img1.jpg", "img02b.jpg"]
            .iter()
            .map(Path::new)
            .collect();
        names.sort_by(|a, b| natural_cmp(a, b));
        let sorted: Vec<_> = names.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(sorted, ["a.jpg", "img1.jpg", "IMG2.jpg", "img02b.jpg", "img10.jpg"]);
    }
}
