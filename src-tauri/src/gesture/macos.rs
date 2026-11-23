//! macOS: reads the left button, the Command key and the cursor position
//! from Core Graphics, for the Cmd+drag gesture.

use super::Sample;
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

const COMBINED_SESSION_STATE: i32 = 0;
const LEFT_BUTTON: u32 = 0;
/// The Command key. Mac uses Cmd+drag, where Windows uses Ctrl+drag.
const FLAG_COMMAND: u64 = 0x0010_0000;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// Position is in points from the top-left of the main display; the wheel
/// controller scales it to physical pixels. These state queries need no
/// special macOS permission.
pub fn sample() -> Sample {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        let (x, y) = if event.is_null() {
            (0.0, 0.0)
        } else {
            let p = CGEventGetLocation(event);
            CFRelease(event);
            (p.x, p.y)
        };
        Sample {
            left: CGEventSourceButtonState(COMBINED_SESSION_STATE, LEFT_BUTTON),
            ctrl: CGEventSourceFlagsState(COMBINED_SESSION_STATE) & FLAG_COMMAND != 0,
            x: x as i32,
            y: y as i32,
        }
    }
}
