#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaiGeneration {
    LegacyPreview,
    Renovated,
}
impl SaiGeneration {
    pub fn name(self) -> &'static str {
        match self {
            Self::LegacyPreview => "LegacyPreview",
            Self::Renovated => "Renovated",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RootLocator {
    FixedRva(usize),
    Signature(super::signature::SignatureLocator<'static>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityStatus {
    ExactKnownBuild,
    SignatureValidated,
    Unsupported,
}

#[derive(Debug)]
pub struct SaiProfile {
    pub display_name: &'static str,
    pub executable_hash: &'static str,
    pub generation: SaiGeneration,
    pub layout: &'static super::layouts::CanvasLayout,
    pub locator: RootLocator,
}

pub fn identify(hash: &str) -> Result<&'static SaiProfile, String> {
    KNOWN_BUILDS
        .iter()
        .copied()
        .find(|p| p.executable_hash == hash)
        .ok_or_else(|| format!("unknown SAI2 build: sai2.exe MD5 {hash}"))
}

pub static KNOWN_BUILDS: &[&SaiProfile] = &[
    &super::builds::preview_2026_07_02b::PROFILE,
    &super::builds::renovated_2026_10_19::PROFILE,
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifies_known_builds_and_rejects_unknown_hashes() {
        assert_eq!(
            identify("6f3f351303cf3896f4c2977925c994c2")
                .unwrap()
                .generation,
            SaiGeneration::LegacyPreview
        );
        assert_eq!(
            identify("44aee0ba61ea9f215b1b4dcb4d6d23e0")
                .unwrap()
                .generation,
            SaiGeneration::Renovated
        );
        for profile in KNOWN_BUILDS {
            assert!(std::ptr::eq(
                identify(profile.executable_hash).unwrap(),
                *profile
            ));
        }
        assert!(
            identify("00000000000000000000000000000000")
                .unwrap_err()
                .contains("unknown SAI2 build")
        );
    }
}
