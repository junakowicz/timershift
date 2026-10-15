//! The egui user interface, plus the background thread that drives the timer.
//!
//! The timer lives in a [`Shared`] engine ticked by its own thread, so periods end
//! and notifications fire on time even while the window is hidden or minimised.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Days, Local, NaiveDate};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Shape, Stroke, Vec2};

use crate::engine::{Engine, PeriodFinished, Phase, Settings, Status};
use crate::icon::TrayLook;
use crate::{notify, storage, tray};

const APP_NAME: &str = "TimerShift";
const WORK_COLOR: Color32 = Color32::from_rgb(232, 93, 77);
const BREAK_COLOR: Color32 = Color32::from_rgb(52, 168, 107);
const TICK: Duration = Duration::from_millis(200);
const SAVE_EVERY: Duration = Duration::from_secs(15);
/// How long the menu bar icon blinks after a period ends.
const BLINK_FOR: Duration = Duration::from_secs(30);
const BLINKS_PER_SECOND: f32 = 2.0;
/// One breath of the pulsing break icon, in seconds.
const PULSE_PERIOD: f32 = 2.0;
/// Frame interval while the menu bar icon is animating.
const ANIMATION_FRAME: Duration = Duration::from_millis(100);

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn phase_color(phase: Phase) -> Color32 {
    match phase {
        Phase::Work => WORK_COLOR,
        Phase::Break => BREAK_COLOR,
    }
}

/// State shared between the UI thread and the ticker thread.
struct Shared {
    engine: Mutex<Engine>,
    data_path: Option<PathBuf>,
    /// Set when a period ends, so the UI can ask the OS for the user's attention.
    attention: AtomicBool,
    /// Set by the UI after a change that should reach the disk soon.
    save_requested: AtomicBool,
}

impl Shared {
    fn engine(&self) -> MutexGuard<'_, Engine> {
        self.engine.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn save_if_dirty(&self) {
        let Some(path) = &self.data_path else { return };
        let data = {
            let mut engine = self.engine();
            if !engine.take_dirty() {
                return;
            }
            storage::SavedData {
                settings: engine.settings.clone(),
                days: engine.days.clone(),
            }
        };
        if let Err(err) = storage::save(path, &data) {
            eprintln!("timershift: could not save {}: {err}", path.display());
            self.engine().mark_dirty();
        }
    }
}

fn spawn_ticker(shared: Arc<Shared>, ctx: egui::Context) {
    thread::Builder::new()
        .name("timershift-ticker".into())
        .spawn(move || {
            let mut last_save = Instant::now();
            loop {
                thread::sleep(TICK);
                let (finished, running, settings) = {
                    let mut engine = shared.engine();
                    let finished = engine.tick(Instant::now(), today());
                    (
                        finished,
                        engine.status() == Status::Running,
                        engine.settings.clone(),
                    )
                };
                if let Some(finished) = &finished {
                    announce(finished, &settings);
                    shared.attention.store(true, Ordering::Relaxed);
                }
                let save_requested = shared.save_requested.swap(false, Ordering::Relaxed);
                if finished.is_some() || save_requested || last_save.elapsed() >= SAVE_EVERY {
                    shared.save_if_dirty();
                    last_save = Instant::now();
                }
                if running || finished.is_some() {
                    ctx.request_repaint();
                }
            }
        })
        .expect("failed to start the timer thread");
}

fn announce(event: &PeriodFinished, settings: &Settings) {
    let sound = settings.sound && (settings.notifications || notify::CHIME_WITHOUT_NOTIFICATIONS);
    if !settings.notifications && !sound {
        return;
    }
    let next_minutes = settings.minutes(event.next);
    let (title, body) = match (event.finished, event.next_started) {
        (Phase::Work, true) => (
            "Time for a break",
            format!("Nice work! Your {next_minutes}-minute break has started."),
        ),
        (Phase::Work, false) => (
            "Time for a break",
            format!("Work period done. Start your {next_minutes}-minute break when you're ready."),
        ),
        (Phase::Break, true) => (
            "Break is over",
            format!("Back to work: {next_minutes} minutes on the clock."),
        ),
        (Phase::Break, false) => (
            "Break is over",
            format!(
                "Press Start when you're ready for the next {next_minutes}-minute work period."
            ),
        ),
    };
    notify::announce(title, &body, settings.notifications, sound);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Timer,
    History,
    Settings,
}

/// The menu bar icon blinks after a period ends, until it times out, the icon is
/// clicked, or the timer is started, paused or skipped.
struct Blink {
    started: Instant,
    /// Phase and status when blinking began; any change counts as acknowledged.
    state: (Phase, Status),
}

pub struct TimerApp {
    shared: Arc<Shared>,
    tray: Option<tray::Tray>,
    /// Picked from the tray menu; handled in `logic` on the main thread.
    tray_actions: mpsc::Receiver<tray::Action>,
    /// Set by the tray's Quit, so that closing isn't turned into hiding.
    quitting: bool,
    blink: Option<Blink>,
    /// Clock for the pulse animation.
    started: Instant,
    tab: Tab,
    window_title: String,
}

impl TimerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
        });

        let data_path = storage::data_path();
        let data = data_path.as_deref().map(storage::load).unwrap_or_default();
        let shared = Arc::new(Shared {
            engine: Mutex::new(Engine::new(data.settings, data.days)),
            data_path,
            attention: AtomicBool::new(false),
            save_requested: AtomicBool::new(false),
        });
        spawn_ticker(Arc::clone(&shared), cc.egui_ctx.clone());

        let started = Instant::now();
        let view = tray_view(&shared.engine(), started, None, 0.0);
        let (sender, tray_actions) = mpsc::channel();
        let tray = tray::Tray::new(view, {
            let ctx = cc.egui_ctx.clone();
            move |action| {
                let _ = sender.send(action);
                // Runs `logic` even while the window is hidden.
                ctx.request_repaint();
            }
        });

        Self {
            shared,
            tray,
            tray_actions,
            quitting: false,
            blink: None,
            started,
            tab: Tab::Timer,
            window_title: APP_NAME.to_owned(),
        }
    }
}

impl TimerApp {
    fn handle_tray_action(&mut self, ctx: &egui::Context, action: tray::Action, now: Instant) {
        match action {
            tray::Action::Toggle => self.shared.engine().toggle(now, today()),
            tray::Action::Skip => self.shared.engine().skip(now, today()),
            tray::Action::Open => show_window(ctx),
            tray::Action::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

fn hide_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    tray::set_in_dock(false);
}

fn show_window(ctx: &egui::Context) {
    tray::set_in_dock(true);
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

impl eframe::App for TimerApp {
    /// Runs before every frame, and also while the window is hidden or minimised.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let actions: Vec<_> = self.tray_actions.try_iter().collect();
        for action in actions {
            self.handle_tray_action(ctx, action, now);
        }
        // With a tray icon to come back to, closing the window just hides it.
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            hide_window(ctx);
        }

        let engine = self.shared.engine();
        if self.shared.attention.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
            self.blink = Some(Blink {
                started: now,
                state: (engine.phase(), engine.status()),
            });
        }

        let Some(tray) = &mut self.tray else { return };
        let clicked = tray.take_clicked();
        if let Some(blink) = &self.blink
            && (clicked
                || now.duration_since(blink.started) >= BLINK_FOR
                || blink.state != (engine.phase(), engine.status()))
        {
            self.blink = None;
        }
        let clock = now.duration_since(self.started).as_secs_f32();
        let view = tray_view(&engine, now, self.blink.as_ref(), clock);
        let pulsing = view.look.dot > 0.0;
        drop(engine);
        tray.update(view);
        if pulsing || self.blink.is_some() {
            ctx.request_repaint_after(ANIMATION_FRAME);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let shared = Arc::clone(&self.shared);
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        // ⌘W (Ctrl+W on Windows) closes the window like its close button.
        let close_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::W);
        if self.tray.is_some() && ui.input_mut(|i| i.consume_shortcut(&close_shortcut)) {
            hide_window(&ctx);
        }
        let mut engine = shared.engine();

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Timer, "Timer");
                ui.selectable_value(&mut self.tab, Tab::History, "History");
                ui.selectable_value(&mut self.tab, Tab::Settings, "Settings");
            });
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Timer => timer_tab(ui, &mut engine, now),
            Tab::History => history_tab(ui, &engine),
            Tab::Settings => {
                let before = engine.settings.clone();
                settings_tab(ui, &mut engine.settings, shared.data_path.as_deref());
                if engine.settings != before {
                    engine.settings_changed(&before, now);
                    shared.save_requested.store(true, Ordering::Relaxed);
                }
            }
        });

        let title = window_title(&engine, now);
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn on_exit(&mut self) {
        // Credit the last moments of a running work period before saving.
        self.shared.engine().tick(Instant::now(), today());
        self.shared.save_if_dirty();
    }
}

fn timer_tab(ui: &mut egui::Ui, engine: &mut Engine, now: Instant) {
    if ui.input(|i| i.key_pressed(egui::Key::Space)) && ui.memory(|m| m.focused().is_none()) {
        engine.toggle(now, today());
    }
    let phase = engine.phase();
    let color = phase_color(phase);

    ui.vertical_centered(|ui| {
        ui.add_space(6.0);
        let state = match engine.status() {
            Status::Idle => "ready",
            Status::Running => "in progress",
            Status::Paused => "paused",
        };
        ui.label(
            RichText::new(format!("{} · {state}", phase.label().to_uppercase()))
                .color(color)
                .strong(),
        );
        ui.add_space(6.0);
        let caption = format!("of {} min", engine.settings.minutes(phase));
        progress_ring(
            ui,
            engine.progress(now),
            color,
            engine.remaining(now),
            &caption,
        );
        ui.add_space(12.0);
        controls(ui, engine, now, color);
        ui.add_space(2.0);
        let next = engine.phase().next();
        ui.label(
            RichText::new(format!(
                "Up next: {} · {} min",
                next.label().to_lowercase(),
                engine.settings.minutes(next)
            ))
            .weak()
            .small(),
        );

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        let day = engine.day(today());
        ui.label(RichText::new("Worked today").weak());
        ui.label(RichText::new(format_hours(day.work())).size(28.0).strong());
        let sessions = match day.sessions {
            1 => "1 session completed".to_owned(),
            n => format!("{n} sessions completed"),
        };
        ui.label(RichText::new(sessions).weak());
    });
}

fn progress_ring(
    ui: &mut egui::Ui,
    progress: f32,
    color: Color32,
    remaining: Duration,
    caption: &str,
) {
    let size = ui.available_width().min(220.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let width = 10.0;
    let radius = size / 2.0 - width;

    painter.circle_stroke(
        center,
        radius,
        Stroke::new(width, color.gamma_multiply(0.18)),
    );
    if progress > 0.0 {
        let point = |t: f32| {
            let angle = TAU * progress * t;
            center + radius * Vec2::new(angle.sin(), -angle.cos())
        };
        let segments = (progress * 128.0).ceil().max(2.0) as usize;
        let points = (0..=segments)
            .map(|i| point(i as f32 / segments as f32))
            .collect();
        painter.add(Shape::line(points, Stroke::new(width, color)));
        painter.circle_filled(point(0.0), width / 2.0, color);
        painter.circle_filled(point(1.0), width / 2.0, color);
    }

    painter.text(
        center - Vec2::new(0.0, 6.0),
        Align2::CENTER_CENTER,
        format_clock(remaining),
        FontId::monospace(size * 0.21),
        ui.visuals().strong_text_color(),
    );
    painter.text(
        center + Vec2::new(0.0, size * 0.17),
        Align2::CENTER_CENTER,
        caption,
        FontId::proportional(13.0),
        ui.visuals().weak_text_color(),
    );
}

fn controls(ui: &mut egui::Ui, engine: &mut Engine, now: Instant, color: Color32) {
    let status = engine.status();
    let main_label = match status {
        Status::Running => "⏸  Pause",
        Status::Paused => "⏵  Resume",
        Status::Idle => "⏵  Start",
    };
    let main_size = Vec2::new(112.0, 34.0);
    let side_size = Vec2::new(80.0, 34.0);
    let total = main_size.x + 2.0 * side_size.x + 2.0 * ui.spacing().item_spacing.x;

    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        let main = egui::Button::new(RichText::new(main_label).color(Color32::WHITE).strong())
            .fill(color)
            .min_size(main_size);
        if ui.add(main).on_hover_text("Space").clicked() {
            engine.toggle(now, today());
        }
        let reset = egui::Button::new("⟲  Reset").min_size(side_size);
        if ui
            .add_enabled(status != Status::Idle, reset)
            .on_hover_text("Restart this period")
            .clicked()
        {
            engine.reset(now, today());
        }
        let skip_hint = format!("Skip to {}", engine.phase().next().label().to_lowercase());
        let skip = egui::Button::new("⏭  Skip").min_size(side_size);
        if ui.add(skip).on_hover_text(skip_hint).clicked() {
            engine.skip(now, today());
        }
    });
}

fn history_tab(ui: &mut egui::Ui, engine: &Engine) {
    let today = today();
    let week: Vec<_> = (0..7)
        .rev()
        .map(|days_ago| {
            let date = today - Days::new(days_ago);
            (date, engine.day(date))
        })
        .collect();
    // Scale bars to the longest day, but never shorter than an hour.
    let longest = week
        .iter()
        .map(|(_, day)| day.work_ms)
        .max()
        .unwrap_or(0)
        .max(3_600_000);

    ui.add_space(4.0);
    section(ui, "Last 7 days");
    egui::Grid::new("history")
        .num_columns(4)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label("");
            ui.label("");
            ui.label(RichText::new("Hours").weak().small());
            ui.label(RichText::new("Sessions").weak().small());
            ui.end_row();
            for (date, day) in &week {
                let name = if *date == today {
                    "Today".to_owned()
                } else {
                    date.format("%a %-d").to_string()
                };
                ui.label(name);
                hours_bar(ui, day.work_ms as f32 / longest as f32);
                ui.label(RichText::new(format_hours(day.work())).strong());
                ui.label(day.sessions.to_string());
                ui.end_row();
            }
        });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    let week_ms: u64 = week.iter().map(|(_, day)| day.work_ms).sum();
    let days_worked = week.iter().filter(|(_, day)| day.work_ms > 0).count() as u64;
    let all_time_ms: u64 = engine.days.values().map(|day| day.work_ms).sum();
    egui::Grid::new("totals")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (label, ms) in [
                ("Last 7 days", week_ms),
                ("Average per day worked", week_ms / days_worked.max(1)),
                ("All time", all_time_ms),
            ] {
                ui.label(label);
                ui.label(RichText::new(format_hours(Duration::from_millis(ms))).strong());
                ui.end_row();
            }
        });
}

fn hours_bar(ui: &mut egui::Ui, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(110.0, 12.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, ui.visuals().widgets.inactive.bg_fill);
    if fraction > 0.0 {
        let mut filled = rect;
        filled.set_width((rect.width() * fraction).max(rect.height()));
        painter.rect_filled(filled, 6.0, WORK_COLOR);
    }
}

fn settings_tab(ui: &mut egui::Ui, settings: &mut Settings, data_path: Option<&Path>) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(4.0);
        section(ui, "Periods");
        egui::Grid::new("periods")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Work");
                ui.add(
                    egui::DragValue::new(&mut settings.work_minutes)
                        .range(1..=240)
                        .speed(0.25)
                        .suffix(" min"),
                );
                ui.end_row();
                ui.label("Break");
                ui.add(
                    egui::DragValue::new(&mut settings.break_minutes)
                        .range(1..=120)
                        .speed(0.25)
                        .suffix(" min"),
                );
                ui.end_row();
            });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Presets").weak());
            for (work, rest) in [(25, 5), (50, 10), (90, 20)] {
                let selected = settings.work_minutes == work && settings.break_minutes == rest;
                if ui
                    .selectable_label(selected, format!("{work} / {rest}"))
                    .clicked()
                {
                    settings.work_minutes = work;
                    settings.break_minutes = rest;
                }
            }
        });

        ui.add_space(12.0);
        section(ui, "Flow");
        ui.checkbox(
            &mut settings.auto_start_breaks,
            "Start breaks automatically",
        );
        ui.checkbox(
            &mut settings.auto_start_work,
            "Start work automatically after a break",
        );

        ui.add_space(12.0);
        section(ui, "Notifications");
        ui.checkbox(&mut settings.notifications, "Notify me when a period ends");
        ui.add_enabled(
            settings.notifications || notify::CHIME_WITHOUT_NOTIFICATIONS,
            egui::Checkbox::new(&mut settings.sound, "Play a sound"),
        );
        if ui.button("Test notification and sound").clicked() {
            notify::announce(APP_NAME, "Notifications are working.", true, settings.sound);
        }

        if tray::SUPPORTED {
            ui.add_space(12.0);
            section(
                ui,
                if cfg!(target_os = "macos") {
                    "Menu bar"
                } else {
                    "System tray"
                },
            );
            ui.checkbox(&mut settings.tray_pulse, "Pulse the icon during breaks");
            if cfg!(target_os = "macos") {
                ui.checkbox(
                    &mut settings.tray_time,
                    "Show the remaining time next to the icon",
                );
            }
        }

        if let Some(path) = data_path {
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!("Data is saved to {}", path.display()))
                    .weak()
                    .small(),
            );
        }
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).strong().size(15.0));
    ui.add_space(2.0);
}

/// What the menu bar icon and its menu show right now. `clock` drives the animations.
fn tray_view(engine: &Engine, now: Instant, blink: Option<&Blink>, clock: f32) -> tray::View {
    let phase = engine.phase();
    let status = engine.status();
    let color = phase_color(phase);
    let mut look = TrayLook {
        color: [color.r(), color.g(), color.b()],
        progress: engine.progress(now),
        track: 0.3,
        dot: 0.0,
        opacity: 1.0,
    };
    match status {
        Status::Idle => look.track = 0.7,
        Status::Paused => look.opacity = 0.5,
        Status::Running if phase == Phase::Break && engine.settings.tray_pulse => {
            let breath = 0.5 - 0.5 * (TAU * clock / PULSE_PERIOD).cos();
            look.dot = 0.14 + 0.18 * breath;
        }
        Status::Running => {}
    }
    if let Some(blink) = blink {
        let t = now.duration_since(blink.started).as_secs_f32();
        if (t * BLINKS_PER_SECOND).fract() >= 0.5 {
            look.opacity *= 0.15;
        }
    }

    let remaining = format_clock(engine.remaining(now));
    let (status_line, toggle) = match status {
        Status::Idle => (
            format!(
                "{} · {} min, ready",
                phase.label(),
                engine.settings.minutes(phase)
            ),
            if phase == Phase::Work {
                "Start work"
            } else {
                "Start break"
            },
        ),
        Status::Running => (format!("{} · {remaining} left", phase.label()), "Pause"),
        Status::Paused => (
            format!("{} · paused, {remaining} left", phase.label()),
            "Resume",
        ),
    };
    tray::View {
        look,
        tooltip: format!("{APP_NAME} · {status_line}"),
        status: status_line,
        toggle,
        skip: if phase == Phase::Work {
            "Skip to break"
        } else {
            "Skip to work"
        },
        title: (engine.settings.tray_time && status != Status::Idle).then_some(remaining),
    }
}

fn window_title(engine: &Engine, now: Instant) -> String {
    let clock = format_clock(engine.remaining(now));
    let phase = engine.phase().label();
    match engine.status() {
        Status::Idle => APP_NAME.to_owned(),
        Status::Running => format!("{clock} {phase} — {APP_NAME}"),
        Status::Paused => format!("{clock} {phase} (paused) — {APP_NAME}"),
    }
}

/// `mm:ss` (or `h:mm:ss`), rounded up so the display reaches 00:00 exactly at the end.
fn format_clock(remaining: Duration) -> String {
    let secs = remaining.as_millis().div_ceil(1000) as u64;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn format_hours(worked: Duration) -> String {
    let minutes = worked.as_secs() / 60;
    let (h, m) = (minutes / 60, minutes % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_rounds_up_to_whole_seconds() {
        assert_eq!(format_clock(Duration::from_secs(25 * 60)), "25:00");
        assert_eq!(
            format_clock(Duration::from_millis(24 * 60_000 + 59_001)),
            "25:00"
        );
        assert_eq!(format_clock(Duration::from_millis(1)), "00:01");
        assert_eq!(format_clock(Duration::ZERO), "00:00");
        assert_eq!(format_clock(Duration::from_secs(90 * 60)), "1:30:00");
    }

    #[test]
    fn hours_are_shown_as_hours_and_minutes() {
        assert_eq!(format_hours(Duration::from_secs(59)), "0m");
        assert_eq!(format_hours(Duration::from_secs(25 * 60)), "25m");
        assert_eq!(
            format_hours(Duration::from_secs(3 * 3600 + 5 * 60)),
            "3h 05m"
        );
    }

    #[test]
    fn tray_icon_follows_the_timer() {
        let mut engine = Engine::new(Settings::default(), Default::default());
        let t0 = Instant::now();
        let idle = tray_view(&engine, t0, None, 0.0);
        assert_eq!(idle.toggle, "Start work");
        assert_eq!(idle.look.progress, 0.0);
        assert_eq!(idle.title, None);

        engine.start(t0);
        let five_min = t0 + Duration::from_secs(5 * 60);
        let work = tray_view(&engine, five_min, None, 0.0);
        assert!((work.look.progress - 0.2).abs() < 1e-3);
        assert_eq!(work.look.dot, 0.0, "only breaks pulse");
        assert_eq!(work.status, "Work · 20:00 left");
        assert_eq!(work.toggle, "Pause");

        engine.skip(five_min, today());
        assert_eq!(
            (engine.phase(), engine.status()),
            (Phase::Break, Status::Running)
        );
        let exhale = tray_view(&engine, five_min, None, 0.0);
        let inhale = tray_view(&engine, five_min, None, PULSE_PERIOD / 2.0);
        assert!(exhale.look.dot > 0.0 && inhale.look.dot > exhale.look.dot);

        let blink = Blink {
            started: five_min,
            state: (Phase::Break, Status::Running),
        };
        let on = tray_view(&engine, five_min, Some(&blink), 0.0);
        let off = tray_view(
            &engine,
            five_min + Duration::from_millis(300),
            Some(&blink),
            0.0,
        );
        assert!(off.look.opacity < on.look.opacity);

        engine.settings.tray_pulse = false;
        assert_eq!(tray_view(&engine, five_min, None, 0.0).look.dot, 0.0);
    }
}
