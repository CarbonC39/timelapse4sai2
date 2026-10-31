//! Lazy mapping from logical recording frames to output video frames.

use crate::recorder::{FrameKind, storage::Event};

const LOGICAL_FPS: f64 = 30.0;
const MAX_EXACT_COUNT: f64 = 9_007_199_254_740_992.0; // 2^53

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    State(usize),
    Final,
}

#[derive(Debug, Clone)]
pub struct Plan {
    states: Vec<usize>,
    frame_count: u64,
    preview_count: u64,
    hold_count: u64,
}

impl Plan {
    pub fn new(
        states: Vec<usize>,
        fps: u32,
        speed: f64,
        hold_end: f64,
        preview_end: f64,
    ) -> Result<Self, String> {
        if !(1..=240).contains(&fps) {
            return Err("fps must be 1..240".into());
        }
        if !speed.is_finite() || speed <= 0.0 {
            return Err("speed must be finite and positive".into());
        }
        for (name, value) in [("hold_end", hold_end), ("preview_end", preview_end)] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and nonnegative"));
            }
        }
        if states.is_empty() {
            return Err("at least one state is required".into());
        }

        let n = states.len() as f64;
        let raw_count = (n * f64::from(fps) / (LOGICAL_FPS * speed)).ceil().max(1.0);
        let frame_count = checked_count(raw_count, "output frame count")?;
        let hold_count = seconds_to_frames(hold_end, fps, "hold_end")?;
        let preview_count = seconds_to_frames(preview_end, fps, "preview_end")?;
        let final_count = hold_count
            .checked_add(preview_count)
            .filter(|&n| n <= MAX_EXACT_COUNT as u64)
            .ok_or_else(|| "final frame count is too large".to_string())?;
        frame_count
            .checked_add(final_count)
            .filter(|&n| n <= MAX_EXACT_COUNT as u64)
            .ok_or_else(|| "total output frame count is too large".to_string())?;

        Ok(Self {
            states,
            frame_count,
            preview_count,
            hold_count,
        })
    }

    pub fn len(&self) -> u64 {
        self.frame_count + self.preview_count + self.hold_count
    }

    pub fn frames(&self) -> impl Iterator<Item = Target> + '_ {
        FrameIter {
            plan: self,
            position: 0,
        }
    }
}

struct FrameIter<'a> {
    plan: &'a Plan,
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
            return Some(Target::Final);
        }
        let i = i - self.plan.preview_count;
        if i >= self.plan.frame_count {
            return Some(Target::Final);
        }
        let count = self.plan.frame_count;
        let n = self.plan.states.len() as u128;
        let slot = if count <= 1 {
            (n - 1) as usize
        } else if n <= 1 {
            0
        } else {
            // Round the endpoint-preserving rational position to nearest integer.
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

    fn mapped(n: usize, fps: u32, speed: f64) -> Vec<Target> {
        Plan::new((0..n).collect(), fps, speed, 0.0, 0.0)
            .unwrap()
            .frames()
            .collect()
    }

    #[test]
    fn maps_logical_states_and_preserves_endpoints() {
        assert_eq!(
            mapped(4, 30, 1.0),
            vec![
                Target::State(0),
                Target::State(1),
                Target::State(2),
                Target::State(3)
            ]
        );
        assert_eq!(
            mapped(4, 60, 1.0),
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
        assert_eq!(mapped(4, 30, 2.0), vec![Target::State(0), Target::State(3)]);
        assert_eq!(mapped(4, 30, 0.5), mapped(4, 60, 1.0));
        assert_eq!(mapped(4, 1, 100.0), vec![Target::State(3)]);
        assert_eq!(mapped(1, 60, 1.0), vec![Target::State(0), Target::State(0)]);
    }

    #[test]
    fn durations_and_final_holds_are_output_fps_based() {
        let p = Plan::new(vec![8, 12], 30, 1.0, 0.1, 0.2).unwrap();
        assert_eq!(p.len(), 11); // 6 preview + 2 mapped + 3 hold frames
        assert_eq!(
            p.frames().collect::<Vec<_>>(),
            [
                vec![Target::Final; 6],
                vec![Target::State(8), Target::State(12)],
                vec![Target::Final; 3],
            ]
            .concat()
        );
        let p = Plan::new(vec![8], 60, 1.0, 0.1, 0.2).unwrap();
        assert_eq!(p.len(), 20);
        let p = Plan::new(vec![0, 1, 2], 1, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(p.frames().collect::<Vec<_>>(), vec![Target::State(2)]);
    }

    #[test]
    fn validates_numeric_inputs_and_counts() {
        for (fps, speed, hold, preview) in [
            (0, 1.0, 0.0, 0.0),
            (241, 1.0, 0.0, 0.0),
            (30, 0.0, 0.0, 0.0),
            (30, f64::NAN, 0.0, 0.0),
            (30, 1.0, -1.0, 0.0),
            (30, 1.0, 0.0, f64::INFINITY),
        ] {
            assert!(Plan::new(vec![0], fps, speed, hold, preview).is_err());
        }
        assert!(Plan::new(vec![], 30, 1.0, 0.0, 0.0).is_err());
        assert!(Plan::new(vec![0], 30, 1.0, f64::MAX, 0.0).is_err());
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
