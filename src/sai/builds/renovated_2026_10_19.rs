use super::super::profile::{RootLocator, SaiGeneration, SaiProfile};

pub static PROFILE: SaiProfile = SaiProfile {
    display_name: "SAI Ver.2 64bit - 2026-10-19 Major Renovated",
    executable_hash: "44aee0ba61ea9f215b1b4dcb4d6d23e0",
    generation: SaiGeneration::Renovated,
    locator: RootLocator::FixedRva(0x47a380),
    layout: &super::super::layouts::renovated_alpha::LAYOUT,
};

pub static SIGNATURE_PROFILE: SaiProfile = SaiProfile {
    display_name: "SAI Ver.2 64bit - Major Renovated (signature validated)",
    executable_hash: "",
    generation: SaiGeneration::Renovated,
    locator: RootLocator::Signature(&super::super::signature::RENOVATED_SIGNATURES),
    layout: &super::super::layouts::renovated_alpha::LAYOUT,
};
