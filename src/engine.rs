//! Timer state machine and per-day work statistics.
//!
//! Everything here is pure logic: the current time and date are passed in,
//! so the behaviour can be unit-tested without waiting for real minutes.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// If the ticker falls this far behind (for example because the computer was
/// asleep), the missing time is treated as a pause rather than as work.
const MAX_TICK_GAP: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Work,
    Break,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Work => "Work",
            Phase::Break => "Break",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Phase::Work => Phase::Break,
            Phase::Break => Phase::Work,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Running,
    Paused,
}

#[derive(Clone, Copy, Debug)]
enum State {
    /// Waiting at the start of the current period.
    Idle,
    Running {
        ends_at: Instant,
    },
    Paused {
        remaining: Duration,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub work_minutes: u32,
    pub break_minutes: u32,
    pub auto_start_breaks: bool,
    pub auto_start_work: bool,
    pub notifications: bool,
    pub sound: bool,
    /// Pulse the menu bar icon during breaks (otherwise it just turns green).
    pub tray_pulse: bool,
    /// Show the remaining time next to the menu bar icon.
    pub tray_time: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            work_minutes: 25,
            break_minutes: 5,
            auto_start_breaks: true,
            auto_start_work: false,
            notifications: true,
            sound: true,
            tray_pulse: true,
            tray_time: false,
        }
    }
}

impl Settings {
    pub fn minutes(&self, phase: Phase) -> u32 {
        match phase {
            Phase::Work => self.work_minutes,
            Phase::Break => self.break_minutes,
        }
        .max(1)
    }

    pub fn duration(&self, phase: Phase) -> Duration {
        Duration::from_secs(u64::from(self.minutes(phase)) * 60)
    }

    fn auto_start(&self, phase: Phase) -> bool {
        match phase {
            Phase::Work => self.auto_start_work,
            Phase::Break => self.auto_start_breaks,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DayStats {
    /// Time spent in running work periods, in milliseconds.
    pub work_ms: u64,
    /// Work periods that ran all the way to the end.
    pub sessions: u32,
}

impl DayStats {
    pub fn work(&self) -> Duration {
        Duration::from_millis(self.work_ms)
    }
}

/// Emitted by [`Engine::tick`] when a period runs out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodFinished {
    pub finished: Phase,
    pub next: Phase,
    /// Whether the next period was started automatically.
    pub next_started: bool,
}

pub struct Engine {
    pub settings: Settings,
    pub days: BTreeMap<NaiveDate, DayStats>,
    phase: Phase,
    state: State,
    /// While running: the instant up to which elapsed time has been credited.
    accounted_until: Instant,
    dirty: bool,
}

impl Engine {
    pub fn new(settings: Settings, days: BTreeMap<NaiveDate, DayStats>) -> Self {
        Self {
            settings,
            days,
            phase: Phase::Work,
            state: State::Idle,
            accounted_until: Instant::now(),
            dirty: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn status(&self) -> Status {
        match self.state {
            State::Idle => Status::Idle,
            State::Running { .. } => Status::Running,
            State::Paused { .. } => Status::Paused,
        }
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        match self.state {
            State::Idle => self.settings.duration(self.phase),
            State::Running { ends_at } => ends_at.saturating_duration_since(now),
            State::Paused { remaining } => remaining,
        }
    }

    /// Fraction of the current period that has elapsed, from 0.0 to 1.0.
    pub fn progress(&self, now: Instant) -> f32 {
        let total = self.settings.duration(self.phase).as_secs_f32();
        (1.0 - self.remaining(now).as_secs_f32() / total).clamp(0.0, 1.0)
    }

    pub fn day(&self, date: NaiveDate) -> DayStats {
        self.days.get(&date).copied().unwrap_or_default()
    }

    /// Starts the current period, or resumes it if paused.
    pub fn start(&mut self, now: Instant) {
        let remaining = match self.state {
            State::Running { .. } => return,
            State::Idle => self.settings.duration(self.phase),
            State::Paused { remaining } => remaining,
        };
        self.state = State::Running {
            ends_at: now + remaining,
        };
        self.accounted_until = now;
    }

    pub fn pause(&mut self, now: Instant, today: NaiveDate) {
        self.account(now, today);
        if let State::Running { ends_at } = self.state {
            self.state = State::Paused {
                remaining: ends_at.saturating_duration_since(now),
            };
        }
    }

    pub fn toggle(&mut self, now: Instant, today: NaiveDate) {
        match self.status() {
            Status::Running => self.pause(now, today),
            Status::Idle | Status::Paused => self.start(now),
        }
    }

    /// Rewinds the current period to its full length and stops the timer.
    pub fn reset(&mut self, now: Instant, today: NaiveDate) {
        self.account(now, today);
        self.state = State::Idle;
    }

    /// Ends the current period early. Work done so far still counts towards
    /// today's hours, but not as a completed session.
    pub fn skip(&mut self, now: Instant, today: NaiveDate) {
        self.account(now, today);
        self.advance(now);
    }

    /// Advances the clock; call this a few times per second.
    pub fn tick(&mut self, now: Instant, today: NaiveDate) -> Option<PeriodFinished> {
        self.account(now, today);
        let State::Running { ends_at } = self.state else {
            return None;
        };
        if now < ends_at {
            return None;
        }
        let finished = self.phase;
        if finished == Phase::Work {
            self.days.entry(today).or_default().sessions += 1;
            self.dirty = true;
        }
        let next_started = self.advance(now);
        Some(PeriodFinished {
            finished,
            next: self.phase,
            next_started,
        })
    }

    /// Call after editing `settings`: stretches or shrinks the current period
    /// so a new length takes effect right away.
    pub fn settings_changed(&mut self, old: &Settings, now: Instant) {
        self.dirty = true;
        let (old_len, new_len) = (old.duration(self.phase), self.settings.duration(self.phase));
        match &mut self.state {
            State::Idle => {}
            State::Running { ends_at } => {
                *ends_at = if new_len >= old_len {
                    *ends_at + (new_len - old_len)
                } else {
                    ends_at
                        .checked_sub(old_len - new_len)
                        .unwrap_or(now)
                        .max(now)
                };
            }
            State::Paused { remaining } => {
                *remaining = if new_len >= old_len {
                    *remaining + (new_len - old_len)
                } else {
                    remaining.saturating_sub(old_len - new_len)
                };
            }
        }
    }

    /// Returns whether anything worth saving changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Moves to the next phase, starting it if the settings say so.
    fn advance(&mut self, now: Instant) -> bool {
        self.phase = self.phase.next();
        self.state = State::Idle;
        let auto_start = self.settings.auto_start(self.phase);
        if auto_start {
            self.start(now);
        }
        auto_start
    }

    /// Credits running work time since the previous call to `today`.
    fn account(&mut self, now: Instant, today: NaiveDate) {
        let State::Running { ends_at } = &mut self.state else {
            return;
        };
        let gap = now.saturating_duration_since(self.accounted_until);
        if gap > MAX_TICK_GAP {
            // We were not running (e.g. the system slept): freeze the timer for that time.
            *ends_at += gap;
            self.accounted_until = now;
            return;
        }
        if self.phase != Phase::Work {
            self.accounted_until = now;
            return;
        }
        let until = now.min(*ends_at);
        let ms = until
            .saturating_duration_since(self.accounted_until)
            .as_millis() as u64;
        if ms > 0 {
            self.days.entry(today).or_default().work_ms += ms;
            // Advance by exactly what was credited so sub-millisecond remainders aren't lost.
            self.accounted_until += Duration::from_millis(ms);
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Duration = Duration::from_secs(1);
    const MIN: Duration = Duration::from_secs(60);

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn engine(settings: Settings) -> (Engine, Instant) {
        (Engine::new(settings, BTreeMap::new()), Instant::now())
    }

    /// Ticks every `step` from `from` until `to`, collecting finished events.
    fn run(
        e: &mut Engine,
        from: Instant,
        to: Instant,
        step: Duration,
        day: NaiveDate,
    ) -> Vec<PeriodFinished> {
        let mut events = Vec::new();
        let mut t = from;
        while t < to {
            t = (t + step).min(to);
            events.extend(e.tick(t, day));
        }
        events
    }

    #[test]
    fn work_period_finishes_and_starts_break() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        let events = run(
            &mut e,
            t0,
            t0 + 25 * MIN + SEC,
            Duration::from_millis(200),
            date(15),
        );
        assert_eq!(
            events,
            vec![PeriodFinished {
                finished: Phase::Work,
                next: Phase::Break,
                next_started: true
            }]
        );
        assert_eq!(e.phase(), Phase::Break);
        assert_eq!(e.status(), Status::Running);
        let day = e.day(date(15));
        assert_eq!(day.sessions, 1);
        assert_eq!(
            day.work(),
            25 * MIN,
            "only the work period counts, not the break"
        );
    }

    #[test]
    fn break_does_not_auto_start_work_by_default() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        let end = t0 + 30 * MIN + SEC;
        let events = run(&mut e, t0, end, SEC, date(15));
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            PeriodFinished {
                finished: Phase::Break,
                next: Phase::Work,
                next_started: false
            }
        );
        assert_eq!(e.status(), Status::Idle);
        assert_eq!(e.remaining(end), 25 * MIN);
        assert_eq!(e.day(date(15)).work(), 25 * MIN);
    }

    #[test]
    fn pause_freezes_remaining_time_and_hours() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        run(&mut e, t0, t0 + 10 * MIN, SEC, date(15));
        e.pause(t0 + 10 * MIN, date(15));
        assert_eq!(e.status(), Status::Paused);
        // A long pause: nothing changes.
        let later = t0 + 70 * MIN;
        assert_eq!(e.tick(later, date(15)), None);
        assert_eq!(e.remaining(later), 15 * MIN);
        assert_eq!(e.day(date(15)).work(), 10 * MIN);
        // Resume and finish.
        e.start(later);
        let events = run(&mut e, later, later + 15 * MIN, SEC, date(15));
        assert_eq!(events.len(), 1);
        assert_eq!(e.day(date(15)).work(), 25 * MIN);
    }

    #[test]
    fn reset_and_skip_keep_worked_time_but_not_sessions() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        run(&mut e, t0, t0 + 5 * MIN, SEC, date(15));
        e.reset(t0 + 5 * MIN, date(15));
        assert_eq!(e.status(), Status::Idle);
        assert_eq!(e.phase(), Phase::Work);
        assert_eq!(e.remaining(t0 + 5 * MIN), 25 * MIN);

        e.start(t0 + 6 * MIN);
        run(&mut e, t0 + 6 * MIN, t0 + 9 * MIN, SEC, date(15));
        e.skip(t0 + 9 * MIN, date(15));
        assert_eq!(e.phase(), Phase::Break);
        assert_eq!(e.status(), Status::Running, "breaks auto-start by default");
        let day = e.day(date(15));
        assert_eq!(day.work(), 8 * MIN);
        assert_eq!(day.sessions, 0);
    }

    #[test]
    fn work_time_is_split_across_midnight() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        run(&mut e, t0, t0 + 10 * MIN, SEC, date(15));
        let events = run(&mut e, t0 + 10 * MIN, t0 + 25 * MIN, SEC, date(16));
        assert_eq!(events.len(), 1);
        assert_eq!(e.day(date(15)).work(), 10 * MIN);
        assert_eq!(e.day(date(16)).work(), 15 * MIN);
        assert_eq!(e.day(date(16)).sessions, 1);
    }

    #[test]
    fn sleep_gap_is_treated_as_a_pause() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        run(&mut e, t0, t0 + 5 * MIN, SEC, date(15));
        // No ticks for an hour, e.g. the laptop lid was closed.
        let woke = t0 + 65 * MIN;
        assert_eq!(e.tick(woke, date(15)), None);
        assert_eq!(e.remaining(woke), 20 * MIN);
        assert_eq!(e.day(date(15)).work(), 5 * MIN);
    }

    #[test]
    fn changing_length_adjusts_the_running_period() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        let now = t0 + 10 * MIN;
        run(&mut e, t0, now, SEC, date(15));
        let old = e.settings.clone();
        e.settings.work_minutes = 50;
        e.settings_changed(&old, now);
        assert_eq!(e.remaining(now), 40 * MIN);

        let old = e.settings.clone();
        e.settings.work_minutes = 5;
        e.settings_changed(&old, now);
        assert_eq!(e.remaining(now), Duration::ZERO);
        assert!(e.tick(now, date(15)).is_some());
    }

    #[test]
    fn sub_millisecond_ticks_do_not_lose_time() {
        let (mut e, t0) = engine(Settings::default());
        e.start(t0);
        run(&mut e, t0, t0 + MIN, Duration::from_micros(1_700), date(15));
        assert_eq!(e.day(date(15)).work(), MIN);
    }
}
