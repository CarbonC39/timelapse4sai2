use super::storage::Manifest;
use crate::sai::canvas::Canvas;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveCanvasIdentity {
    pub pid: u32,
    pub address: usize,
    pub id: Option<u32>,
}

impl LiveCanvasIdentity {
    pub fn new(pid: u32, canvas: &Canvas) -> Self {
        Self { pid, address: canvas.address, id: canvas.id }
    }

    pub fn matches(self, pid: u32, canvas: &Canvas) -> bool {
        self.pid == pid && self.address == canvas.address && self.id == canvas.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMatch {
    SameLiveObject,
    SamePath,
    Ambiguous,
    DifferentCanvas,
}

impl SessionMatch {
    pub fn classify(
        manifest: &Manifest,
        current: &Canvas,
        previous: Option<LiveCanvasIdentity>,
        pid: u32,
    ) -> Self {
        if previous.is_some_and(|identity| identity.matches(pid, current)) {
            return Self::SameLiveObject;
        }

        let saved = normalize_comparison_path(manifest.effective_canvas_path());
        let current_path = normalize_comparison_path(&current.short_path);
        if saved.is_empty() || current_path.is_empty() {
            return Self::Ambiguous;
        }
        if saved == current_path { Self::SamePath } else { Self::DifferentCanvas }
    }

    pub fn requires_confirmation(self) -> bool {
        matches!(self, Self::Ambiguous | Self::DifferentCanvas)
    }

    pub fn ensure_allowed(self, allow: bool) -> Result<(), String> {
        if self.requires_confirmation() && !allow {
            Err("canvas identity is uncertain or different; pass --allow-canvas-mismatch to continue".into())
        } else {
            Ok(())
        }
    }
}

/// Return a stable key for session lookup, resolving filesystem aliases where possible.
pub fn session_key(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() || has_windows_drive(path) {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    });
    PathBuf::from(normalize_lexical_path(&path))
}

pub fn build_warning(manifest: &Manifest, current_hash: &str) -> Option<String> {
    (manifest.source_executable_md5 != current_hash).then(|| {
        "This session was created with a different SAI2 build. The current build is compatible."
            .into()
    })
}

fn has_windows_drive(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn normalize_comparison_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let (prefix, rest) = if path.starts_with("//") {
        ("//".to_string(), path.trim_start_matches('/'))
    } else if path.starts_with('/') {
        ("/".to_string(), path.trim_start_matches('/'))
    } else if path.as_bytes().get(1) == Some(&b':') {
        if path[2..].starts_with('/') {
            (format!("{}/", &path[..2]), path[3..].trim_start_matches('/'))
        } else {
            (path[..2].to_string(), &path[2..])
        }
    } else {
        (String::new(), path.as_str())
    };
    let rooted = prefix == "/" || prefix == "//" || prefix.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." if !parts.is_empty() && parts.last() != Some(&"..") => { parts.pop(); }
            ".." if !rooted => parts.push(".."),
            ".." => {}
            p => parts.push(p),
        }
    }
    format!("{prefix}{}", parts.join("/")).to_lowercase()
}

fn normalize_lexical_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let absolute = raw.starts_with('/') || has_windows_drive(path);
    let mut parts: Vec<&str> = Vec::new();
    for component in raw.split('/') {
        match component {
            "" | "." => {}
            ".." if !parts.is_empty() && parts.last() != Some(&"..") => { parts.pop(); }
            ".." if !absolute => parts.push(".."),
            ".." => {}
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    let result = if raw.starts_with('/') { format!("/{joined}") } else { joined };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(address: usize, id: Option<u32>, path: &str) -> Canvas {
        Canvas { address, id, width: 100, height: 80, name: "art".into(), short_path: path.into() }
    }

    fn manifest(path: &str) -> Manifest {
        Manifest {
            format_version: 1, sai_profile: "p".into(), sai_generation: None,
            source_executable_md5: "old-hash".into(), initial_width: 100, initial_height: 80,
            canvas_name: "old".into(), canvas_path: "old-path.sai".into(),
            latest_canvas_name: None, latest_canvas_path: Some(path.into()),
            tile_size: 256, channel_order: "BGRX".into(), creation_time_unix_ms: 0,
            recorder_version: "test".into(), identity_limit: "test".into(),
        }
    }

    #[test]
    fn live_identity_survives_save_save_as_and_resize() {
        let mut before = canvas(10, Some(4), "");
        before.name = "NewCanvas1".into();
        let identity = LiveCanvasIdentity::new(7, &before);
        let saved = canvas(10, Some(4), "C:/work/first.sai");
        assert_eq!(SessionMatch::classify(&manifest(""), &saved, Some(identity), 7), SessionMatch::SameLiveObject);
        let save_as = canvas(10, Some(4), "D:/other/renamed.sai");
        assert_eq!(SessionMatch::classify(&manifest("first.sai"), &save_as, Some(identity), 7), SessionMatch::SameLiveObject);
        let mut resized = canvas(10, Some(4), "D:/other/renamed.sai");
        resized.width = 200;
        resized.height = 150;
        assert_eq!(SessionMatch::classify(&manifest("renamed.sai"), &resized, Some(identity), 7), SessionMatch::SameLiveObject);
    }

    #[test]
    fn path_fallback_handles_pid_case_and_separator_differences() {
        assert_eq!(SessionMatch::classify(&manifest("C:\\Art\\Final.sai"), &canvas(22, None, "c:/art//final.sai"), None, 9), SessionMatch::SamePath);
        assert_ne!(normalize_comparison_path("/art/file.sai"), normalize_comparison_path("art/file.sai"));
        assert_ne!(normalize_comparison_path("C:/art/file.sai"), normalize_comparison_path("C:art/file.sai"));
        assert_eq!(normalize_comparison_path("//server/share/a.sai"), normalize_comparison_path("\\\\SERVER\\share\\a.sai"));
    }

    #[test]
    fn policy_and_closed_reopened_canvas() {
        let old = LiveCanvasIdentity::new(7, &canvas(10, Some(4), "a.sai"));
        assert_eq!(SessionMatch::classify(&manifest("a.sai"), &canvas(11, Some(4), "a.sai"), Some(old), 7), SessionMatch::SamePath);
        assert_eq!(SessionMatch::classify(&manifest("a.sai"), &canvas(11, Some(4), "b.sai"), None, 7), SessionMatch::DifferentCanvas);
        assert_eq!(SessionMatch::classify(&manifest(""), &canvas(11, None, ""), None, 7), SessionMatch::Ambiguous);
        assert!(SessionMatch::Ambiguous.requires_confirmation());
        assert!(SessionMatch::DifferentCanvas.ensure_allowed(false).unwrap_err().contains("--allow-canvas-mismatch"));
        assert!(SessionMatch::DifferentCanvas.ensure_allowed(true).is_ok());
        assert!(SessionMatch::Ambiguous.ensure_allowed(false).is_err());
        assert!(SessionMatch::Ambiguous.ensure_allowed(true).is_ok());
        assert!(!SessionMatch::SamePath.requires_confirmation());
    }

    #[test]
    fn effective_path_falls_back_and_hash_warning_is_exactly_gated() {
        let mut m = manifest("ignored.sai");
        m.latest_canvas_path = None;
        assert_eq!(SessionMatch::classify(&m, &canvas(1, None, "old-path.sai"), None, 1), SessionMatch::SamePath);
        assert!(build_warning(&m, "old-hash").is_none());
        assert_eq!(build_warning(&m, "new-hash").unwrap(), "This session was created with a different SAI2 build. The current build is compatible.");
    }
}
