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
}

#[derive(Debug, Clone, Copy)]
pub struct CanvasLayout {
    pub next_offset: usize,
    pub id_offset: Option<usize>,
    pub width_offset: usize,
    pub height_offset: usize,
    pub name_offset: usize,
    pub short_path_offset: usize,
    pub string_bytes: usize,
    pub tiles: TileLayout,
}

#[derive(Debug, Clone, Copy)]
pub struct TileLayout {
    /// Inline array entry for level 0: canvas -> slot -> map -> row tree.
    pub level0_offset: usize,
    pub canvas_grid_offsets: Option<(usize, usize)>,
    pub tree_offset: usize,
    pub width_offset: usize,
    pub height_offset: usize,
    pub nx_offset: usize,
    pub ny_offset: usize,
}

#[derive(Debug)]
pub struct SaiProfile {
    pub display_name: &'static str,
    pub executable_hash: &'static str,
    pub generation: SaiGeneration,
    pub layout: &'static CanvasLayout,
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
    &super::preview_2026_07_02b::PROFILE,
    &super::renovated_2026_10_19::PROFILE,
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
