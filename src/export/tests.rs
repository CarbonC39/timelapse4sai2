use super::*;
use crate::recorder::{
    Config, FrameKind, TILE_BYTES,
    storage::{Manifest, Session, Store},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use xxhash_rust::xxh3::xxh3_128;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "timelapse4sai2-export-test-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn session(&self) {
        let manifest = Manifest {
            format_version: 1,
            sai_profile: "test".into(),
            sai_generation: None,
            source_executable_md5: "test-md5".into(),
            initial_width: 257,
            initial_height: 1,
            canvas_name: "fixture".into(),
            latest_canvas_name: None,
            latest_canvas_path: None,
            canvas_path: "/fixture".into(),
            tile_size: 256,
            channel_order: "BGRX".into(),
            creation_time_unix_ms: 0,
            recorder_version: "test".into(),
            identity_limit: "test".into(),
        };
        let mut store = Store::open(self.path(), manifest, Config::default()).unwrap();

        let initial_a = tile(10, 20, 30);
        let initial_b = tile(40, 50, 60);
        begin(&mut store, FrameKind::Stable, 257, 1);
        write_tile(&mut store, (0, 0), &initial_a);
        write_tile(&mut store, (1, 0), &initial_b);
        store.end().unwrap();

        // This provisional delta must still be replayed when its batch is skipped.
        let provisional_a = tile(70, 80, 90);
        begin(&mut store, FrameKind::Provisional, 257, 1);
        write_tile(&mut store, (0, 0), &provisional_a);
        store.end().unwrap();

        let stable_b = tile(100, 110, 120);
        begin(&mut store, FrameKind::Stable, 257, 1);
        write_tile(&mut store, (1, 0), &stable_b);
        store.end().unwrap();

        // Resizing forces a full new tile grid and a checkpoint.
        let resized_a = tile(130, 140, 150);
        let resized_b = tile(160, 170, 180);
        begin(&mut store, FrameKind::Stable, 512, 1);
        write_tile(&mut store, (0, 0), &resized_a);
        write_tile(&mut store, (1, 0), &resized_b);
        store.end().unwrap();

        let final_a = tile(190, 200, 210);
        begin(&mut store, FrameKind::Provisional, 512, 1);
        write_tile(&mut store, (0, 0), &final_a);
        store.end().unwrap();
        drop(store);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tile(b: u8, g: u8, r: u8) -> Vec<u8> {
    let mut raw = vec![0; TILE_BYTES];
    for px in raw.chunks_exact_mut(4) {
        px.copy_from_slice(&[b, g, r, 0]);
    }
    raw
}

fn begin(store: &mut Store, kind: FrameKind, width: u32, height: u32) {
    store.begin(kind, 0, width, height);
}

fn write_tile(store: &mut Store, coord: (u32, u32), raw: &[u8]) {
    store.tile(coord, xxh3_128(raw), raw).unwrap();
}

#[test]
fn scan_filters_states_but_replay_keeps_skipped_provisional_delta() {
    let fixture = Fixture::new();
    fixture.session();
    let mut session = Session::open(fixture.path()).unwrap();

    let all = scan(&mut session, false).unwrap();
    assert_eq!(all.states, vec![0, 1, 2, 3, 4]);
    assert_eq!((all.max_width, all.max_height), (512, 1));

    let stable_only = scan(&mut session, true).unwrap();
    assert_eq!(stable_only.states, vec![0, 2, 3]);

    let mut replay = Replay::new(257, 1).unwrap();
    session.seek(0).unwrap();
    for _ in 0..=stable_only.states[1] {
        let events = session.next_batch().unwrap().unwrap();
        apply_batch(&mut replay, &events, &mut session).unwrap();
    }
    assert_eq!((replay.width, replay.height), (257, 1));
    // Stable batch 2 only changes tile 1; its tile 0 must inherit batch 1's
    // provisional update even though batch 1 is omitted from selected states.
    assert_eq!(&replay.rgb[0..3], &[90, 80, 70]);
    assert_eq!(&replay.rgb[256 * 3..256 * 3 + 3], &[120, 110, 100]);
}

#[test]
fn checkpoint_seek_final_replay_matches_full_sequential_replay() {
    let fixture = Fixture::new();
    fixture.session();
    let mut session = Session::open(fixture.path()).unwrap();
    let scanned = scan(&mut session, false).unwrap();

    let from_checkpoint = final_replay(&mut session, &scanned).unwrap();
    assert_eq!((from_checkpoint.width, from_checkpoint.height), (512, 1));
    assert_eq!(&from_checkpoint.rgb[0..3], &[210, 200, 190]);

    session.seek(0).unwrap();
    let mut sequential = Replay::new(257, 1).unwrap();
    while let Some(events) = session.next_batch().unwrap() {
        apply_batch(&mut sequential, &events, &mut session).unwrap();
    }
    assert_eq!(from_checkpoint.width, sequential.width);
    assert_eq!(from_checkpoint.height, sequential.height);
    assert_eq!(from_checkpoint.rgb, sequential.rgb);
}

#[test]
fn session_lock_blocks_writer_and_readonly_open_preserves_incomplete_tail() {
    let fixture = Fixture::new();
    fixture.session();
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(fixture.path().join("manifest.json")).unwrap()).unwrap();
    let writer = Store::open(fixture.path(), manifest, Config::default()).unwrap();
    assert!(Session::open(fixture.path()).is_err());
    drop(writer);

    let timeline = fixture.path().join("timeline.bin");
    let mut file = OpenOptions::new().append(true).open(&timeline).unwrap();
    file.write_all(&[0xAA, 0xBB, 0xCC]).unwrap();
    file.flush().unwrap();
    drop(file);
    let size_before = fs::metadata(&timeline).unwrap().len();

    let mut session = Session::open(fixture.path()).unwrap();
    let mut batches = 0;
    while session.next_batch().unwrap().is_some() {
        batches += 1;
    }
    assert_eq!(batches, 5);
    drop(session);
    assert_eq!(fs::metadata(&timeline).unwrap().len(), size_before);
}

#[test]
fn public_plan_reports_filtered_captures_exact_timing_and_dimensions() {
    let fixture = Fixture::new();
    fixture.session();
    let mut session = Session::open(fixture.path()).unwrap();
    let options = Options {
        skip_provisional: true,
        encoder: super::Encoder::X264,
        hold_end: 0.05,
        preview_end: 0.05,
        ..Options::default()
    };
    let prepared = super::prepare(&mut session, &options).unwrap();
    assert_eq!(
        prepared.schedule.frames().count() as u64,
        prepared.plan.output_video_frames
    );
    let plan = prepared.plan;
    assert_eq!(plan.source_captures, 5);
    assert_eq!(plan.selected_captures, 3);
    assert_eq!(plan.output_video_frames, 7); // 3 visuals + 2 preview + 2 hold
    assert_eq!(plan.duration_ms, 233);
    assert_eq!((plan.output_width, plan.output_height), (512, 2));
    assert_eq!((plan.fps, plan.speed), (30, 1.0));
    assert_eq!((plan.preview_end_ms, plan.hold_end_ms), (67, 67));
}
