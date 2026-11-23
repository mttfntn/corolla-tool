//! Ctrl+drag detection. Each OS supplies `sample()`, which reads the left
//! button, the Ctrl key (Cmd on macOS) and the cursor position. The state machine here is
//! shared, so both platforms behave the same and it can be unit-tested.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::sample;
#[cfg(target_os = "windows")]
pub use windows::sample;

/// Movement needed with the button held before a press counts as a drag.
const DRAG_THRESHOLD: i64 = 6;
/// Wait this long past the threshold before showing the catcher. Covering the
/// cursor before the source app has begun its drag stops it receiving mouse
/// moves, and Explorer then never starts the drag at all.
const ARM_DELAY_MS: u64 = 120;
/// The catcher window stays up at least this long, and until the cursor has
/// moved `EXPIRE_DISTANCE`. OS drag systems only report a drag target when the
/// mouse moves, so a stationary cursor must not count as "no files".
/// Generous, because Explorer can take several hundred ms to begin its drag;
/// waiting is harmless since the window is invisible and the source app holds
/// the mouse capture.
const EXPIRE_MS: u64 = 1200;
const EXPIRE_DISTANCE: i64 = 12;
const FOLLOW_DISTANCE: i64 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Sample {
    pub left: bool,
    pub ctrl: bool,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal {
    /// Ctrl+drag started: put the catcher window under the cursor.
    Arm { x: i32, y: i32 },
    /// The cursor moved: keep the catcher under it, so a fast drag can't escape.
    Follow { x: i32, y: i32 },
    /// The cursor moved over the catcher and no files arrived: not a file drag.
    Expired,
    /// The mouse button came up.
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    /// `ready_since`: when Ctrl and the drag threshold were first both met.
    Pressed { x: i32, y: i32, ready_since: Option<u64> },
    Armed { x: i32, y: i32, at_ms: u64, follow_x: i32, follow_y: i32 },
    Spent,
}

pub struct Detector {
    state: State,
    was_left: bool,
}

fn far_enough(ax: i32, ay: i32, bx: i32, by: i32, distance: i64) -> bool {
    let (dx, dy) = ((ax - bx) as i64, (ay - by) as i64);
    dx * dx + dy * dy >= distance * distance
}

impl Detector {
    pub fn new() -> Self {
        // `was_left: true` ignores a button already held when the app starts.
        Self { state: State::Idle, was_left: true }
    }

    pub fn step(&mut self, s: Sample, now_ms: u64) -> Option<Signal> {
        let pressed_now = s.left && !self.was_left;
        self.was_left = s.left;
        match self.state {
            State::Idle => {
                if pressed_now {
                    self.state = State::Pressed { x: s.x, y: s.y, ready_since: None };
                }
                None
            }
            State::Pressed { x, y, ready_since } => {
                if !s.left {
                    self.state = State::Idle;
                    return None;
                }
                let ready = s.ctrl && far_enough(x, y, s.x, s.y, DRAG_THRESHOLD);
                let since = if ready { ready_since.or(Some(now_ms)) } else { None };
                if since.is_some_and(|t| now_ms.saturating_sub(t) >= ARM_DELAY_MS) {
                    self.state = State::Armed {
                        x: s.x,
                        y: s.y,
                        at_ms: now_ms,
                        follow_x: s.x,
                        follow_y: s.y,
                    };
                    Some(Signal::Arm { x: s.x, y: s.y })
                } else {
                    self.state = State::Pressed { x, y, ready_since: since };
                    None
                }
            }
            State::Armed { x, y, at_ms, follow_x, follow_y } => {
                if !s.left {
                    self.state = State::Idle;
                    Some(Signal::Release)
                } else if now_ms.saturating_sub(at_ms) >= EXPIRE_MS
                    && far_enough(x, y, s.x, s.y, EXPIRE_DISTANCE)
                {
                    self.state = State::Spent;
                    Some(Signal::Expired)
                } else if far_enough(follow_x, follow_y, s.x, s.y, FOLLOW_DISTANCE) {
                    self.state =
                        State::Armed { x, y, at_ms, follow_x: s.x, follow_y: s.y };
                    Some(Signal::Follow { x: s.x, y: s.y })
                } else {
                    None
                }
            }
            State::Spent => {
                if !s.left {
                    self.state = State::Idle;
                    Some(Signal::Release)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(left: bool, ctrl: bool, x: i32, y: i32) -> Sample {
        Sample { left, ctrl, x, y }
    }

    fn fresh() -> Detector {
        let mut d = Detector::new();
        assert_eq!(d.step(s(false, false, 0, 0), 0), None);
        d
    }

    /// Presses and drags to (10, 0) until armed; returns the arming time.
    fn arm(d: &mut Detector, start: u64) -> u64 {
        d.step(s(true, true, 0, 0), start);
        d.step(s(true, true, 10, 0), start + 1);
        let at = start + 1 + ARM_DELAY_MS;
        assert_eq!(d.step(s(true, true, 10, 0), at), Some(Signal::Arm { x: 10, y: 0 }));
        at
    }

    #[test]
    fn ctrl_then_drag_arms() {
        let mut d = fresh();
        d.step(s(true, true, 0, 0), 10);
        assert_eq!(d.step(s(true, true, 3, 0), 20), None);
        assert_eq!(d.step(s(true, true, 8, 0), 30), None, "past threshold, still waiting");
        assert_eq!(d.step(s(true, true, 9, 0), 149), None);
        assert_eq!(d.step(s(true, true, 9, 0), 150), Some(Signal::Arm { x: 9, y: 0 }));
    }

    #[test]
    fn drag_then_ctrl_arms() {
        let mut d = fresh();
        d.step(s(true, false, 0, 0), 10);
        assert_eq!(d.step(s(true, false, 50, 0), 20), None);
        assert_eq!(d.step(s(true, true, 50, 0), 30), None);
        assert_eq!(d.step(s(true, true, 50, 0), 150), Some(Signal::Arm { x: 50, y: 0 }));
    }

    #[test]
    fn letting_go_of_ctrl_restarts_the_wait() {
        let mut d = fresh();
        d.step(s(true, true, 0, 0), 10);
        d.step(s(true, true, 50, 0), 20);
        d.step(s(true, false, 50, 0), 100);
        assert_eq!(d.step(s(true, true, 50, 0), 130), None);
        assert_eq!(d.step(s(true, true, 50, 0), 200), None);
        assert_eq!(d.step(s(true, true, 50, 0), 250), Some(Signal::Arm { x: 50, y: 0 }));
    }

    #[test]
    fn plain_drag_never_arms() {
        let mut d = fresh();
        d.step(s(true, false, 0, 0), 10);
        assert_eq!(d.step(s(true, false, 200, 0), 20), None);
        assert_eq!(d.step(s(false, false, 200, 0), 30), None);
    }

    #[test]
    fn ctrl_click_without_moving_never_arms() {
        let mut d = fresh();
        d.step(s(true, true, 0, 0), 10);
        assert_eq!(d.step(s(false, true, 2, 0), 20), None);
    }

    #[test]
    fn stationary_cursor_does_not_expire() {
        let mut d = fresh();
        arm(&mut d, 10);
        assert_eq!(d.step(s(true, true, 10, 0), 5000), None);
    }

    #[test]
    fn moving_without_files_expires_once_then_releases() {
        let mut d = fresh();
        let armed_at = arm(&mut d, 10);
        assert_eq!(d.step(s(true, true, 40, 0), armed_at + 100), Some(Signal::Follow { x: 40, y: 0 }));
        assert_eq!(d.step(s(true, true, 42, 0), armed_at + 110), None, "small moves don't follow");
        assert_eq!(d.step(s(true, true, 42, 0), armed_at + EXPIRE_MS), Some(Signal::Expired));
        assert_eq!(d.step(s(true, true, 90, 0), armed_at + EXPIRE_MS + 1), None);
        assert_eq!(d.step(s(false, true, 90, 0), armed_at + EXPIRE_MS + 2), Some(Signal::Release));
    }

    #[test]
    fn gesture_can_repeat() {
        let mut d = fresh();
        for round in 0..3u64 {
            let armed_at = arm(&mut d, round * 1000);
            assert_eq!(d.step(s(false, true, 10, 0), armed_at + 1), Some(Signal::Release));
            d.step(s(false, false, 10, 0), armed_at + 2);
        }
    }

    #[test]
    fn button_held_at_startup_is_ignored() {
        let mut d = Detector::new();
        assert_eq!(d.step(s(true, true, 0, 0), 0), None);
        assert_eq!(d.step(s(true, true, 100, 0), 10), None);
    }
}
