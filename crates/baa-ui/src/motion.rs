//! Retargetable springs (PLAN.md §10). The physics is gpui's analytic
//! `SpringConfig::step`; this only adds a target, settling, and reduce-motion.

use std::collections::HashMap;

use gpui::{SpringConfig, SpringState};

/// Arc-ish snappy spring: settles in ~300 ms with a hint of overshoot.
const SNAPPY: SpringConfig = SpringConfig::new(400.0, 32.0, 1.0);
/// Quicker, for things the pointer summons (the collapsed sidebar): ~180 ms.
const QUICK: SpringConfig = SpringConfig::new(900.0, 50.0, 1.0);

/// Hover fades: critically damped, ~120 ms, never overshoots a colour.
const HOVER: SpringConfig = SpringConfig::new(1400.0, 75.0, 1.0);
/// Presses: dips fast, springs back with a little bounce.
const PRESS: SpringConfig = SpringConfig::new(1800.0, 50.0, 1.0);
/// Things that travel (selection pills, rows growing in): quick, slight settle.
const GLIDE: SpringConfig = SpringConfig::new(700.0, 46.0, 1.0);

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

    fn with(config: SpringConfig, value: f32) -> Self {
        Self {
            config,
            ..Self::new(value)
        }
    }

    pub fn target(&self) -> f32 {
        self.target
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

/// What a keyed spring animates; picks its feel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feel {
    Hover,
    Press,
    /// Selected-ness, a sliding indicator's position, a row growing in or out.
    Glide,
}

/// Name and number, like gpui's element ids ("tab", 7).
pub type Key = (&'static str, u64);

/// Springs for the many small things the pointer touches, by key. An absent
/// spring reads as 0; one is dropped once it settles back at 0.
#[derive(Default)]
pub struct Motion {
    springs: HashMap<(Key, Feel), Spring>,
}

impl Motion {
    pub fn get(&self, key: Key, feel: Feel) -> f32 {
        self.springs.get(&(key, feel)).map_or(0.0, Spring::value)
    }

    /// Head for `target` from wherever it is now.
    pub fn to(&mut self, key: Key, feel: Feel, target: f32) {
        self.springs
            .entry((key, feel))
            .or_insert_with(|| Spring::with(config(feel), 0.0))
            .set_target(target);
    }

    /// Start over at `start`, then head for `target`.
    pub fn from(&mut self, key: Key, feel: Feel, start: f32, target: f32) {
        let mut spring = Spring::with(config(feel), start);
        spring.set_target(target);
        self.springs.insert((key, feel), spring);
    }

    #[cfg(test)]
    pub fn springs_target(&self, key: Key, feel: Feel) -> Option<f32> {
        self.springs.get(&(key, feel)).map(Spring::target)
    }

    /// Drops every spring whose key has this name (a closed menu's rows).
    pub fn forget(&mut self, name: &str) {
        self.springs.retain(|((n, _), _), _| *n != name);
    }

    /// Returns whether another frame is needed.
    pub fn tick(&mut self, dt: f32, reduce_motion: bool) -> bool {
        let mut moving = false;
        self.springs.retain(|_, spring| {
            let more = spring.tick(dt, reduce_motion);
            moving |= more;
            more || spring.target() != 0.0
        });
        moving
    }
}

fn config(feel: Feel) -> SpringConfig {
    match feel {
        Feel::Hover => HOVER,
        Feel::Press => PRESS,
        Feel::Glide => GLIDE,
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
    fn motion_keeps_what_is_held_and_drops_what_settles_at_rest() {
        let mut m = Motion::default();
        m.to(("tab", 1), Feel::Hover, 1.0);
        m.to(("tab", 2), Feel::Hover, 1.0);
        while m.tick(1.0 / 60.0, false) {}
        assert_eq!(m.get(("tab", 1), Feel::Hover), 1.0);
        m.to(("tab", 1), Feel::Hover, 0.0);
        while m.tick(1.0 / 60.0, false) {}
        assert_eq!(m.springs.len(), 1, "settled at 0 is forgotten");
        m.forget("tab");
        assert!(m.springs.is_empty());
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

/// How close two clicks must be to count as a double-click. Ours, not the
/// system's: macOS's default is slow enough that opening and closing a folder
/// at a moderate pace reads as a rename.
pub(crate) const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(250);

thread_local! {
    static LAST_CLICK: std::cell::Cell<Option<(&'static str, u64, std::time::Instant)>> =
        const { std::cell::Cell::new(None) };
}

/// Records a click on `target`; true when it completes a double-click. A
/// third quick click starts over rather than counting again.
pub(crate) fn double_click(target: (&'static str, u64)) -> bool {
    let now = std::time::Instant::now();
    LAST_CLICK.with(|last| {
        let double = last
            .get()
            .is_some_and(|(tag, id, at)| (tag, id) == target && now - at <= DOUBLE_CLICK);
        last.set(if double { None } else { Some((target.0, target.1, now)) });
        double
    })
}

#[cfg(test)]
mod double_click_tests {
    use super::*;

    #[test]
    fn two_quick_clicks_on_one_target() {
        assert!(!double_click(("a", 1)));
        assert!(double_click(("a", 1)));
        assert!(!double_click(("a", 1)), "a third click starts a new pair");
        assert!(!double_click(("b", 1)), "another target");
        std::thread::sleep(DOUBLE_CLICK + std::time::Duration::from_millis(20));
        assert!(!double_click(("b", 1)), "too slow");
    }
}
