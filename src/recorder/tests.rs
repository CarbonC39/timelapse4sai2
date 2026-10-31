use super::*;
use crate::{
    recorder::storage::Event,
    sai::{builds::preview_2026_07_02b::PROFILE, reader::Reader, reader::tests as fixture},
};
use std::{fs, path::PathBuf, sync::mpsc};

fn dir(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "sai-recorder-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    path
}

fn manifest() -> Manifest {
    Manifest {
        format_version: 1,
        sai_profile: "test".into(),
        sai_generation: None,
        source_executable_md5: "test".into(),
        initial_width: 257,
        initial_height: 1,
        canvas_name: "synthetic".into(),
        latest_canvas_name: None,
        latest_canvas_path: None,
        canvas_path: "synthetic.sai".into(),
        tile_size: 256,
        channel_order: "BGRX".into(),
        creation_time_unix_ms: 0,
        recorder_version: "test".into(),
        identity_limit: "test".into(),
    }
}

#[test]
fn cancellable_recording_drains_initial_capture_and_reports_final_stats() {
    use std::sync::{Arc, atomic::AtomicBool};
    let mem = fixture::fixture();
    let (reader, canvas) = fixture_reader(&mem);
    let path = dir("cancel");
    let cancel = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::channel();
    let config = Config {
        no_input_watch: true,
        ..Config::default()
    };
    let metrics = record_cancellable(&reader, &canvas, &path, config, &cancel, move |s| {
        let _ = tx.send(s);
    })
    .unwrap();
    assert_eq!(metrics.logical_frames, 1);
    assert!(
        rx.try_iter()
            .last()
            .is_some_and(|s| s.captures == 1 && s.disk_bytes > 0)
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn latest_canvas_metadata_survives_reopen_and_summary_uses_it() {
    let path = dir("latest-metadata");
    {
        let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
        store.update_latest_canvas_metadata("Renamed canvas".into(), "D:/work/new.sai".into()).unwrap();
    }
    let session = storage::Session::open(&path).unwrap();
    assert_eq!(session.manifest.canvas_name, "synthetic");
    assert_eq!(session.manifest.canvas_path, "synthetic.sai");
    assert_eq!(session.manifest.effective_canvas_name(), "Renamed canvas");
    assert_eq!(session.manifest.effective_canvas_path(), "D:/work/new.sai");
    assert_eq!(session.summary().unwrap().canvas_name, "Renamed canvas");
    drop(session);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn changed_build_warns_and_different_saved_canvas_requires_override() {
    use std::sync::{atomic::AtomicBool, Arc};

    let mem = fixture::fixture();
    let base_reader = Reader {
        resolved_root_rva: None,
        source_hash: Some("changed-build"),
        memory: &mem,
        base: fixture::BASE,
        profile: &PROFILE,
    };
    let canvas = base_reader.canvases().unwrap().remove(0);
    let path = dir("matching");
    {
        let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
        store.update_canvas_metadata(&canvas).unwrap();
    }
    let cancel = Arc::new(AtomicBool::new(true));
    let metrics = record_cancellable_with_match(
        &base_reader,
        &canvas,
        &path,
        Config { no_input_watch: true, ..Config::default() },
        &cancel,
        42,
        None,
        |_, _| false,
        |_| {},
    ).unwrap();
    assert!(metrics.warnings.iter().any(|warning| warning.contains("different SAI2 build")));

    let mut wrong_canvas = canvas.clone();
    wrong_canvas.short_path = "D:/elsewhere/other.sai".into();
    {
        let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
        store.update_canvas_metadata(&wrong_canvas).unwrap();
    }
    let rejected = record_cancellable(
        &base_reader,
        &canvas,
        &path,
        Config { no_input_watch: true, ..Config::default() },
        &cancel,
        |_| {},
    );
    assert!(rejected.unwrap_err().contains("--allow-canvas-mismatch"));

    let allowed = record_cancellable(
        &base_reader,
        &canvas,
        &path,
        Config { no_input_watch: true, allow_canvas_mismatch: true, ..Config::default() },
        &cancel,
        |_| {},
    );
    assert!(allowed.is_ok());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn same_live_canvas_updates_metadata_across_unsaved_save_and_save_as() {
    use std::sync::{atomic::AtomicBool, Arc};

    let mut mem = fixture::fixture();
    mem.utf16(fixture::CANVAS + 0x2ac, "Untitled\0");
    mem.put(fixture::CANVAS + 0x4ac, &[0, 0]);
    let path = dir("save-rounds");
    let cancel = Arc::new(AtomicBool::new(true));
    let config = Config { no_input_watch: true, ..Config::default() };
    let identity = {
        let (reader, canvas) = fixture_reader(&mem);
        let identity = matching::LiveCanvasIdentity::new(42, &canvas);
        record_cancellable_with_match(
            &reader, &canvas, &path, config.clone(), &cancel, 42, None,
            |_, _| panic!("first recording should not request authorization"), |_| {},
        ).unwrap();
        identity
    };

    mem.utf16(fixture::CANVAS + 0x2ac, "Project\0");
    mem.utf16(fixture::CANVAS + 0x4ac, "D:/work/project.sai\0");
    {
        let (reader, canvas) = fixture_reader(&mem);
        record_cancellable_with_match(
            &reader, &canvas, &path, config.clone(), &cancel, 42, Some(identity),
            |_, _| panic!("same live canvas should not request authorization"), |_| {},
        ).unwrap();
    }

    mem.utf16(fixture::CANVAS + 0x2ac, "Renamed\0");
    mem.utf16(fixture::CANVAS + 0x4ac, "D:/work/renamed.sai\0");
    {
        let (reader, canvas) = fixture_reader(&mem);
        record_cancellable_with_match(
            &reader, &canvas, &path, config, &cancel, 42, Some(identity),
            |_, _| panic!("same live canvas should not request authorization"), |_| {},
        ).unwrap();
    }

    let session = storage::Session::open(&path).unwrap();
    assert_eq!(session.manifest.canvas_name, "Untitled");
    assert_eq!(session.manifest.canvas_path, "");
    assert_eq!(session.manifest.effective_canvas_name(), "Renamed");
    assert_eq!(session.manifest.effective_canvas_path(), "D:/work/renamed.sai");
    drop(session);
    fs::remove_dir_all(path).unwrap();
}

fn fixture_reader(
    mem: &fixture::FakeMemory,
) -> (Reader<'_, fixture::FakeMemory>, crate::sai::canvas::Canvas) {
    let reader = Reader {
        resolved_root_rva: None,
        source_hash: None,
        memory: mem,
        base: fixture::BASE,
        profile: &PROFILE,
    };
    let canvas = reader.canvases().unwrap().remove(0);
    (reader, canvas)
}

fn apply(store: &mut Store, rx: &mpsc::Receiver<Message>) {
    while let Ok(msg) = rx.try_recv() {
        match msg {
            Message::Begin(k, t, w, h) => store.begin(k, t, w, h),
            Message::Tile(c, id, raw) => store.tile(c, id, &raw).unwrap(),
            Message::End => store.end().unwrap(),
            Message::Abort => store.abort(),
            Message::Metadata(canvas) => store.update_canvas_metadata(&canvas).unwrap(),
        }
    }
}

fn persisted_events(path: &std::path::Path) -> Vec<Event> {
    let bytes = fs::read(path.join("timeline.bin")).unwrap();
    let mut offset = 0;
    let mut out = Vec::new();
    while offset + 20 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if offset + 20 + len > bytes.len() {
            break;
        }
        let data = &bytes[offset + 20..offset + 20 + len];
        assert_eq!(
            xxhash_rust::xxh3::xxh3_128(data),
            u128::from_le_bytes(bytes[offset + 4..offset + 20].try_into().unwrap())
        );
        out.extend(bincode::deserialize::<Vec<Event>>(data).unwrap());
        offset += 20 + len;
    }
    out
}

fn capture_once(
    mem: &fixture::FakeMemory,
    capture: &mut CaptureState,
    store: &mut Store,
    kind: FrameKind,
) {
    let (reader, canvas) = fixture_reader(mem);
    let (tx, rx) = mpsc::sync_channel(16);
    capture
        .capture(&reader, &canvas, kind, &tx, &mut Metrics::default())
        .unwrap();
    apply(store, &rx);
}

#[test]
fn initial_capture_stores_full_canvas_and_zeroes_unused_channels_and_padding() {
    let mut mem = fixture::fixture();
    // Noise in unused X bytes and outside the canvas must not affect identity.
    mem.put(fixture::TILE0 + 3, &[7]);
    mem.put(fixture::TILE0 + 4 * 10 + 3, &[9]);
    let (reader, canvas) = fixture_reader(&mem);
    let path = dir("initial");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let (tx, rx) = mpsc::sync_channel(16);
    let mut capture = CaptureState::default();
    let mut metrics = Metrics::default();
    capture
        .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
        .unwrap();
    apply(&mut store, &rx);
    assert_eq!(store.state.width, 257);
    assert_eq!(store.state.height, 1);
    assert_eq!(store.state.tiles.len(), 2);
    assert_eq!(metrics.tiles_scanned, 2);
    assert_eq!(metrics.raw_bytes_read, 2 * TILE_BYTES as u64);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn unchanged_candidate_emits_no_tiles_or_logical_frame() {
    let mem = fixture::fixture();
    let (reader, canvas) = fixture_reader(&mem);
    let path = dir("unchanged");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let (tx, rx) = mpsc::sync_channel(16);
    let mut capture = CaptureState::default();
    let mut metrics = Metrics::default();
    capture
        .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
        .unwrap();
    apply(&mut store, &rx);
    capture
        .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
        .unwrap();
    apply(&mut store, &rx);
    assert_eq!(metrics.capture_candidates, 2);
    assert_eq!(store.metrics.logical_frames, 1);
    assert_eq!(store.metrics.tiles_changed, 2);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn changed_tile_candidate_sends_only_the_changed_tile() {
    let mut mem = fixture::fixture();
    let path = dir("delta");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let (tx, rx) = mpsc::sync_channel(16);
    let mut capture = CaptureState::default();
    let mut metrics = Metrics::default();
    {
        let (reader, canvas) = fixture_reader(&mem);
        capture
            .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
            .unwrap();
        apply(&mut store, &rx);
    }
    mem.put(fixture::TILE1, &[91]);
    {
        let (reader, canvas) = fixture_reader(&mem);
        capture
            .capture(&reader, &canvas, FrameKind::Provisional, &tx, &mut metrics)
            .unwrap();
        let messages: Vec<_> = rx.try_iter().collect();
        assert!(matches!(
            messages.as_slice(),
            [Message::Begin(..), Message::Tile((1, 0), ..), Message::End, Message::Metadata(_)]
        ));
        for msg in messages {
            match msg {
                Message::Begin(k, t, w, h) => store.begin(k, t, w, h),
                Message::Tile(c, id, raw) => store.tile(c, id, &raw).unwrap(),
                Message::End => store.end().unwrap(),
                Message::Abort => store.abort(),
                Message::Metadata(canvas) => store.update_canvas_metadata(&canvas).unwrap(),
            }
        }
        assert_eq!(store.metrics.provisional_frames, 1);
        assert_eq!(canvas.width, 257);
    }
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn capture_failure_aborts_candidate_without_committing_state() {
    let mut mem = fixture::fixture();
    let path = dir("abort");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let (tx, rx) = mpsc::sync_channel(16);
    let mut capture = CaptureState::default();
    let mut metrics = Metrics::default();
    {
        let (reader, canvas) = fixture_reader(&mem);
        capture
            .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
            .unwrap();
    }
    apply(&mut store, &rx);
    let before = store.state.clone();
    // Break the second tile pointer after the candidate begins, forcing Abort.
    mem.ptr(fixture::ROW + 8, 0);
    let (reader, canvas) = fixture_reader(&mem);
    assert!(
        capture
            .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
            .is_err()
    );
    apply(&mut store, &rx);
    assert_eq!(store.state, before);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn missing_canvas_stops_capture_before_sending_a_candidate() {
    let mut mem = fixture::fixture();
    let canvas = {
        let (reader, canvas) = fixture_reader(&mem);
        let _ = reader;
        canvas
    };
    let (tx, rx) = mpsc::sync_channel(16);
    let mut capture = CaptureState::default();
    mem.ptr(fixture::SENTINEL, 0);
    let reader = Reader {
        resolved_root_rva: None,
        source_hash: None,
        memory: &mem,
        base: fixture::BASE,
        profile: &PROFILE,
    };
    assert!(
        capture
            .capture(
                &reader,
                &canvas,
                FrameKind::Stable,
                &tx,
                &mut Metrics::default()
            )
            .unwrap_err()
            .contains("closed or replaced")
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn resize_replaces_grid_and_commits_checkpoint() {
    let mut mem = fixture::fixture();
    let path = dir("resize");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let (tx, rx) = mpsc::sync_channel(16);
    let mut capture = CaptureState::default();
    let mut metrics = Metrics::default();
    {
        let (reader, canvas) = fixture_reader(&mem);
        capture
            .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
            .unwrap();
    }
    apply(&mut store, &rx);
    mem.u32(fixture::CANVAS + 0x250, 1);
    mem.u32(fixture::MAP + 0x10, 1);
    mem.u32(fixture::MAP + 0x18, 1);
    let (reader, canvas) = fixture_reader(&mem);
    capture
        .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
        .unwrap();
    apply(&mut store, &rx);
    assert_eq!((store.state.width, store.state.height), (1, 1));
    assert_eq!(store.state.tiles.len(), 1);
    assert_eq!(store.metrics.checkpoint_count, 2);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn duplicate_tile_content_is_deduplicated_in_storage() {
    let mut mem = fixture::fixture();
    mem.put(fixture::TILE1, &[30, 20, 10, 0]);
    // Both first pixels equal after X normalization; the rest of the tile is zero.
    mem.put(fixture::TILE1 + 4, &vec![0; TILE_BYTES - 4]);
    mem.put(fixture::TILE0 + 4, &vec![0; TILE_BYTES - 4]);
    let (reader, canvas) = fixture_reader(&mem);
    let path = dir("dedup");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let (tx, rx) = mpsc::sync_channel(16);
    let mut capture = CaptureState::default();
    capture
        .capture(
            &reader,
            &canvas,
            FrameKind::Stable,
            &tx,
            &mut Metrics::default(),
        )
        .unwrap();
    apply(&mut store, &rx);
    assert_eq!(store.metrics.unique_blobs, 1);
    assert_eq!(store.metrics.deduplicated_blobs, 1);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn reopening_truncates_incomplete_timeline_tail_and_recovers_state() {
    let mem = fixture::fixture();
    let (reader, canvas) = fixture_reader(&mem);
    let path = dir("recovery");
    {
        let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
        let (tx, rx) = mpsc::sync_channel(16);
        CaptureState::default()
            .capture(
                &reader,
                &canvas,
                FrameKind::Stable,
                &tx,
                &mut Metrics::default(),
            )
            .unwrap();
        apply(&mut store, &rx);
    }
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path.join("timeline.bin"))
        .unwrap();
    file.write_all(&[8, 0, 0]).unwrap();
    drop(file);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path.join("blobs.bin"))
        .unwrap();
    file.write_all(&[8, 0, 0]).unwrap();
    drop(file);
    let reopened = Store::open(&path, manifest(), Config::default()).unwrap();
    assert_eq!(reopened.state.width, 257);
    assert_eq!(reopened.state.tiles.len(), 2);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn undo_sequence_reuses_previous_blob_and_records_each_delta() {
    let mut mem = fixture::fixture();
    let path = dir("undo");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    let a = store.state.tiles[&(0, 0)];
    mem.put(fixture::TILE0, &[31]);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Provisional);
    let b = store.state.tiles[&(0, 0)];
    assert_ne!(a, b);
    mem.put(fixture::TILE0, &[32]);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    let c = store.state.tiles[&(0, 0)];
    assert_ne!(b, c);
    mem.put(fixture::TILE0, &[31]);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    assert_eq!(store.state.tiles[&(0, 0)], b);
    assert!(store.metrics.deduplicated_blobs > 0);
    assert_eq!(store.metrics.logical_frames, 4);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn multi_tile_delta_persists_both_changed_coordinates() {
    let mut mem = fixture::fixture();
    let path = dir("multi-delta");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    mem.put(fixture::TILE0, &[31]);
    mem.put(fixture::TILE1, &[91]);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Provisional);
    let events = persisted_events(&path);
    assert!(events.iter().any(|e|matches!(e,Event::Frame{kind:FrameKind::Provisional,changed_tiles,..} if changed_tiles.iter().map(|(c,_)|*c).collect::<Vec<_>>()==vec![(0,0),(1,0)])));
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn checkpoint_zero_disables_both_triggers_and_periodic_and_ratio_triggers_work() {
    let mut mem = fixture::fixture();
    let path = dir("checkpoint-zero");
    let config = Config {
        checkpoint_every_events: 0,
        checkpoint_changed_ratio: 0,
        ..Config::default()
    };
    let mut store = Store::open(&path, manifest(), config).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    let initial_checkpoints = persisted_events(&path)
        .iter()
        .filter(|e| matches!(e, Event::Checkpoint { .. }))
        .count();
    mem.put(fixture::TILE0, &[31]);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    assert_eq!(
        persisted_events(&path)
            .iter()
            .filter(|e| matches!(e, Event::Checkpoint { .. }))
            .count(),
        initial_checkpoints
    );
    drop(store);
    fs::remove_dir_all(&path).unwrap();

    let path = dir("checkpoint-periodic");
    let config = Config {
        checkpoint_every_events: 2,
        checkpoint_changed_ratio: 0,
        ..Config::default()
    };
    let mut store = Store::open(&path, manifest(), config).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    assert!(
        persisted_events(&path)
            .iter()
            .any(|e| matches!(e, Event::Checkpoint { .. }))
    );
    drop(store);
    fs::remove_dir_all(&path).unwrap();

    let path = dir("checkpoint-ratio");
    let config = Config {
        checkpoint_every_events: 0,
        checkpoint_changed_ratio: 50,
        ..Config::default()
    };
    let mut store = Store::open(&path, manifest(), config).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    assert!(
        persisted_events(&path)
            .iter()
            .any(|e| matches!(e, Event::Checkpoint { .. }))
    );
    drop(store);
    fs::remove_dir_all(&path).unwrap();
}

#[test]
fn persisted_frame_events_keep_stable_and_provisional_kinds() {
    let mut mem = fixture::fixture();
    let path = dir("frame-kinds");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    mem.put(fixture::TILE0, &[31]);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Provisional);
    let kinds: Vec<_> = persisted_events(&path)
        .into_iter()
        .filter_map(|e| {
            if let Event::Frame { kind, .. } = e {
                Some(kind)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(kinds, vec![FrameKind::Stable, FrameKind::Provisional]);
    drop(store);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn resize_persists_resize_and_checkpoint_events() {
    let mut mem = fixture::fixture();
    let path = dir("resize-events");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    mem.u32(fixture::CANVAS + 0x250, 1);
    mem.u32(fixture::MAP + 0x10, 1);
    mem.u32(fixture::MAP + 0x18, 1);
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    let events = persisted_events(&path);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::CanvasResize {
            old_width: 257,
            new_width: 1,
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::Checkpoint{width:1,nx:1,tiles,..} if tiles.len()==1))
    );
    drop(store);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn reopening_reports_corrupt_blob_record_and_missing_referenced_blob() {
    let mem = fixture::fixture();
    let path = dir("corrupt-blob");
    let mut store = Store::open(&path, manifest(), Config::default()).unwrap();
    let mut capture = CaptureState::default();
    capture_once(&mem, &mut capture, &mut store, FrameKind::Stable);
    drop(store);
    let blob_path = path.join("blobs.bin");
    let mut bytes = fs::read(&blob_path).unwrap();
    bytes[20] ^= 0x40;
    fs::write(&blob_path, &bytes).unwrap();
    assert!(
        Store::open(&path, manifest(), Config::default())
            .err()
            .unwrap()
            .contains("record checksum mismatch")
    );
    // A valid but truncated blob tail is ignored; timeline references then fail clearly.
    fs::write(&blob_path, []).unwrap();
    assert!(
        Store::open(&path, manifest(), Config::default())
            .err()
            .unwrap()
            .contains("missing referenced blob")
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn both_profiles_share_capture_storage_and_offline_replay() {
    use crate::sai::builds::{preview_2026_07_02b, renovated_2026_10_19};
    let mut states = Vec::new();
    for (profile, mut mem) in [
        (&preview_2026_07_02b::PROFILE, fixture::fixture()),
        (&renovated_2026_10_19::PROFILE, fixture::renovated_fixture()),
    ] {
        let path = dir(profile.generation.name());
        let mut capture = CaptureState::default();
        let (tx, rx) = mpsc::sync_channel(16);
        let mut metrics = Metrics::default();
        let mut store = {
            let reader = Reader {
                resolved_root_rva: None,
                source_hash: None,
                memory: &mem,
                base: fixture::BASE,
                profile,
            };
            let canvas = reader.canvases().unwrap().remove(0);
            let manifest = recording_manifest(&reader, &canvas);
            assert_eq!(manifest.sai_profile, profile.display_name);
            assert_eq!(manifest.source_executable_md5, profile.executable_hash);
            assert_eq!(
                manifest.sai_generation.as_deref(),
                Some(profile.generation.name())
            );
            Store::open(&path, manifest, Config::default()).unwrap()
        };
        for step in 0..3 {
            if step == 2 {
                mem.put(fixture::TILE1, &[91]);
            }
            let reader = Reader {
                resolved_root_rva: None,
                source_hash: None,
                memory: &mem,
                base: fixture::BASE,
                profile,
            };
            let canvas = reader.canvases().unwrap().remove(0);
            capture
                .capture(&reader, &canvas, FrameKind::Stable, &tx, &mut metrics)
                .unwrap();
            apply(&mut store, &rx);
        }
        assert_eq!(store.metrics.logical_frames, 2);
        assert_eq!(store.metrics.tiles_changed, 3);
        states.push(store.state.clone());
        drop(store);
        storage::verify(&path).unwrap();
        let mut session = storage::Session::open(&path).unwrap();
        assert!(session.next_batch().unwrap().is_some());
        let raw = session
            .load_blob(states.last().unwrap().tiles[&(1, 0)])
            .unwrap();
        assert_eq!(&raw[..4], &[91, 80, 70, 0]);
        drop(session);
        // Existing v1 manifests without generation remain readable.
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("sai_generation");
        fs::write(
            path.join("manifest.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        assert!(
            storage::Session::open(&path)
                .unwrap()
                .manifest
                .sai_generation
                .is_none()
        );
        fs::remove_dir_all(path).unwrap();
    }
    assert_eq!(states[0], states[1]);
}
