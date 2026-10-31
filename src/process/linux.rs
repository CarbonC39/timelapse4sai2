use super::ProcessMemory;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct LinuxProcess {
    pub pid: u32,
    /// Address at which the executable's PE headers are mapped.
    pub base_address: usize,
    pub module_path: PathBuf,
}

#[derive(Debug, Clone)]
struct MapRegion {
    start: usize,
    end: usize,
    readable: bool,
    offset: u64,
    path: Option<PathBuf>,
}

impl LinuxProcess {
    pub fn executable_path(&self) -> PathBuf {
        // Resolve Bottles/Flatpak paths in the target's mount namespace.
        PathBuf::from(format!("/proc/{}/root", self.pid)).join(
            self.module_path
                .strip_prefix("/")
                .unwrap_or(&self.module_path),
        )
    }

    /// Find the unique running process whose command line, comm, or mapped image
    /// identifies it as sai2.exe. If `pid` is supplied, only that process is
    /// considered.
    pub fn discover(pid: Option<u32>) -> Result<Self, String> {
        let pids: Vec<u32> = if let Some(pid) = pid {
            vec![pid]
        } else {
            let mut found = Vec::new();
            for entry in fs::read_dir("/proc").map_err(|e| format!("cannot read /proc: {e}"))? {
                let entry = entry.map_err(|e| format!("cannot enumerate /proc: {e}"))?;
                if let Some(name) = entry.file_name().to_str() {
                    if let Ok(n) = name.parse::<u32>() {
                        found.push(n);
                    }
                }
            }
            found
        };

        let mut matches = Vec::new();
        for candidate_pid in pids {
            if let Some(process) = Self::inspect(candidate_pid)? {
                matches.push(process);
            }
        }
        match matches.len() {
            0 => Err(match pid {
                Some(pid) => format!("PID {pid} is not a readable process running sai2.exe"),
                None => "could not find a readable process running sai2.exe".into(),
            }),
            1 => Ok(matches.remove(0)),
            _ => Err(format!(
                "multiple sai2.exe processes found (PIDs: {}); specify a PID",
                matches
                    .iter()
                    .map(|p| p.pid.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn inspect(pid: u32) -> Result<Option<Self>, String> {
        let root = PathBuf::from(format!("/proc/{pid}"));
        let cmdline = fs::read(root.join("cmdline")).ok();
        let comm = fs::read(root.join("comm")).ok();
        let maps_text = match fs::read_to_string(root.join("maps")) {
            Ok(v) => v,
            Err(_) => return Ok(None), // process exited or procfs access denied
        };
        let maps = parse_maps(&maps_text);
        let image_maps: Vec<&MapRegion> = maps
            .iter()
            .filter(|m| m.path.as_ref().is_some_and(|p| is_sai_image(p)))
            .collect();
        let name_match = cmdline.as_deref().is_some_and(|b| contains_sai_image(b))
            || comm.as_deref().is_some_and(|b| contains_sai_image(b));
        // Launchers may include sai2.exe in their arguments before the game
        // process exists. A mapped executable image is required to identify it.
        if image_maps.is_empty() || !name_match {
            return Ok(None);
        }

        let mut bases = Vec::new();
        for region in &image_maps {
            // The PE header starts at file offset zero. Do not derive an image
            // base by subtracting a maps file offset from an arbitrary segment.
            if region.offset == 0 && region.readable && valid_pe_headers(pid, &maps, region.start)?
            {
                bases.push(region.start);
            }
        }
        bases.sort_unstable();
        bases.dedup();
        if bases.len() != 1 {
            return Err(format!(
                "could not identify a unique valid PE header mapping for sai2.exe in PID {pid}"
            ));
        }
        let module_path = image_maps
            .iter()
            .find_map(|m| m.path.clone())
            .unwrap_or_default();
        Ok(Some(Self {
            pid,
            base_address: bases[0],
            module_path,
        }))
    }
}

pub fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

impl ProcessMemory for LinuxProcess {
    fn read_into(&self, address: usize, dst: &mut [u8]) -> Result<(), String> {
        if dst.is_empty() {
            return Ok(());
        }
        address
            .checked_add(dst.len())
            .ok_or("requested address range overflows")?;
        // process_vm_readv validates remote mappings in the kernel. Re-reading
        // /proc/maps for every tile dominates capture cost and cannot prevent races.
        let mut done = 0usize;
        while done < dst.len() {
            let local = libc::iovec {
                iov_base: unsafe { dst.as_mut_ptr().add(done) }.cast(),
                iov_len: dst.len() - done,
            };
            let remote_address = address
                .checked_add(done)
                .ok_or_else(|| "requested address range overflows".to_string())?;
            let remote = libc::iovec {
                iov_base: remote_address as *mut libc::c_void,
                iov_len: dst.len() - done,
            };
            let n = unsafe {
                libc::process_vm_readv(self.pid as libc::pid_t, &local, 1, &remote, 1, 0)
            };
            if n < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(access_error(self.pid, "read process memory", error));
            }
            if n == 0 {
                return Err(format!(
                    "short read from PID {} at {:#x}: no bytes returned",
                    self.pid,
                    address + done
                ));
            }
            done += n as usize;
        }
        Ok(())
    }
}

fn access_error(pid: u32, action: &str, error: io::Error) -> String {
    let hint = if matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES)) {
        " (access may be restricted by Yama ptrace_scope; run as the target user or adjust ptrace permissions)"
    } else {
        ""
    };
    format!("cannot {action} for PID {pid}: {error}{hint}")
}

fn is_sai_image(path: &Path) -> bool {
    let raw = path.as_os_str().as_bytes();
    let path = String::from_utf8_lossy(raw)
        .trim_end_matches(" (deleted)")
        .to_ascii_lowercase();
    path.rsplit('/')
        .next()
        .is_some_and(|name| name == "sai2.exe")
}

fn contains_sai_image(bytes: &[u8]) -> bool {
    bytes.split(|b| *b == 0).any(|arg| {
        let s = String::from_utf8_lossy(arg).to_ascii_lowercase();
        s.trim()
            .rsplit(['/', '\\'])
            .next()
            .is_some_and(|name| name == "sai2.exe")
    })
}

fn parse_maps(text: &str) -> Vec<MapRegion> {
    let mut maps = Vec::new();
    for line in text.lines() {
        // The first five fields have no whitespace; the pathname may contain
        // spaces, so retain the remainder of the original line after inode.
        let mut fields = line.split_whitespace();
        let Some(range) = fields.next() else { continue };
        let Some(perms) = fields.next() else { continue };
        let Some(offset) = fields.next() else {
            continue;
        };
        if fields.next().is_none() {
            continue;
        } // device
        let Some(_inode) = fields.next() else {
            continue;
        };
        let mut path_start = 0;
        for _ in 0..5 {
            while path_start < line.len() && line.as_bytes()[path_start].is_ascii_whitespace() {
                path_start += 1;
            }
            while path_start < line.len() && !line.as_bytes()[path_start].is_ascii_whitespace() {
                path_start += 1;
            }
        }
        let mut ends = range.splitn(2, '-');
        let (Some(start), Some(end)) = (
            ends.next().and_then(|v| usize::from_str_radix(v, 16).ok()),
            ends.next().and_then(|v| usize::from_str_radix(v, 16).ok()),
        ) else {
            continue;
        };
        let Ok(offset) = u64::from_str_radix(offset, 16) else {
            continue;
        };
        let path = line[path_start..]
            .trim()
            .strip_prefix('/')
            .map(|p| PathBuf::from(format!("/{p}")));
        if start < end {
            maps.push(MapRegion {
                start,
                end,
                readable: perms.as_bytes().first() == Some(&b'r'),
                offset,
                path,
            });
        }
    }
    maps.sort_by_key(|m| m.start);
    maps
}

fn ensure_readable_range(maps: &[MapRegion], start: usize, end: usize) -> Result<(), String> {
    let mut cursor = start;
    for m in maps {
        if m.end <= cursor {
            continue;
        }
        if m.start > cursor || !m.readable {
            break;
        }
        cursor = cursor.max(m.end.min(end));
        if cursor == end {
            return Ok(());
        }
    }
    Err(format!(
        "address range {start:#x}..{end:#x} is not fully covered by readable mappings"
    ))
}

fn valid_pe_headers(pid: u32, maps: &[MapRegion], base: usize) -> Result<bool, String> {
    let mut dos = [0u8; 64];
    read_remote(pid, maps, base, &mut dos)?;
    if &dos[..2] != b"MZ" {
        return Ok(false);
    }
    let pe_offset = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]) as usize;
    if pe_offset > 16 * 1024 * 1024 {
        return Ok(false);
    }
    let pe_address = match base.checked_add(pe_offset) {
        Some(v) => v,
        None => return Ok(false),
    };
    let mut header = [0u8; 26];
    read_remote(pid, maps, pe_address, &mut header)?;
    Ok(is_x64_pe32_plus_header(&dos, &header))
}

fn is_x64_pe32_plus_header(dos: &[u8], header: &[u8]) -> bool {
    dos.len() >= 64
        && header.len() >= 26
        && &dos[..2] == b"MZ"
        && &header[..4] == b"PE\0\0"
        && u16::from_le_bytes([header[4], header[5]]) == 0x8664
        && u16::from_le_bytes([header[24], header[25]]) == 0x20b
}

fn read_remote(pid: u32, maps: &[MapRegion], address: usize, dst: &mut [u8]) -> Result<(), String> {
    let end = address
        .checked_add(dst.len())
        .ok_or_else(|| "address overflow".to_string())?;
    ensure_readable_range(maps, address, end)?;
    let mut done = 0;
    while done < dst.len() {
        let local = libc::iovec {
            iov_base: unsafe { dst.as_mut_ptr().add(done) }.cast(),
            iov_len: dst.len() - done,
        };
        let remote = libc::iovec {
            iov_base: (address + done) as *mut libc::c_void,
            iov_len: dst.len() - done,
        };
        let n = unsafe { libc::process_vm_readv(pid as libc::pid_t, &local, 1, &remote, 1, 0) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(access_error(pid, "read PE header", e));
        }
        if n == 0 {
            return Err("short PE header read".into());
        }
        done += n as usize;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_wine_windows_paths_and_comm_newline() {
        assert!(contains_sai_image(b"C:\\sai2-0702\\sai2.exe\0"));
        assert!(contains_sai_image(b"sai2.exe\n"));
        assert!(!contains_sai_image(b"python3\0/app/bottles\0"));
    }

    #[test]
    fn parses_repeated_spaces_and_pathnames_with_spaces() {
        let maps = parse_maps(
            "00400000-00401000  r--p  00000000  08:01  1234  /tmp/SAI2.exe copy/sai2.exe\n",
        );
        assert_eq!(maps.len(), 1);
        assert_eq!(
            maps[0].path.as_deref(),
            Some(Path::new("/tmp/SAI2.exe copy/sai2.exe"))
        );
        assert!(is_sai_image(maps[0].path.as_deref().unwrap()));
        assert!(maps[0].readable);
        assert_eq!(maps[0].offset, 0);
    }

    #[test]
    fn readable_range_can_cross_adjacent_maps_but_not_gaps_or_unreadable_maps() {
        let region = |start, end, readable| MapRegion {
            start,
            end,
            readable,
            offset: 0,
            path: None,
        };
        let adjacent = [region(10, 15, true), region(15, 20, true)];
        assert!(ensure_readable_range(&adjacent, 12, 18).is_ok());
        let gap = [region(10, 14, true), region(15, 20, true)];
        assert!(ensure_readable_range(&gap, 12, 18).is_err());
        let unreadable = [region(10, 15, true), region(15, 20, false)];
        assert!(ensure_readable_range(&unreadable, 12, 18).is_err());
    }

    #[test]
    fn validates_x64_pe32_plus_headers() {
        let mut dos = [0u8; 64];
        let mut header = [0u8; 26];
        dos[..2].copy_from_slice(b"MZ");
        header[..4].copy_from_slice(b"PE\0\0");
        header[4..6].copy_from_slice(&0x8664u16.to_le_bytes());
        header[24..26].copy_from_slice(&0x20bu16.to_le_bytes());
        assert!(is_x64_pe32_plus_header(&dos, &header));

        header[4..6].copy_from_slice(&0x014cu16.to_le_bytes());
        assert!(!is_x64_pe32_plus_header(&dos, &header));
        header[4..6].copy_from_slice(&0x8664u16.to_le_bytes());
        header[24..26].copy_from_slice(&0x10bu16.to_le_bytes());
        assert!(!is_x64_pe32_plus_header(&dos, &header));
    }
}
