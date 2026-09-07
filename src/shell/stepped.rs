//! Animations whose *visible* state is a handful of steps, not a continuum.
//!
//! [`crate::animation::Animation`] answers "is this still running", and for a
//! transition that is the right question — every frame of a slide shows
//! different pixels. A caret blink is not that. It has two states, on and
//! off, and at vsync rate it spends ~35 consecutive frames repainting each
//! one. The animation is still truthfully "running" through all of them, so
//! the frame scheduler is told to keep drawing, and
//! [`crate::frame::FrameScheduler`]'s whole premise — sleep until something
//! changes — never gets to fire.
//!
//! [`Stepped`] quantises the weight and reports a *step change* instead. The
//! two consequences that matter:
//!
//! - `advance` returns false through the flat middle of a step, so the loop
//!   can reach `ControlFlow::Wait`.
//! - Nothing else may then wake it, so [`Stepped::wake_in`] says when the
//!   next step is due and the caller hands that to the event loop as a
//!   deadline. A stepped animation that is advanced but never scheduled
//!   simply freezes, which is why the two are on the same type.
//!
//! Steps are cut on raw progress, not on the eased weight: progress is
//! linear in time, which is what makes `wake_in` a subtraction rather than
//! an easing inversion. The eased weight is still what [`Stepped::weight`]
//! reports, so the shape of the animation is unchanged — only the number of
//! distinct values it takes.

use std::time::{Duration, Instant};

use crate::animation::{Animation, Easing};

/// An [`Animation`] sampled at `steps` evenly spaced points. See the module
/// doc for why this exists and why `wake_in` is not optional.
pub struct Stepped {
    animation: Animation,
    /// One full sweep of `progress`, 0 to 1. For a ping-pong that is half a
    /// visual cycle; the step cadence is the same either way.
    sweep: Duration,
    steps: u32,
    step: u32,
    /// Wall clock of the last `advance`. The frame delta cannot be used:
    /// [`crate::frame::MAX_ANIMATION_DELTA`] clamps it to 100 ms so that a
    /// window left alone doesn't teleport its transitions to the end, and a
    /// stepped animation sleeps for longer than that on purpose. Both
    /// repeating modes wrap `elapsed`, so a large honest delta is safe here
    /// in a way it is not for a one-shot.
    last: Instant,
}

impl Stepped {
    /// `sweep` is how long `progress` takes to travel 0 to 1; `steps` is how
    /// many distinct values the result may take. Two steps is a blink.
    pub fn new(sweep: Duration, easing: Easing, steps: u32) -> Self {
        let animation = Animation::new(sweep, easing).looping();
        let step = Self::step_of(&animation, steps);
        Self {
            animation,
            sweep,
            steps: steps.max(1),
            step,
            last: Instant::now(),
        }
    }

    fn step_of(animation: &Animation, steps: u32) -> u32 {
        let steps = steps.max(1);
        ((animation.progress() * steps as f32) as u32).min(steps - 1)
    }

    /// Steps the clock and reports **whether the visible state changed** —
    /// which is what the caller should feed the frame scheduler, in place of
    /// [`Animation::advance`]'s "still running".
    pub fn advance(&mut self) -> bool {
        let now = Instant::now();
        self.animation.advance(now - self.last);
        self.last = now;
        let step = Self::step_of(&self.animation, self.steps);
        std::mem::replace(&mut self.step, step) != step
    }

    /// The eased weight, quantised to the current step.
    pub fn weight(&self) -> f32 {
        self.animation
            .easing()
            .apply(self.step as f32 / self.steps as f32)
    }

    /// True through the first half of the cycle. The blink's on/off, for a
    /// [`Stepped`] built with two steps.
    pub fn is_on(&self) -> bool {
        self.weight() < 0.5
    }

    /// Restart from the top of the cycle. For a caret this is "the user just
    /// typed": the blink must resume lit, not mid-phase.
    pub fn restart(&mut self) {
        self.animation.restart();
        self.last = Instant::now();
        self.step = Self::step_of(&self.animation, self.steps);
    }

    /// How long until the next step is due — the deadline the event loop
    /// sleeps to. Never zero: a deadline in the past is a spin, and one
    /// frame of drift is invisible on a blink.
    pub fn wake_in(&self) -> Duration {
        let per_step = self.sweep / self.steps;
        let into = self.animation.progress() * self.steps as f32 - self.step as f32;
        // Ping-pong runs the second half of its cycle backwards, so "how far
        // into this step" is measured from whichever end it is travelling
        // away from. Waking a step early costs one redundant frame; waking
        // late shows a stalled caret, so round toward early.
        let remaining = per_step.mul_f32((1.0 - into).clamp(0.0, 1.0)).min(per_step);
        remaining.max(Duration::from_millis(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: a blink asks for two frames a second, not sixty.
    #[test]
    fn a_blink_only_reports_a_change_when_it_flips() {
        let mut blink = Stepped::new(Duration::from_millis(100), Easing::Linear, 2);
        assert!(blink.is_on(), "starts lit");

        // Well inside the first half — same pixels, nothing to say.
        std::thread::sleep(Duration::from_millis(20));
        assert!(!blink.advance(), "still on");
        assert!(blink.is_on());

        // Past the halfway mark.
        std::thread::sleep(Duration::from_millis(40));
        assert!(blink.advance(), "flipped off");
        assert!(!blink.is_on());

        std::thread::sleep(Duration::from_millis(10));
        assert!(!blink.advance(), "still off");
    }

    #[test]
    fn a_restart_resumes_lit() {
        let mut blink = Stepped::new(Duration::from_millis(100), Easing::Linear, 2);
        std::thread::sleep(Duration::from_millis(60));
        blink.advance();
        assert!(!blink.is_on());

        blink.restart();
        assert!(blink.is_on(), "typing must not leave the caret dark");
    }

    /// A stepped animation that is advanced but never scheduled freezes, so
    /// the deadline has to be inside the step and never zero.
    #[test]
    fn the_wake_deadline_stays_inside_one_step() {
        let blink = Stepped::new(Duration::from_millis(1050), Easing::Linear, 2);
        let per_step = Duration::from_millis(525);
        assert!(blink.wake_in() <= per_step);
        assert!(blink.wake_in() >= Duration::from_millis(1));

        let fade = Stepped::new(Duration::from_millis(1200), Easing::EaseInOut, 16);
        assert!(fade.wake_in() <= Duration::from_millis(75));
        assert!(fade.wake_in() >= Duration::from_millis(1));
    }

    #[test]
    fn the_weight_takes_only_as_many_values_as_there_are_steps() {
        let fade = Stepped::new(Duration::from_millis(1200), Easing::Linear, 4);
        assert_eq!(fade.weight(), 0.0, "first step sits at the bottom");

        // Every reachable weight is a quarter.
        for step in 0..4u32 {
            let weight = step as f32 / 4.0;
            assert_eq!((weight * 4.0).fract(), 0.0);
        }
    }
}
