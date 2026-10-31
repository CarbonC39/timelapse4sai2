//! Lazy mapping from logical recording frames to output video frames.

use crate::recorder::{FrameKind, storage::Event};

const MAX_EXACT_COUNT: f64 = 9_007_199_254_740_992.0; // 2^53

#[derive(Debug, Clone, Copy)]
pub(super) struct Timing {
    pub(super) drawing_duration_s: f64,
    pub(super) frame_count: u64,
    pub(super) preview_count: u64,
    pub(super) crossfade_count: u64,
    pub(super) hold_count: u64,
}

pub(super) fn timing(
    selected_captures: u64,
    fps: u32,
    pace: f64,
    hold_end: f64,
    preview_end: f64,
) -> Result<Timing, String> {
    if !(1..=240).contains(&fps) {
        return Err("fps must be 1..240".into());
    }
    if !pace.is_finite() || !(0.1..=1000.0).contains(&pace) {
        return Err("pace must be in 0.1..=1000".into());
    }
    for (name, value) in [("hold_end", hold_end), ("preview_end", preview_end)] {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("{name} must be finite and nonnegative"));
        }
    }
    if selected_captures == 0 {
        return Err("at least one state is required".into());
    }

    let drawing_duration_s = selected_captures as f64 / pace;
    let frame_count = seconds_to_frames(drawing_duration_s, fps, "output frame count")?.max(1);
    let hold_count = seconds_to_frames(hold_end, fps, "hold_end")?;
    let preview_count = seconds_to_frames(preview_end, fps, "preview_end")?;
    let crossfade_count =
        seconds_to_frames(preview_end.min(0.5), fps, "crossfade frame count")?.min(preview_count);
    let final_count = hold_count
        .checked_add(preview_count)
        .filter(|&n| n <= MAX_EXACT_COUNT as u64)
        .ok_or_else(|| "final frame count is too large".to_string())?;
    frame_count
        .checked_add(final_count)
        .filter(|&n| n <= MAX_EXACT_COUNT as u64)
        .ok_or_else(|| "total output frame count is too large".to_string())?;

    Ok(Timing {
        drawing_duration_s,
        frame_count,
        preview_count,
        crossfade_count,
        hold_count,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    State(usize),
    Crossfade {
        first: usize,
        frame: u64,
        frames: u64,
    },
    Final,
}

#[derive(Debug, Clone)]
pub struct FrameSchedule {
    states: Vec<usize>,
    pace: f64,
    frame_count: u64,
    preview_count: u64,
    crossfade_count: u64,
    hold_count: u64,
}

impl FrameSchedule {
    pub fn new(
        states: Vec<usize>,
        fps: u32,
        pace: f64,
        hold_end: f64,
        preview_end: f64,
    ) -> Result<Self, String> {
        let computed = timing(states.len() as u64, fps, pace, hold_end, preview_end)?;

        Ok(Self {
            states,
            pace,
            frame_count: computed.frame_count,
            preview_count: computed.preview_count,
            crossfade_count: computed.crossfade_count,
            hold_count: computed.hold_count,
        })
    }

    pub fn len(&self) -> u64 {
        self.frame_count + self.preview_count + self.hold_count
    }

    #[cfg(test)]
    pub fn visual_frames(&self) -> u64 {
        self.frame_count
    }

    pub fn preview_frames(&self) -> u64 {
        self.preview_count
    }

    pub fn hold_frames(&self) -> u64 {
        self.hold_count
    }

    pub fn drawing_duration_s(&self) -> f64 {
        self.states.len() as f64 / self.pace
    }

    pub fn first_drawing_frame(&self) -> usize {
        if self.frame_count == 1 {
            *self
                .states
                .last()
                .expect("schedule requires at least one state")
        } else {
            self.states[0]
        }
    }

    #[cfg(test)]
    pub fn crossfade_frames(&self) -> u64 {
        self.crossfade_count
    }

    pub fn frames(&self) -> impl Iterator<Item = Target> + '_ {
        FrameIter {
            plan: self,
            position: 0,
        }
    }
}

struct FrameIter<'a> {
    plan: &'a FrameSchedule,
    position: u64,
}

impl Iterator for FrameIter<'_> {
    type Item = Target;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.plan.len() {
            return None;
        }
        let i = self.position;
        self.position += 1;
        if i < self.plan.preview_count {
            let crossfade_start = self.plan.preview_count - self.plan.crossfade_count;
            if i >= crossfade_start {
                return Some(Target::Crossfade {
                    first: self.plan.first_drawing_frame(),
                    frame: i - crossfade_start,
                    frames: self.plan.crossfade_count,
                });
            }
            return Some(Target::Final);
        }
        let i = i - self.plan.preview_count;
        if i >= self.plan.frame_count {
            return Some(Target::Final);
        }
        let n = self.plan.states.len() as u128;
        let count = self.plan.frame_count;
        let slot = if count <= 1 {
            (n - 1) as usize
        } else if n <= 1 {
            0
        } else {
            // Keep both endpoints while distributing intermediate samples
            // evenly. Integer arithmetic makes the schedule deterministic.
            ((u128::from(i) * (n - 1) + u128::from(count - 1) / 2) / u128::from(count - 1)) as usize
        };
        Some(Target::State(self.plan.states[slot]))
    }
}

fn checked_count(value: f64, name: &str) -> Result<u64, String> {
    if !value.is_finite() || value < 0.0 || value > MAX_EXACT_COUNT {
        return Err(format!("{name} is too large"));
    }
    Ok(value as u64)
}

fn seconds_to_frames(seconds: f64, fps: u32, name: &str) -> Result<u64, String> {
    let count = (seconds * f64::from(fps)).round();
    checked_count(count, name)
}

pub fn batch_kind(events: &[Event]) -> Option<FrameKind> {
    events.iter().find_map(|event| match event {
        Event::Frame { kind, .. } | Event::CanvasResize { kind, .. } => Some(*kind),
        Event::Transform { .. } => Some(FrameKind::Stable),
        Event::Checkpoint { .. } => None,
    })
}

pub fn include(kind: FrameKind, skip_provisional: bool) -> bool {
    !skip_provisional || kind == FrameKind::Stable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped(n: usize, fps: u32, pace: f64) -> Vec<Target> {
        FrameSchedule::new((0..n).collect(), fps, pace, 0.0, 0.0)
            .unwrap()
            .frames()
            .collect()
    }

    #[test]
    fn maps_logical_states_and_preserves_endpoints() {
        assert_eq!(
            mapped(4, 30, 30.0),
            vec![
                Target::State(0),
                Target::State(1),
                Target::State(2),
                Target::State(3)
            ]
        );
        assert_eq!(
            mapped(4, 60, 30.0),
            vec![
                Target::State(0),
                Target::State(0),
                Target::State(1),
                Target::State(1),
                Target::State(2),
                Target::State(2),
                Target::State(3),
                Target::State(3)
            ]
        );
        assert_eq!(
            mapped(4, 30, 60.0),
            vec![Target::State(0), Target::State(3)]
        );
        assert_eq!(mapped(4, 30, 15.0), mapped(4, 60, 30.0));
        assert_eq!(mapped(4, 1, 1000.0), vec![Target::State(3)]);
        assert_eq!(
            mapped(1, 60, 30.0),
            vec![Target::State(0), Target::State(0)]
        );
    }

    #[test]
    fn pace_sets_duration_independently_of_output_fps() {
        assert_eq!(
            mapped(4, 60, 60.0),
            vec![
                Target::State(0),
                Target::State(1),
                Target::State(2),
                Target::State(3)
            ]
        );
        assert_eq!(
            mapped(4, 30, 60.0),
            vec![Target::State(0), Target::State(3)]
        );
        let thirty = FrameSchedule::new(vec![0, 1, 2, 3], 30, 60.0, 0.0, 0.0).unwrap();
        let sixty = FrameSchedule::new(vec![0, 1, 2, 3], 60, 60.0, 0.0, 0.0).unwrap();
        assert_eq!((thirty.visual_frames(), sixty.visual_frames()), (2, 4));
        assert_eq!(thirty.drawing_duration_s(), 4.0 / 60.0);
        assert_eq!(sixty.drawing_duration_s(), thirty.drawing_duration_s());

        let thirty = FrameSchedule::new(vec![0; 900], 30, 15.0, 0.0, 0.0).unwrap();
        let sixty = FrameSchedule::new(vec![0; 900], 60, 15.0, 0.0, 0.0).unwrap();
        assert_eq!(thirty.visual_frames(), 1800);
        assert_eq!(sixty.visual_frames(), 3600);
        assert_eq!(thirty.drawing_duration_s(), 60.0);
        assert_eq!(sixty.drawing_duration_s(), 60.0);
    }

    #[test]
    fn durations_and_final_holds_are_output_fps_based() {
        let p = FrameSchedule::new(vec![8, 12], 30, 30.0, 0.1, 0.2).unwrap();
        assert_eq!(p.len(), 11); // 6 preview + 2 mapped + 3 hold frames
        assert_eq!(
            p.frames().collect::<Vec<_>>(),
            [
                (0..6)
                    .map(|frame| Target::Crossfade {
                        first: 8,
                        frame,
                        frames: 6
                    })
                    .collect(),
                vec![Target::State(8), Target::State(12)],
                vec![Target::Final; 3],
            ]
            .concat()
        );
        let p = FrameSchedule::new(vec![8], 60, 30.0, 0.1, 0.2).unwrap();
        assert_eq!(p.len(), 20);
        let p = FrameSchedule::new(vec![0, 1, 2], 1, 30.0, 0.0, 0.0).unwrap();
        assert_eq!(p.frames().collect::<Vec<_>>(), vec![Target::State(2)]);
    }

    #[test]
    fn fractional_hold_and_preview_durations_round_to_nearest_output_frame() {
        let p = FrameSchedule::new(vec![0], 30, 30.0, 0.05, 0.05).unwrap();
        assert_eq!(p.preview_frames(), 2);
        assert_eq!(p.hold_frames(), 2);
        assert_eq!(p.visual_frames(), 1);
        assert_eq!(p.len(), 5);
    }

    #[test]
    fn preview_crossfade_uses_only_the_preview_tail() {
        let p = FrameSchedule::new(vec![4, 9], 30, 15.0, 0.0, 3.0).unwrap();
        let frames = p.frames().collect::<Vec<_>>();
        assert_eq!(p.preview_frames(), 90);
        assert_eq!(p.crossfade_frames(), 15);
        assert_eq!(p.len(), p.visual_frames() + 90);
        assert_eq!(frames[..75], [Target::Final; 75]);
        assert_eq!(
            &frames[75..90],
            &(0..15)
                .map(|frame| Target::Crossfade {
                    first: 4,
                    frame,
                    frames: 15
                })
                .collect::<Vec<_>>()[..]
        );

        let short = FrameSchedule::new(vec![7, 11], 30, 15.0, 0.0, 0.2).unwrap();
        assert_eq!(short.preview_frames(), 6);
        assert_eq!(short.crossfade_frames(), 6);
        assert!(
            short
                .frames()
                .take(6)
                .all(|target| matches!(target, Target::Crossfade { .. }))
        );
    }

    #[test]
    fn first_drawing_state_accounts_for_mapping_and_metadata_filtering() {
        let no_preview = FrameSchedule::new(vec![2, 5, 9], 30, 15.0, 0.0, 0.0).unwrap();
        assert_eq!(no_preview.frames().next(), Some(Target::State(2)));
        assert_eq!(no_preview.first_drawing_frame(), 2);

        // State IDs are the indices retained after filtering metadata batches.
        let retained_states = [0, 2, 5];
        let several = FrameSchedule::new(retained_states.to_vec(), 30, 15.0, 0.0, 0.0).unwrap();
        assert_eq!(several.first_drawing_frame(), 0);

        // A one-frame drawing schedule selects the final endpoint as its only State.
        let one = FrameSchedule::new(vec![3, 8], 1, 1000.0, 0.0, 0.0).unwrap();
        assert_eq!(one.visual_frames(), 1);
        assert_eq!(one.first_drawing_frame(), 8);
        assert_eq!(one.frames().next(), Some(Target::State(8)));
    }

    #[test]
    fn validates_numeric_inputs_and_counts() {
        for (fps, pace, hold, preview) in [
            (0, 1.0, 0.0, 0.0),
            (241, 1.0, 0.0, 0.0),
            (30, 0.0, 0.0, 0.0),
            (30, f64::NAN, 0.0, 0.0),
            (30, 1000.1, 0.0, 0.0),
            (30, 1.0, -1.0, 0.0),
            (30, 1.0, 0.0, f64::INFINITY),
        ] {
            assert!(FrameSchedule::new(vec![0], fps, pace, hold, preview).is_err());
        }
        assert!(FrameSchedule::new(vec![], 30, 30.0, 0.0, 0.0).is_err());
        assert!(FrameSchedule::new(vec![0], 30, 30.0, f64::MAX, 0.0).is_err());
    }

    #[test]
    fn filters_provisional_frames_and_finds_batch_kind() {
        assert!(include(FrameKind::Provisional, false));
        assert!(!include(FrameKind::Provisional, true));
        assert!(include(FrameKind::Stable, true));
        let cp = Event::Checkpoint {
            width: 1,
            height: 1,
            nx: 1,
            ny: 1,
            tiles: vec![],
        };
        assert_eq!(batch_kind(&[cp]), None);
        let event = Event::Frame {
            kind: FrameKind::Provisional,
            timestamp_ms: 0,
            changed_tiles: vec![],
        };
        assert_eq!(batch_kind(&[event]), Some(FrameKind::Provisional));
        let transform = Event::Transform {
            timestamp_ms: 1,
            transform: crate::recorder::storage::Transform::Rotate180,
        };
        assert_eq!(batch_kind(&[transform]), Some(FrameKind::Stable));
    }
}
