//! Icons drawn in code, so there are no image assets to ship: the app icon and
//! the frames of the menu bar (tray) icon.
//!
//! Drawing happens in "icon units", where the image spans -1..1 on both axes.

use std::f32::consts::TAU;
use std::path::Path;

const APP_BACKGROUND: [f32; 3] = [232.0, 93.0, 77.0];

/// How the menu bar icon looks in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrayLook {
    pub color: [u8; 3],
    /// Elapsed part of the period, drawn as an arc clockwise from 12 o'clock.
    pub progress: f32,
    /// Opacity of the full ring behind the arc.
    pub track: f32,
    /// Radius of a filled centre dot in icon units (0 = no dot).
    pub dot: f32,
    /// Opacity of the whole icon, for dimming and blinking.
    pub opacity: f32,
}

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))] // no tray there yet
impl TrayLook {
    /// Frames with the same key look identical, so the icon needn't be redrawn.
    pub fn key(&self) -> [u16; 7] {
        let q = |v: f32, steps: f32| (v.clamp(0.0, 1.0) * steps).round() as u16;
        let [r, g, b] = self.color.map(u16::from);
        [
            r,
            g,
            b,
            q(self.progress, 200.0),
            q(self.track, 20.0),
            q(self.dot, 100.0),
            q(self.opacity, 40.0),
        ]
    }
}

/// An anti-aliased ring, sampled per pixel.
struct Ring {
    radius: f32,
    half_width: f32,
    /// Pixels per icon unit.
    scale: f32,
}

impl Ring {
    /// Coverage (0..1) of a shape at signed distance `dist` (positive = inside).
    fn cover(&self, dist: f32) -> f32 {
        (dist * self.scale + 0.5).clamp(0.0, 1.0)
    }

    /// Coverage of the whole ring, and of its arc running clockwise from
    /// 12 o'clock through `sweep` radians with round ends.
    fn sample(&self, x: f32, y: f32, sweep: f32) -> (f32, f32) {
        let band = self.cover(self.half_width - (x.hypot(y) - self.radius).abs());
        if sweep <= 0.0 {
            return (band, 0.0);
        }
        let angle = x.atan2(-y).rem_euclid(TAU);
        let arc = if angle <= sweep { band } else { 0.0 };
        let cap = |a: f32| {
            let dist = (x - a.sin() * self.radius).hypot(y + a.cos() * self.radius);
            self.cover(self.half_width - dist)
        };
        (band, arc.max(cap(0.0)).max(cap(sweep)))
    }
}

/// Straight (non-premultiplied) RGBA pixels, shading each pixel centre in icon units.
fn pixels(size: u32, mut shade: impl FnMut(f32, f32) -> [u8; 4]) -> Vec<u8> {
    let half = size as f32 / 2.0;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            out.extend(shade(
                (px as f32 + 0.5) / half - 1.0,
                (py as f32 + 0.5) / half - 1.0,
            ));
        }
    }
    out
}

/// The app icon: a tomato-red disc with a white progress ring.
pub fn rgba(size: u32) -> Vec<u8> {
    let ring = Ring {
        radius: 0.5,
        half_width: 0.075,
        scale: size as f32 / 2.0,
    };
    pixels(size, |x, y| {
        let (band, arc) = ring.sample(x, y, 0.72 * TAU);
        let white = arc.max(band * 0.3);
        let [r, g, b] = APP_BACKGROUND.map(|c| (c + (255.0 - c) * white) as u8);
        [r, g, b, (ring.cover(0.8 - x.hypot(y)) * 255.0) as u8]
    })
}

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
/// One frame of the menu bar icon: a coloured progress ring, optionally with a centre dot.
pub fn tray_rgba(size: u32, look: &TrayLook) -> Vec<u8> {
    let ring = Ring {
        radius: 0.62,
        half_width: 0.14,
        scale: size as f32 / 2.0,
    };
    pixels(size, |x, y| {
        let (band, arc) = ring.sample(x, y, look.progress.clamp(0.0, 1.0) * TAU);
        let dot = if look.dot > 0.0 {
            ring.cover(look.dot - x.hypot(y))
        } else {
            0.0
        };
        let alpha = arc.max(band * look.track).max(dot) * look.opacity;
        let [r, g, b] = look.color;
        [r, g, b, (alpha * 255.0) as u8]
    })
}

pub fn icon_data(size: u32) -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: rgba(size),
        width: size,
        height: size,
    }
}

pub fn write_png(path: &Path, size: u32) -> image::ImageResult<()> {
    image::save_buffer(
        path,
        &rgba(size),
        size,
        size,
        image::ExtendedColorType::Rgba8,
    )
}
