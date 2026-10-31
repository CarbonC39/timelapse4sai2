//! Windows foreground input activity watcher based on the system idle timer.

use std::io;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

pub struct InputWatcher {
    last_input_tick: u32,
}

impl InputWatcher {
    pub fn new() -> Result<Self, String> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return Err(input_error(io::Error::last_os_error().to_string()));
        }
        Ok(Self {
            last_input_tick: info.dwTime,
        })
    }

    /// Returns true when GetLastInputInfo reports a newer user input event.
    pub fn poll_activity(&mut self, _now_ms: u64) -> Result<bool, String> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return Err(input_error(io::Error::last_os_error().to_string()));
        }
        let changed = info.dwTime != self.last_input_tick;
        self.last_input_tick = info.dwTime;
        Ok(changed)
    }
}

fn input_error(detail: String) -> String {
    format!(
        "input activity watcher unavailable ({detail}); run with --no-input-watch to use timer-only capture"
    )
}
