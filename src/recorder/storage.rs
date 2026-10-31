//! Version 1: little-endian length + XXH3-128 checksum + bincode payload.
//! Hash identity assumes negligible XXH3-128 collisions; verify_identity is the
//! extension point for a future byte-equality collision check.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use xxhash_rust::xxh3::xxh3_128;
const MAX_RECORD: usize = 64 * 1024 * 1024;
pub type BlobId = u128;
pub type Coord = (u32, u32);
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Transform {
    Rotate90CW,
    Rotate90CCW,
    Rotate180,
    FlipHorizontal,
    FlipVertical,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Event {
    Frame {
        kind: FrameKind,
        timestamp_ms: u64,
        changed_tiles: Vec<(Coord, BlobId)>,
    },
    Checkpoint {
        width: u32,
        height: u32,
        nx: u32,
        ny: u32,
        tiles: Vec<(Coord, BlobId)>,
    },
    CanvasResize {
        kind: FrameKind,
        timestamp_ms: u64,
        old_width: u32,
        old_height: u32,
        new_width: u32,
        new_height: u32,
        origin: Option<(i32, i32)>,
    },
    Transform {
        timestamp_ms: u64,
        transform: Transform,
    },
}
#[derive(Default, Debug, Clone, PartialEq)]
pub struct State {
    pub width: u32,
    pub height: u32,
    pub tiles: std::collections::BTreeMap<Coord, BlobId>,
}
#[derive(Serialize, Deserialize)]
struct Blob {
    id: BlobId,
    compressed: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub sai_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sai_generation: Option<String>,
    pub source_executable_md5: String,
    pub initial_width: u32,
    pub initial_height: u32,
    pub canvas_name: String,
    pub canvas_path: String,
    pub tile_size: u32,
    pub channel_order: String,
    pub creation_time_unix_ms: u64,
    pub recorder_version: String,
    pub identity_limit: String,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn append<T: Serialize>(file: &mut File, value: &T) -> Result<(), String> {
    let data = bincode::serialize(value).map_err(err)?;
    if data.len() > MAX_RECORD {
        return Err("record exceeds 64 MiB limit".into());
    }
    file.write_all(&(data.len() as u32).to_le_bytes())
        .map_err(err)?;
    file.write_all(&xxh3_128(&data).to_le_bytes())
        .map_err(err)?;
    file.write_all(&data).map_err(err)
}
// Only incomplete tails are ignored. A complete but corrupt record is an error.
fn next(file: &mut File) -> Result<Option<Vec<u8>>, String> {
    let start = file.stream_position().map_err(err)?;
    let remaining = file.metadata().map_err(err)?.len() - start;
    if remaining < 20 {
        return Ok(None);
    }
    let mut header = [0; 20];
    file.read_exact(&mut header).map_err(err)?;
    let n = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    if n > MAX_RECORD {
        return Err("invalid record length".into());
    }
    if remaining < 20 + n as u64 {
        return Ok(None);
    }
    let mut data = vec![0; n];
    file.read_exact(&mut data).map_err(err)?;
    if xxh3_128(&data) != u128::from_le_bytes(header[4..].try_into().unwrap()) {
        return Err("record checksum mismatch".into());
    }
    Ok(Some(data))
}
fn open(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(err)
}
fn verify_identity(id: BlobId, raw: &[u8]) -> Result<(), String> {
    if raw.len() != TILE_BYTES || xxh3_128(raw) != id {
        Err(format!("corrupt blob {id:032x}"))
    } else {
        Ok(())
    }
}
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionStats {
    pub tracked_duration_ms: u64,
    pub recording_runs: u64,
    pub captures: u64,
    pub stable_captures: u64,
    pub provisional_captures: u64,
    pub last_updated_unix_ms: u64,
}
impl SessionStats {
    fn capture(&mut self, kind: FrameKind) {
        self.captures += 1;
        match kind {
            FrameKind::Stable => self.stable_captures += 1,
            FrameKind::Provisional => self.provisional_captures += 1,
        }
    }
    fn clear_captures(&mut self) {
        self.captures = 0;
        self.stable_captures = 0;
        self.provisional_captures = 0;
    }
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let mut file = File::create(dir.join("stats.tmp")).map_err(err)?;
        file.write_all(&serde_json::to_vec_pretty(self).map_err(err)?).map_err(err)?;
        file.sync_all().map_err(err)?;
        fs::rename(dir.join("stats.tmp"), dir.join("stats.json")).map_err(err)?;
        crate::process::sync_directory(dir).map_err(err)
    }
}
fn read_stats(dir: &Path) -> (SessionStats, Vec<String>) {
    match fs::read(dir.join("stats.json")) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(stats) => (stats, vec![]),
            Err(e) => (SessionStats::default(), vec![format!("Invalid stats.json; tracked duration and recording runs reset to zero: {e}")]),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (SessionStats::default(), vec![]),
        Err(e) => (SessionStats::default(), vec![format!("Cannot read stats.json; tracked duration and recording runs reset to zero: {e}")]),
    }
}
#[derive(Debug, Clone)]
pub struct RecordingStats {
    pub current_run_elapsed_ms: u64,
    pub total_tracked_duration_ms: u64,
    pub captures: u64,
    pub stable_captures: u64,
    pub provisional_captures: u64,
}
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub tracked_duration_ms: u64,
    pub captures: u64,
    pub stable_captures: u64,
    pub provisional_captures: u64,
    pub unique_blobs: u64,
    pub disk_bytes: u64,
    pub initial_width: u32,
    pub initial_height: u32,
    pub current_width: u32,
    pub current_height: u32,
    pub canvas_name: String,
}
pub fn disk_bytes(dir: &Path) -> Result<u64, String> {
    let mut bytes = 0;
    for name in ["manifest.json", "blobs.bin", "timeline.bin", "session.lock", "stats.json", "stats.tmp", "manifest.tmp"] {
        match fs::metadata(dir.join(name)) {
            Ok(metadata) if metadata.is_file() => bytes += metadata.len(),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Ok(bytes)
}

pub struct Store {
    dir: std::path::PathBuf,
    pub stats: SessionStats,
    pub warnings: Vec<String>,
    run_started: Option<Instant>,
    last_stats_flush: Option<Instant>,
    _lock: File,
    blobs: File,
    timeline: File,
    index: HashMap<BlobId, u64>,
    pub state: State,
    pub metrics: Metrics,
    config: Config,
    since_checkpoint: u64,
    pending: Option<(FrameKind, u64, State, Vec<(Coord, BlobId)>)>,
}
impl Store {
    pub fn open(dir: &Path, manifest: Manifest, config: Config) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(err)?;
        let lock = open(&dir.join("session.lock"))?;
        if lock.try_lock().is_err() {
            return Err("session is already open by another recorder".into());
        }
        let path = dir.join("manifest.json");
        if path.exists() {
            let old: Manifest =
                serde_json::from_slice(&fs::read(&path).map_err(err)?).map_err(err)?;
            if old.format_version != 1
                || old.source_executable_md5 != manifest.source_executable_md5
                || old.canvas_name != manifest.canvas_name
                || old.canvas_path != manifest.canvas_path
            {
                return Err("session manifest does not match canvas/profile".into());
            }
        } else {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(dir.join("manifest.tmp"))
                .map_err(err)?;
            f.write_all(&serde_json::to_vec_pretty(&manifest).map_err(err)?)
                .map_err(err)?;
            f.sync_all().map_err(err)?;
            fs::rename(dir.join("manifest.tmp"), &path).map_err(err)?;
            crate::process::sync_directory(dir).map_err(err)?;
        }
        let (mut stats, warnings) = read_stats(dir);
        stats.clear_captures();
        let mut s = Self {
            dir: dir.to_owned(), stats, warnings, run_started: None, last_stats_flush: None,
            _lock: lock,
            blobs: open(&dir.join("blobs.bin"))?,
            timeline: open(&dir.join("timeline.bin"))?,
            index: HashMap::new(),
            state: State::default(),
            metrics: Metrics::default(),
            config,
            since_checkpoint: 0,
            pending: None,
        };
        let mut end = 0;
        loop {
            let offset = s.blobs.stream_position().map_err(err)?;
            let Some(data) = next(&mut s.blobs)? else {
                break;
            };
            let blob: Blob = bincode::deserialize(&data).map_err(err)?;
            let raw = zstd::bulk::decompress(&blob.compressed, TILE_BYTES)
                .map_err(|e| format!("corrupt blob {:032x}: {e}", blob.id))?;
            verify_identity(blob.id, &raw)?;
            s.index.insert(blob.id, offset);
            end = s.blobs.stream_position().map_err(err)?;
        }
        s.blobs.set_len(end).map_err(err)?;
        s.blobs.seek(SeekFrom::End(0)).map_err(err)?;
        end = 0;
        while let Some(data) = next(&mut s.timeline)? {
            let events: Vec<Event> = bincode::deserialize(&data).map_err(err)?;
            for event in events {
                s.apply(&event)?;
            }
            end = s.timeline.stream_position().map_err(err)?;
        }
        s.timeline.set_len(end).map_err(err)?;
        s.timeline.seek(SeekFrom::End(0)).map_err(err)?;
        if let Err(e) = s.stats.save(dir) {
            s.warnings.push(format!("Could not repair stats.json cache: {e}"));
        }
        Ok(s)
    }
    pub fn start_recording(&mut self) -> Result<(), String> {
        if self.run_started.is_none() {
            self.run_started = Some(Instant::now());
            self.stats.recording_runs = self.stats.recording_runs.saturating_add(1);
            self.flush_stats()?;
        }
        Ok(())
    }
    pub fn recording_stats(&self) -> RecordingStats {
        let elapsed = self.run_started.map(|start| start.elapsed().as_millis().min(u64::MAX as u128) as u64).unwrap_or(0);
        RecordingStats {
            current_run_elapsed_ms: elapsed,
            total_tracked_duration_ms: self.stats.tracked_duration_ms.saturating_add(elapsed),
            captures: self.stats.captures,
            stable_captures: self.stats.stable_captures,
            provisional_captures: self.stats.provisional_captures,
        }
    }
    pub fn flush_stats(&mut self) -> Result<(), String> {
        let mut snapshot = self.stats.clone();
        snapshot.tracked_duration_ms = self.recording_stats().total_tracked_duration_ms;
        snapshot.last_updated_unix_ms = timestamp();
        // Ensure persisted captures never precede their referenced data after a crash.
        self.blobs.sync_all().map_err(err)?;
        self.timeline.sync_all().map_err(err)?;
        snapshot.save(&self.dir)?;
        self.last_stats_flush = Some(Instant::now());
        Ok(())
    }
    pub fn persist_stats_if_due(&mut self) -> Result<(), String> {
        if self.run_started.is_some() && self.last_stats_flush.is_none_or(|last| last.elapsed().as_secs() >= 30) {
            self.flush_stats()?;
        }
        Ok(())
    }
    fn apply(&mut self, event: &Event) -> Result<(), String> {
        if let Event::Frame {kind, ..} | Event::CanvasResize {kind, ..} = event {
            self.stats.capture(*kind);
        }
        let tiles = match event {
            Event::Frame { changed_tiles, .. } => {
                self.since_checkpoint += 1;
                changed_tiles
            }
            Event::Checkpoint {
                width,
                height,
                nx,
                ny,
                tiles,
            } => {
                if *nx != width.div_ceil(256)
                    || *ny != height.div_ceil(256)
                    || tiles.len() != (*nx as usize) * (*ny as usize)
                {
                    return Err("invalid checkpoint grid".into());
                }
                self.state = State {
                    width: *width,
                    height: *height,
                    ..State::default()
                };
                self.since_checkpoint = 0;
                tiles
            }
            Event::CanvasResize { .. } | Event::Transform { .. } => return Ok(()),
        };
        for &(coord, id) in tiles {
            if !self.index.contains_key(&id) {
                return Err(format!("missing referenced blob {id:032x}"));
            }
            self.state.tiles.insert(coord, id);
        }
        Ok(())
    }
    pub fn begin(&mut self, kind: FrameKind, timestamp: u64, width: u32, height: u32) {
        let mut state = self.state.clone();
        if (state.width, state.height) != (width, height) {
            state = State {
                width,
                height,
                ..State::default()
            };
        }
        self.pending = Some((kind, timestamp, state, Vec::new()));
    }
    pub fn tile(&mut self, coord: Coord, id: BlobId, raw: &[u8]) -> Result<(), String> {
        verify_identity(id, raw)?;
        let pending = self.pending.as_mut().ok_or("tile outside candidate")?;
        if pending.2.tiles.get(&coord) == Some(&id) {
            return Ok(());
        }
        if self.index.contains_key(&id) {
            self.metrics.deduplicated_blobs += 1;
        } else {
            let compressed = zstd::bulk::compress(raw, self.config.zstd_level).map_err(err)?;
            self.metrics.compressed_bytes_written += compressed.len() as u64;
            let offset = self.blobs.stream_position().map_err(err)?;
            append(&mut self.blobs, &Blob { id, compressed })?;
            self.index.insert(id, offset);
            self.metrics.unique_blobs += 1;
        }
        pending.2.tiles.insert(coord, id);
        pending.3.push((coord, id));
        Ok(())
    }
    pub fn abort(&mut self) {
        self.pending = None;
    }
    pub fn end(&mut self) -> Result<(), String> {
        let (kind, timestamp, state, changes) =
            self.pending.take().ok_or("end outside candidate")?;
        let resized = (state.width, state.height) != (self.state.width, self.state.height);
        if changes.is_empty() && !resized {
            return Ok(());
        }
        let total = state.width.div_ceil(256) as usize * state.height.div_ceil(256) as usize;
        if state.tiles.len() != total {
            return Err("incomplete candidate tile state".into());
        }
        let mut events = Vec::new();
        if resized && self.state.width != 0 {
            events.push(Event::CanvasResize {
                kind,
                timestamp_ms: timestamp,
                old_width: self.state.width,
                old_height: self.state.height,
                new_width: state.width,
                new_height: state.height,
                origin: None,
            });
        }
        // Resize is a structural boundary, not an ordinary tile delta.
        if !resized || self.state.width == 0 {
            events.push(Event::Frame {
                kind,
                timestamp_ms: timestamp,
                changed_tiles: changes.clone(),
            });
        }
        self.since_checkpoint += 1;
        let checkpoint = resized
            || (self.config.checkpoint_every_events > 0
                && self.since_checkpoint >= self.config.checkpoint_every_events)
            || (self.config.checkpoint_changed_ratio > 0
                && changes.len() * 100 >= total * self.config.checkpoint_changed_ratio as usize);
        if checkpoint {
            events.push(Event::Checkpoint {
                width: state.width,
                height: state.height,
                nx: state.width.div_ceil(256),
                ny: state.height.div_ceil(256),
                tiles: state.tiles.iter().map(|(&c, &b)| (c, b)).collect(),
            });
        }
        self.blobs.flush().map_err(err)?;
        append(&mut self.timeline, &events)?;
        self.timeline.flush().map_err(err)?;
        self.state = state;
        self.stats.capture(kind);
        self.metrics.logical_frames += 1;
        if kind == FrameKind::Provisional {
            self.metrics.provisional_frames += 1;
        }
        self.metrics.tiles_changed += changes.len() as u64;
        if checkpoint {
            self.metrics.checkpoint_count += 1;
            self.since_checkpoint = 0;
        }
        Ok(())
    }
}

/// Validate all stored blob identities and timeline references.
pub struct VerificationReport {
    pub summary: SessionSummary,
    pub warnings: Vec<String>,
}
pub fn verify(dir: &Path) -> Result<VerificationReport, String> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(dir.join("manifest.json")).map_err(err)?).map_err(err)?;
    let store = Store::open(dir, manifest, Config::default())?;
    let warnings = store.warnings.clone();
    drop(store);
    Ok(VerificationReport { summary: Session::open(dir)?.summary()?, warnings })
}

/// Read-only access to a stopped session. Incomplete tails are ignored, never truncated.
pub type SessionReader = Session;
pub struct Session {
    dir: std::path::PathBuf,
    pub stats: SessionStats,
    pub warnings: Vec<String>,
    current_width: u32,
    current_height: u32,
    _lock: File,
    pub manifest: Manifest,
    blobs: File,
    timeline: File,
    index: HashMap<BlobId, u64>,
}
impl Session {
    pub fn open(dir: &Path) -> Result<Self, String> {
        let lock = File::open(dir.join("session.lock")).map_err(err)?;
        if lock.try_lock_shared().is_err() {
            return Err("session is open by a recorder; stop recording before exporting".into());
        }
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).map_err(err)?)
                .map_err(err)?;
        if manifest.format_version != 1
            || manifest.tile_size != 256
            || !manifest.channel_order.starts_with("BGRX")
        {
            return Err("unsupported session format, tile size or channel order".into());
        }
        let mut blobs = File::open(dir.join("blobs.bin")).map_err(err)?;
        let mut index = HashMap::new();
        loop {
            let offset = blobs.stream_position().map_err(err)?;
            let Some(data) = next(&mut blobs)? else { break };
            let blob: Blob = bincode::deserialize(&data).map_err(err)?;
            index.insert(blob.id, offset);
        }
        let (mut stats, warnings) = read_stats(dir);
        stats.clear_captures();
        let mut timeline = File::open(dir.join("timeline.bin")).map_err(err)?;
        let (mut current_width, mut current_height) = (manifest.initial_width, manifest.initial_height);
        while let Some(data) = next(&mut timeline)? {
            let events: Vec<Event> = bincode::deserialize(&data).map_err(err)?;
            for event in events {
                match event {
                    Event::Frame {kind, ..} => stats.capture(kind),
                    Event::CanvasResize {kind, new_width, new_height, ..} => {
                        stats.capture(kind); current_width = new_width; current_height = new_height;
                    }
                    Event::Checkpoint {width, height, ..} => {current_width = width; current_height = height;}
                    Event::Transform {transform: Transform::Rotate90CW | Transform::Rotate90CCW, ..} => {std::mem::swap(&mut current_width, &mut current_height);}
                    _ => (),
                }
            }
        }
        timeline.seek(SeekFrom::Start(0)).map_err(err)?;
        Ok(Self {
            dir: dir.to_owned(), stats, warnings, current_width, current_height,
            _lock: lock,
            manifest,
            blobs,
            timeline,
            index,
        })
    }
    pub fn summary(&self) -> Result<SessionSummary, String> {
        Ok(SessionSummary {
            tracked_duration_ms: self.stats.tracked_duration_ms,
            captures: self.stats.captures,
            stable_captures: self.stats.stable_captures,
            provisional_captures: self.stats.provisional_captures,
            unique_blobs: self.index.len() as u64,
            disk_bytes: disk_bytes(&self.dir)?,
            initial_width: self.manifest.initial_width,
            initial_height: self.manifest.initial_height,
            current_width: self.current_width,
            current_height: self.current_height,
            canvas_name: self.manifest.canvas_name.clone(),
        })
    }
    pub fn position(&mut self) -> Result<u64, String> {
        self.timeline.stream_position().map_err(err)
    }
    pub fn seek(&mut self, offset: u64) -> Result<(), String> {
        self.timeline.seek(SeekFrom::Start(offset)).map_err(err)?;
        Ok(())
    }
    pub fn next_batch(&mut self) -> Result<Option<Vec<Event>>, String> {
        let Some(data) = next(&mut self.timeline)? else {
            return Ok(None);
        };
        let events: Vec<Event> = bincode::deserialize(&data).map_err(err)?;
        for event in &events {
            let tiles = match event {
                Event::Frame { changed_tiles, .. } => changed_tiles,
                Event::Checkpoint { tiles, .. } => tiles,
                _ => continue,
            };
            for (_, id) in tiles {
                if !self.index.contains_key(id) {
                    return Err(format!("missing referenced blob {id:032x}"));
                }
            }
        }
        Ok(Some(events))
    }
    pub fn load_blob(&mut self, id: BlobId) -> Result<Vec<u8>, String> {
        let offset = *self
            .index
            .get(&id)
            .ok_or_else(|| format!("missing referenced blob {id:032x}"))?;
        self.blobs.seek(SeekFrom::Start(offset)).map_err(err)?;
        let data = next(&mut self.blobs)?.ok_or("incomplete blob record")?;
        let blob: Blob = bincode::deserialize(&data).map_err(err)?;
        if blob.id != id {
            return Err("blob index mismatch".into());
        }
        let raw = zstd::bulk::decompress(&blob.compressed, TILE_BYTES).map_err(err)?;
        verify_identity(id, &raw)?;
        Ok(raw)
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, Manifest) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("sai-stats-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let manifest = Manifest {
            format_version: 1, sai_profile: "test".into(), sai_generation: None,
            source_executable_md5: "test".into(), initial_width: 1, initial_height: 1,
            canvas_name: "canvas".into(), canvas_path: "canvas".into(), tile_size: 256,
            channel_order: "BGRX".into(), creation_time_unix_ms: 0,
            recorder_version: "test".into(), identity_limit: "test".into(),
        };
        (dir, manifest)
    }
    #[test]
    fn monotonic_runs_and_atomic_cache() {
        let (dir, manifest) = fixture();
        let mut store = Store::open(&dir, manifest, Config::default()).unwrap();
        assert_eq!(store.recording_stats().total_tracked_duration_ms, 0);
        assert_eq!(store.stats.recording_runs, 0);
        store.start_recording().unwrap();
        store.run_started = Some(Instant::now() - std::time::Duration::from_millis(1234));
        store.flush_stats().unwrap();
        let (first, warnings) = read_stats(&dir);
        assert!(warnings.is_empty());
        assert!(first.tracked_duration_ms >= 1234);
        assert_eq!(first.recording_runs, 1);
        assert!(!dir.join("stats.tmp").exists());
        drop(store);
        let manifest = serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let mut store = Store::open(&dir, manifest, Config::default()).unwrap();
        store.start_recording().unwrap();
        store.run_started = Some(Instant::now() - std::time::Duration::from_millis(567));
        store.flush_stats().unwrap();
        let (second, _) = read_stats(&dir);
        assert!(second.tracked_duration_ms >= first.tracked_duration_ms + 567);
        assert_eq!(second.recording_runs, 2);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn heartbeat_is_low_frequency_and_never_changes_timeline() {
        let (dir, manifest) = fixture();
        let mut store = Store::open(&dir, manifest, Config::default()).unwrap();
        store.start_recording().unwrap();
        let before = fs::read(dir.join("stats.json")).unwrap();
        store.run_started = Some(Instant::now() - std::time::Duration::from_secs(31));
        store.persist_stats_if_due().unwrap();
        assert_eq!(fs::read(dir.join("stats.json")).unwrap(), before);
        store.last_stats_flush = Some(Instant::now() - std::time::Duration::from_secs(31));
        store.persist_stats_if_due().unwrap();
        assert!(read_stats(&dir).0.tracked_duration_ms >= 31_000);
        store.flush_stats().unwrap();
        assert!(read_stats(&dir).0.tracked_duration_ms < 32_000);
        assert_eq!(fs::metadata(dir.join("timeline.bin")).unwrap().len(), 0);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_corrupt_and_stale_cache_are_recovered_from_timeline() {
        let (dir, manifest) = fixture();
        let mut store = Store::open(&dir, manifest, Config::default()).unwrap();
        let raw = vec![0; TILE_BYTES];
        let id = xxh3_128(&raw);
        for (kind, width) in [(FrameKind::Stable, 1), (FrameKind::Provisional, 2)] {
            store.begin(kind, 0, width, 1);
            store.tile((0, 0), id, &raw).unwrap();
            store.end().unwrap();
        }
        drop(store);
        let stale = SessionReader::open(&dir).unwrap();
        assert_eq!(stale.stats.captures, 2);
        assert_eq!(read_stats(&dir).0.captures, 0);
        drop(stale);
        fs::remove_file(dir.join("stats.json")).unwrap();
        let reader = SessionReader::open(&dir).unwrap();
        let summary = reader.summary().unwrap();
        assert_eq!((summary.captures, summary.stable_captures, summary.provisional_captures), (2, 1, 1));
        assert_eq!(summary.tracked_duration_ms, 0);
        assert_eq!((summary.current_width, summary.current_height, summary.unique_blobs), (2, 1, 1));
        assert_eq!(summary.disk_bytes, fs::read_dir(&dir).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum::<u64>());
        drop(reader);
        fs::write(dir.join("stats.json"), b"broken").unwrap();
        let reader = SessionReader::open(&dir).unwrap();
        assert_eq!(reader.warnings.len(), 1);
        assert_eq!(reader.stats.captures, 2);
        drop(reader);
        let manifest = serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let store = Store::open(&dir, manifest, Config::default()).unwrap();
        assert_eq!(store.warnings.len(), 1);
        assert_eq!(read_stats(&dir).0.captures, 2);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}
