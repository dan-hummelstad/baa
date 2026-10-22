//! Retargetable springs (PLAN.md §10). The physics is gpui's analytic
//! `SpringConfig::step`; this only adds a target, settling, and reduce-motion.

use gpui::{SpringConfig, SpringState};

/// Arc-ish snappy spring: settles in ~300 ms with a hint of overshoot.
const SNAPPY: SpringConfig = SpringConfig::new(400.0, 32.0, 1.0);
/// Quicker, for things the pointer summons (the collapsed sidebar): ~180 ms.
const QUICK: SpringConfig = SpringConfig::new(900.0, 50.0, 1.0);

const EPSILON: f32 = 0.001;
/// A dropped frame must not turn into a teleport.
const MAX_DT: f32 = 1.0 / 30.0;

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    config: SpringConfig,
    state: SpringState,
    target: f32,
}

impl Spring {
    pub fn new(value: f32) -> Self {
        Self {
            config: SNAPPY,
            state: SpringState {
                position: value,
                velocity: 0.0,
            },
            target: value,
        }
    }

    pub fn quick(value: f32) -> Self {
        Self {
            config: QUICK,
            ..Self::new(value)
        }
    }

    pub fn value(&self) -> f32 {
        self.state.position
    }

    /// Velocity is kept, so retargeting mid-flight never jerks.
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Jump with no animation (initial layout, finger tracking).
    pub fn snap(&mut self, value: f32) {
        self.target = value;
        self.state = SpringState {
            position: value,
            velocity: 0.0,
        };
    }

    fn is_moving(&self) -> bool {
        !self.config.is_settled(self.state, self.target, EPSILON)
    }

    /// Advance by `dt` seconds. Returns whether another frame is needed.
    /// With `reduce_motion` springs become snaps (§10).
    pub fn tick(&mut self, dt: f32, reduce_motion: bool) -> bool {
        if reduce_motion || !self.is_moving() {
            self.snap(self.target);
            return false;
        }
        self.state = self
            .config
            .step(self.state, self.target, dt.clamp(0.0, MAX_DT));
        if self.is_moving() {
            return true;
        }
        self.snap(self.target);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settles_on_target() {
        let mut s = Spring::new(0.0);
        s.set_target(1.0);
        let mut frames = 0;
        while s.tick(1.0 / 60.0, false) {
            frames += 1;
            assert!(frames < 600, "never settled");
        }
        assert_eq!(s.value(), 1.0);
        assert!(frames > 5, "should animate, not snap");
    }

    #[test]
    fn reduce_motion_snaps() {
        let mut s = Spring::new(0.0);
        s.set_target(5.0);
        assert!(!s.tick(1.0 / 60.0, true));
        assert_eq!(s.value(), 5.0);
    }

    #[test]
    fn retarget_keeps_velocity() {
        let mut s = Spring::new(0.0);
        s.set_target(1.0);
        for _ in 0..5 {
            s.tick(1.0 / 60.0, false);
        }
        let before = s.value();
        s.set_target(-1.0);
        s.tick(1.0 / 60.0, false);
        // Still moving forward for a moment: momentum carried over.
        assert!(s.value() > before);
    }
}
