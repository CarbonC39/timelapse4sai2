use std::collections::BTreeMap;

use crate::recorder::{
    TILE_BYTES,
    storage::{BlobId, Coord, Event, Transform},
};

const TILE: u32 = 256;
const MAX_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_EDGE: u32 = 100_000;

fn zeroed(len: usize) -> Result<Vec<u8>, String> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|e| format!("cannot allocate frame buffer: {e}"))?;
    v.resize(len, 0);
    Ok(v)
}

#[derive(Debug, Clone)]
pub struct Replay {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    active: BTreeMap<Coord, BlobId>,
}

impl Replay {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let len = frame_len(width, height)?;
        Ok(Self {
            width,
            height,
            rgb: zeroed(len)?,
            active: BTreeMap::new(),
        })
    }

    pub fn apply(
        &mut self,
        event: &Event,
        load: &mut impl FnMut(BlobId) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        match event {
            Event::Frame { changed_tiles, .. } => {
                let mut seen = BTreeMap::new();
                for &(coord, id) in changed_tiles {
                    self.validate_coord(coord)?;
                    if seen.insert(coord, id).is_some() {
                        return Err("duplicate tile coordinate in frame".into());
                    }
                }
                for (coord, id) in seen {
                    self.patch_tile(coord, id, load)?;
                    self.active.insert(coord, id);
                }
            }
            Event::Checkpoint {
                width,
                height,
                nx,
                ny,
                tiles,
            } => {
                let count = dims_tiles(*width, *height)?;
                if (*nx, *ny) != count {
                    return Err("checkpoint tile grid dimensions do not match canvas".into());
                }
                let mut grid = BTreeMap::new();
                for &(coord, id) in tiles {
                    if coord.0 >= *nx || coord.1 >= *ny {
                        return Err("checkpoint tile coordinate outside grid".into());
                    }
                    if grid.insert(coord, id).is_some() {
                        return Err("duplicate tile coordinate in checkpoint".into());
                    }
                }
                if grid.len()
                    != (*nx as usize)
                        .checked_mul(*ny as usize)
                        .ok_or("checkpoint grid overflow")?
                {
                    return Err("checkpoint does not contain the full tile grid".into());
                }
                if (self.width, self.height) != (*width, *height) {
                    self.resize_centered(*width, *height)?;
                }
                let changed: Vec<_> = grid
                    .iter()
                    .filter(|(c, id)| self.active.get(c) != Some(*id))
                    .map(|(c, id)| (*c, *id))
                    .collect();
                for (coord, id) in changed {
                    self.patch_tile(coord, id, load)?;
                }
                self.active = grid;
            }
            Event::CanvasResize {
                old_width,
                old_height,
                new_width,
                new_height,
                origin,
                ..
            } => {
                if (self.width, self.height) != (*old_width, *old_height) {
                    return Err("resize old dimensions do not match replay canvas".into());
                }
                let mut next = zeroed(frame_len(*new_width, *new_height)?)?;
                let (ox, oy) = origin
                    .map(|(x, y)| (x as i64, y as i64))
                    .unwrap_or_else(|| {
                        (
                            (*new_width as i64 - *old_width as i64) / 2,
                            (*new_height as i64 - *old_height as i64) / 2,
                        )
                    });
                copy_offset(
                    &self.rgb,
                    self.width,
                    self.height,
                    &mut next,
                    *new_width,
                    *new_height,
                    ox,
                    oy,
                );
                self.rgb = next;
                self.width = *new_width;
                self.height = *new_height;
                self.active.clear();
            }
            Event::Transform { transform, .. } => self.transform(transform)?,
        }
        Ok(())
    }

    fn validate_coord(&self, (x, y): Coord) -> Result<(), String> {
        let (nx, ny) = dims_tiles(self.width, self.height)?;
        if x >= nx || y >= ny {
            return Err("tile coordinate outside replay canvas".into());
        }
        Ok(())
    }

    fn patch_tile(
        &mut self,
        (tx, ty): Coord,
        id: BlobId,
        load: &mut impl FnMut(BlobId) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        let bytes = load(id)?;
        if bytes.len() != TILE_BYTES {
            return Err(format!("tile blob {id} has invalid length"));
        }
        let x0 = tx * TILE;
        let y0 = ty * TILE;
        let w = TILE.min(self.width - x0);
        let h = TILE.min(self.height - y0);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let src = (y * TILE as usize + x) * 4;
                let dst = ((y0 as usize + y) * self.width as usize + x0 as usize + x) * 3;
                self.rgb[dst..dst + 3].copy_from_slice(&[
                    bytes[src + 2],
                    bytes[src + 1],
                    bytes[src],
                ]);
            }
        }
        Ok(())
    }

    fn resize_centered(&mut self, width: u32, height: u32) -> Result<(), String> {
        let mut next = zeroed(frame_len(width, height)?)?;
        copy_offset(
            &self.rgb,
            self.width,
            self.height,
            &mut next,
            width,
            height,
            (width as i64 - self.width as i64) / 2,
            (height as i64 - self.height as i64) / 2,
        );
        self.rgb = next;
        self.width = width;
        self.height = height;
        self.active.clear();
        Ok(())
    }

    fn transform(&mut self, transform: &Transform) -> Result<(), String> {
        let (nw, nh) = match transform {
            Transform::Rotate90CW | Transform::Rotate90CCW => (self.height, self.width),
            _ => (self.width, self.height),
        };
        let mut out = zeroed(frame_len(nw, nh)?)?;
        for y in 0..self.height {
            for x in 0..self.width {
                let (dx, dy) = match transform {
                    Transform::Rotate90CW => (self.height - 1 - y, x),
                    Transform::Rotate90CCW => (y, self.width - 1 - x),
                    Transform::Rotate180 => (self.width - 1 - x, self.height - 1 - y),
                    Transform::FlipHorizontal => (self.width - 1 - x, y),
                    Transform::FlipVertical => (x, self.height - 1 - y),
                };
                let s = ((y as usize * self.width as usize) + x as usize) * 3;
                let d = ((dy as usize * nw as usize) + dx as usize) * 3;
                out[d..d + 3].copy_from_slice(&self.rgb[s..s + 3]);
            }
        }
        self.rgb = out;
        self.width = nw;
        self.height = nh;
        self.active.clear();
        Ok(())
    }
}

fn frame_len(w: u32, h: u32) -> Result<usize, String> {
    if w == 0 || h == 0 {
        return Err("canvas dimensions must be nonzero".into());
    }
    if w > MAX_EDGE || h > MAX_EDGE {
        return Err("canvas edge exceeds 100000 pixels".into());
    }
    let n = (w as u64)
        .checked_mul(h as u64)
        .and_then(|v| v.checked_mul(3))
        .ok_or("canvas size overflow")?;
    if n > MAX_BYTES {
        return Err("canvas exceeds 1 GiB limit".into());
    }
    usize::try_from(n).map_err(|_| "canvas too large".into())
}
fn dims_tiles(w: u32, h: u32) -> Result<(u32, u32), String> {
    frame_len(w, h)?;
    Ok((w.div_ceil(TILE), h.div_ceil(TILE)))
}
fn copy_offset(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], dw: u32, dh: u32, ox: i64, oy: i64) {
    for sy in 0..sh {
        let dy = sy as i64 + oy;
        if !(0..dh as i64).contains(&dy) {
            continue;
        }
        let sx0 = 0.max(-ox) as u32;
        let sx1 = sw.min((dw as i64 - ox).max(0) as u32);
        if sx0 >= sx1 {
            continue;
        }
        let a = ((sy as usize * sw as usize) + sx0 as usize) * 3;
        let b = ((dy as usize * dw as usize) + (sx0 as i64 + ox) as usize) * 3;
        let n = (sx1 - sx0) as usize * 3;
        dst[b..b + n].copy_from_slice(&src[a..a + n]);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasMode {
    Max,
    Initial,
}
impl CanvasMode {
    pub fn parse(v: &str) -> Result<Self, String> {
        match v.to_ascii_lowercase().as_str() {
            "max" => Ok(Self::Max),
            "initial" => Ok(Self::Initial),
            _ => Err(format!(
                "unknown canvas mode '{v}' (expected max or initial)"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Auto,
    P720,
    P1080,
    P1440,
    P2160,
    /// Percentage of the logical canvas's longest edge, before the encoder cap.
    Percent(u32),
}
impl Size {
    pub fn parse(v: &str) -> Result<Self, String> {
        let lower = v.to_ascii_lowercase();
        if let Some(percent) = lower.strip_suffix('%') {
            let percent = percent
                .parse::<u32>()
                .ok()
                .filter(|percent| (1..=400).contains(percent))
                .ok_or_else(|| format!("invalid output size percentage '{v}'"))?;
            return Ok(Self::Percent(percent));
        }
        match lower.as_str() {
            "auto" => Ok(Self::Auto),
            "720" | "720p" => Ok(Self::P720),
            "1080" | "1080p" => Ok(Self::P1080),
            "1440" | "1440p" => Ok(Self::P1440),
            "2160" | "2160p" => Ok(Self::P2160),
            _ => Err(format!(
                "unknown output size '{v}' (expected auto, 720, 1080, 1440, 2160, or N%)"
            )),
        }
    }
    fn edge(self, logical_max: u32) -> Option<u32> {
        match self {
            Self::Auto => None,
            Self::P720 => Some(720),
            Self::P1080 => Some(1080),
            Self::P1440 => Some(1440),
            Self::P2160 => Some(2160),
            Self::Percent(percent) => {
                let edge = logical_max as u64 * percent as u64;
                Some((edge.div_ceil(100)).min(u32::MAX as u64) as u32)
            }
        }
    }
}

pub struct Spatial {
    pub width: u32,
    pub height: u32,
    logical_width: u32,
    logical_height: u32,
    content_width: u32,
    content_height: u32,
}
impl Spatial {
    pub fn new(
        logical_width: u32,
        logical_height: u32,
        size: Size,
        max_dimension: u32,
    ) -> Result<Self, String> {
        // This virtual canvas is sampled directly, never allocated. Its maximum
        // width and height may come from different historical states.
        if logical_width == 0
            || logical_height == 0
            || logical_width > 100_000
            || logical_height > 100_000
        {
            return Err("invalid logical canvas dimensions".into());
        }
        if max_dimension < 2 {
            return Err("maximum encoder dimension must be at least 2".into());
        }
        let logical_max = logical_width.max(logical_height);
        let cap = size
            .edge(logical_max)
            .unwrap_or(max_dimension)
            .min(max_dimension)
            .max(2)
            & !1;
        let scale = (cap as f64 / logical_max as f64)
            .min(if size == Size::Auto { 1.0 } else { f64::MAX });
        let w = ((logical_width as f64 * scale).round() as u32).max(1);
        let h = ((logical_height as f64 * scale).round() as u32).max(1);
        let width = w + (w & 1);
        let height = h + (h & 1);
        frame_len(width, height)?;
        Ok(Self {
            width,
            height,
            logical_width,
            logical_height,
            content_width: w,
            content_height: h,
        })
    }

    pub fn render(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
        let expected = frame_len(width, height)?;
        if rgb.len() != expected {
            return Err("RGB input length does not match dimensions".into());
        }
        let scale_x = self.content_width as f64 / self.logical_width as f64;
        let scale_y = self.content_height as f64 / self.logical_height as f64;
        let outw = self.content_width;
        let outh = self.content_height;
        let offset_x = (self.logical_width as i64 - width as i64) / 2;
        let offset_y = (self.logical_height as i64 - height as i64) / 2;
        let mut out = zeroed(frame_len(self.width, self.height)?)?;
        out.fill(128);
        for oy in 0..outh.min(self.height) {
            for ox in 0..outw.min(self.width) {
                let lx = ((ox as f64 + 0.5) / scale_x - 0.5)
                    .clamp(0.0, self.logical_width.saturating_sub(1) as f64);
                let ly = ((oy as f64 + 0.5) / scale_y - 0.5)
                    .clamp(0.0, self.logical_height.saturating_sub(1) as f64);
                let x0 = lx.floor() as i64;
                let y0 = ly.floor() as i64;
                let x1 = x0 + 1;
                let y1 = y0 + 1;
                let fx = lx - x0 as f64;
                let fy = ly - y0 as f64;
                let d = ((oy as usize * self.width as usize) + ox as usize) * 3;
                for c in 0..3 {
                    let at = |x: i64, y: i64| -> f64 {
                        if x < 0
                            || y < 0
                            || x >= self.logical_width as i64
                            || y >= self.logical_height as i64
                        {
                            return 128.0;
                        }
                        let sx = x - offset_x;
                        let sy = y - offset_y;
                        if sx < 0 || sy < 0 || sx >= width as i64 || sy >= height as i64 {
                            128.0
                        } else {
                            rgb[((sy as usize * width as usize) + sx as usize) * 3 + c] as f64
                        }
                    };
                    out[d + c] = (at(x0, y0) * (1.0 - fx) * (1.0 - fy)
                        + at(x1, y0) * fx * (1.0 - fy)
                        + at(x0, y1) * (1.0 - fx) * fy
                        + at(x1, y1) * fx * fy)
                        .round() as u8;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::FrameKind;

    fn tile(pixels: &[[u8; 3]]) -> Vec<u8> {
        let mut data = vec![0; TILE_BYTES];
        for (i, rgb) in pixels.iter().enumerate() {
            let p = i * 4;
            data[p..p + 4].copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
        }
        data
    }
    fn tile_grid(width: usize, pixels: &[[u8; 3]]) -> Vec<u8> {
        let mut data = vec![0; TILE_BYTES];
        for (i, rgb) in pixels.iter().enumerate() {
            let p = ((i / width) * TILE as usize + i % width) * 4;
            data[p..p + 4].copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
        }
        data
    }
    fn frame(tiles: Vec<(Coord, BlobId)>) -> Event {
        Event::Frame {
            kind: FrameKind::Stable,
            timestamp_ms: 0,
            changed_tiles: tiles,
        }
    }
    fn checkpoint(width: u32, height: u32, tiles: Vec<(Coord, BlobId)>) -> Event {
        Event::Checkpoint {
            width,
            height,
            nx: width.div_ceil(TILE),
            ny: height.div_ceil(TILE),
            tiles,
        }
    }
    fn apply_tile(replay: &mut Replay, id: BlobId, blob: &[u8]) {
        replay
            .apply(&frame(vec![((0, 0), id)]), &mut |_| Ok(blob.to_vec()))
            .unwrap();
    }

    #[test]
    fn spatial_centers_with_gray_and_crops() {
        let red = [200, 0, 0];
        let input = vec![
            red[0], red[1], red[2], red[0], red[1], red[2], red[0], red[1], red[2], red[0], red[1],
            red[2],
        ];
        let centered = Spatial::new(4, 2, Size::Auto, 10)
            .unwrap()
            .render(&input, 2, 2)
            .unwrap();
        assert_eq!(
            &centered[0..12],
            &[128, 128, 128, 200, 0, 0, 200, 0, 0, 128, 128, 128]
        );
        let wide: Vec<u8> = (0..4)
            .flat_map(|x| [x * 20, 0, 0])
            .chain((0..4).flat_map(|x| [x * 20, 0, 0]))
            .collect();
        let cropped = Spatial::new(2, 2, Size::Auto, 10)
            .unwrap()
            .render(&wide, 4, 2)
            .unwrap();
        assert_eq!(cropped[0], 20);
        assert_eq!(cropped[3], 40);
    }

    #[test]
    fn spatial_pads_only_trailing_odd_pixel() {
        let spatial = Spatial::new(3, 2, Size::Auto, 10).unwrap();
        assert_eq!((spatial.width, spatial.height), (4, 2));
        let input = vec![7; 3 * 2 * 3];
        let result = spatial.render(&input, 3, 2).unwrap();
        assert_eq!(&result[0..9], &[7; 9]);
        assert_eq!(&result[9..12], &[128; 3]);
    }

    #[test]
    fn spatial_percent_scales_from_logical_canvas() {
        let half = Spatial::new(400, 200, Size::Percent(50), 4096).unwrap();
        assert_eq!((half.width, half.height), (200, 100));
        let capped = Spatial::new(4000, 2000, Size::Percent(50), 1000).unwrap();
        assert_eq!((capped.width, capped.height), (1000, 500));
    }

    #[test]
    fn spatial_upscale_clamps_at_logical_edges() {
        let spatial = Spatial::new(2, 2, Size::P1080, 4).unwrap();
        let rendered = spatial.render(&vec![40; 12], 2, 2).unwrap();
        assert_eq!((spatial.width, spatial.height), (4, 4));
        assert_eq!(&rendered[0..3], &[40; 3]);
        assert_eq!(&rendered[45..48], &[40; 3]);
    }

    #[test]
    fn resize_origin_places_old_canvas_at_offset() {
        let mut r = Replay::new(2, 1).unwrap();
        apply_tile(&mut r, 1, &tile(&[[10, 0, 0], [20, 0, 0]]));
        let e = Event::CanvasResize {
            kind: FrameKind::Stable,
            timestamp_ms: 1,
            old_width: 2,
            old_height: 1,
            new_width: 3,
            new_height: 1,
            origin: Some((1, 0)),
        };
        r.apply(&e, &mut |_| unreachable!()).unwrap();
        assert_eq!(r.rgb, [0, 0, 0, 10, 0, 0, 20, 0, 0]);
    }

    #[test]
    fn transforms_have_expected_pixels_and_clear_tile_identity() {
        let transforms = [
            (
                Transform::Rotate90CW,
                3,
                2,
                vec![5, 0, 0, 3, 0, 0, 1, 0, 0, 6, 0, 0, 4, 0, 0, 2, 0, 0],
            ),
            (
                Transform::Rotate90CCW,
                3,
                2,
                vec![2, 0, 0, 4, 0, 0, 6, 0, 0, 1, 0, 0, 3, 0, 0, 5, 0, 0],
            ),
            (
                Transform::Rotate180,
                2,
                3,
                vec![6, 0, 0, 5, 0, 0, 4, 0, 0, 3, 0, 0, 2, 0, 0, 1, 0, 0],
            ),
            (
                Transform::FlipHorizontal,
                2,
                3,
                vec![2, 0, 0, 1, 0, 0, 4, 0, 0, 3, 0, 0, 6, 0, 0, 5, 0, 0],
            ),
            (
                Transform::FlipVertical,
                2,
                3,
                vec![5, 0, 0, 6, 0, 0, 3, 0, 0, 4, 0, 0, 1, 0, 0, 2, 0, 0],
            ),
        ];
        for (transform, w, h, expected) in transforms {
            let mut r = Replay::new(2, 3).unwrap();
            apply_tile(
                &mut r,
                1,
                &tile_grid(
                    2,
                    &[
                        [1, 0, 0],
                        [2, 0, 0],
                        [3, 0, 0],
                        [4, 0, 0],
                        [5, 0, 0],
                        [6, 0, 0],
                    ],
                ),
            );
            r.apply(
                &Event::Transform {
                    timestamp_ms: 1,
                    transform,
                },
                &mut |_| unreachable!(),
            )
            .unwrap();
            assert_eq!((r.width, r.height), (w, h));
            assert_eq!(r.rgb, expected);
            apply_tile(&mut r, 2, &tile(&[[9, 0, 0]]));
            assert_eq!(&r.rgb[0..3], &[9, 0, 0]);
        }
    }

    #[test]
    fn checkpoint_skips_unchanged_blobs_but_validates_grid() {
        let mut r = Replay::new(2, 1).unwrap();
        let blob = tile(&[[12, 0, 0], [13, 0, 0]]);
        let mut calls = 0;
        let event = checkpoint(2, 1, vec![((0, 0), 7)]);
        r.apply(&event, &mut |_| {
            calls += 1;
            Ok(blob.clone())
        })
        .unwrap();
        r.apply(&event, &mut |_| {
            calls += 1;
            Ok(blob.clone())
        })
        .unwrap();
        assert_eq!(calls, 1);
        let invalid = Event::Checkpoint {
            width: 2,
            height: 1,
            nx: 1,
            ny: 1,
            tiles: vec![],
        };
        assert!(r.apply(&invalid, &mut |_| unreachable!()).is_err());
    }
}
