mod scheduler;
pub mod matching;
pub mod storage;
#[cfg(test)]
mod tests;
use crate::{
    process::ProcessMemory,
    sai::{canvas::Canvas, reader::Reader},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use storage::{Manifest, Store};
use xxhash_rust::xxh3::xxh3_128;
pub const TILE_BYTES: usize = 256 * 256 * 4;
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FrameKind {
    Stable,
    Provisional,
}
#[derive(Debug, Clone)]
pub struct Config {
    pub idle_delay_ms: u64,
    pub min_capture_gap_ms: u64,
    pub max_active_gap_ms: u64,
    pub zstd_level: i32,
    pub checkpoint_every_events: u64,
    pub checkpoint_changed_ratio: u32,
    pub no_input_watch: bool,
    pub allow_canvas_mismatch: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            idle_delay_ms: 100,
            min_capture_gap_ms: 250,
            max_active_gap_ms: 2500,
            zstd_level: 1,
            checkpoint_every_events: 500,
            checkpoint_changed_ratio: 70,
            no_input_watch: false,
            allow_canvas_mismatch: false,
        }
    }
}
impl Config {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut c = Self::default();
        let mut i = 0;
        while i < args.len() {
            let flag = &args[i];
            if flag == "--no-input-watch" || flag == "--allow-canvas-mismatch" {
                if flag == "--no-input-watch" {
                    c.no_input_watch = true;
                } else {
                    c.allow_canvas_mismatch = true;
                }
                i += 1;
                continue;
            }
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("missing value for {flag}"))?;
            macro_rules! number {
                () => {
                    value
                        .parse()
                        .map_err(|_| format!("invalid value for {flag}"))?
                };
            }
            match flag.as_str() {
                "--idle-delay-ms" => c.idle_delay_ms = number!(),
                "--min-capture-gap-ms" => c.min_capture_gap_ms = number!(),
                "--max-active-gap-ms" => c.max_active_gap_ms = number!(),
                "--zstd-level" => c.zstd_level = number!(),
                "--checkpoint-every-events" => c.checkpoint_every_events = number!(),
                "--checkpoint-changed-ratio" => c.checkpoint_changed_ratio = number!(),
                _ => return Err(format!("unknown record option {flag}")),
            }
            i += 2;
        }
        if c.max_active_gap_ms == 0 || c.min_capture_gap_ms == 0 {
            return Err("capture gaps must be positive".into());
        }
        if c.checkpoint_changed_ratio > 100 {
            return Err("checkpoint ratio must be 0..100".into());
        }
        if !(-7..=22).contains(&c.zstd_level) {
            return Err("zstd level must be -7..22".into());
        }
        Ok(c)
    }
}
#[derive(Default, Debug)]
pub struct Metrics {
    pub warnings: Vec<String>,
    pub capture_candidates: u64,
    pub logical_frames: u64,
    pub provisional_frames: u64,
    pub tiles_scanned: u64,
    pub tiles_changed: u64,
    pub unique_blobs: u64,
    pub deduplicated_blobs: u64,
    pub raw_bytes_read: u64,
    pub compressed_bytes_written: u64,
    pub checkpoint_count: u64,
}
pub enum Message {
    Begin(FrameKind, u64, u32, u32),
    Tile(storage::Coord, u128, Vec<u8>),
    End,
    Abort,
    Metadata(Canvas),
}
#[derive(Default)]
pub struct CaptureState {
    width: u32,
    height: u32,
    hashes: Vec<u128>,
    recycled: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
}
fn send(tx: &SyncSender<Message>, m: Message) -> Result<(), String> {
    tx.send(m).map_err(|_| "storage worker stopped".into())
}
pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl CaptureState {
    pub fn capture<M: ProcessMemory>(
        &mut self,
        reader: &Reader<M>,
        identity: &Canvas,
        kind: FrameKind,
        tx: &SyncSender<Message>,
        metrics: &mut Metrics,
    ) -> Result<(), String> {
        metrics.capture_candidates += 1;
        let canvas = reader
            .canvases()?
            .into_iter()
            .find(|c| c.same_live_identity(identity))
            .ok_or("selected canvas closed or replaced; stop recording")?;
        let map = reader.read_tile_map(&canvas)?;
        let resized = (self.width, self.height) != (canvas.width, canvas.height);
        let mut hashes = Vec::with_capacity(map.nx as usize * map.ny as usize);
        let mut tile = vec![0; TILE_BYTES];
        send(
            tx,
            Message::Begin(kind, timestamp(), canvas.width, canvas.height),
        )?;
        let result = (|| {
            for y in 0..map.ny {
                for x in 0..map.nx {
                    reader.read_tile_into(&map, x, y, &mut tile)?;
                    metrics.tiles_scanned += 1;
                    metrics.raw_bytes_read += TILE_BYTES as u64;
                    // Canonical BGRX: ignore unused fourth channel and out-of-canvas padding.
                    let valid_w = (canvas.width - x * 256).min(256) as usize;
                    let valid_h = (canvas.height - y * 256).min(256) as usize;
                    for dy in 0..256 {
                        let row = &mut tile[dy * 1024..(dy + 1) * 1024];
                        if dy >= valid_h {
                            row.fill(0);
                        } else {
                            row[valid_w * 4..].fill(0);
                            for px in row[..valid_w * 4].chunks_exact_mut(4) {
                                px[3] = 0;
                            }
                        }
                    }
                    let hash = xxh3_128(&tile);
                    let index = hashes.len();
                    hashes.push(hash);
                    if resized || self.hashes.get(index) != Some(&hash) {
                        let mut outgoing = self
                            .recycled
                            .as_ref()
                            .and_then(|rx| rx.try_recv().ok())
                            .unwrap_or_else(|| vec![0; TILE_BYTES]);
                        outgoing.copy_from_slice(&tile);
                        send(tx, Message::Tile((x, y), hash, outgoing))?;
                    }
                }
            }
            reader.validate_tile_map(&canvas, &map)?;
            let after = reader.read_canvas(canvas.address)?;
            if !after.same_live_identity(&canvas)
                || (after.width, after.height) != (canvas.width, canvas.height)
                || !reader.canvases()?.iter().any(|c| c.same_live_identity(&canvas))
            {
                return Err("canvas changed during capture; retry".into());
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                send(tx, Message::End)?;
                self.width = canvas.width;
                self.height = canvas.height;
                send(tx, Message::Metadata(canvas))?;
                self.hashes = hashes;
                Ok(())
            }
            Err(e) => {
                send(tx, Message::Abort)?;
                Err(e)
            }
        }
    }
}
static STOP: AtomicBool = AtomicBool::new(false);
fn recording_manifest<M: ProcessMemory>(reader: &Reader<M>, canvas: &Canvas) -> Manifest {
    Manifest {
        format_version: 1,
        sai_profile: reader.profile.display_name.into(),
        sai_generation: Some(reader.profile.generation.name().into()),
        source_executable_md5: reader
            .source_hash
            .unwrap_or(reader.profile.executable_hash)
            .into(),
        initial_width: canvas.width,
        initial_height: canvas.height,
        canvas_name: canvas.name.clone(),
        canvas_path: canvas.short_path.clone(),
        latest_canvas_name: None,
        latest_canvas_path: None,
        tile_size: 256,
        channel_order: "BGRX (X and edge padding zeroed)".into(),
        creation_time_unix_ms: timestamp(),
        recorder_version: env!("CARGO_PKG_VERSION").into(),
        identity_limit: concat!(
            "Name/path are descriptive, not globally stable document IDs. ",
            "Live selection uses address + SAI id when available; reopening requires explicit ",
            "canvas selection; same-name replacement cannot be proven identical."
        )
        .into(),
    }
}
pub fn record<M: ProcessMemory>(
    reader: &Reader<M>,
    canvas: &Canvas,
    dir: &Path,
    config: Config,
) -> Result<Metrics, String> {
    record_with_status(reader, canvas, dir, config, |_| {})
}

/// The observer receives a snapshot at most four times per second on the storage thread.
pub fn record_with_status<M: ProcessMemory>(
    reader: &Reader<M>,
    canvas: &Canvas,
    dir: &Path,
    config: Config,
    observer: impl FnMut(storage::RecordingStats) + Send + 'static,
) -> Result<Metrics, String> {
    record_inner(reader, canvas, dir, config, &STOP, true, 0, None, None::<fn(&Manifest, matching::SessionMatch) -> bool>, observer)
}

/// Record until `cancel` is set. This entry point does not install or modify the
/// process-wide Ctrl-C handler and is suitable for embedding in a GUI.
pub fn record_cancellable<M: ProcessMemory>(
    reader: &Reader<M>,
    canvas: &Canvas,
    dir: &Path,
    config: Config,
    cancel: &AtomicBool,
    observer: impl FnMut(storage::RecordingStats) + Send + 'static,
) -> Result<Metrics, String> {
    record_inner(reader, canvas, dir, config, cancel, false, 0, None, None::<fn(&Manifest, matching::SessionMatch) -> bool>, observer)
}

pub fn record_cancellable_with_match<M: ProcessMemory>(
    reader: &Reader<M>,
    canvas: &Canvas,
    dir: &Path,
    config: Config,
    cancel: &AtomicBool,
    pid: u32,
    previous: Option<matching::LiveCanvasIdentity>,
    authorize: impl FnOnce(&Manifest, matching::SessionMatch) -> bool,
    observer: impl FnMut(storage::RecordingStats) + Send + 'static,
) -> Result<Metrics, String> {
    record_inner(reader, canvas, dir, config, cancel, false, pid, previous, Some(authorize), observer)
}

fn record_inner<M: ProcessMemory>(
    reader: &Reader<M>,
    canvas: &Canvas,
    dir: &Path,
    config: Config,
    cancel: &AtomicBool,
    use_signal: bool,
    pid: u32,
    previous: Option<matching::LiveCanvasIdentity>,
    authorize: Option<impl FnOnce(&Manifest, matching::SessionMatch) -> bool>,
    mut observer: impl FnMut(storage::RecordingStats) + Send + 'static,
) -> Result<Metrics, String> {
    let mut input = if config.no_input_watch {
        None
    } else {
        Some(crate::process::InputWatcher::new()?)
    };
    let current = reader
        .canvases()?
        .into_iter()
        .find(|c| c.same_live_identity(canvas))
        .ok_or("selected canvas closed or replaced; select the canvas again")?;
    let canvas = &current;
    let manifest = recording_manifest(reader, canvas);
    let mut store = Store::open(dir, manifest, config.clone())?;
    if !store.is_new() {
        let decision = matching::SessionMatch::classify(store.manifest(), canvas, previous, pid);
        if decision.requires_confirmation() && !config.allow_canvas_mismatch {
            if let Some(authorize) = authorize {
                if !authorize(store.manifest(), decision) {
                    return Err("Recording cancelled.".into());
                }
            } else {
                decision.ensure_allowed(false)?;
            }
        }
        if let Some(warning) = matching::build_warning(
            store.manifest(),
            reader.source_hash.unwrap_or(reader.profile.executable_hash),
        ) {
            store.warnings.push(warning);
        }
    }
    store.update_canvas_metadata(canvas)?;
    if use_signal {
        STOP.store(false, Ordering::Relaxed);
        crate::process::install_stop_handler(&STOP)?;
    }
    let (tx, rx) = sync_channel(2);
    let (recycle_tx, recycle_rx) = sync_channel(4);
    store.start_recording()?;
    let worker = std::thread::spawn(move || -> Result<Metrics, String> {
        let mut store = store;
        let mut last_update = Instant::now();
        observer(store.recording_stats());
        loop {
            let message = match rx.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(message) => message,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    store.persist_stats_if_due()?;
                    observer(store.recording_stats());
                    last_update = Instant::now();
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            match message {
                Message::Begin(k, t, w, h) => store.begin(k, t, w, h),
                Message::Tile(c, h, raw) => {
                    store.tile(c, h, &raw)?;
                    let _ = recycle_tx.try_send(raw);
                }
                Message::End => store.end()?,
                Message::Abort => store.abort(),
                Message::Metadata(canvas) => {
                    if store.manifest().effective_canvas_name() != canvas.name
                        || store.manifest().effective_canvas_path() != canvas.short_path
                    {
                        store.update_canvas_metadata(&canvas)?;
                    }
                }
            }
            store.persist_stats_if_due()?;
            if last_update.elapsed().as_millis() >= 250 {
                observer(store.recording_stats());
                last_update = Instant::now();
            }
        }
        store.flush_stats()?;
        observer(store.recording_stats());
        store.metrics.warnings = store.warnings.clone();
        Ok(store.metrics)
    });
    let start = Instant::now();
    let mut scheduler = scheduler::Scheduler::new(
        config.idle_delay_ms,
        config.min_capture_gap_ms,
        config.max_active_gap_ms,
        0,
    );
    let mut capture = CaptureState {
        recycled: Some(recycle_rx),
        ..CaptureState::default()
    };
    let mut metrics = Metrics::default();
    let result = (|| {
        capture.capture(reader, canvas, FrameKind::Stable, &tx, &mut metrics)?;
        scheduler.captured(start.elapsed().as_millis() as u64, FrameKind::Stable);
        let mut failures = 0;
        let mut last_live_check = Instant::now();
        while !cancel.load(Ordering::Relaxed) {
            if last_live_check.elapsed().as_millis() >= 100 {
                if !reader.canvases()?.iter().any(|c| c.same_live_identity(canvas)) {
                    return Err("selected canvas closed or replaced; select the canvas again".into());
                }
                last_live_check = Instant::now();
            }
            let now = start.elapsed().as_millis() as u64;
            if let Some(input) = input.as_mut() {
                if input.poll_activity(now)? {
                    scheduler.activity(now);
                }
            } else {
                scheduler.activity(now);
            }
            if let Some(kind) = scheduler.due(now) {
                match capture.capture(reader, canvas, kind, &tx, &mut metrics) {
                    Ok(()) => {
                        failures = 0;
                        scheduler.captured(start.elapsed().as_millis() as u64, kind);
                    }
                    Err(e) => {
                        scheduler.failed(start.elapsed().as_millis() as u64);
                        failures += 1;
                        if failures >= 10
                            || e.contains("closed or replaced")
                            || e.contains("storage worker")
                        {
                            return Err(e);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        capture.capture(reader, canvas, FrameKind::Stable, &tx, &mut metrics)?;
        Ok(())
    })();
    drop(tx);
    let storage = worker.join().map_err(|_| "storage worker panicked")??;
    metrics.warnings = storage.warnings;
    metrics.logical_frames = storage.logical_frames;
    metrics.provisional_frames = storage.provisional_frames;
    metrics.tiles_changed = storage.tiles_changed;
    metrics.unique_blobs = storage.unique_blobs;
    metrics.deduplicated_blobs = storage.deduplicated_blobs;
    metrics.compressed_bytes_written = storage.compressed_bytes_written;
    metrics.checkpoint_count = storage.checkpoint_count;
    result?;
    Ok(metrics)
}
