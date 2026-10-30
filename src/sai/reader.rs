use super::{
    canvas::Canvas,
    profile::{RootLocator, SaiProfile},
};
const MAX_DIMENSION: u32 = 100_000;
const MAX_RGB_BYTES: usize = 1024 * 1024 * 1024;
use crate::process::ProcessMemory;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileMap {
    pub nx: u32,
    pub ny: u32,
    pub(crate) map: usize,
    pub(crate) slot: usize,
    level0_address: usize,
    pub(crate) tree: usize,
    pub(crate) metadata: [u8; 32],
}

pub struct Reader<'a, M: ProcessMemory> {
    pub memory: &'a M,
    pub base: usize,
    pub profile: &'static SaiProfile,
}
impl<M: ProcessMemory> Reader<'_, M> {
    fn bytes<const N: usize>(&self, address: usize) -> Result<[u8; N], String> {
        let mut bytes = [0; N];
        self.memory.read_into(address, &mut bytes)?;
        Ok(bytes)
    }
    fn at(address: usize, offset: usize) -> Result<usize, String> {
        address
            .checked_add(offset)
            .ok_or_else(|| "address overflow".into())
    }
    fn ptr(&self, address: usize) -> Result<usize, String> {
        let p = u64::from_le_bytes(self.bytes(address)?);
        if p > 0x0000_7fff_ffff_ffff {
            return Err(format!("invalid pointer {p:#x} at {address:#x}"));
        }
        Ok(p as usize)
    }
    fn required_ptr(&self, address: usize) -> Result<usize, String> {
        let p = self.ptr(address)?;
        if p == 0 {
            Err(format!("null pointer at {address:#x}"))
        } else {
            Ok(p)
        }
    }
    fn u32(&self, address: usize) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(address)?))
    }
    fn string(&self, address: usize) -> Result<String, String> {
        let mut bytes = vec![0; self.profile.layout.string_bytes];
        self.memory.read_into(address, &mut bytes)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let end = units
            .iter()
            .position(|&u| u == 0)
            .ok_or("unterminated UTF-16 string")?;
        String::from_utf16(&units[..end]).map_err(|e| format!("invalid UTF-16: {e}"))
    }
    pub fn read_canvas(&self, p: usize) -> Result<Canvas, String> {
        let l = self.profile.layout;
        let width = self.u32(Self::at(p, l.width_offset)?)?;
        let height = self.u32(Self::at(p, l.height_offset)?)?;
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(format!(
                "invalid canvas dimensions {width}x{height} at {p:#x}"
            ));
        }
        Ok(Canvas {
            address: p,
            id: l
                .id_offset
                .map(|offset| self.u32(Self::at(p, offset)?))
                .transpose()?,
            width,
            height,
            name: self.string(Self::at(p, l.name_offset)?)?,
            short_path: self.string(Self::at(p, l.short_path_offset)?)?,
        })
    }
    pub fn canvases(&self) -> Result<Vec<Canvas>, String> {
        // Original get_canvas_list(): read the sentinel pointer, collect, discard sentinel.
        let RootLocator::FixedRva(rva) = self.profile.locator;
        let sentinel = self.required_ptr(Self::at(self.base, rva)?)?;
        let mut p = self.ptr(Self::at(sentinel, self.profile.layout.next_offset)?)?;
        let mut seen = HashSet::from([sentinel]);
        let mut result = Vec::new();
        while p != 0 {
            if !seen.insert(p) {
                return Err(format!("cycle in canvas list at {p:#x}"));
            }
            if result.len() >= 1024 {
                return Err("canvas list exceeds 1024 entries".into());
            }
            result.push(self.read_canvas(p)?);
            p = self.ptr(Self::at(p, self.profile.layout.next_offset)?)?;
        }
        Ok(result)
    }
    pub fn read_tile_map(&self, canvas: &Canvas) -> Result<TileMap, String> {
        let l = self.profile.layout.tiles;
        let level0_address = Self::at(canvas.address, l.level0_offset)?;
        let slot = self.required_ptr(level0_address)?;
        let map = self.required_ptr(slot)?;
        let metadata = self.bytes::<32>(map)?;
        let tree = self.required_ptr(Self::at(map, l.tree_offset)?)?;
        let width = self.u32(Self::at(map, l.width_offset)?)?;
        let height = self.u32(Self::at(map, l.height_offset)?)?;
        let nx = self.u32(Self::at(map, l.nx_offset)?)?;
        let ny = self.u32(Self::at(map, l.ny_offset)?)?;
        if let Some((nx_offset, ny_offset)) = l.canvas_grid_offsets {
            let canvas_nx = self.u32(Self::at(canvas.address, nx_offset)?)?;
            let canvas_ny = self.u32(Self::at(canvas.address, ny_offset)?)?;
            if (canvas_nx, canvas_ny) != (canvas.width.div_ceil(256), canvas.height.div_ceil(256))
                || (nx, ny) != (canvas_nx, canvas_ny)
            {
                return Err(format!(
                    "invalid canvas tile grid: {canvas_nx}x{canvas_ny}, map {nx}x{ny}"
                ));
            }
        }
        if (width, height) != (canvas.width, canvas.height)
            || nx != width.div_ceil(256)
            || ny != height.div_ceil(256)
        {
            return Err(format!(
                "invalid level-0 tile map: {width}x{height}, tiles {nx}x{ny}"
            ));
        }
        Ok(TileMap {
            nx,
            ny,
            map,
            slot,
            level0_address,
            tree,
            metadata,
        })
    }
    pub fn validate_tile_map(&self, canvas: &Canvas, map: &TileMap) -> Result<(), String> {
        if self.read_tile_map(canvas)? != *map {
            return Err("tile map changed during capture; retry".into());
        }
        Ok(())
    }
    pub fn read_tile_into(
        &self,
        map: &TileMap,
        x: u32,
        y: u32,
        dst: &mut [u8],
    ) -> Result<(), String> {
        if dst.len() != 256 * 256 * 4 {
            return Err("tile buffer must be exactly 256x256 RGBA bytes".into());
        }
        if x >= map.nx || y >= map.ny {
            return Err("tile coordinates out of range".into());
        }
        if self.required_ptr(map.level0_address)? != map.slot
            || self.required_ptr(map.slot)? != map.map
            || self.required_ptr(Self::at(map.map, self.profile.layout.tiles.tree_offset)?)?
                != map.tree
        {
            return Err("tile map changed during capture; retry".into());
        }
        let row_slot = Self::at(map.tree, y as usize * 8)?;
        let row = self.required_ptr(row_slot)?;
        let tile_slot = Self::at(row, x as usize * 8)?;
        let tile = self.required_ptr(tile_slot)?;
        self.memory.read_into(tile, dst)?;
        if self.ptr(tile_slot)? != tile || self.ptr(row_slot)? != row {
            return Err("tile changed during capture; retry".into());
        }
        Ok(())
    }
    pub fn composite(&self, canvas: &Canvas, rgba_order: bool) -> Result<Vec<u8>, String> {
        if self.read_canvas(canvas.address)? != *canvas {
            return Err("canvas changed before capture; retry".into());
        }
        let map = self.read_tile_map(canvas)?;
        let (width, height, nx, ny) = (canvas.width, canvas.height, map.nx, map.ny);
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(3))
            .filter(|&n| n <= MAX_RGB_BYTES)
            .ok_or("capture exceeds 1 GiB RGB limit")?;
        let mut rgb = Vec::new();
        rgb.try_reserve_exact(len).map_err(|e| e.to_string())?;
        rgb.resize(len, 0);
        let mut tile = vec![0; 256 * 256 * 4];
        for y in 0..ny as usize {
            for x in 0..nx as usize {
                self.read_tile_into(&map, x as u32, y as u32, &mut tile)?;
                for dy in 0..256.min(height as usize - y * 256) {
                    for dx in 0..256.min(width as usize - x * 256) {
                        let src = (dy * 256 + dx) * 4;
                        let dst = ((y * 256 + dy) * width as usize + x * 256 + dx) * 3;
                        rgb[dst..dst + 3].copy_from_slice(&[
                            tile[src + if rgba_order { 0 } else { 2 }],
                            tile[src + 1],
                            tile[src + if rgba_order { 2 } else { 0 }],
                        ]);
                    }
                }
            }
        }
        if self.validate_tile_map(canvas, &map).is_err()
            || self.read_canvas(canvas.address)? != *canvas
            || !self.canvases()?.contains(canvas)
        {
            return Err("canvas changed during capture; retry".into());
        }
        Ok(rgb)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::sai::preview_2026_07_02b::{PROFILE, SESSION_OFFSET};

    pub(crate) const BASE: usize = 0x100;
    pub(crate) const SENTINEL: usize = 0x1000;
    pub(crate) const CANVAS: usize = 0x2000;
    pub(crate) const SLOT: usize = 0x3000;
    pub(crate) const MAP: usize = 0x4000;
    pub(crate) const TREE: usize = 0x5000;
    pub(crate) const ROW: usize = 0x6000;
    pub(crate) const TILE0: usize = 0x7000;
    pub(crate) const TILE1: usize = 0x47000;

    pub(crate) struct FakeMemory(pub(crate) Vec<u8>);

    impl FakeMemory {
        pub(crate) fn new() -> Self {
            Self(vec![0; 5 * 1024 * 1024])
        }

        pub(crate) fn put(&mut self, address: usize, bytes: &[u8]) {
            self.0[address..address + bytes.len()].copy_from_slice(bytes);
        }

        pub(crate) fn u32(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }

        pub(crate) fn ptr(&mut self, address: usize, value: usize) {
            self.put(address, &(value as u64).to_le_bytes());
        }

        pub(crate) fn utf16(&mut self, address: usize, value: &str) {
            for (i, unit) in value.encode_utf16().enumerate() {
                self.put(address + i * 2, &unit.to_le_bytes());
            }
        }
    }

    impl ProcessMemory for FakeMemory {
        fn read_into(&self, address: usize, dst: &mut [u8]) -> Result<(), String> {
            let end = address.checked_add(dst.len()).ok_or("address overflow")?;
            let src = self
                .0
                .get(address..end)
                .ok_or_else(|| format!("out of range read at {address:#x}"))?;
            dst.copy_from_slice(src);
            Ok(())
        }
    }

    pub(crate) fn fixture() -> FakeMemory {
        let mut mem = FakeMemory::new();
        // Session -> sentinel -> canvas -> null. The sentinel itself is omitted.
        mem.ptr(BASE + SESSION_OFFSET, SENTINEL);
        mem.ptr(SENTINEL, CANVAS);
        mem.ptr(CANVAS, 0);

        mem.u32(CANVAS + 0x20, 42);
        mem.ptr(CANVAS + 0x28, SLOT);
        mem.u32(CANVAS + 0x250, 257);
        mem.u32(CANVAS + 0x254, 1);
        mem.utf16(CANVAS + 0x2ac, "synthetic");
        mem.utf16(CANVAS + 0x4ac, "synthetic.sai");

        mem.ptr(SLOT, MAP);
        mem.ptr(MAP + 8, TREE);
        mem.u32(MAP + 0x10, 257);
        mem.u32(MAP + 0x14, 1);
        mem.u32(MAP + 0x18, 2);
        mem.u32(MAP + 0x1c, 1);
        mem.ptr(TREE, ROW);
        mem.ptr(ROW, TILE0);
        mem.ptr(ROW + 8, TILE1);

        // Distinct BGRA values at the left edge, tile boundary, and crop edge.
        mem.put(TILE0, &[30, 20, 10, 255]);
        mem.put(TILE0 + 255 * 4, &[60, 50, 40, 255]);
        mem.put(TILE1, &[90, 80, 70, 255]);
        mem
    }

    /// Isolated fixture for the 2026-10-19 layout. It deliberately writes no
    /// legacy-preview session or canvas fields, so profile mixups fail closed.
    pub(crate) fn renovated_fixture() -> FakeMemory {
        let mut mem = FakeMemory::new();
        mem.ptr(BASE + 0x47a380, SENTINEL);
        mem.ptr(SENTINEL, CANVAS);
        mem.ptr(CANVAS, 0);

        mem.u32(CANVAS + 0x28, 257);
        mem.u32(CANVAS + 0x2c, 1);
        mem.ptr(CANVAS + 0x48, SLOT);
        mem.u32(CANVAS + 0x180, 2);
        mem.u32(CANVAS + 0x184, 1);
        mem.utf16(CANVAS + 0x8f8, "renovated synthetic");
        mem.utf16(CANVAS + 0xb00, "renovated.sai");

        mem.ptr(SLOT, MAP);
        mem.ptr(MAP + 8, TREE);
        mem.u32(MAP + 0x10, 257);
        mem.u32(MAP + 0x14, 1);
        mem.u32(MAP + 0x18, 2);
        mem.u32(MAP + 0x1c, 1);
        mem.ptr(TREE, ROW);
        mem.ptr(ROW, TILE0);
        mem.ptr(ROW + 8, TILE1);
        mem.put(TILE0, &[30, 20, 10, 255]);
        mem.put(TILE0 + 255 * 4, &[60, 50, 40, 255]);
        mem.put(TILE1, &[90, 80, 70, 255]);

        mem
    }

    #[test]
    fn composites_and_crops_two_bgra_tiles_and_skips_sentinel() {
        let mem = fixture();
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &PROFILE,
        };
        let canvases = reader.canvases().unwrap();
        assert_eq!(canvases.len(), 1);
        let canvas = &canvases[0];
        assert_eq!((canvas.id, canvas.width, canvas.height), (Some(42), 257, 1));
        assert_eq!(
            (&*canvas.name, &*canvas.short_path),
            ("synthetic", "synthetic.sai")
        );

        let rgb = reader.composite(canvas, false).unwrap();
        assert_eq!(rgb.len(), 257 * 3);
        assert_eq!(&rgb[0..3], &[10, 20, 30]);
        assert_eq!(&rgb[255 * 3..255 * 3 + 3], &[40, 50, 60]);
        assert_eq!(&rgb[256 * 3..], &[70, 80, 90]);
        assert_eq!(reader.composite(canvas, true).unwrap()[..3], [30, 20, 10]);
    }

    #[test]
    fn rejects_invalid_tile_count() {
        let mut mem = fixture();
        mem.u32(MAP + 0x18, 1);
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &PROFILE,
        };
        let canvas = reader.canvases().unwrap().remove(0);
        assert!(
            reader
                .composite(&canvas, false)
                .unwrap_err()
                .contains("invalid level-0 tile map")
        );
    }

    #[test]
    fn rejects_cyclic_canvas_list() {
        let mut mem = fixture();
        mem.ptr(CANVAS, CANVAS);
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &PROFILE,
        };
        assert!(
            reader
                .canvases()
                .unwrap_err()
                .contains("cycle in canvas list")
        );
    }

    #[test]
    fn renovated_profile_reads_and_composites_expected_pixels() {
        use crate::sai::renovated_2026_10_19::PROFILE as RENOVATED;

        let mem = renovated_fixture();
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &RENOVATED,
        };
        let canvas = reader.canvases().unwrap().remove(0);
        assert_eq!((canvas.id, canvas.width, canvas.height), (None, 257, 1));
        assert_eq!(
            (&*canvas.name, &*canvas.short_path),
            ("renovated synthetic", "renovated.sai")
        );
        assert_eq!(
            reader.composite(&canvas, false).unwrap(),
            fixture_reader_rgb()
        );
    }

    fn fixture_reader_rgb() -> Vec<u8> {
        let mem = fixture();
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &PROFILE,
        };
        let canvas = reader.canvases().unwrap().remove(0);
        reader.composite(&canvas, false).unwrap()
    }

    #[test]
    fn renovated_fixture_rejects_legacy_profile() {
        let mem = renovated_fixture();
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &PROFILE,
        };
        assert!(reader.canvases().unwrap_err().contains("null pointer"));
    }

    #[test]
    fn renovated_reader_rejects_canvas_map_grid_mismatch() {
        use crate::sai::renovated_2026_10_19::PROFILE as RENOVATED;

        let mut mem = renovated_fixture();
        mem.u32(CANVAS + 0x180, 1);
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &RENOVATED,
        };
        let canvas = reader.canvases().unwrap().remove(0);
        assert!(
            reader
                .read_tile_map(&canvas)
                .unwrap_err()
                .contains("invalid canvas tile grid")
        );
    }

    #[test]
    fn renovated_reader_detects_pointer_and_metadata_changes() {
        use crate::sai::renovated_2026_10_19::PROFILE as RENOVATED;

        let mut mem = renovated_fixture();
        let (_canvas, map) = {
            let reader = Reader {
                memory: &mem,
                base: BASE,
                profile: &RENOVATED,
            };
            let canvas = reader.canvases().unwrap().remove(0);
            let map = reader.read_tile_map(&canvas).unwrap();
            (canvas, map)
        };
        let mut tile = vec![0; 256 * 256 * 4];

        mem.ptr(SLOT, TILE0);
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &RENOVATED,
        };
        assert!(
            reader
                .read_tile_into(&map, 0, 0, &mut tile)
                .unwrap_err()
                .contains("tile map changed")
        );

        mem.ptr(SLOT, MAP);
        let (canvas, stable_map) = {
            let reader = Reader {
                memory: &mem,
                base: BASE,
                profile: &RENOVATED,
            };
            let canvas = reader.canvases().unwrap().remove(0);
            let map = reader.read_tile_map(&canvas).unwrap();
            (canvas, map)
        };
        mem.ptr(MAP + 8, ROW);
        let reader = Reader {
            memory: &mem,
            base: BASE,
            profile: &RENOVATED,
        };
        assert!(
            reader
                .validate_tile_map(&canvas, &stable_map)
                .unwrap_err()
                .contains("tile map changed")
        );
    }
    #[test]
    fn renovated_capture_rejects_invalidation_during_tile_read() {
        use std::cell::RefCell;
        struct MutatingMemory {
            memory: RefCell<FakeMemory>,
            changed_address: usize,
            replacement: Vec<u8>,
        }
        impl ProcessMemory for MutatingMemory {
            fn read_into(&self, address: usize, dst: &mut [u8]) -> Result<(), String> {
                let mut memory = self.memory.borrow_mut();
                memory.read_into(address, dst)?;
                if address == TILE0 {
                    memory.put(self.changed_address, &self.replacement);
                }
                Ok(())
            }
        }
        for (changed_address, replacement) in [
            (CANVAS + 0x28, 258u32.to_le_bytes().to_vec()),
            (CANVAS + 0x48, 0u64.to_le_bytes().to_vec()),
            (ROW, 0u64.to_le_bytes().to_vec()),
            (MAP, 1u64.to_le_bytes().to_vec()),
        ] {
            let memory = MutatingMemory {
                memory: RefCell::new(renovated_fixture()),
                changed_address,
                replacement,
            };
            let reader = Reader {
                memory: &memory,
                base: BASE,
                profile: &crate::sai::renovated_2026_10_19::PROFILE,
            };
            let canvas = reader.canvases().unwrap().remove(0);
            assert!(
                reader.composite(&canvas, false).is_err(),
                "mutation at {changed_address:#x}"
            );
        }
    }
}
