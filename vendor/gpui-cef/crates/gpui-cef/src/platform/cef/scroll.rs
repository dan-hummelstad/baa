//! Wheel input, released once per display frame. Sending every OS event as it
//! arrives puts uneven steps into frames (two events in one, none in the next)
//! and rounding each to whole pixels loses the fractions; both read as jitter.

/// Share of the remaining mouse-wheel distance scrolled each frame.
// ponytail: per frame, not per second. Ceiling: on a 60 Hz display a notch
// takes twice as long (~170 ms instead of ~85 ms to 90%). Upgrade: scale by dt.
const GLIDE: f32 = 0.2;

#[derive(Default, Debug)]
pub(crate) struct Scroll {
    /// Trackpad deltas: already smooth (and with momentum) from the OS.
    direct: (f32, f32),
    /// Mouse-wheel notches still to animate.
    glide: (f32, f32),
    /// Sub-pixel remainder carried into the next frame.
    carry: (f32, f32),
}

impl Scroll {
    pub(crate) fn push(&mut self, dx: f32, dy: f32, precise: bool) {
        let target = if precise { &mut self.direct } else { &mut self.glide };
        target.0 += dx;
        target.1 += dy;
    }

    /// Whole pixels to scroll this frame, if any.
    pub(crate) fn step(&mut self) -> Option<(i32, i32)> {
        let x = axis(&mut self.direct.0, &mut self.glide.0, &mut self.carry.0);
        let y = axis(&mut self.direct.1, &mut self.glide.1, &mut self.carry.1);
        (x != 0 || y != 0).then_some((x, y))
    }
}

fn axis(direct: &mut f32, glide: &mut f32, carry: &mut f32) -> i32 {
    let step = if glide.abs() < 0.5 { *glide } else { *glide * GLIDE };
    *glide -= step;
    let total = std::mem::take(direct) + step + *carry;
    let whole = total.trunc();
    *carry = total - whole;
    whole as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(scroll: &mut Scroll) -> (Vec<i32>, i32) {
        // A second of frames; quiet frames (under a pixel) are skipped, not the end.
        let steps: Vec<i32> = (0..120).filter_map(|_| scroll.step().map(|(_, y)| y)).collect();
        let sum = steps.iter().sum();
        (steps, sum)
    }

    #[test]
    fn a_wheel_notch_glides_out_completely_with_shrinking_steps() {
        let mut s = Scroll::default();
        s.push(0., 120., false);
        let (steps, sum) = drain(&mut s);
        assert!((119..=120).contains(&sum), "{steps:?}");
        assert!(steps.len() > 5, "{steps:?}");
        assert!(steps.windows(2).all(|w| w[0] >= w[1]), "{steps:?}");
    }

    #[test]
    fn trackpad_fractions_are_carried_not_dropped() {
        let mut s = Scroll::default();
        let mut sum = 0;
        for _ in 0..10 {
            s.push(0., 0.4, true);
            sum += s.step().map_or(0, |(_, y)| y);
        }
        assert_eq!(sum, 4);
    }

    #[test]
    fn trackpad_events_in_one_frame_are_sent_together() {
        let mut s = Scroll::default();
        s.push(0., -3., true);
        s.push(0., -5., true);
        assert_eq!(s.step(), Some((0, -8)));
        assert_eq!(s.step(), None);
    }
}
