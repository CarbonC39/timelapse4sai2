//! Offline exporter for ordered visual states; wall-clock timestamps are metadata.
mod ffmpeg;
mod frame_map;
mod replay;

use crate::recorder::storage::{Event, Session, Transform};
use ffmpeg::VideoWriter;
pub use ffmpeg::{Encoder, Quality};
use frame_map::FrameSchedule;
pub use frame_map::Target;
pub use replay::{CanvasMode, Size};
use replay::{Replay, Spatial};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

pub const HELP: &str = "timelapse4sai2 export SESSION_DIR OUTPUT.mp4 [OPTIONS]
  --fps 30                    Output FPS (1..240); does not change drawing duration
  --pace <FLOAT>              Playback pace in visual captures per second (default 15; 0.1..1000)
  --encoder auto|nvenc|x264    Default auto; probes NVENC, falls back to libx264
  --quality draft|balanced|high|archival  Default balanced
  --size auto|720|1080|1440|2160|N%
                              Longest edge in pixels, or percentage of the
                              logical canvas; auto keeps natural size, capped
                              to encoder limits; aspect ratio preserved
  --canvas-mode max|initial   Default max: center on historical maximum canvas;
                              initial: center-crop to initial canvas. No scaling
                              during placement; unknown resize origin is centered
  --hold-end 3                Final artwork hold in seconds (0 disables)
  --preview-end 2             Final artwork preview before drawing, in seconds
  --skip-provisional          Only stable states (default includes provisional)
  --overwrite                 Replace existing output only after encoding succeeds
Timestamps and idle time are ignored. Durations round to the nearest output frame.
Output: H.264 MP4 / yuv420p; odd dimensions pad 1px on right/bottom.";

#[derive(Debug, PartialEq)]
pub struct Options {
    pub fps: u32,
    pub pace: f64,
    pub encoder: Encoder,
    pub quality: Quality,
    pub size: Size,
    pub canvas_mode: CanvasMode,
    pub hold_end: f64,
    pub preview_end: f64,
    pub skip_provisional: bool,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportPlan {
    pub source_captures: u64,
    pub selected_captures: u64,
    pub output_video_frames: u64,
    pub duration_ms: u64,
    /// Ideal drawing duration before rounding to output video frames.
    pub drawing_duration_s: f64,
    pub output_width: u32,
    pub output_height: u32,
    pub fps: u32,
    pub pace: f64,
    pub preview_end_ms: u64,
    pub hold_end_ms: u64,
}

impl ExportPlan {
    /// Refresh timing from the selected capture count without rescanning a session.
    /// Capture selection and output dimensions must already match the options.
    pub fn update_timing(&mut self, options: &Options) -> Result<(), String> {
        let timing = frame_map::timing(
            self.selected_captures,
            options.fps,
            options.pace,
            options.hold_end,
            options.preview_end,
        )?;
        self.output_video_frames = timing.frame_count + timing.preview_count + timing.hold_count;
        self.duration_ms = frames_to_ms(self.output_video_frames, options.fps);
        self.drawing_duration_s = timing.drawing_duration_s;
        self.fps = options.fps;
        self.pace = options.pace;
        self.preview_end_ms = frames_to_ms(timing.preview_count, options.fps);
        self.hold_end_ms = frames_to_ms(timing.hold_count, options.fps);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    pub plan: ExportPlan,
    pub warnings: Vec<String>,
}

struct Prepared {
    plan: ExportPlan,
    scan: Scan,
    schedule: FrameSchedule,
    spatial: Spatial,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            fps: 30,
            pace: 15.0,
            encoder: Encoder::Auto,
            quality: Quality::Balanced,
            size: Size::Auto,
            canvas_mode: CanvasMode::Max,
            hold_end: 3.0,
            preview_end: 2.0,
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
                    | "--pace"
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
                "--pace" => o.pace = number!(),
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
        FrameSchedule::new(vec![0], o.fps, o.pace, o.hold_end, o.preview_end)?;
        Ok(o)
    }
}

struct Scan {
    states: Vec<usize>,
    source_captures: usize,
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
        source_captures: 0,
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
        if let Some(kind) = frame_map::batch_kind(&events) {
            result.source_captures += 1;
            if frame_map::include(kind, skip_provisional) {
                result.states.push(result.batches);
            }
        }
        result.batches += 1;
    }
    if result.batches == 0 {
        return Err("session contains no captured canvas".into());
    }
    Ok(result)
}

fn frames_to_ms(frames: u64, fps: u32) -> u64 {
    ((u128::from(frames) * 1000 + u128::from(fps) / 2) / u128::from(fps)) as u64
}

fn prepare(session: &mut Session, options: &Options) -> Result<Prepared, String> {
    let scan = scan(session, options.skip_provisional)?;
    if scan.states.is_empty() {
        return Err(
            "session has no visual states after filtering; try including provisional states".into(),
        );
    }
    let schedule = FrameSchedule::new(
        scan.states.clone(),
        options.fps,
        options.pace,
        options.hold_end,
        options.preview_end,
    )?;
    let (logical_width, logical_height) = match options.canvas_mode {
        CanvasMode::Max => (scan.max_width, scan.max_height),
        CanvasMode::Initial => (
            session.manifest.initial_width,
            session.manifest.initial_height,
        ),
    };
    // Auto's limit is its conservative NVENC-compatible cap; no ffmpeg probe
    // or rendering is needed to calculate planned dimensions.
    let spatial = Spatial::new(
        logical_width,
        logical_height,
        options.size,
        ffmpeg::max_dimension(options.encoder),
    )?;
    let plan = ExportPlan {
        source_captures: scan.source_captures as u64,
        selected_captures: scan.states.len() as u64,
        output_video_frames: schedule.len(),
        duration_ms: frames_to_ms(schedule.len(), options.fps),
        drawing_duration_s: schedule.drawing_duration_s(),
        output_width: spatial.width,
        output_height: spatial.height,
        fps: options.fps,
        pace: options.pace,
        preview_end_ms: frames_to_ms(schedule.preview_frames(), options.fps),
        hold_end_ms: frames_to_ms(schedule.hold_frames(), options.fps),
    };
    Ok(Prepared {
        plan,
        scan,
        schedule,
        spatial,
    })
}

pub fn plan(session: &mut Session, options: &Options) -> Result<ExportPlan, String> {
    Ok(prepare(session, options)?.plan)
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

/// Replay the final visual state of a session. Intended for embedders that need
/// a final thumbnail without invoking the video exporter.
pub fn replay_final(session: &mut Session) -> Result<replay::Replay, String> {
    let scan = scan(session, false)?;
    final_replay(session, &scan)
}

pub fn dump_final(dir: &Path, output: &Path) -> Result<(), String> {
    let mut session = Session::open(dir)?;
    let scan = scan(&mut session, false)?;
    let replay = final_replay(&mut session, &scan)?;
    crate::image::write_png(output, replay.width, replay.height, &replay.rgb)?;
    Ok(())
}

pub fn run(dir: &Path, output: &Path, options: Options) -> Result<ExportReport, String> {
    run_with_progress(dir, output, options, |_, _| {})
}

/// Export while reporting encoded video frame progress. The callback is called
/// at start, at most every 100ms while encoding, and at completion.
pub fn run_with_progress(
    dir: &Path,
    output: &Path,
    options: Options,
    mut progress: impl FnMut(u64, u64),
) -> Result<ExportReport, String> {
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
    let mut session = Session::open(dir)?;
    let prepared = prepare(&mut session, &options)?;
    let warnings = session.warnings.clone();
    let encoder = ffmpeg::select_encoder(options.encoder)?;
    let Prepared {
        plan,
        scan: scanned,
        schedule,
        spatial,
    } = prepared;
    let total = plan.output_video_frames;
    let mut completed = 0u64;
    let mut last_progress = Instant::now();
    progress(completed, total);
    let mut writer = VideoWriter::start(
        output,
        spatial.width,
        spatial.height,
        options.fps,
        encoder,
        options.quality,
        options.overwrite,
    )?;
    render_frames(&mut session, &scanned, &schedule, &spatial, |pixels| {
        writer.write_frame(pixels)?;
        completed += 1;
        if last_progress.elapsed() >= Duration::from_millis(100) || completed == total {
            progress(completed, total);
            last_progress = Instant::now();
        }
        Ok(())
    })?;
    writer.finish()?;
    Ok(ExportReport { plan, warnings })
}

/// Render the schedule in order, sharing the first drawing image across the
/// opening crossfade and the drawing phase, and streaming pixels to the encoder.
fn render_frames(
    session: &mut Session,
    scanned: &Scan,
    schedule: &FrameSchedule,
    spatial: &Spatial,
    mut emit: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut final_frame = if schedule.preview_frames() > 0 {
        let final_state = final_replay(session, scanned)?;
        Some(spatial.render(&final_state.rgb, final_state.width, final_state.height)?)
    } else {
        None
    };
    session.seek(0)?;
    let mut replay = Replay::new(
        session.manifest.initial_width,
        session.manifest.initial_height,
    )?;
    let mut next_batch = 0usize;
    let mut cached_batch = None;
    let mut cached = None;
    let mut blended = Vec::new();
    for target in schedule.frames() {
        let batch = match target {
            Target::State(batch) | Target::Crossfade { first: batch, .. } => Some(batch),
            Target::Final => None,
        };
        if let Some(batch) = batch {
            if cached_batch != Some(batch) {
                while next_batch <= batch {
                    let events = session
                        .next_batch()?
                        .ok_or("session ended unexpectedly during replay")?;
                    apply_batch(&mut replay, &events, session)?;
                    next_batch += 1;
                }
                cached = Some(spatial.render(&replay.rgb, replay.width, replay.height)?);
                cached_batch = Some(batch);
            }
        }
        if target == Target::Final && final_frame.is_none() {
            while let Some(events) = session.next_batch()? {
                apply_batch(&mut replay, &events, session)?;
            }
            final_frame = Some(spatial.render(&replay.rgb, replay.width, replay.height)?);
        }
        let pixels = match target {
            Target::Final => final_frame.as_deref().ok_or("missing final frame")?,
            Target::State(_) => cached.as_deref().ok_or("missing drawing frame")?,
            Target::Crossfade { frame, frames, .. } => {
                let final_pixels = final_frame.as_deref().ok_or("missing final frame")?;
                let first_pixels = cached.as_deref().ok_or("missing first drawing frame")?;
                let t = if frames <= 1 {
                    1.0
                } else {
                    frame as f64 / (frames - 1) as f64
                };
                blended.resize(final_pixels.len(), 0);
                for ((out, &end), &first) in blended.iter_mut().zip(final_pixels).zip(first_pixels)
                {
                    *out = (f64::from(end) * (1.0 - t) + f64::from(first) * t).round() as u8;
                }
                &blended
            }
        };
        emit(pixels)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_complete_cli_and_rejects_invalid_values() {
        assert_eq!(Options::parse(&[]).unwrap().pace, 15.0);
        let args = [
            "--fps",
            "60",
            "--pace",
            "7.5",
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
            (o.fps, o.pace, o.encoder, o.quality, o.size, o.canvas_mode),
            (
                60,
                7.5,
                Encoder::Nvenc,
                Quality::High,
                Size::P2160,
                CanvasMode::Initial
            )
        );
        assert!(o.overwrite && o.skip_provisional);
        assert_eq!(Size::parse("50%").unwrap(), Size::Percent(50));
        assert_eq!(Size::parse("720p").unwrap(), Size::P720);
        for args in [
            vec!["--pace", "NaN"],
            vec!["--pace", "0.09"],
            vec!["--pace", "1001"],
            vec!["--hold-end", "-1"],
            vec!["--fps", "0"],
            vec!["--encoder", "other"],
            vec!["--size", "1280"],
            vec!["--size", "0%"],
            vec!["--size", "500%"],
            vec!["--size", "half"],
            vec!["--pace"],
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
