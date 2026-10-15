//! The menu bar icon (macOS) / system tray icon (Windows) and its menu.
//!
//! Linux is not supported yet: tray icons there need a GTK event loop on a
//! thread of their own, so `Tray::new` returns `None`.

use crate::icon::TrayLook;

pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// What the user picked from the tray menu.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))] // no tray there yet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Skip,
    Open,
    Quit,
}

/// Everything the tray shows; [`Tray::update`] only touches what changed.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub look: TrayLook,
    pub tooltip: String,
    /// The first, greyed-out menu line, e.g. "Work · 12:34 left".
    pub status: String,
    pub toggle: &'static str,
    pub skip: &'static str,
    /// Text next to the icon (macOS only).
    pub title: Option<String>,
}

pub use imp::Tray;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::{Action, View};
    use crate::icon;

    /// Pixels per side; the menu bar shows it at 22pt, so this is crisp on Retina.
    const ICON_SIZE: u32 = 44;

    pub struct Tray {
        icon: TrayIcon,
        status: MenuItem,
        toggle: MenuItem,
        skip: MenuItem,
        clicked: Arc<AtomicBool>,
        shown: Option<View>,
    }

    impl Tray {
        /// Must be called on the main thread once the event loop is running.
        /// `on_action` runs on the main thread whenever a menu item is picked.
        pub fn new(view: View, on_action: impl Fn(Action) + Send + Sync + 'static) -> Option<Self> {
            let status = MenuItem::new(&view.status, false, None);
            let toggle = MenuItem::new(view.toggle, true, None);
            let skip = MenuItem::new(view.skip, true, None);
            let open = MenuItem::new("Open TimerShift", true, None);
            let quit = MenuItem::new("Quit TimerShift", true, None);
            let menu = Menu::with_items(&[
                &status,
                &PredefinedMenuItem::separator(),
                &toggle,
                &skip,
                &PredefinedMenuItem::separator(),
                &open,
                &quit,
            ])
            .inspect_err(|err| eprintln!("timershift: could not build the tray menu: {err}"))
            .ok()?;

            let actions = [
                (toggle.id().clone(), Action::Toggle),
                (skip.id().clone(), Action::Skip),
                (open.id().clone(), Action::Open),
                (quit.id().clone(), Action::Quit),
            ];
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some((_, action)) = actions.iter().find(|(id, _)| *id == event.id) {
                    on_action(*action);
                }
            }));

            // Clicks count as "seen it" and stop the blinking. Handling the events here
            // also keeps them (mouse moves included) from piling up in a channel.
            let clicked = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&clicked);
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if matches!(event, TrayIconEvent::Click { .. }) {
                    flag.store(true, Ordering::Relaxed);
                }
            }));

            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip(&view.tooltip)
                .with_icon(render(&view)?)
                .build()
                .inspect_err(|err| eprintln!("timershift: could not create the tray icon: {err}"))
                .ok()?;
            if let Some(title) = &view.title {
                icon.set_title(Some(title));
            }

            Some(Self {
                icon,
                status,
                toggle,
                skip,
                clicked,
                shown: Some(view),
            })
        }

        /// Whether the icon was clicked since the last call.
        pub fn take_clicked(&self) -> bool {
            self.clicked.swap(false, Ordering::Relaxed)
        }

        pub fn update(&mut self, view: View) {
            let old = self.shown.take();
            let old = old.as_ref();
            if old.is_none_or(|old| old.look.key() != view.look.key())
                && let Some(image) = render(&view)
                && let Err(err) = self.icon.set_icon(Some(image))
            {
                eprintln!("timershift: could not update the tray icon: {err}");
            }
            if old.is_none_or(|old| old.tooltip != view.tooltip) {
                let _ = self.icon.set_tooltip(Some(&view.tooltip));
            }
            if old.is_none_or(|old| old.status != view.status) {
                self.status.set_text(&view.status);
            }
            if old.is_none_or(|old| old.toggle != view.toggle) {
                self.toggle.set_text(view.toggle);
            }
            if old.is_none_or(|old| old.skip != view.skip) {
                self.skip.set_text(view.skip);
            }
            if old.is_none_or(|old| old.title != view.title) {
                self.icon.set_title(view.title.as_deref());
            }
            self.shown = Some(view);
        }
    }

    fn render(view: &View) -> Option<Icon> {
        let rgba = icon::tray_rgba(ICON_SIZE, &view.look);
        Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE)
            .inspect_err(|err| eprintln!("timershift: bad tray icon: {err}"))
            .ok()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::{Action, View};

    pub struct Tray;

    impl Tray {
        pub fn new(
            _view: View,
            _on_action: impl Fn(Action) + Send + Sync + 'static,
        ) -> Option<Self> {
            None
        }

        pub fn take_clicked(&self) -> bool {
            false
        }

        pub fn update(&mut self, _view: View) {}
    }
}
