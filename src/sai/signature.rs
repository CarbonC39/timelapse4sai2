//! Read-only PE32+ signature scanning for locating global variables.

const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const PE32_PLUS_MAGIC: u16 = 0x20b;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

/// A masked instruction signature whose RIP-relative displacement points at a global.
/// `mask[i] == true` compares the corresponding pattern byte; false is a wildcard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignatureLocator<'a> {
    pub pattern: &'a [u8],
    pub mask: &'a [bool],
    /// Byte offset of the 4-byte signed RIP displacement within the pattern.
    pub rip_disp_offset: usize,
    /// Offset from the start of the pattern to the end of the RIP-relative instruction.
    pub instruction_end_offset: usize,
}

/// Stable Renovated root reference seen in 2026-08-13 through 2026-10-19 builds.
pub const RENOVATED_PATTERN: &[u8] = &[
    0x40, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x3d, 0, 0, 0, 0, 0x48, 0x85, 0xff, 0x0f, 0x84,
    0, 0, 0, 0, 0x48, 0x89, 0x5c,
];
pub const RENOVATED_MASK: &[bool] = &[
    true, true, true, true, true, true, true, true, true, false, false, false, false, true, true,
    true, true, true, false, false, false, false, true, true, true,
];
pub const RENOVATED_SIGNATURE: SignatureLocator<'static> = SignatureLocator {
    pattern: RENOVATED_PATTERN,
    mask: RENOVATED_MASK,
    rip_disp_offset: 9,
    instruction_end_offset: 13,
};

pub const RENOVATED_ROOT_PATTERN: &[u8] = &[
    0x48, 0x8b, 0x1d, 0, 0, 0, 0, 0x48, 0x8b, 0x79, 0x68, 0x48, 0x85, 0xdb, 0x74, 0x41, 0x48, 0x89,
    0x74, 0x24, 0x30,
];
pub const RENOVATED_ROOT_MASK: &[bool] = &[
    true, true, true, false, false, false, false, true, true, true, true, true, true, true, true,
    true, true, true, true, true, true,
];
pub const RENOVATED_ROOT_SIGNATURE: SignatureLocator<'static> = SignatureLocator {
    pattern: RENOVATED_ROOT_PATTERN,
    mask: RENOVATED_ROOT_MASK,
    rip_disp_offset: 3,
    instruction_end_offset: 7,
};

pub const RENOVATED_SIGNATURES: &[SignatureLocator<'static>] =
    &[RENOVATED_SIGNATURE, RENOVATED_ROOT_SIGNATURE];

/// Resolve independent fallback signatures, requiring unique matches and agreement.
pub fn resolve_signatures(
    signatures: &[SignatureLocator<'_>],
    image: &[u8],
) -> Result<usize, String> {
    let mut resolved = None;
    for signature in signatures {
        if let Some(rva) = signature.resolve_optional(image)? {
            if resolved.is_some_and(|previous| previous != rva) {
                return Err(format!(
                    "signature targets disagree: {:#x} and {rva:#x}",
                    resolved.unwrap()
                ));
            }
            resolved = Some(rva);
        }
    }
    resolved.ok_or_else(|| "no signature matched in .text".into())
}

#[derive(Debug, Clone, Copy)]
struct Section<'a> {
    name: &'a [u8; 8],
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
    characteristics: u32,
}

impl Section<'_> {
    fn named(&self, name: &[u8]) -> bool {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(8);
        &self.name[..end] == name
    }

    fn executable(&self) -> bool {
        self.characteristics & IMAGE_SCN_MEM_EXECUTE != 0
    }

    fn writable_data(&self) -> bool {
        self.characteristics & IMAGE_SCN_MEM_WRITE != 0 && !self.executable()
    }

    fn virtual_span(&self) -> u64 {
        u64::from(self.virtual_size.max(self.raw_size))
    }
}

fn read_u16(image: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = image
        .get(offset..offset.checked_add(2).ok_or("PE offset overflow")?)
        .ok_or_else(|| format!("truncated PE image at {offset:#x}"))?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u32(image: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = image
        .get(offset..offset.checked_add(4).ok_or("PE offset overflow")?)
        .ok_or_else(|| format!("truncated PE image at {offset:#x}"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn parse_sections(image: &[u8]) -> Result<Vec<Section<'_>>, String> {
    if image.get(..2) != Some(b"MZ") {
        return Err("invalid PE image: missing MZ signature".into());
    }
    let pe_offset = read_u32(image, 0x3c)? as usize;
    let signature_end = pe_offset.checked_add(4).ok_or("PE offset overflow")?;
    if image.get(pe_offset..signature_end) != Some(b"PE\0\0") {
        return Err("invalid PE image: missing PE signature".into());
    }
    let coff = signature_end;
    if read_u16(image, coff)? != IMAGE_FILE_MACHINE_AMD64 {
        return Err("unsupported PE machine; expected AMD64".into());
    }
    let section_count = usize::from(read_u16(image, coff + 2)?);
    if section_count == 0 {
        return Err("invalid PE image: no sections".into());
    }
    let optional_size = usize::from(read_u16(image, coff + 16)?);
    let optional = coff.checked_add(20).ok_or("PE offset overflow")?;
    if read_u16(image, optional)? != PE32_PLUS_MAGIC {
        return Err("unsupported PE optional header; expected PE32+".into());
    }
    let section_table = optional
        .checked_add(optional_size)
        .ok_or("PE section table offset overflow")?;
    let table_size = section_count
        .checked_mul(40)
        .ok_or("PE section table size overflow")?;
    let table_end = section_table
        .checked_add(table_size)
        .ok_or("PE section table end overflow")?;
    if table_end > image.len() {
        return Err("truncated PE section table".into());
    }

    let mut sections = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let offset = section_table + index * 40;
        let name: &[u8; 8] = image[offset..offset + 8].try_into().unwrap();
        let virtual_size = read_u32(image, offset + 8)?;
        let virtual_address = read_u32(image, offset + 12)?;
        let raw_size = read_u32(image, offset + 16)?;
        let raw_offset = read_u32(image, offset + 20)?;
        let characteristics = read_u32(image, offset + 36)?;
        let raw_end = u64::from(raw_offset) + u64::from(raw_size);
        if raw_size > 0 && raw_end > image.len() as u64 {
            return Err(format!("section {index} raw data extends past PE image"));
        }
        sections.push(Section {
            name,
            virtual_address,
            virtual_size,
            raw_offset,
            raw_size,
            characteristics,
        });
    }
    Ok(sections)
}

impl SignatureLocator<'_> {
    /// Find the unique masked signature in `.text`, resolve its RIP-relative target,
    /// and require that target to reside in a writable, non-executable section.
    #[allow(dead_code)] // Also useful for validating each signature independently offline.
    pub fn resolve(&self, image: &[u8]) -> Result<usize, String> {
        self.resolve_optional(image)?
            .ok_or_else(|| "signature did not match in .text".into())
    }

    fn resolve_optional(&self, image: &[u8]) -> Result<Option<usize>, String> {
        if self.pattern.is_empty() || self.pattern.len() != self.mask.len() {
            return Err(
                "invalid signature: pattern and mask must have equal nonzero lengths".into(),
            );
        }
        let displacement_end = self
            .rip_disp_offset
            .checked_add(4)
            .ok_or("signature displacement offset overflow")?;
        if displacement_end > self.pattern.len()
            || self.instruction_end_offset < displacement_end
            || self.instruction_end_offset > self.pattern.len()
        {
            return Err(
                "invalid signature: RIP displacement or instruction length is out of range".into(),
            );
        }

        let sections = parse_sections(image)?;
        let text = sections
            .iter()
            .find(|section| section.named(b".text") && section.executable())
            .ok_or("PE image has no executable .text section")?;
        let text_start = text.raw_offset as usize;
        let text_len = text.raw_size as usize;
        let text_end = text_start
            .checked_add(text_len)
            .ok_or(".text range overflow")?;
        let text_bytes = image
            .get(text_start..text_end)
            .ok_or(".text raw data extends past PE image")?;
        if self.pattern.len() > text_bytes.len() {
            return Err("signature is longer than .text section".into());
        }

        let mut match_offset = None;
        for offset in 0..=text_bytes.len() - self.pattern.len() {
            if self
                .pattern
                .iter()
                .zip(self.mask.iter())
                .enumerate()
                .all(|(i, (&expected, &exact))| !exact || text_bytes[offset + i] == expected)
            {
                if match_offset.replace(offset).is_some() {
                    return Err("signature matched more than once in .text".into());
                }
            }
        }
        let Some(offset) = match_offset else {
            return Ok(None);
        };
        let instruction_rva = u64::from(text.virtual_address)
            .checked_add(offset as u64)
            .ok_or("signature RVA overflow")?;
        let disp_offset = offset
            .checked_add(self.rip_disp_offset)
            .and_then(|n| text_start.checked_add(n))
            .ok_or("signature displacement file offset overflow")?;
        let displacement = i32::from_le_bytes(
            image
                .get(disp_offset..disp_offset + 4)
                .ok_or("truncated signature displacement")?
                .try_into()
                .unwrap(),
        );
        let next_instruction = instruction_rva
            .checked_add(self.instruction_end_offset as u64)
            .ok_or("instruction RVA overflow")?;
        let target = i128::from(next_instruction) + i128::from(displacement);
        if !(0..=i128::from(u32::MAX)).contains(&target) {
            return Err("RIP-relative target RVA is out of PE range".into());
        }
        let target = target as u32;
        let in_data = sections.iter().any(|section| {
            section.writable_data()
                && u64::from(target) >= u64::from(section.virtual_address)
                && u64::from(target) < u64::from(section.virtual_address) + section.virtual_span()
        });
        if !in_data {
            return Err(format!(
                "RIP-relative target {target:#x} is outside writable non-executable sections"
            ));
        }
        Ok(Some(target as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_pe() -> Vec<u8> {
        let mut image = vec![0; 0x800];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        image[0x80..0x84].copy_from_slice(b"PE\0\0");
        image[0x84..0x86].copy_from_slice(&IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
        image[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        image[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
        image[0x98..0x9a].copy_from_slice(&PE32_PLUS_MAGIC.to_le_bytes());
        let sections = 0x188;
        image[sections..sections + 5].copy_from_slice(b".text");
        image[sections + 8..sections + 12].copy_from_slice(&0x200u32.to_le_bytes());
        image[sections + 12..sections + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        image[sections + 16..sections + 20].copy_from_slice(&0x200u32.to_le_bytes());
        image[sections + 20..sections + 24].copy_from_slice(&0x400u32.to_le_bytes());
        image[sections + 36..sections + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());
        let data = sections + 40;
        image[data..data + 5].copy_from_slice(b".data");
        image[data + 8..data + 12].copy_from_slice(&0x100u32.to_le_bytes());
        image[data + 12..data + 16].copy_from_slice(&0x3000u32.to_le_bytes());
        image[data + 16..data + 20].copy_from_slice(&0x100u32.to_le_bytes());
        image[data + 20..data + 24].copy_from_slice(&0x600u32.to_le_bytes());
        image[data + 36..data + 40].copy_from_slice(&0xc000_0040u32.to_le_bytes());
        // 6-byte RIP-relative instruction at RVA 0x1010 -> global at RVA 0x3020.
        let instr = 0x410;
        image[instr..instr + 2].copy_from_slice(&[0x48, 0x8b]);
        image[instr + 2..instr + 6].copy_from_slice(&0x200au32.to_le_bytes());
        image
    }

    fn locator() -> SignatureLocator<'static> {
        SignatureLocator {
            pattern: &[0x48, 0x8b, 0, 0, 0, 0],
            mask: &[true, true, false, false, false, false],
            rip_disp_offset: 2,
            instruction_end_offset: 6,
        }
    }

    #[test]
    fn resolves_unique_rip_reference_into_writable_data() {
        assert_eq!(locator().resolve(&synthetic_pe()).unwrap(), 0x3020);
    }

    #[test]
    fn rejects_duplicate_signature() {
        let mut image = synthetic_pe();
        let duplicate = image[0x410..0x416].to_vec();
        image[0x430..0x436].copy_from_slice(&duplicate);
        assert!(
            locator()
                .resolve(&image)
                .unwrap_err()
                .contains("more than once")
        );
    }

    #[test]
    fn rejects_missing_signature_and_non_data_target() {
        let mut image = synthetic_pe();
        image[0x410] = 0x90;
        assert!(
            locator()
                .resolve(&image)
                .unwrap_err()
                .contains("did not match")
        );

        let mut image = synthetic_pe();
        image[0x412..0x416].copy_from_slice(&0i32.to_le_bytes());
        assert!(
            locator()
                .resolve(&image)
                .unwrap_err()
                .contains("outside writable")
        );
    }

    #[test]
    fn fallback_resolves_missing_primary_and_rejects_disagreement() {
        let fallback = SignatureLocator {
            pattern: &[0x49, 0x8b, 0, 0, 0, 0],
            mask: &[true, true, false, false, false, false],
            rip_disp_offset: 2,
            instruction_end_offset: 6,
        };
        let mut image = synthetic_pe();
        image[0x430..0x432].copy_from_slice(&[0x49, 0x8b]);
        image[0x432..0x436].copy_from_slice(&0x1feai32.to_le_bytes());
        image[0x410] = 0x90;
        assert_eq!(
            resolve_signatures(&[locator(), fallback], &image),
            Ok(0x3020)
        );

        image[0x410] = 0x48;
        image[0x432..0x436].copy_from_slice(&0x1ffai32.to_le_bytes());
        assert!(
            resolve_signatures(&[locator(), fallback], &image)
                .unwrap_err()
                .contains("disagree")
        );
    }

    #[test]
    fn resolves_available_renovated_pe_samples() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for (directory, expected_rva) in [
            // These two dates are derived from both signatures, not supplied ground truth.
            ("0813", 0x472380),
            ("0824", 0x474380),
            ("0902", 0x476280),
            ("0925", 0x478280),
            ("1019", 0x47a380),
        ] {
            let path = root.join(directory).join("sai2.exe");
            if !path.is_file() {
                continue;
            }
            let image = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            for signature in RENOVATED_SIGNATURES {
                assert_eq!(
                    signature.resolve(&image),
                    Ok(expected_rva),
                    "unexpected or non-unique Renovated signature in {}",
                    path.display()
                );
            }
            assert_eq!(
                resolve_signatures(RENOVATED_SIGNATURES, &image),
                Ok(expected_rva),
                "fallback signatures disagree in {}",
                path.display()
            );
        }
    }
}
