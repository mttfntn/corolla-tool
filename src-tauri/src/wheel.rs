//! Owns the one overlay window: where it is, what it shows and which circle the
//! cursor is on. Lock order rule: never call a window method while holding
//! `phase`, because window calls from the gesture thread wait on the main
//! thread, which may itself be waiting for `phase` in a drag event.

use crate::gesture::Signal;
use crate::settings::Store;
use crate::tools::{self, Item};
use crate::{engines, formats, queue::Queue};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

/// Logical pixels. The window is 640 wide, so nothing may reach past 320.
const BASE_ORBIT: f64 = 165.0;
const BASE_CIRCLE: f64 = 56.0;
const LIMIT: f64 = 296.0;
/// Space kept between neighbouring circles.
const GAP: f64 = 10.0;
/// A drop this near a circle still counts: wrists aren't exact.
const HIT_MARGIN: f64 = 10.0;
/// How long after button-up a drop may still arrive before giving up.
const DROP_GRACE: Duration = Duration::from_millis(400);

/// Where circles sit: evenly around one ring, the first at the top, clockwise.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub count: usize,
    pub orbit: f64,
    pub circle: f64,
    /// Radius of the dark disc behind everything.
    pub disc: f64,
}

/// The ring widens to fit more circles, and they only shrink once it's full.
pub fn ring(count: usize) -> Ring {
    let half_step = (std::f64::consts::PI / count.max(1) as f64).sin();
    let needed = if count > 2 { (BASE_CIRCLE + GAP / 2.0) / half_step } else { BASE_ORBIT };
    let wanted = needed.max(BASE_ORBIT);
    let (orbit, circle) = if wanted + BASE_CIRCLE <= LIMIT {
        (wanted, BASE_CIRCLE)
    } else {
        let circle = (LIMIT * half_step - GAP / 2.0) / (1.0 + half_step);
        (LIMIT - circle, circle)
    };
    Ring { count, orbit, circle, disc: orbit + circle + 16.0 }
}

impl Ring {
    /// Offset of a circle's centre from the wheel's. Angle 0 is up, growing clockwise.
    pub fn centre(&self, index: usize) -> (f64, f64) {
        let angle = index as f64 * std::f64::consts::TAU / self.count.max(1) as f64;
        (self.orbit * angle.sin(), -self.orbit * angle.cos())
    }

    /// Which circle is at an offset from the wheel's centre, in logical pixels.
    pub fn hit(&self, dx: f64, dy: f64) -> Option<usize> {
        (0..self.count).find(|&i| {
            let (x, y) = self.centre(i);
            (dx - x).hypot(dy - y) <= self.circle + HIT_MARGIN
        })
    }
}

enum Phase {
    Hidden,
    /// Invisible window under the cursor, waiting to learn if files are dragged.
    Catching,
    Open { items: Vec<Item>, ring: Ring, hover: Option<usize> },
}

#[derive(Serialize, Clone)]
struct PlacedItem {
    #[serde(flatten)]
    item: Item,
    x: f64,
    y: f64,
}

#[derive(Serialize, Clone)]
struct OpenPayload {
    items: Vec<PlacedItem>,
    circle: f64,
    disc: f64,
    count: usize,
    total_bytes: u64,
}

pub struct Controller {
    app: AppHandle,
    phase: Mutex<Phase>,
    /// Bumped whenever the wheel opens or closes, so stale timers do nothing.
    generation: AtomicU64,
    pub enabled: AtomicBool,
}

impl Controller {
    pub fn new(app: AppHandle) -> Arc<Self> {
        Arc::new(Self {
            app,
            phase: Mutex::new(Phase::Hidden),
            generation: AtomicU64::new(0),
            enabled: AtomicBool::new(true),
        })
    }

    fn window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window("wheel")
    }

    pub fn on_signal(self: &Arc<Self>, signal: Signal) {
        match signal {
            Signal::Arm { x, y } => {
                if !self.enabled.load(Ordering::Relaxed) {
                    return;
                }
                self.generation.fetch_add(1, Ordering::SeqCst);
                *self.phase.lock().unwrap() = Phase::Catching;
                let _ = self.app.emit("wheel:catch", ());
                if let Some(w) = self.window() {
                    let _ = w.set_ignore_cursor_events(false);
                    place_centered(&w, x, y);
                    let _ = w.show();
                }
            }
            Signal::Follow { x, y } => {
                let catching = matches!(*self.phase.lock().unwrap(), Phase::Catching);
                if catching {
                    if let Some(w) = self.window() {
                        place_centered(&w, x, y);
                    }
                }
            }
            Signal::Expired => {
                let catching = matches!(*self.phase.lock().unwrap(), Phase::Catching);
                if catching {
                    self.settle();
                }
            }
            Signal::Release => {
                let (catching, open) = {
                    let phase = self.phase.lock().unwrap();
                    (matches!(*phase, Phase::Catching), matches!(*phase, Phase::Open { .. }))
                };
                if catching {
                    self.settle();
                } else if open {
                    // The drop event may still be on its way; close only if it never comes.
                    let this = self.clone();
                    let generation = self.generation.load(Ordering::SeqCst);
                    std::thread::spawn(move || {
                        std::thread::sleep(DROP_GRACE);
                        if this.generation.load(Ordering::SeqCst) == generation {
                            this.settle();
                        }
                    });
                }
            }
        }
    }

    pub fn on_drag_enter(&self, paths: Vec<PathBuf>) {
        {
            let phase = self.phase.lock().unwrap();
            if matches!(*phase, Phase::Hidden) {
                return;
            }
        }
        let items = items_for(&self.app, &paths);
        let ring = ring(items.len());
        let placed: Vec<PlacedItem> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (x, y) = ring.centre(i);
                PlacedItem { item: item.clone(), x, y }
            })
            .collect();
        let total_bytes = paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let payload =
            OpenPayload { items: placed, circle: ring.circle, disc: ring.disc, count: paths.len(), total_bytes };
        *self.phase.lock().unwrap() = Phase::Open { items, ring, hover: None };
        let _ = self.app.emit("wheel:open", payload);
    }

    /// `x` and `y` are physical pixels relative to the window.
    pub fn on_drag_over(&self, x: f64, y: f64) {
        let Some(over) = self.circle_at(x, y) else { return };
        let changed = {
            let mut phase = self.phase.lock().unwrap();
            match &mut *phase {
                Phase::Open { hover, .. } if *hover != over => {
                    *hover = over;
                    true
                }
                _ => false,
            }
        };
        if changed {
            let _ = self.app.emit("wheel:hover", over);
        }
    }

    pub fn on_drag_leave(&self) {
        let changed = {
            let mut phase = self.phase.lock().unwrap();
            match &mut *phase {
                Phase::Open { hover, .. } if hover.is_some() => {
                    *hover = None;
                    true
                }
                _ => false,
            }
        };
        if changed {
            let _ = self.app.emit("wheel:hover", Option::<usize>::None);
        }
    }

    pub fn on_drop(self: &Arc<Self>, paths: Vec<PathBuf>, x: f64, y: f64) {
        let chosen = self.circle_at(x, y).flatten().and_then(|i| {
            match &*self.phase.lock().unwrap() {
                Phase::Open { items, .. } => items.get(i).and_then(|item| item.action.clone()),
                _ => None,
            }
        });
        if let Some(action) = chosen {
            self.app.state::<Arc<Queue>>().enqueue(paths, action);
        }
        self.settle();
    }

    /// Outer None: no wheel is open. Inner None: the cursor isn't on a usable circle.
    /// A greyed-out circle counts as nothing, so it can't highlight or be dropped on.
    fn circle_at(&self, x: f64, y: f64) -> Option<Option<usize>> {
        let ring = match &*self.phase.lock().unwrap() {
            Phase::Open { ring, .. } => *ring,
            _ => return None,
        };
        let w = self.window()?;
        let scale = w.scale_factor().ok()?;
        let size = w.outer_size().ok()?;
        let dx = (x - size.width as f64 / 2.0) / scale;
        let dy = (y - size.height as f64 / 2.0) / scale;
        let hit = ring.hit(dx, dy);
        let phase = self.phase.lock().unwrap();
        let Phase::Open { items, .. } = &*phase else { return None };
        Some(hit.filter(|&i| items.get(i).is_some_and(|item| !item.disabled)))
    }

    /// The gesture is over: show progress if a conversion is running, else hide.
    pub fn settle(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.phase.lock().unwrap() = Phase::Hidden;
        self.show_progress_or_hide();
    }

    /// Called by the queue on every change.
    pub fn on_queue_changed(&self) {
        let idle = matches!(*self.phase.lock().unwrap(), Phase::Hidden);
        if idle {
            self.show_progress_or_hide();
        }
    }

    fn show_progress_or_hide(&self) {
        let Some(w) = self.window() else { return };
        match self.app.state::<Arc<Queue>>().snapshot() {
            Some(progress) => {
                let _ = self.app.emit("wheel:progress", progress);
                // The progress must never get in the way of what's underneath it.
                let _ = w.set_ignore_cursor_events(true);
                let _ = w.show();
            }
            None => {
                let _ = self.app.emit("wheel:hide", ());
                let _ = w.hide();
            }
        }
    }
}

fn place_centered(w: &WebviewWindow, cursor_x: i32, cursor_y: i32) {
    #[cfg(target_os = "macos")]
    let (cursor_x, cursor_y) = {
        // gesture::sample reports points on macOS.
        let scale = w.scale_factor().unwrap_or(1.0);
        ((cursor_x as f64 * scale) as i32, (cursor_y as f64 * scale) as i32)
    };
    // Twice: crossing onto a monitor with another DPI changes the window size.
    for _ in 0..2 {
        let Ok(size) = w.outer_size() else { return };
        let (width, height) = (size.width as i32, size.height as i32);
        let (mut x, mut y) = (cursor_x - width / 2, cursor_y - height / 2);
        if let Ok(Some(monitor)) = w.monitor_from_point(cursor_x as f64, cursor_y as f64) {
            let (mx, my) = (monitor.position().x, monitor.position().y);
            let (mw, mh) = (monitor.size().width as i32, monitor.size().height as i32);
            x = x.clamp(mx, (mx + mw - width).max(mx));
            y = y.clamp(my, (my + mh - height).max(my));
        }
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

/// What to offer. Empty if nothing is usable, for instance with a folder in the drag.
fn items_for(app: &AppHandle, paths: &[PathBuf]) -> Vec<Item> {
    let detected: Vec<Option<String>> = paths
        .iter()
        .map(|p| if p.is_file() { formats::format_of(p) } else { None })
        .collect();
    if detected.iter().any(Option::is_none) {
        return Vec::new();
    }
    let list: Vec<String> = detected.into_iter().flatten().collect();
    let found = app.state::<Arc<engines::Tools>>();
    let settings = app.state::<Arc<Store>>().get();
    let items = tools::offer(
        &list,
        &settings.tools,
        &settings.layout,
        &|from, to| engines::can_convert(&found, from, to),
        &|from, op| engines::can_process(&found, from, op),
        &|all| engines::can_combine(&found, all),
    );
    if items.iter().all(|item| item.disabled) {
        return Vec::new();
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_few_circles_keep_the_mockups_spacing() {
        let few = ring(7);
        assert_eq!(few.orbit, BASE_ORBIT);
        assert_eq!(few.circle, BASE_CIRCLE);
    }

    #[test]
    fn circles_never_overlap_or_leave_the_window() {
        for count in 1..=24 {
            let r = ring(count);
            assert!(r.orbit + r.circle <= LIMIT + 0.001, "{count} circles leave the window");
            assert!(r.disc <= 320.0, "{count} circles: disc {}", r.disc);
            assert!(r.orbit - r.circle > 40.0, "{count} circles reach the hub");
            if count >= 2 {
                let (a, b) = (r.centre(0), r.centre(1));
                let apart = (a.0 - b.0).hypot(a.1 - b.1);
                assert!(apart >= 2.0 * r.circle + GAP - 0.001, "{count} circles overlap");
            }
        }
    }

    #[test]
    fn circles_stay_a_usable_size_for_any_realistic_count() {
        assert_eq!(ring(11).circle, BASE_CIRCLE, "a JPG's usual set");
        assert!(ring(16).circle >= 40.0);
    }

    #[test]
    fn the_centre_and_the_gaps_are_not_circles() {
        let r = ring(8);
        assert_eq!(r.hit(0.0, 0.0), None);
        assert_eq!(r.hit(0.0, -300.0), None, "past the ring");
        let (a, b) = (r.centre(0), r.centre(1));
        assert_eq!(ring(3).hit(0.0, 165.0), None, "between circles on a sparse ring");
        assert!(r.hit((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0).is_some(), "tight rings forgive small misses");
    }

    #[test]
    fn circles_run_clockwise_from_the_top() {
        let r = ring(4);
        assert_eq!(r.hit(0.0, -r.orbit), Some(0));
        assert_eq!(r.hit(r.orbit, 0.0), Some(1));
        assert_eq!(r.hit(0.0, r.orbit), Some(2));
        assert_eq!(r.hit(-r.orbit, 0.0), Some(3));
    }

    #[test]
    fn every_circle_is_reachable_at_every_count() {
        for count in 1..=24 {
            let r = ring(count);
            for i in 0..count {
                let (x, y) = r.centre(i);
                assert_eq!(r.hit(x, y), Some(i), "{i} of {count}");
            }
        }
    }
}
