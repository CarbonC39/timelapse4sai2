use std::collections::HashSet;
use crate::process::ProcessMemory;
use super::{canvas::Canvas, preview_2026_07_02b::*};

pub struct Reader<'a, M: ProcessMemory> { pub memory: &'a M, pub base: usize }
impl<M: ProcessMemory> Reader<'_, M> {
    fn bytes<const N: usize>(&self, address: usize) -> Result<[u8; N], String> {
        let mut bytes = [0; N]; self.memory.read_into(address, &mut bytes)?; Ok(bytes)
    }
    fn at(address: usize, offset: usize) -> Result<usize, String> {
        address.checked_add(offset).ok_or_else(|| "address overflow".into())
    }
    fn ptr(&self, address: usize) -> Result<usize, String> {
        let p = u64::from_le_bytes(self.bytes(address)?);
        if p > 0x0000_7fff_ffff_ffff { return Err(format!("invalid pointer {p:#x} at {address:#x}")); }
        Ok(p as usize)
    }
    fn required_ptr(&self, address: usize) -> Result<usize, String> {
        let p = self.ptr(address)?;
        if p == 0 { Err(format!("null pointer at {address:#x}")) } else { Ok(p) }
    }
    fn u32(&self, address: usize) -> Result<u32, String> { Ok(u32::from_le_bytes(self.bytes(address)?)) }
    fn string(&self, address: usize) -> Result<String, String> {
        let bytes = self.bytes::<512>(address)?;
        let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        let end = units.iter().position(|&u| u == 0).ok_or("unterminated UTF-16 string")?;
        String::from_utf16(&units[..end]).map_err(|e| format!("invalid UTF-16: {e}"))
    }
    fn canvas(&self, p: usize) -> Result<Canvas, String> {
        let width = self.u32(Self::at(p, 0x250)?)?;
        let height = self.u32(Self::at(p, 0x254)?)?;
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(format!("invalid canvas dimensions {width}x{height} at {p:#x}"));
        }
        Ok(Canvas { address: p, id: self.u32(Self::at(p, 0x20)?)?, width, height,
            name: self.string(Self::at(p, 0x2ac)?)?, short_path: self.string(Self::at(p, 0x4ac)?)? })
    }
    pub fn canvases(&self) -> Result<Vec<Canvas>, String> {
        // Original get_canvas_list(): read the sentinel pointer, collect, discard sentinel.
        let sentinel = self.required_ptr(Self::at(self.base, SESSION_OFFSET)?)?;
        let mut p = self.ptr(sentinel)?;
        let mut seen = HashSet::from([sentinel]);
        let mut result = Vec::new();
        while p != 0 {
            if !seen.insert(p) { return Err(format!("cycle in canvas list at {p:#x}")); }
            if result.len() >= 1024 { return Err("canvas list exceeds 1024 entries".into()); }
            result.push(self.canvas(p)?);
            p = self.ptr(p)?;
        }
        Ok(result)
    }
    pub fn composite(&self, canvas: &Canvas, rgba_order: bool) -> Result<Vec<u8>, String> {
        if self.canvas(canvas.address)? != *canvas { return Err("canvas changed before capture; retry".into()); }
        let map_slot = self.required_ptr(Self::at(canvas.address, 0x28)?)?;
        let map = self.required_ptr(map_slot)?;
        let metadata = self.bytes::<32>(map)?;
        let tree = self.required_ptr(Self::at(map, 8)?)?;
        let width = self.u32(Self::at(map, 0x10)?)?;
        let height = self.u32(Self::at(map, 0x14)?)?;
        let nx = self.u32(Self::at(map, 0x18)?)?;
        let ny = self.u32(Self::at(map, 0x1c)?)?;
        if (width, height) != (canvas.width, canvas.height) || nx != width.div_ceil(256) || ny != height.div_ceil(256) {
            return Err(format!("invalid level-0 tile map: {width}x{height}, tiles {nx}x{ny}"));
        }
        let len = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(3)).filter(|&n| n <= MAX_RGB_BYTES).ok_or("capture exceeds 1 GiB RGB limit")?;
        let mut rgb = Vec::new(); rgb.try_reserve_exact(len).map_err(|e| e.to_string())?; rgb.resize(len, 0);
        let mut tile = vec![0; 256*256*4];
        for y in 0..ny as usize {
            let row = self.required_ptr(Self::at(tree, y*8)?)?;
            for x in 0..nx as usize {
                let tile_address = self.required_ptr(Self::at(row, x*8)?)?;
                self.memory.read_into(tile_address, &mut tile)?;
                for dy in 0..256.min(height as usize-y*256) {
                    for dx in 0..256.min(width as usize-x*256) {
                        let src = (dy*256+dx)*4;
                        let dst = ((y*256+dy)*width as usize+x*256+dx)*3;
                        rgb[dst..dst+3].copy_from_slice(&[tile[src+if rgba_order {0} else {2}], tile[src+1], tile[src+if rgba_order {2} else {0}]]);
                    }
                }
                if self.ptr(Self::at(row, x*8)?)? != tile_address { return Err("tile changed during capture; retry".into()); }
            }
            if self.ptr(Self::at(tree, y*8)?)? != row { return Err("tile row changed during capture; retry".into()); }
        }
        if self.bytes::<32>(map)? != metadata || self.required_ptr(map_slot)? != map || self.canvas(canvas.address)? != *canvas || !self.canvases()?.contains(canvas) {
            return Err("canvas changed during capture; retry".into());
        }
        Ok(rgb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: usize = 0x100;
    const SENTINEL: usize = 0x1000;
    const CANVAS: usize = 0x2000;
    const SLOT: usize = 0x3000;
    const MAP: usize = 0x4000;
    const TREE: usize = 0x5000;
    const ROW: usize = 0x6000;
    const TILE0: usize = 0x7000;
    const TILE1: usize = 0x47000;

    struct FakeMemory(Vec<u8>);

    impl FakeMemory {
        fn new() -> Self {
            Self(vec![0; 4 * 1024 * 1024])
        }

        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.0[address..address + bytes.len()].copy_from_slice(bytes);
        }

        fn u32(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }

        fn ptr(&mut self, address: usize, value: usize) {
            self.put(address, &(value as u64).to_le_bytes());
        }

        fn utf16(&mut self, address: usize, value: &str) {
            for (i, unit) in value.encode_utf16().enumerate() {
                self.put(address + i * 2, &unit.to_le_bytes());
            }
        }
    }

    impl ProcessMemory for FakeMemory {
        fn read_into(&self, address: usize, dst: &mut [u8]) -> Result<(), String> {
            let end = address.checked_add(dst.len()).ok_or("address overflow")?;
            let src = self.0.get(address..end).ok_or_else(|| format!("out of range read at {address:#x}"))?;
            dst.copy_from_slice(src);
            Ok(())
        }
    }

    fn fixture() -> FakeMemory {
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

    #[test]
    fn composites_and_crops_two_bgra_tiles_and_skips_sentinel() {
        let mem = fixture();
        let reader = Reader { memory: &mem, base: BASE };
        let canvases = reader.canvases().unwrap();
        assert_eq!(canvases.len(), 1);
        let canvas = &canvases[0];
        assert_eq!((canvas.id, canvas.width, canvas.height), (42, 257, 1));
        assert_eq!((&*canvas.name, &*canvas.short_path), ("synthetic", "synthetic.sai"));

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
        let reader = Reader { memory: &mem, base: BASE };
        let canvas = reader.canvases().unwrap().remove(0);
        assert!(reader.composite(&canvas, false).unwrap_err().contains("invalid level-0 tile map"));
    }

    #[test]
    fn rejects_cyclic_canvas_list() {
        let mut mem = fixture();
        mem.ptr(CANVAS, CANVAS);
        let reader = Reader { memory: &mem, base: BASE };
        assert!(reader.canvases().unwrap_err().contains("cycle in canvas list"));
    }
}
