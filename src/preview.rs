//! Small RGB previews for live canvases and completed sessions.
use crate::{
    process::ProcessMemory,
    recorder::storage::Session,
    sai::{canvas::Canvas, reader::Reader},
};
use std::path::Path;

/// An owned RGB thumbnail. `rgb` is always bounded by `max_pixels` passed to
/// the constructor functions.
#[derive(Debug, Clone)]
pub struct PreviewImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

fn dimensions(
    width: u32,
    height: u32,
    max_edge: u32,
    max_pixels: u64,
) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 || max_edge == 0 || max_pixels == 0 {
        return Err("preview limits and dimensions must be positive".into());
    }
    let scale = (max_edge as f64 / width.max(height) as f64)
        .min(1.0)
        .min((max_pixels as f64 / (width as f64 * height as f64)).sqrt());
    let mut w = (width as f64 * scale).floor().max(1.0) as u32;
    let mut h = (height as f64 * scale).floor().max(1.0) as u32;
    // Rounding a very thin image up to one pixel must still honor the budget.
    if u64::from(w) * u64::from(h) > max_pixels {
        if w >= h {
            w = (max_pixels / u64::from(h)).max(1) as u32;
        } else {
            h = (max_pixels / u64::from(w)).max(1) as u32;
        }
    }
    Ok((w, h))
}

impl PreviewImage {
    /// Read a live canvas directly into a thumbnail without allocating a full
    /// size composite. Pixel sampling is nearest-neighbour and preserves aspect ratio.
    pub fn from_live<M: ProcessMemory>(
        reader: &Reader<M>,
        canvas: &Canvas,
        max_edge: u32,
        max_pixels: u64,
    ) -> Result<Self, String> {
        if reader.read_canvas(canvas.address)? != *canvas {
            return Err("canvas changed before preview; retry".into());
        }
        let map = reader.read_tile_map(canvas)?;
        let (width, height) = dimensions(canvas.width, canvas.height, max_edge, max_pixels)?;
        let len = width as usize * height as usize * 3;
        let mut rgb = vec![0; len];
        let mut tile = vec![0; 256 * 256 * 4];
        for ty in 0..map.ny {
            for tx in 0..map.nx {
                let x0 = tx * 256;
                let y0 = ty * 256;
                let x1 = (x0 + 256).min(canvas.width);
                let y1 = (y0 + 256).min(canvas.height);
                let ox0 = ceil_div(x0 as u64 * width as u64, canvas.width as u64) as u32;
                let ox1 = ceil_div(x1 as u64 * width as u64, canvas.width as u64) as u32;
                let oy0 = ceil_div(y0 as u64 * height as u64, canvas.height as u64) as u32;
                let oy1 = ceil_div(y1 as u64 * height as u64, canvas.height as u64) as u32;
                if ox0 >= ox1 || oy0 >= oy1 {
                    continue;
                }
                reader.read_tile_into(&map, tx, ty, &mut tile)?;
                for oy in oy0..oy1 {
                    let sy = ((oy as u64 * canvas.height as u64) / height as u64) as u32;
                    for ox in ox0..ox1 {
                        let sx = ((ox as u64 * canvas.width as u64) / width as u64) as u32;
                        let s = ((sy - y0) as usize * 256 + (sx - x0) as usize) * 4;
                        let d = ((oy as usize * width as usize) + ox as usize) * 3;
                        rgb[d..d + 3].copy_from_slice(&[tile[s + 2], tile[s + 1], tile[s]]);
                    }
                }
            }
        }
        reader.validate_tile_map(canvas, &map)?;
        if reader.read_canvas(canvas.address)? != *canvas || !reader.canvases()?.contains(canvas) {
            return Err("canvas changed during preview; retry".into());
        }
        Ok(Self { width, height, rgb })
    }

    /// Replay a session's final state and retain only a bounded thumbnail.
    pub fn from_session_final(dir: &Path, max_edge: u32, max_pixels: u64) -> Result<Self, String> {
        let mut session = Session::open(dir)?;
        let mut replay = crate::export::replay_final(&mut session)?;
        let (width, height) = dimensions(replay.width, replay.height, max_edge, max_pixels)?;
        if width == replay.width && height == replay.height {
            return Ok(Self {
                width,
                height,
                rgb: replay.rgb,
            });
        }
        let mut rgb = vec![0; width as usize * height as usize * 3];
        for y in 0..height {
            for x in 0..width {
                let sx = (x as u64 * replay.width as u64 / width as u64) as usize;
                let sy = (y as u64 * replay.height as u64 / height as u64) as usize;
                let s = (sy * replay.width as usize + sx) * 3;
                let d = (y as usize * width as usize + x as usize) * 3;
                rgb[d..d + 3].copy_from_slice(&replay.rgb[s..s + 3]);
            }
        }
        replay.rgb.clear();
        Ok(Self { width, height, rgb })
    }
}

fn ceil_div(n: u64, d: u64) -> u64 {
    n / d + u64::from(n % d != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sai::{
        builds::preview_2026_07_02b::PROFILE,
        reader::tests::{BASE, fixture},
    };

    #[test]
    fn live_preview_is_bounded_cropped_and_rgb_ordered() {
        let memory = fixture();
        let reader = Reader {
            memory: &memory,
            base: BASE,
            profile: &PROFILE,
            resolved_root_rva: None,
            source_hash: None,
        };
        let canvas = reader.canvases().unwrap().remove(0);
        let preview = PreviewImage::from_live(&reader, &canvas, 1, 1).unwrap();
        assert_eq!((preview.width, preview.height), (1, 1));
        assert_eq!(preview.rgb, [10, 20, 30]);
        assert_eq!(dimensions(100_000, 1, 512, 1).unwrap(), (1, 1));
    }
}
