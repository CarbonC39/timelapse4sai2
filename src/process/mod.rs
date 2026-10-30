//! Process memory access backends.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::LinuxProcess;

/// Reads bytes from another process' address space.
pub trait ProcessMemory {
    fn read_into(&self, address: usize, dst: &mut [u8]) -> Result<(), String>;
}
