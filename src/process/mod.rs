//! Process memory access backends.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::LinuxProcess as Process;
#[cfg(target_os = "linux")]
pub use linux::sync_directory;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::WindowsProcess as Process;
#[cfg(target_os = "windows")]
pub use windows::sync_directory;

#[cfg(not(any(
    target_os = "linux",
    all(target_os = "windows", target_arch = "x86_64")
)))]
compile_error!("supported platforms are Linux and Windows x64");

#[cfg(target_os = "linux")]
#[path = "input_linux.rs"]
mod input;
#[cfg(target_os = "windows")]
#[path = "input_windows.rs"]
mod input;
pub use input::InputWatcher;

#[cfg(target_os = "linux")]
#[path = "signals_linux.rs"]
mod signals;
#[cfg(target_os = "windows")]
#[path = "signals_windows.rs"]
mod signals;
pub use signals::install_stop_handler;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    NotFound(String),
    Access(String),
    Other(String),
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(s) | Self::Access(s) | Self::Other(s) => f.write_str(s),
        }
    }
}

pub fn discover(pid: Option<u32>) -> Result<Process, String> {
    Process::discover(pid)
}

/// Discover SAI while preserving whether it was absent or inaccessible.
pub fn discover_detailed(pid: Option<u32>) -> Result<Process, DiscoveryError> {
    Process::discover_detailed(pid)
}

/// Hash the on-disk executable, using the backend's filesystem namespace.
pub fn executable_hash(process: &Process) -> Result<String, String> {
    use std::io::Read;
    let path = process.executable_path();
    let mut file = std::fs::File::open(&path)
        .map_err(|e| format!("cannot open executable {}: {e}", path.display()))?;
    let mut digest = md5::Context::new();
    let mut block = [0u8; 65536];
    loop {
        let n = file
            .read(&mut block)
            .map_err(|e| format!("cannot read executable {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        digest.consume(&block[..n]);
    }
    Ok(format!("{:x}", digest.compute()))
}

/// Reads bytes from another process' address space.
pub trait ProcessMemory {
    fn read_into(&self, address: usize, dst: &mut [u8]) -> Result<(), String>;
}
