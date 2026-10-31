//! SAI process discovery and compatibility validation shared by frontends.
use crate::{
    process::{self, Process},
    sai::{
        self,
        canvas::Canvas,
        profile::{CompatibilityStatus, RootLocator, SaiProfile},
    },
};
use std::fmt;

pub struct Connection {
    pub process: Process,
    pub executable_md5: String,
    pub profile: &'static SaiProfile,
    pub compatibility: CompatibilityStatus,
    pub resolved_root_rva: Option<usize>,
    pub canvases: Vec<Canvas>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionError {
    NotFound(String),
    Access(String),
    Process(String),
    Unsupported(String),
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(detail)
            | Self::Access(detail)
            | Self::Process(detail)
            | Self::Unsupported(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for ConnectionError {}

impl Connection {
    pub fn discover(pid: Option<u32>) -> Result<Self, String> {
        discover(pid)
    }
    pub fn discover_detailed(pid: Option<u32>) -> Result<Self, ConnectionError> {
        discover_detailed(pid)
    }
    pub fn canvases(&self) -> &[Canvas] {
        &self.canvases
    }
    pub fn reader(&self) -> sai::reader::Reader<'_, Process> {
        sai::reader::Reader {
            memory: &self.process,
            base: self.process.base_address,
            profile: self.profile,
            resolved_root_rva: self.resolved_root_rva,
            source_hash: Some(&self.executable_md5),
        }
    }
}

pub fn discover(pid: Option<u32>) -> Result<Connection, String> {
    discover_detailed(pid).map_err(|e| e.to_string())
}

/// Discover a process and classify compatibility failures without requiring
/// callers to inspect error text. Process lookup/access failures are distinct
/// from executable or runtime layout incompatibility.
pub fn discover_detailed(pid: Option<u32>) -> Result<Connection, ConnectionError> {
    let process = process::discover_detailed(pid).map_err(|e| match e {
        process::DiscoveryError::NotFound(detail) => ConnectionError::NotFound(detail),
        process::DiscoveryError::Access(detail) => ConnectionError::Access(detail),
        process::DiscoveryError::Other(detail) => ConnectionError::Process(detail),
    })?;
    let hash = process::executable_hash(&process).map_err(ConnectionError::Process)?;
    let (profile, initial_status, resolved_root_rva) = match sai::profile::identify(&hash) {
        Ok(profile) => (profile, CompatibilityStatus::ExactKnownBuild, None),
        Err(_) => {
            let profile = &sai::builds::renovated_2026_10_19::SIGNATURE_PROFILE;
            let RootLocator::Signature(signatures) = profile.locator else {
                unreachable!()
            };
            let image = std::fs::read(process.executable_path()).map_err(|e| {
                ConnectionError::Unsupported(format!(
                    "cannot read executable for signature scan: {e}"
                ))
            })?;
            if format!("{:x}", md5::compute(&image)) != hash {
                return Err(ConnectionError::Unsupported(
                    "executable changed during signature scan".into(),
                ));
            }
            let rva = sai::signature::resolve_signatures(signatures, &image).map_err(|e| {
                ConnectionError::Unsupported(format!("Renovated signature scan failed: {e}"))
            })?;
            (profile, CompatibilityStatus::NeedsCanvas, Some(rva))
        }
    };
    let reader = sai::reader::Reader {
        memory: &process,
        base: process.base_address,
        profile,
        resolved_root_rva,
        source_hash: Some(&hash),
    };
    let (compatibility, canvases) = if initial_status == CompatibilityStatus::NeedsCanvas {
        let canvases = reader.canvases().map_err(ConnectionError::Process)?;
        if canvases.is_empty() {
            (CompatibilityStatus::NeedsCanvas, canvases)
        } else {
            (
                CompatibilityStatus::SignatureValidated,
                reader
                    .validate_runtime_layout()
                    .map_err(ConnectionError::Unsupported)?,
            )
        }
    } else {
        (
            initial_status,
            reader.canvases().map_err(ConnectionError::Process)?,
        )
    };
    Ok(Connection {
        process,
        executable_md5: hash,
        profile,
        compatibility,
        resolved_root_rva,
        canvases,
    })
}
