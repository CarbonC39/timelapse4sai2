# Recording format v1

This document describes the on-disk format used by `timelapse4sai2` sessions.
A session stores deterministic tile deltas, not screen recordings or full frames.

## Session directory

| File | Purpose |
| --- | --- |
| `manifest.json` | Human-readable metadata and compatibility information. |
| `blobs.bin` | Append-only compressed tile records. |
| `timeline.bin` | Append-only batches of timeline events. |
| `session.lock` | Advisory lock. Recorders hold it exclusively; readers take a shared lock. |
| `stats.json` | Optional statistics cache. Rebuilt where possible, but not authoritative. |

`manifest.json` is written atomically through `manifest.tmp` and contains:

- `format_version` (`1`)
- `sai_profile`, `sai_generation`, `source_executable_md5`
- `initial_width`, `initial_height`
- `canvas_name`, `canvas_path`
- `tile_size` (`256`)
- `channel_order` (`BGRX...`)
- `creation_time_unix_ms`, `recorder_version`
- `identity_limit`

Readers reject sessions with an unsupported format version, tile size, or channel
order. `stats.json` is only a cache: captures are recomputed from the timeline,
while tracked duration and recording-run counts may reset if the cache is lost.

## Common record framing

Both `blobs.bin` and `timeline.bin` are sequences of length-prefixed records:

```text
record := u32_le(payload_len)
       || u128_le(XXH3_128(payload))
       || payload
```

- The header is 20 bytes: a 4-byte length followed by a 16-byte checksum.
- `payload_len` must not exceed 64 MiB.
- Payloads use bincode 1.3 default serialization: little-endian, fixed-width
  integers, Rust field order and enum-variant order as defined in
  `src/recorder/storage.rs`.
- There is no file header. The format version lives in `manifest.json`.
- A record is complete only when its full declared payload is present.
- Incomplete trailing records are treated as an interrupted tail.

## Blob records

Each `blobs.bin` payload is a serialized `Blob`:

```rust
struct Blob {
    id: u128,
    compressed: Vec<u8>,
}
```

`compressed` is a zstd-compressed, canonical `256 x 256 x 4` BGRX tile:
262144 decoded bytes. The tile may be partially outside the logical canvas; unused
pixels are zeroed.

`id` is `XXH3_128(uncompressed_tile)`, not a hash of the compressed bytes. Blob
identity allows deduplication within a session's history; the same IDs can also
be recognized across sessions. XXH3-128 collision resistance is assumed;
`verify_identity` is the extension point for a future byte-equality collision
check.

## Timeline records

Each `timeline.bin` payload is a serialized `Vec<Event>`. The vector is an atomic
candidate batch: all events in one record share a checksum and either all validate
or none are accepted.

Event variants appear in this wire order:

| Index | Event | Fields |
| --- | --- | --- |
| `0` | `Frame` | `kind`, `timestamp_ms`, `changed_tiles: Vec<(Coord, BlobId)>` |
| `1` | `Checkpoint` | `width`, `height`, `nx`, `ny`, `tiles: Vec<(Coord, BlobId)>` |
| `2` | `CanvasResize` | `kind`, `timestamp_ms`, `old_width`, `old_height`, `new_width`, `new_height`, `origin: Option<(i32, i32)>` |
| `3` | `Transform` | `timestamp_ms`, `transform` |

`Coord` is `(u32 x, u32 y)` in 256-pixel tile units. `FrameKind` is
`Stable = 0` and `Provisional = 1`.

### Frame

A frame lists only tiles whose blob IDs differ from the current state. Applying a
frame patches those tiles; it does not restate the whole canvas. Timestamps are
metadata only and do not determine playback duration.

### Checkpoint

A checkpoint contains the complete `nx * ny` tile grid for the current canvas.
Each entry stores a coordinate and blob ID, not pixel data. `nx` and `ny` must
equal `ceil(width / 256)` and `ceil(height / 256)`.

Checkpoints are written when any configured trigger fires:

- the canvas is resized;
- the number of changed candidates since the last checkpoint reaches
  `checkpoint-every-events` (default `500`);
- changed tiles reach `checkpoint-changed-ratio` percent of the current grid
  (default `70`).

Either count or ratio trigger can be disabled by setting it to `0`. Initial state
and resize always produce a checkpoint.

### CanvasResize

`CanvasResize` records a structural canvas-size change. A resize event is
immediately followed by a checkpoint in the same batch. The checkpoint is the new
state; a resize is not combined with a normal `Frame` delta unless it is the
initial capture (`old_width == 0`).

`origin` is `None` when SAI's resize origin is unknown. Replay centers the old
canvas in that case.

### Transform

`Transform` models `Rotate90CW`, `Rotate90CCW`, `Rotate180`,
`FlipHorizontal`, and `FlipVertical`. The current recorder does not detect
canvas transforms; the event exists for importers and future capture backends.

## Batch composition

| Situation | Timeline batch |
| --- | --- |
| Initial capture | `Frame` + `Checkpoint` |
| Canvas resize | `CanvasResize` + `Checkpoint` |
| Changed candidate | `Frame` + optional `Checkpoint` |
| Unchanged candidate | No record |

Only visible composite changes produce a frame. Candidate batches are written in
capture order, and each candidate must reference a complete tile state.

## Commit and recovery

The writer commits new blob records before the timeline batch that references
them. It flushes both files around each batch. `stats.json` is flushed
periodically (currently every 30 seconds) and during clean start/stop handling.

There is no `fsync` per event. This protects against incomplete process writes,
not arbitrary power-loss reordering by the filesystem or hardware.

On recorder open:

1. Every complete blob record is decoded, decompressed, and verified.
2. Timeline batches are streamed to rebuild the latest state.
3. Incomplete trailing records are truncated and ignored.
4. A complete but corrupt record is an error.
5. A timeline event that references a missing blob is an error.
6. Orphan blobs from an aborted capture remain valid for deduplication.

Read-only readers use a shared lock and refuse to open a session held by an active
recorder. They ignore incomplete tails but never truncate them. The `verify`
command uses recovery mode and may trim an incomplete tail.

## Limitations

- XXH3-128 is treated as a collision-resistant identifier, not a cryptographic
  signature or byte-equality proof.
- There is no persistent seek-offset index in v1. Startup scans files in
  sequence; checkpoints are complete state indexes for replay and future tooling.
- Capture is not an atomic snapshot while SAI is actively painting. Prefer idle
  moments for validation.
- Do not edit or copy a session while recording. Stop the recorder and back up
  the directory first.
