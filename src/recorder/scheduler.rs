use super::FrameKind;

/// Capture cadence driven by user activity and bounded by minimum/maximum
/// intervals. Timestamps are monotonic milliseconds supplied by the caller.
pub struct Scheduler {
    idle_ms: u64,
    min_ms: u64,
    max_ms: u64,
    last_activity_ms: u64,
    last_capture_ms: u64,
    stable_captured: bool,
}

impl Scheduler {
    pub fn new(idle_ms: u64, min_ms: u64, max_ms: u64, now_ms: u64) -> Self {
        assert!(
            min_ms > 0 && max_ms > 0,
            "scheduler intervals must be positive"
        );
        Self {
            idle_ms,
            min_ms,
            max_ms,
            last_activity_ms: now_ms,
            last_capture_ms: now_ms,
            stable_captured: false,
        }
    }

    /// Mark any observed input. Activity starts a new idle period.
    pub fn activity(&mut self, now_ms: u64) {
        self.last_activity_ms = now_ms;
        self.stable_captured = false;
    }

    /// Returns the next capture kind, respecting min spacing. Stable frames
    /// take priority after an idle period; active sessions get a provisional
    /// frame at the maximum interval.
    pub fn due(&self, now_ms: u64) -> Option<FrameKind> {
        if now_ms.saturating_sub(self.last_capture_ms) < self.min_ms {
            return None;
        }
        let idle = now_ms.saturating_sub(self.last_activity_ms) >= self.idle_ms;
        if idle {
            if !self.stable_captured {
                Some(FrameKind::Stable)
            } else {
                None
            }
        } else if now_ms.saturating_sub(self.last_capture_ms) >= self.max_ms {
            Some(FrameKind::Provisional)
        } else {
            None
        }
    }

    /// Record a completed capture. Stable state is cleared only when a stable
    /// capture actually occurs; a pending stable capture survives cooldown.
    pub fn captured(&mut self, now_ms: u64, kind: FrameKind) {
        self.last_capture_ms = now_ms;
        if kind == FrameKind::Stable {
            self.stable_captured = true;
        }
    }

    /// Advance cooldown after a candidate failed, retaining any pending stable
    /// frame for the next eligible attempt.
    pub fn failed(&mut self, now_ms: u64) {
        self.last_capture_ms = now_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::Config;

    #[test]
    fn default_cadence_captures_continuous_activity_and_short_pauses() {
        let config = Config::default();
        let mut s = Scheduler::new(
            config.idle_delay_ms,
            config.min_capture_gap_ms,
            config.max_active_gap_ms,
            0,
        );
        s.captured(0, FrameKind::Stable);
        for now in (100..=2500).step_by(100) {
            s.activity(now);
            if now < 2500 {
                assert_eq!(s.due(now), None);
            }
        }
        assert_eq!(s.due(2500), Some(FrameKind::Provisional));
        s.captured(2500, FrameKind::Provisional);
        s.activity(2550);
        assert_eq!(s.due(2749), None);
        assert_eq!(s.due(2750), Some(FrameKind::Stable));
    }

    #[test]
    fn delays_stable_until_idle_and_minimum_spacing() {
        let mut s = Scheduler::new(150, 300, 3000, 0);
        s.captured(0, FrameKind::Stable); // root's initial frame
        s.activity(100);
        assert_eq!(s.due(249), None);
        assert_eq!(s.due(400), Some(FrameKind::Stable)); // idle since 100; min elapsed
        s.captured(400, FrameKind::Stable);
        assert_eq!(s.due(500), None);
    }

    #[test]
    fn provisional_cadence_and_pending_stable_survive_cooldown() {
        let mut s = Scheduler::new(150, 500, 1000, 0);
        s.captured(0, FrameKind::Stable);
        s.activity(900);
        assert_eq!(s.due(1000), Some(FrameKind::Provisional));
        s.captured(1000, FrameKind::Provisional);
        s.activity(1100);
        // Idle threshold is reached before min spacing; stable remains due.
        assert_eq!(s.due(1300), None);
        assert_eq!(s.due(1500), Some(FrameKind::Stable));
    }

    #[test]
    fn provisional_capture_after_idle_does_not_consume_stable_and_failure_preserves_it() {
        let mut s = Scheduler::new(100, 400, 200, 0); // max may be below min
        s.captured(0, FrameKind::Stable);
        s.activity(390);
        assert_eq!(s.due(400), Some(FrameKind::Provisional));
        s.captured(400, FrameKind::Provisional);
        assert_eq!(s.due(800), Some(FrameKind::Stable));
        s.failed(800);
        assert_eq!(s.due(1199), None);
        assert_eq!(s.due(1200), Some(FrameKind::Stable));
    }
}
