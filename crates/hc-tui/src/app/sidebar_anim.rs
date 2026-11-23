//! Smooth, deterministic sidebar collapse/expand width animation.
//!
//! All time enters as explicit `Instant`s so unit tests drive progress without
//! wall-clock sleeps. While an animation is active, `maybe_draw` defers PTY
//! size claims (`settling`) so the underlying terminal is resized once at the
//! settled target width rather than thrashing every intermediate frame.

use std::time::{Duration, Instant};

use super::App;

/// Total duration for a full collapse/expand transition.
pub(crate) const SIDEBAR_ANIMATION_DURATION: Duration = Duration::from_millis(150);

/// Maximum sidebar width that renders the compact rail (`»`) instead of the
/// expanded sidebar (`«`) during a transition.
pub(crate) const COMPACT_RENDER_MAX_WIDTH: u16 = 5;

const ANIMATION_TICK: Duration = super::sched::FRAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SidebarAnimation {
    pub start: Instant,
    pub duration: Duration,
    pub from_width: u16,
    pub to_width: u16,
    pub next_tick: Instant,
}

impl SidebarAnimation {
    /// Starts an animation between `from_width` and `to_width`, scaling the
    /// duration proportionally to `full_span` so mid-animation reversals feel
    /// responsive. Returns `None` when no width change is needed.
    pub(crate) fn start(
        now: Instant,
        from_width: u16,
        to_width: u16,
        full_span: u16,
    ) -> Option<Self> {
        if from_width == to_width {
            return None;
        }
        let span = full_span.max(1);
        let dist = from_width.abs_diff(to_width).min(span);
        let duration_ms = (SIDEBAR_ANIMATION_DURATION.as_millis() as u64 * u64::from(dist)
            / u64::from(span))
        .max(1);
        let duration = Duration::from_millis(duration_ms);
        let end = now + duration;
        Some(Self {
            start: now,
            duration,
            from_width,
            to_width,
            next_tick: (now + ANIMATION_TICK).min(end),
        })
    }

    pub(crate) fn end(&self) -> Instant {
        self.start + self.duration
    }

    pub(crate) fn is_finished(&self, now: Instant) -> bool {
        now >= self.end()
    }

    /// Interpolated sidebar width at `now` using a cubic ease-out curve.
    pub(crate) fn width_at(&self, now: Instant) -> u16 {
        if now <= self.start {
            return self.from_width;
        }
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= self.duration {
            return self.to_width;
        }
        let t = (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        let u = 1.0 - t;
        let eased = 1.0 - u * u * u;
        let from = f32::from(self.from_width);
        let to = f32::from(self.to_width);
        (from + (to - from) * eased).round().max(0.0) as u16
    }
}

impl App {
    /// Sidebar width to draw at `now`, respecting mobile mode and column bounds.
    pub(crate) fn effective_sidebar_width(&self, cols: u16, now: Instant) -> u16 {
        let cfg = self.shell.layout_config();
        if cols <= cfg.mobile_width_threshold {
            return 0;
        }
        match self.sidebar_animation {
            Some(anim) => anim.width_at(now).min(cols.saturating_sub(1)),
            None => cfg.target_sidebar_width(
                cols,
                self.shell_state.sidebar_collapsed,
                self.sidebar_width,
            ),
        }
    }

    /// Toggles `sidebar_collapsed`, starts or reverses the width animation from
    /// the current rendered width at `now`, and persists the preference.
    pub(in crate::app) fn toggle_sidebar(&mut self, now: Instant) {
        let cfg = self.shell.layout_config();
        let cols = self.area.0;
        let from_width = self.effective_sidebar_width(cols, now);
        self.shell_state.sidebar_collapsed = !self.shell_state.sidebar_collapsed;
        self.sidebar_collapsed_manual = true;
        let to_width =
            cfg.target_sidebar_width(cols, self.shell_state.sidebar_collapsed, self.sidebar_width);
        let expanded_width = cfg.target_sidebar_width(cols, false, self.sidebar_width);
        let collapsed_width = cfg.target_sidebar_width(cols, true, self.sidebar_width);
        let full_span = expanded_width.abs_diff(collapsed_width);
        self.sidebar_animation = SidebarAnimation::start(now, from_width, to_width, full_span);
        self.persist_chrome_preferences();
    }

    pub(super) fn sidebar_animation_deadline(&self) -> Option<Instant> {
        self.sidebar_animation.map(|anim| anim.next_tick)
    }

    /// Clears a finished animation before layout/settling checks run.
    pub(super) fn settle_sidebar_animation(&mut self, now: Instant) {
        if self
            .sidebar_animation
            .is_some_and(|anim| anim.is_finished(now))
        {
            self.sidebar_animation = None;
        }
    }

    /// Advances the sidebar animation tick cadence; returns `true` when a frame
    /// should be marked dirty.
    pub(super) fn tick_sidebar_animation(&mut self, now: Instant) -> bool {
        let Some(mut anim) = self.sidebar_animation else {
            return false;
        };
        if anim.is_finished(now) {
            self.sidebar_animation = None;
            return true;
        }
        if now >= anim.next_tick {
            anim.next_tick = (now + ANIMATION_TICK).min(anim.end());
            self.sidebar_animation = Some(anim);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_returns_none_when_widths_match() {
        let t0 = Instant::now();
        assert_eq!(SidebarAnimation::start(t0, 24, 24, 20), None);
    }

    #[test]
    fn ease_out_collapse_and_expand_are_monotonic_and_hit_endpoints() {
        let t0 = Instant::now();
        let collapse = SidebarAnimation::start(t0, 24, 4, 20).unwrap();
        assert_eq!(collapse.width_at(t0), 24);
        let mid = collapse.width_at(t0 + Duration::from_millis(50));
        assert!(
            mid < 24 && mid > 4,
            "expected mid width in (4, 24), got {mid}"
        );
        // Cubic ease-out covers more than half the distance in the first third.
        assert!(mid <= 14, "expected ease-out fast start, got {mid}");
        assert_eq!(collapse.width_at(t0 + SIDEBAR_ANIMATION_DURATION), 4);
        assert_eq!(collapse.width_at(t0 + Duration::from_millis(500)), 4);

        let expand = SidebarAnimation::start(t0, 4, 24, 20).unwrap();
        assert_eq!(expand.width_at(t0), 4);
        let mid_exp = expand.width_at(t0 + Duration::from_millis(50));
        assert!(mid_exp > 4 && mid_exp < 24);
        assert!(mid_exp >= 14, "expected ease-out fast start, got {mid_exp}");
        assert_eq!(expand.width_at(t0 + SIDEBAR_ANIMATION_DURATION), 24);
    }

    #[test]
    fn proportional_duration_scales_for_partial_reversal() {
        let t0 = Instant::now();
        let full = SidebarAnimation::start(t0, 24, 4, 20).unwrap();
        let half = SidebarAnimation::start(t0, 14, 24, 20).unwrap();
        assert_eq!(full.duration, SIDEBAR_ANIMATION_DURATION);
        assert_eq!(half.duration, SIDEBAR_ANIMATION_DURATION / 2);
    }
}
