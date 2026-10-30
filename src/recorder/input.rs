//! Nonblocking Linux evdev activity watcher.
//!
//! The recorder intentionally watches `/dev/input/event*` directly so it does
//! not depend on a desktop session or an additional input library.

use std::{
    collections::HashSet,
    ffi::CString,
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::PathBuf,
};

pub struct InputWatcher {
    devices: Vec<(PathBuf, OwnedFd)>,
    next_scan_ms: u64,
}

impl InputWatcher {
    pub fn new() -> Result<Self, String> {
        let mut watcher = Self {
            devices: Vec::new(),
            next_scan_ms: 0,
        };
        watcher.scan_devices()?;
        watcher.next_scan_ms = 5_000;
        Ok(watcher)
    }

    fn scan_devices(&mut self) -> Result<(), String> {
        let entries = fs::read_dir("/dev/input")
            .map_err(|e| input_error(format!("cannot read /dev/input: {e}")))?;
        let mut found = Vec::new();
        let known: HashSet<_> = self.devices.iter().map(|(p, _)| p.clone()).collect();
        let mut failures = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("event") {
                continue;
            }
            if known.contains(&path) {
                continue;
            }
            let Ok(cpath) = CString::new(path.as_os_str().as_bytes()) else {
                continue;
            };
            let rc = unsafe {
                libc::open(
                    cpath.as_ptr(),
                    libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if rc >= 0 {
                // SAFETY: open returned a new owned file descriptor.
                found.push((path, unsafe { OwnedFd::from_raw_fd(rc) }));
            } else if failures.len() < 3 {
                failures.push(format!(
                    "{}: {}",
                    path.display(),
                    std::io::Error::last_os_error()
                ));
            }
        }
        if !found.is_empty() {
            // Keep descriptors already opened; newly connected devices are added
            // during periodic scans. The path list is stable for a scan.
            self.devices.extend(found);
            return Ok(());
        }
        if !self.devices.is_empty() {
            return Ok(());
        }
        let detail = if failures.is_empty() {
            "no event devices found".to_owned()
        } else {
            failures.join("; ")
        };
        Err(input_error(detail))
    }

    /// Drain pending input records without waiting. Returns true if any real
    /// key, pointer, or tablet event arrived since the previous call.
    pub fn poll_activity(&mut self, now_ms: u64) -> Result<bool, String> {
        let mut activity = false;
        let mut dead = Vec::new();
        for (idx, (_, fd)) in self.devices.iter().enumerate() {
            loop {
                let mut ev = std::mem::MaybeUninit::<libc::input_event>::uninit();
                let n = unsafe {
                    libc::read(
                        fd.as_raw_fd(),
                        ev.as_mut_ptr().cast(),
                        std::mem::size_of::<libc::input_event>(),
                    )
                };
                if n == std::mem::size_of::<libc::input_event>() as isize {
                    // SAFETY: a complete input_event record was read.
                    let ev = unsafe { ev.assume_init() };
                    // Linux input-event type values from <linux/input-event-codes.h>.
                    if matches!(ev.type_, 1 | 2 | 3) {
                        activity = true;
                    }
                } else if n == 0 {
                    dead.push(idx);
                    break;
                } else if n < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        break;
                    }
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    dead.push(idx);
                    break;
                } else {
                    // Evdev records are atomic; a short read indicates a removed
                    // or unusable descriptor.
                    dead.push(idx);
                    break;
                }
            }
        }
        for idx in dead.into_iter().rev() {
            self.devices.swap_remove(idx);
        }
        if now_ms >= self.next_scan_ms {
            self.scan_devices()?;
            self.next_scan_ms = now_ms.saturating_add(5_000);
        }
        if self.devices.is_empty() {
            return Err(input_error("all input event devices disconnected".into()));
        }
        Ok(activity)
    }
}

fn input_error(detail: String) -> String {
    format!(
        "input activity watcher unavailable ({detail}); grant access to /dev/input/event* or rerun with --no-input-watch to use timer-only capture"
    )
}
