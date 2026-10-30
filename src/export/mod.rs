//! Offline exporter for ordered visual states; wall-clock timestamps are metadata.
mod ffmpeg;
mod frame_map;
mod replay;

use crate::recorder::storage::{Event, Session, Transform};
use ffmpeg::{Encoder, Quality, VideoWriter};
use frame_map::{Plan, Target};
use replay::{CanvasMode, Replay, Size, Spatial};
use std::{
    collections::BTreeSet,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

pub const HELP: &str = "timelapse4sai2 export SESSION_DIR OUTPUT.mp4 [OPTIONS]
  --fps 30                    Output FPS (1..240); does not change drawing speed
  --speed 1.0                 Drawing playback multiplier; 1x = 30 visual states/s
  --encoder auto|nvenc|x264    Default auto; probes NVENC, falls back to libx264
  --quality draft|balanced|high|archival  Default balanced
  --size auto|1080|1440|2160   Longest edge in pixels; auto keeps natural size,
                              capped to encoder limits; aspect ratio preserved
  --canvas-mode max|initial   Default max: center on historical maximum canvas;
                              initial: center-crop to initial canvas. No scaling
                              during placement; unknown resize origin is centered
  --hold-end 3                Final artwork hold in seconds (0 disables)
  --preview-end 0             Final artwork preview before drawing, in seconds
  --skip-provisional          Only stable states (default includes provisional)
  --overwrite                 Replace existing output only after encoding succeeds
Timestamps and idle time are ignored. Durations round to the nearest output frame.
Output: H.264 MP4 / yuv420p; odd dimensions pad 1px on right/bottom.";

#[derive(Debug)]
pub struct Options {
    fps: u32,
    speed: f64,
    encoder: Encoder,
    quality: Quality,
    size: Size,
    canvas_mode: CanvasMode,
    hold_end: f64,
    preview_end: f64,
    skip_provisional: bool,
    overwrite: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            fps: 30,
            speed: 1.0,
            encoder: Encoder::Auto,
            quality: Quality::Balanced,
            size: Size::Auto,
            canvas_mode: CanvasMode::Max,
            hold_end: 3.0,
            preview_end: 0.0,
            skip_provisional: false,
            overwrite: false,
        }
    }
}
impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut o = Self::default();
        let mut i = 0;
        while i < args.len() {
            let flag = args[i].as_str();
            match flag {
                "--skip-provisional" => {
                    o.skip_provisional = true;
                    i += 1;
                    continue;
                }
                "--overwrite" => {
                    o.overwrite = true;
                    i += 1;
                    continue;
                }
                _ => (),
            }
            if !matches!(
                flag,
                "--fps"
                    | "--speed"
                    | "--encoder"
                    | "--quality"
                    | "--size"
                    | "--canvas-mode"
                    | "--hold-end"
                    | "--preview-end"
            ) {
                return Err(format!("unknown export option {flag}"));
            }
            let v = args
                .get(i + 1)
                .ok_or_else(|| format!("missing value for {flag}"))?;
            macro_rules! number {
                () => {
                    v.parse()
                        .map_err(|_| format!("invalid value for {flag}: {v}"))?
                };
            }
            match flag {
                "--fps" => o.fps = number!(),
                "--speed" => o.speed = number!(),
                "--hold-end" => o.hold_end = number!(),
                "--preview-end" => o.preview_end = number!(),
                "--encoder" => o.encoder = Encoder::parse(v)?,
                "--quality" => o.quality = Quality::parse(v)?,
                "--size" => o.size = Size::parse(v)?,
                "--canvas-mode" => o.canvas_mode = CanvasMode::parse(v)?,
                _ => unreachable!(),
            }
            i += 2;
        }
        Plan::new(vec![0], o.fps, o.speed, o.hold_end, o.preview_end)?;
        Ok(o)
    }
}

struct Scan {
    states: Vec<usize>,
    max_width: u32,
    max_height: u32,
    // Last checkpoint's record offset, event index, and canvas dimensions.
    checkpoint: Option<(u64, usize, u32, u32)>,
    batches: usize,
}
fn dimensions(w: u32, h: u32) -> Result<(), String> {
    if w == 0
        || h == 0
        || w > 100_000
        || h > 100_000
        || w as u64 * h as u64 * 3 > 1024 * 1024 * 1024
    {
        return Err(format!("invalid or oversized session canvas: {w}x{h}"));
    }
    Ok(())
}
fn scan(session: &mut Session, skip_provisional: bool) -> Result<Scan, String> {
    let (mut w, mut h) = (
        session.manifest.initial_width,
        session.manifest.initial_height,
    );
    dimensions(w, h)?;
    let mut result = Scan {
        states: vec![],
        max_width: w,
        max_height: h,
        checkpoint: None,
        batches: 0,
    };
    let mut grid = BTreeSet::new();
    let mut transformed = false;
    session.seek(0)?;
    loop {
        let offset = session.position()?;
        let Some(events) = session.next_batch()? else {
            break;
        };
        for (index, event) in events.iter().enumerate() {
            match event {
                Event::Frame { changed_tiles, .. } => {
                    let mut seen = BTreeSet::new();
                    for &(coord, _) in changed_tiles {
                        if coord.0 >= w.div_ceil(256)
                            || coord.1 >= h.div_ceil(256)
                            || !seen.insert(coord)
                        {
                            return Err("invalid or duplicate frame tile coordinate".into());
                        }
                        grid.insert(coord);
                    }
                }
                Event::Checkpoint {
                    width,
                    height,
                    nx,
                    ny,
                    tiles,
                } => {
                    dimensions(*width, *height)?;
                    if (*width, *height) != (w, h)
                        || (*nx, *ny) != (w.div_ceil(256), h.div_ceil(256))
                    {
                        return Err("checkpoint dimensions do not match session state".into());
                    }
                    grid.clear();
                    for &(coord, _) in tiles {
                        if coord.0 >= *nx || coord.1 >= *ny || !grid.insert(coord) {
                            return Err("invalid or duplicate checkpoint tile coordinate".into());
                        }
                    }
                    if grid.len() != *nx as usize * *ny as usize {
                        return Err("incomplete checkpoint grid".into());
                    }
                    transformed = false;
                    result.checkpoint = Some((offset, index, w, h));
                }
                Event::CanvasResize {
                    old_width,
                    old_height,
                    new_width,
                    new_height,
                    ..
                } => {
                    if (w, h) != (*old_width, *old_height) {
                        return Err("resize old dimensions do not match canvas".into());
                    }
                    (w, h) = (*new_width, *new_height);
                    grid.clear();
                    transformed = false;
                }
                Event::Transform { transform, .. } => {
                    if !transformed
                        && grid.len() != w.div_ceil(256) as usize * h.div_ceil(256) as usize
                    {
                        return Err("transform requires a complete existing canvas".into());
                    }
                    if matches!(transform, Transform::Rotate90CW | Transform::Rotate90CCW) {
                        (w, h) = (h, w);
                    }
                    grid.clear();
                    transformed = true;
                }
            }
            dimensions(w, h)?;
            result.max_width = result.max_width.max(w);
            result.max_height = result.max_height.max(h);
        }
        if !transformed && grid.len() != w.div_ceil(256) as usize * h.div_ceil(256) as usize {
            return Err("session visual state has an incomplete tile grid".into());
        }
        if frame_map::batch_kind(&events)
            .is_some_and(|kind| frame_map::include(kind, skip_provisional))
        {
            result.states.push(result.batches);
        }
        result.batches += 1;
    }
    if result.batches == 0 {
        return Err("session contains no captured canvas".into());
    }
    Ok(result)
}
fn apply_batch(replay: &mut Replay, events: &[Event], session: &mut Session) -> Result<(), String> {
    for event in events {
        replay.apply(event, &mut |id| session.load_blob(id))?;
    }
    Ok(())
}
fn final_replay(session: &mut Session, scan: &Scan) -> Result<Replay, String> {
    let mut replay;
    if let Some((offset, index, width, height)) = scan.checkpoint {
        session.seek(offset)?;
        replay = Replay::new(width, height)?;
        let events = session.next_batch()?.ok_or("missing final checkpoint")?;
        apply_batch(&mut replay, &events[index..], session)?;
    } else {
        session.seek(0)?;
        replay = Replay::new(
            session.manifest.initial_width,
            session.manifest.initial_height,
        )?;
    }
    while let Some(events) = session.next_batch()? {
        apply_batch(&mut replay, &events, session)?;
    }
    Ok(replay)
}

pub fn dump_final(dir: &Path, output: &Path) -> Result<(), String> {
    let mut session = Session::open(dir)?;
    let scan = scan(&mut session, false)?;
    let replay = final_replay(&mut session, &scan)?;
    crate::image::write_png(output, replay.width, replay.height, &replay.rgb)?;
    println!(
        "Wrote final {}x{} canvas to {}",
        replay.width,
        replay.height,
        output.display()
    );
    Ok(())
}

pub fn run(dir: &Path, output: &Path, options: Options) -> Result<(), String> {
    if !output
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("mp4"))
    {
        return Err("unsupported output extension; use .mp4".into());
    }
    if !options.overwrite && output.symlink_metadata().is_ok() {
        return Err(format!(
            "output already exists: {} (use --overwrite)",
            output.display()
        ));
    }
    eprintln!("Reading and validating session...");
    let mut session = Session::open(dir)?;
    let scanned = scan(&mut session, options.skip_provisional)?;
    if scanned.states.is_empty() {
        return Err(
            "session has no visual states after filtering; try including provisional states".into(),
        );
    }
    let plan = Plan::new(
        scanned.states.clone(),
        options.fps,
        options.speed,
        options.hold_end,
        options.preview_end,
    )?;
    let encoder = ffmpeg::select_encoder(options.encoder)?;
    let (w, h) = match options.canvas_mode {
        CanvasMode::Max => (scanned.max_width, scanned.max_height),
        CanvasMode::Initial => (
            session.manifest.initial_width,
            session.manifest.initial_height,
        ),
    };
    let spatial = Spatial::new(w, h, options.size, ffmpeg::max_dimension(encoder))?;
    eprintln!(
        "Replaying session: {} states; encoding {}x{} at {} fps with {}...",
        scanned.states.len(),
        spatial.width,
        spatial.height,
        options.fps,
        encoder.name()
    );
    let mut final_frame = None;
    if options.preview_end > 0.0 {
        let final_state = final_replay(&mut session, &scanned)?;
        final_frame =
            Some(spatial.render(&final_state.rgb, final_state.width, final_state.height)?);
    }
    session.seek(0)?;
    let mut replay = Replay::new(
        session.manifest.initial_width,
        session.manifest.initial_height,
    )?;
    let mut writer = VideoWriter::start(
        output,
        spatial.width,
        spatial.height,
        options.fps,
        encoder,
        options.quality,
        options.overwrite,
    )?;
    let mut next_batch = 0usize;
    let mut cached = None;
    let mut previous = None;
    let mut updated = Instant::now();
    for (frame, target) in plan.frames().enumerate() {
        if target != Target::Final && Some(target) != previous {
            let Target::State(batch) = target else {
                unreachable!()
            };
            while next_batch <= batch {
                let events = session
                    .next_batch()?
                    .ok_or("session ended unexpectedly during replay")?;
                apply_batch(&mut replay, &events, &mut session)?;
                next_batch += 1;
            }
            cached = Some(spatial.render(&replay.rgb, replay.width, replay.height)?);
        }
        if target == Target::Final && final_frame.is_none() {
            while let Some(events) = session.next_batch()? {
                apply_batch(&mut replay, &events, &mut session)?;
            }
            final_frame = Some(spatial.render(&replay.rgb, replay.width, replay.height)?);
        }
        let pixels = if target == Target::Final {
            final_frame.as_ref()
        } else {
            cached.as_ref()
        };
        writer.write_frame(pixels.ok_or("missing rendered frame")?)?;
        previous = Some(target);
        if updated.elapsed() >= Duration::from_millis(250) || frame as u64 + 1 == plan.len() {
            eprint!("\rFrames: {} / {}", frame + 1, plan.len());
            let _ = std::io::stderr().flush();
            updated = Instant::now();
        }
    }
    eprintln!();
    writer.finish()?;
    println!("Exported {} frames to {}", plan.len(), output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_complete_cli_and_rejects_invalid_values() {
        let args = [
            "--fps",
            "60",
            "--speed",
            "2",
            "--encoder",
            "nvenc",
            "--quality",
            "high",
            "--size",
            "2160",
            "--canvas-mode",
            "initial",
            "--hold-end",
            "4",
            "--preview-end",
            "1",
            "--skip-provisional",
            "--overwrite",
        ]
        .map(String::from);
        let o = Options::parse(&args).unwrap();
        assert_eq!(
            (o.fps, o.speed, o.encoder, o.quality, o.size, o.canvas_mode),
            (
                60,
                2.0,
                Encoder::Nvenc,
                Quality::High,
                Size::P2160,
                CanvasMode::Initial
            )
        );
        assert!(o.overwrite && o.skip_provisional);
        for args in [
            vec!["--speed", "NaN"],
            vec!["--hold-end", "-1"],
            vec!["--fps", "0"],
            vec!["--encoder", "other"],
            vec!["--size", "1280"],
            vec!["--speed"],
            vec!["--extra"],
        ] {
            assert!(
                Options::parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err()
            );
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod integration_tests;
