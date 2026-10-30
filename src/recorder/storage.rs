//! Version 1: little-endian length + XXH3-128 checksum + bincode payload.
//! Hash identity assumes negligible XXH3-128 collisions; verify_identity is the
//! extension point for a future byte-equality collision check.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
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
pub struct Store {
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
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
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
            File::open(dir).map_err(err)?.sync_all().map_err(err)?;
        }
        let mut s = Self {
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
        Ok(s)
    }
    fn apply(&mut self, event: &Event) -> Result<(), String> {
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

/// Validate and summarize a stopped session without
/// attaching to the SAI process. Opening the store performs blob checks,
/// truncates incomplete tails, and replays the timeline to validate references.
pub fn verify(dir: &Path) -> Result<(), String> {
    let manifest_path = dir.join("manifest.json");
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(err)?).map_err(err)?;
    let store = Store::open(dir, manifest, Config::default())?;
    let blob_count = store.index.len();
    let blob_bytes = fs::metadata(dir.join("blobs.bin")).map_err(err)?.len();
    let mut timeline = File::open(dir.join("timeline.bin")).map_err(err)?;
    let mut frame_count = 0u64;
    let mut stable_count = 0u64;
    let mut provisional_count = 0u64;
    let mut resize_count = 0u64;
    let mut checkpoint_count = 0u64;
    let mut delta_tiles = 0u64;
    let mut blob_references = 0u64;
    let mut referenced_ids = std::collections::HashSet::new();
    while let Some(data) = next(&mut timeline)? {
        let events: Vec<Event> = bincode::deserialize(&data).map_err(err)?;
        for event in events {
            match event {
                Event::Frame {
                    kind,
                    timestamp_ms,
                    changed_tiles,
                } => {
                    frame_count += 1;
                    match kind {
                        FrameKind::Stable => stable_count += 1,
                        FrameKind::Provisional => provisional_count += 1,
                    }
                    delta_tiles += changed_tiles.len() as u64;
                    blob_references += changed_tiles.len() as u64;
                    referenced_ids.extend(changed_tiles.iter().map(|(_, id)| *id));
                    println!(
                        "frame {frame_count}: {kind:?} at {timestamp_ms} ms, {} changed tiles",
                        changed_tiles.len()
                    );
                }
                Event::CanvasResize {
                    kind,
                    timestamp_ms,
                    old_width,
                    old_height,
                    new_width,
                    new_height,
                    ..
                } => {
                    resize_count += 1;
                    println!(
                        "resize {resize_count}: {kind:?} at {timestamp_ms} ms, {old_width}x{old_height} -> {new_width}x{new_height}"
                    );
                }
                Event::Checkpoint {
                    width,
                    height,
                    nx,
                    ny,
                    tiles,
                } => {
                    checkpoint_count += 1;
                    blob_references += tiles.len() as u64;
                    referenced_ids.extend(tiles.iter().map(|(_, id)| *id));
                    println!(
                        "checkpoint {checkpoint_count}: {width}x{height}, grid {nx}x{ny}, {} tiles",
                        tiles.len()
                    );
                }
                Event::Transform {
                    timestamp_ms,
                    transform,
                } => {
                    println!("transform: {transform:?} at {timestamp_ms} ms");
                }
            }
        }
    }
    let nx = store.state.width.div_ceil(256);
    let ny = store.state.height.div_ceil(256);
    println!(
        "verified: {frame_count} frames ({stable_count} stable, {provisional_count} provisional), {resize_count} resizes, {checkpoint_count} checkpoints"
    );
    println!(
        "blobs: {blob_count} unique stored, {} referenced ids, {} references, {} repeated references; {blob_bytes} file bytes",
        referenced_ids.len(),
        blob_references,
        blob_references.saturating_sub(referenced_ids.len() as u64)
    );
    println!(
        "final canvas: {}x{}, grid {}x{}, {} tiles; {delta_tiles} timeline delta tile entries",
        store.state.width,
        store.state.height,
        nx,
        ny,
        store.state.tiles.len()
    );
    Ok(())
}

/// Read-only access to a stopped session. Incomplete tails are ignored, never truncated.
pub struct Session {
    _lock: File,
    pub manifest: Manifest,
    blobs: File,
    timeline: File,
    index: HashMap<BlobId, u64>,
}
impl Session {
    pub fn open(dir: &Path) -> Result<Self, String> {
        let lock = File::open(dir.join("session.lock")).map_err(err)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
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
            let raw = zstd::bulk::decompress(&blob.compressed, TILE_BYTES).map_err(err)?;
            verify_identity(blob.id, &raw)?;
            index.insert(blob.id, offset);
        }
        Ok(Self {
            _lock: lock,
            manifest,
            blobs,
            timeline: File::open(dir.join("timeline.bin")).map_err(err)?,
            index,
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
