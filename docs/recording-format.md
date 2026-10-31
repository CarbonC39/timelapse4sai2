# Recording format 1

Session directory:

- `manifest.json`: readable metadata, profile, source executable MD5, initial dimensions,
  canvas name/path, tile/channel encoding, creation Unix milliseconds, recorder version,
  document identity limitations.
- `blobs.bin`: append-only records of `Blob { id: u128, compressed: Vec<u8> }`.
  Each decoded zstd payload is exactly 262144 bytes of canonical BGRX tile data.
- `timeline.bin`: append-only records of `Vec<Event>`; vector is an atomic candidate batch.
- `session.lock`: advisory exclusive writer lock held for session lifetime.

Every record is `u32 payload_length` + `u128 XXH3(payload)` + payload, all header integers
little endian. Payload is bincode 1.3 default serialization (little endian, fixed integers).
Maximum payload length is 64 MiB. There is no file header; version is in manifest.
Rust field/variant order is defined by `src/recorder/storage.rs`, `Event` and `Blob`.
Blob ID is XXH3-128 of the canonical uncompressed bytes, not of compressed bytes.
Coordinate is `(u32 x, u32 y)` in tile units. Hash collision resistance is assumed,
not cryptographic integrity or byte-equality collision detection.

Event variants in wire order:

0. Frame: kind (`Stable=0`, `Provisional=1`), Unix timestamp milliseconds, changed coordinates and blob IDs.
1. Checkpoint: width, height, nx, ny (u32), complete coordinate/blob index. No repeated pixels.
2. CanvasResize: kind, Unix timestamp milliseconds, old/new width/height (u32), optional `(i32 origin_x, i32 origin_y)`.
   `None` means unknown. Immediately followed by a forced checkpoint in the same record.
3. Transform: timestamp and transform enum (`Rotate90CW`, `Rotate90CCW`, `Rotate180`,
   `FlipHorizontal`, `FlipVertical`). Model only; recorder does not detect these.

Initial Frame and checkpoint are one batch. Resize emits a structural event and checkpoint,
not a normal tile delta. Other changed candidates emit Frame plus optional checkpoint.
Unchanged candidates write no timeline record. Timeline wall time does not define playback duration.
Checkpoint frequency counts changed candidates since the last checkpoint; ratio is changed tile
count / current grid size, inclusive of threshold. Either trigger may be independently disabled.

Writer completes all new blob records before writing the referring timeline batch.
No fsync per event: this protects against incomplete process writes, not arbitrary power-loss
reordering by filesystem/hardware. A loss of blob data referenced by a complete event is an error.
On open, decode and verify every complete blob record; stream timeline batches to rebuild the
last state. Incomplete final records are ignored and truncated before further append. A complete
record with bad checksum, bad zstd/hash/length, or a missing referenced blob is an explicit error.
A killed capture can leave valid orphan blobs, which remain available for deduplication.
No history images are retained during load. Startup currently scans the files in sequence;
checkpoint records provide complete seek indexes for future exporters, but there is no separate
persistent seek-offset index in v1. `Store::open` is the recovery entry point.

Do not edit or copy a session while recording. Back up the directory after stopping.
