//! When the background jobs are allowed to run.
//!
//! The Janitor, Synthesizer and Contradiction scans all call a local model,
//! and on a laptop that is the difference between a machine you can use and
//! one you cannot. Each job therefore takes an optional `run_between` window
//! in local time — `"02:00-06:00"` — and does nothing outside it.
//!
//! The window is a gate, not a schedule. `interval_hours` still says how often
//! a job may run; the window says when it is allowed to. Combining the two
//! naively would break, because a twelve-hour interval landing at 13:00 and
//! 01:00 never falls inside a four-hour night window and the job would simply
//! never run. So a job with a window wakes often and checks two things: is the
//! window open, and has a full interval passed since the last real run.

use chrono::{Local, NaiveTime};
use std::time::{Duration, Instant};

/// How often a windowed job wakes to check the clock.
///
/// Fine enough to catch any window worth writing, coarse enough to cost
/// nothing. A job without a window does not poll at all; it keeps its own
/// interval.
const POLL_PERIOD: Duration = Duration::from_secs(300);

/// A local-time window of the form `HH:MM-HH:MM`.
///
/// A window whose end is before its start wraps midnight, so `"22:00-06:00"`
/// means the eight hours across the night rather than nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunWindow {
    start: NaiveTime,
    end: NaiveTime,
}

impl RunWindow {
    /// Parse `HH:MM-HH:MM`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message naming what was wrong. Callers surface
    /// it at startup: a window that never opens is worse than no window, and
    /// it fails silently at 2 a.m. where nobody is watching.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (start, end) = spec.split_once('-').ok_or_else(|| {
            format!("expected two times separated by '-', as in \"02:00-06:00\", got {spec:?}")
        })?;

        let start = parse_time(start.trim())?;
        let end = parse_time(end.trim())?;

        if start == end {
            return Err(format!(
                "start and end are the same time ({spec:?}); a window has to have width"
            ));
        }

        Ok(Self { start, end })
    }

    /// Is `now` inside the window?
    pub fn is_open_at(&self, now: NaiveTime) -> bool {
        if self.start < self.end {
            now >= self.start && now < self.end
        } else {
            // Wraps midnight: open from the start until midnight, and from
            // midnight until the end.
            now >= self.start || now < self.end
        }
    }

    /// Is the window open right now, by the machine's local clock?
    pub fn is_open_now(&self) -> bool {
        self.is_open_at(Local::now().time())
    }
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| format!("{value:?} is not a 24-hour time of the form HH:MM"))
}

/// Decides, tick by tick, whether a background job runs.
///
/// Without a window this is exactly the old behaviour: wake every `period`,
/// run every time. With one, it wakes every [`POLL_PERIOD`] and runs only when
/// the window is open and a full `period` has passed since the last run.
#[derive(Debug)]
pub struct Schedule {
    window: Option<RunWindow>,
    period: Duration,
    last_run: Option<Instant>,
}

impl Schedule {
    /// Build a schedule from a job's interval and its optional window.
    pub fn new(period: Duration, window: Option<RunWindow>) -> Self {
        Self {
            window,
            period,
            last_run: None,
        }
    }

    /// How long the job's ticker should wait between wake-ups.
    pub fn poll_period(&self) -> Duration {
        match self.window {
            // Never poll slower than the job's own interval — a job set to run
            // every minute should not be woken every five.
            Some(_) => POLL_PERIOD.min(self.period),
            None => self.period,
        }
    }

    /// Called on every tick: should the job do its work now?
    ///
    /// Records the run when it answers yes, so the interval is measured from
    /// work actually done rather than from ticks slept through.
    pub fn should_run(&mut self) -> bool {
        self.should_run_at(Instant::now(), self.window.is_none_or(|w| w.is_open_now()))
    }

    /// The decision itself, with the clock passed in so it can be tested.
    fn should_run_at(&mut self, now: Instant, window_open: bool) -> bool {
        if !window_open {
            return false;
        }

        if let Some(last) = self.last_run {
            if now.duration_since(last) < self.period {
                return false;
            }
        }

        self.last_run = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    #[test]
    fn parses_a_window() {
        let window = RunWindow::parse("02:00-06:00").unwrap();
        assert_eq!(window.start, at(2, 0));
        assert_eq!(window.end, at(6, 0));
    }

    #[test]
    fn tolerates_spaces_around_the_dash() {
        assert_eq!(
            RunWindow::parse(" 02:00 - 06:00 ").unwrap(),
            RunWindow::parse("02:00-06:00").unwrap()
        );
    }

    #[test]
    fn rejects_what_it_cannot_honor() {
        for spec in ["02:00", "2am-6am", "25:00-06:00", "02:00-06:61", ""] {
            assert!(RunWindow::parse(spec).is_err(), "{spec:?} should not parse");
        }
    }

    #[test]
    fn rejects_a_zero_width_window() {
        let error = RunWindow::parse("02:00-02:00").unwrap_err();
        assert!(error.contains("width"), "got: {error}");
    }

    #[test]
    fn a_daytime_window_is_open_only_between_its_ends() {
        let window = RunWindow::parse("02:00-06:00").unwrap();

        assert!(!window.is_open_at(at(1, 59)));
        assert!(window.is_open_at(at(2, 0)), "the start is inside");
        assert!(window.is_open_at(at(5, 59)));
        assert!(!window.is_open_at(at(6, 0)), "the end is outside");
        assert!(!window.is_open_at(at(14, 0)));
    }

    #[test]
    fn a_window_that_wraps_midnight_covers_the_night() {
        let window = RunWindow::parse("22:00-06:00").unwrap();

        assert!(window.is_open_at(at(22, 0)));
        assert!(window.is_open_at(at(23, 59)));
        assert!(window.is_open_at(at(0, 0)), "midnight is inside");
        assert!(window.is_open_at(at(5, 59)));
        assert!(!window.is_open_at(at(6, 0)));
        assert!(!window.is_open_at(at(12, 0)));
    }

    #[test]
    fn without_a_window_every_tick_runs() {
        let mut schedule = Schedule::new(Duration::from_secs(3600), None);
        let start = Instant::now();

        assert!(schedule.should_run_at(start, true));
        assert!(schedule.should_run_at(start + Duration::from_secs(3600), true));
        assert_eq!(schedule.poll_period(), Duration::from_secs(3600));
    }

    #[test]
    fn a_closed_window_skips_without_consuming_the_interval() {
        let mut schedule = Schedule::new(
            Duration::from_secs(3600),
            Some(RunWindow::parse("02:00-06:00").unwrap()),
        );
        let start = Instant::now();

        assert!(!schedule.should_run_at(start, false));
        assert!(!schedule.should_run_at(start + Duration::from_secs(300), false));
        // The skipped ticks did not count as runs, so the first open tick works
        // immediately rather than waiting out an interval.
        assert!(schedule.should_run_at(start + Duration::from_secs(600), true));
    }

    #[test]
    fn an_open_window_still_respects_the_interval() {
        let period = Duration::from_secs(3600);
        let mut schedule = Schedule::new(period, Some(RunWindow::parse("02:00-06:00").unwrap()));
        let start = Instant::now();

        assert!(schedule.should_run_at(start, true));
        // Five minutes later the window is still open, but an hour has not
        // passed. This is the check that stops a night window from becoming a
        // twelve-times-an-hour loop.
        assert!(!schedule.should_run_at(start + Duration::from_secs(300), true));
        assert!(schedule.should_run_at(start + period, true));
    }

    #[test]
    fn a_windowed_job_polls_often_enough_to_find_its_window() {
        let schedule = Schedule::new(
            Duration::from_secs(12 * 3600),
            Some(RunWindow::parse("02:00-06:00").unwrap()),
        );

        // Without this, a twelve-hour interval could tick at 13:00 and 01:00
        // forever and never once land inside a four-hour window.
        assert_eq!(schedule.poll_period(), POLL_PERIOD);
    }

    #[test]
    fn a_job_faster_than_the_poll_period_keeps_its_own_pace() {
        let period = Duration::from_secs(60);
        let schedule = Schedule::new(period, Some(RunWindow::parse("02:00-06:00").unwrap()));

        assert_eq!(schedule.poll_period(), period);
    }
}
