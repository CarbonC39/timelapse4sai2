mod input;
mod scheduler;
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
}
impl Default for Config {
    fn default() -> Self {
        Self {
            idle_delay_ms: 150,
            min_capture_gap_ms: 300,
            max_active_gap_ms: 3000,
            zstd_level: 1,
            checkpoint_every_events: 500,
            checkpoint_changed_ratio: 70,
            no_input_watch: false,
        }
    }
}
impl Config {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut c = Self::default();
        let mut i = 0;
        while i < args.len() {
            let flag = &args[i];
            if flag == "--no-input-watch" {
                c.no_input_watch = true;
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
            .find(|c| c.address == identity.address && c.id == identity.id)
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
            if reader.read_canvas(canvas.address)? != canvas
                || !reader.canvases()?.contains(&canvas)
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
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}
pub fn record<M: ProcessMemory>(
    reader: &Reader<M>,
    canvas: &Canvas,
    dir: &Path,
    hash: &str,
    config: Config,
) -> Result<(), String> {
    let mut input = if config.no_input_watch {
        None
    } else {
        Some(input::InputWatcher::new()?)
    };
    let manifest = Manifest {
        format_version: 1,
        sai_profile: "SAI2 x64 2026-07-02b Technical Preview".into(),
        source_executable_md5: hash.into(),
        initial_width: canvas.width,
        initial_height: canvas.height,
        canvas_name: canvas.name.clone(),
        canvas_path: canvas.short_path.clone(),
        tile_size: 256,
        channel_order: "BGRX (X and edge padding zeroed)".into(),
        creation_time_unix_ms: timestamp(),
        recorder_version: env!("CARGO_PKG_VERSION").into(),
        identity_limit: concat!(
            "Name/path are descriptive, not globally stable document IDs. ",
            "Live selection uses address + SAI id; reopening requires explicit ",
            "canvas selection; same-name replacement cannot be proven identical."
        )
        .into(),
    };
    let store = Store::open(dir, manifest, config.clone())?;
    STOP.store(false, Ordering::Relaxed);
    unsafe {
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
    }
    let (tx, rx) = sync_channel(2);
    let (recycle_tx, recycle_rx) = sync_channel(4);
    let worker = std::thread::spawn(move || -> Result<Metrics, String> {
        let mut store = store;
        for message in rx {
            match message {
                Message::Begin(k, t, w, h) => store.begin(k, t, w, h),
                Message::Tile(c, h, raw) => {
                    store.tile(c, h, &raw)?;
                    let _ = recycle_tx.try_send(raw);
                }
                Message::End => store.end()?,
                Message::Abort => store.abort(),
            }
        }
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
    eprintln!(
        "Recording {} to {}; Ctrl-C stops and drains storage.",
        canvas.name,
        dir.display()
    );
    let result = (|| {
        capture.capture(reader, canvas, FrameKind::Stable, &tx, &mut metrics)?;
        scheduler.captured(start.elapsed().as_millis() as u64, FrameKind::Stable);
        let mut failures = 0;
        while !STOP.load(Ordering::Relaxed) {
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
                        eprintln!("Candidate discarded: {e}");
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
    metrics.logical_frames = storage.logical_frames;
    metrics.provisional_frames = storage.provisional_frames;
    metrics.tiles_changed = storage.tiles_changed;
    metrics.unique_blobs = storage.unique_blobs;
    metrics.deduplicated_blobs = storage.deduplicated_blobs;
    metrics.compressed_bytes_written = storage.compressed_bytes_written;
    metrics.checkpoint_count = storage.checkpoint_count;
    eprintln!("{metrics:#?}");
    result
}
