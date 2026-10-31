//! Native Windows x64 process discovery and read-only memory access.
use super::{DiscoveryError, ProcessMemory};
use std::{
    ffi::OsString,
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_BAD_LENGTH, ERROR_INSUFFICIENT_BUFFER,
        ERROR_NO_MORE_FILES, ERROR_PARTIAL_COPY, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    },
    System::{
        Diagnostics::{
            Debug::ReadProcessMemory,
            ToolHelp::{
                CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
                PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPMODULE,
                TH32CS_SNAPPROCESS,
            },
        },
        Threading::{
            IsWow64Process, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
            QueryFullProcessImageNameW,
        },
    },
};

const PRIVILEGE_HINT: &str = "Could not open SAI2 process for memory reading. Try running timelapse4sai2 at the same privilege level as SAI2.";

pub fn sync_directory(_path: &Path) -> io::Result<()> {
    // Windows does not support opening a directory as a file for FlushFileBuffers.
    // The manifest file is flushed before its rename, so there is no directory fsync equivalent here.
    Ok(())
}

/// Owns exactly one valid Win32 handle, including snapshot handles.
#[derive(Debug)]
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(raw: HANDLE) -> Option<Self> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(raw))
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: construction excludes invalid handles; this is the sole owner.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[derive(Debug)]
pub struct WindowsProcess {
    pub pid: u32,
    pub base_address: usize,
    pub module_path: PathBuf,
    handle: OwnedHandle,
}

impl WindowsProcess {
    pub fn discover(pid: Option<u32>) -> Result<Self, String> {
        Self::discover_detailed(pid).map_err(|e| e.to_string())
    }

    pub fn discover_detailed(pid: Option<u32>) -> Result<Self, DiscoveryError> {
        let candidates = candidate_pids(pid).map_err(DiscoveryError::Other)?;
        // Decide ambiguity before opening candidates: an inaccessible instance must
        // never make us silently select a different one.
        let pid = match candidates.as_slice() {
            [] => {
                return Err(DiscoveryError::NotFound(match pid {
                    Some(pid) => format!("PID {pid} is not running sai2.exe"),
                    None => "could not find a process running sai2.exe".into(),
                }));
            }
            [pid] => *pid,
            _ => {
                return Err(DiscoveryError::Other(format!(
                    "multiple sai2.exe processes found (PIDs: {}); specify --pid PID",
                    candidates
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };
        // No debug privilege, write access, or all-access handle is requested.
        let raw =
            unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        let handle = OwnedHandle::new(raw).ok_or_else(|| {
            DiscoveryError::Access(win_error(pid, "open process for memory reading", unsafe {
                GetLastError()
            }))
        })?;
        let module_path = image_path(pid, handle.0).map_err(DiscoveryError::Access)?;
        if !module_path
            .file_name()
            .is_some_and(|n| is_sai_name(&n.encode_wide().collect::<Vec<_>>()))
        {
            return Err(DiscoveryError::NotFound(format!(
                "PID {pid} no longer identifies sai2.exe"
            )));
        }
        let mut wow64 = 0;
        if unsafe { IsWow64Process(handle.0, &mut wow64) } == 0 {
            return Err(DiscoveryError::Access(win_error(
                pid,
                "query process architecture",
                unsafe { GetLastError() },
            )));
        }
        if wow64 != 0 {
            return Err(DiscoveryError::Other(format!(
                "PID {pid}: 32-bit SAI2 is unsupported; expected x64 PE32+"
            )));
        }
        let base_address = module_base(pid).map_err(DiscoveryError::Other)?;
        let process = Self {
            pid,
            base_address,
            module_path,
            handle,
        };
        process.validate_image().map_err(DiscoveryError::Other)?;
        Ok(process)
    }

    pub fn executable_path(&self) -> PathBuf {
        self.module_path.clone()
    }

    fn validate_image(&self) -> Result<(), String> {
        let invalid = || {
            format!(
                "PID {}: unsupported SAI2 image; expected x64 PE32+ (32-bit SAI2 is unsupported)",
                self.pid
            )
        };
        let mut dos = [0; 64];
        self.read_into(self.base_address, &mut dos)?;
        if &dos[..2] != b"MZ" {
            return Err(invalid());
        }
        let offset = u32::from_le_bytes(dos[0x3c..0x40].try_into().unwrap()) as usize;
        if offset > 16 * 1024 * 1024 {
            return Err(invalid());
        }
        let address = self.base_address.checked_add(offset).ok_or_else(invalid)?;
        let mut pe = [0; 26];
        self.read_into(address, &mut pe)?;
        if &pe[..4] != b"PE\0\0"
            || pe[4..6] != 0x8664u16.to_le_bytes()
            || pe[24..26] != 0x20bu16.to_le_bytes()
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl ProcessMemory for WindowsProcess {
    fn read_into(&self, address: usize, dst: &mut [u8]) -> Result<(), String> {
        if dst.is_empty() {
            return Ok(());
        }
        address.checked_add(dst.len()).ok_or_else(|| {
            format!(
                "PID {}: memory range overflow at address {address:#x}, length {}",
                self.pid,
                dst.len()
            )
        })?;
        let mut read = 0;
        // SAFETY: dst is writable for its length; the remote address is validated
        // by Windows. A partial transfer is never accepted as a complete tile.
        let ok = unsafe {
            ReadProcessMemory(
                self.handle.0,
                address as *const _,
                dst.as_mut_ptr().cast(),
                dst.len(),
                &mut read,
            )
        };
        if ok == 0 || read != dst.len() {
            let code = if ok == 0 {
                unsafe { GetLastError() }
            } else {
                ERROR_PARTIAL_COPY
            };
            return Err(format!(
                "{}; address {address:#x}, length {}, bytes read {read}",
                win_error(self.pid, "read process memory", code),
                dst.len()
            ));
        }
        Ok(())
    }
}

fn win_error(pid: u32, action: &str, code: u32) -> String {
    let hint = if code == ERROR_ACCESS_DENIED {
        format!(" {PRIVILEGE_HINT}")
    } else {
        String::new()
    };
    format!(
        "cannot {action} for PID {pid}: Win32 error {code} ({}){hint}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

fn snapshot(flags: u32, pid: u32) -> Result<OwnedHandle, String> {
    // Module lists can change while Windows takes the snapshot. Bound retries
    // so a persistently unstable target reports an error instead of hanging.
    for _ in 0..8 {
        if let Some(handle) = OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(flags, pid) }) {
            return Ok(handle);
        }
        let code = unsafe { GetLastError() };
        if code != ERROR_BAD_LENGTH {
            return Err(win_error(pid, "create process/module snapshot", code));
        }
    }
    Err(win_error(
        pid,
        "create process/module snapshot after retries",
        ERROR_BAD_LENGTH,
    ))
}

fn candidate_pids(selected: Option<u32>) -> Result<Vec<u32>, String> {
    let snapshot = snapshot(TH32CS_SNAPPROCESS, 0)?;
    // SAFETY: these POD Win32 structures are initialized with their required size.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut ok = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    let mut pids = Vec::new();
    while ok != 0 {
        if selected.is_none_or(|pid| pid == entry.th32ProcessID) && is_sai_name(&entry.szExeFile) {
            pids.push(entry.th32ProcessID);
        }
        ok = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    let code = unsafe { GetLastError() };
    if code != ERROR_NO_MORE_FILES {
        return Err(win_error(0, "enumerate processes", code));
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

fn module_base(pid: u32) -> Result<usize, String> {
    let snapshot = snapshot(TH32CS_SNAPMODULE, pid)?;
    let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
    let mut ok = unsafe { Module32FirstW(snapshot.0, &mut entry) };
    let mut bases = Vec::new();
    while ok != 0 {
        if is_sai_name(&entry.szModule) {
            bases.push(entry.modBaseAddr as usize);
        }
        ok = unsafe { Module32NextW(snapshot.0, &mut entry) };
    }
    let code = unsafe { GetLastError() };
    if code != ERROR_NO_MORE_FILES {
        return Err(win_error(pid, "enumerate modules", code));
    }
    match bases.as_slice() {
        [base] if *base != 0 => Ok(*base),
        _ => Err(format!(
            "could not identify a unique sai2.exe module base in PID {pid}"
        )),
    }
}

fn image_path(pid: u32, handle: HANDLE) -> Result<PathBuf, String> {
    let mut buffer = vec![0; 512];
    loop {
        let mut length = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } != 0 {
            return Ok(PathBuf::from(OsString::from_wide(
                &buffer[..length as usize],
            )));
        }
        let code = unsafe { GetLastError() };
        if code != ERROR_INSUFFICIENT_BUFFER || buffer.len() >= 32768 {
            return Err(win_error(pid, "query executable path", code));
        }
        buffer.resize((buffer.len() * 2).min(32768), 0);
    }
}

fn is_sai_name(wide: &[u16]) -> bool {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    wide[..end].len() == 8
        && wide[..end]
            .iter()
            .zip(b"sai2.exe")
            .all(|(&a, &b)| a <= 0x7f && (a as u8).eq_ignore_ascii_case(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_exact_executable_name() {
        let matches = |s: &str| is_sai_name(&s.encode_utf16().collect::<Vec<_>>());
        assert!(matches("SAI2.exe\0ignored"));
        assert!(!matches("other-sai2.exe"));
        assert!(!matches("sai2.exe.bak"));
        assert!(!matches("sai2.exe\u{ff00}"));
    }

    #[test]
    fn access_denied_retains_code_and_explains_privilege_alignment() {
        let error = win_error(42, "read process memory", ERROR_ACCESS_DENIED);
        assert!(error.contains("PID 42"));
        assert!(error.contains("Win32 error 5"));
        assert!(error.contains(PRIVILEGE_HINT));
        assert!(!win_error(42, "read process memory", ERROR_PARTIAL_COPY).contains(PRIVILEGE_HINT));
    }

    #[test]
    fn reads_current_process_and_reports_invalid_address_context() {
        let pid = std::process::id();
        let handle = OwnedHandle::new(unsafe { OpenProcess(PROCESS_VM_READ, 0, pid) }).unwrap();
        let process = WindowsProcess {
            pid,
            base_address: 0,
            module_path: PathBuf::new(),
            handle,
        };
        let source = [7u8, 11, 19, 31];
        let mut target = [0; 4];
        process
            .read_into(source.as_ptr() as usize, &mut target)
            .unwrap();
        assert_eq!(source, target);
        process.read_into(usize::MAX, &mut []).unwrap();
        let error = process.read_into(0, &mut target).unwrap_err();
        for context in [
            format!("PID {pid}"),
            "address 0x0".into(),
            "length 4".into(),
            "Win32 error".into(),
        ] {
            assert!(error.contains(&context), "{error}");
        }
    }
}
