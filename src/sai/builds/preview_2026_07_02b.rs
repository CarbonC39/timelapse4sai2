use super::super::profile::{RootLocator, SaiGeneration, SaiProfile};

pub const SESSION_OFFSET: usize = 0x32eb20;
pub const MD5: &str = "6f3f351303cf3896f4c2977925c994c2";
pub static PROFILE: SaiProfile = SaiProfile {
    display_name: "SAI2 x64 2026-07-02b Technical Preview",
    executable_hash: MD5,
    generation: SaiGeneration::LegacyPreview,
    layout: &super::super::layouts::legacy_preview::LAYOUT,
    locator: RootLocator::FixedRva(SESSION_OFFSET),
};
