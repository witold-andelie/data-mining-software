//! Renders the application icon. Ignored unless asked for:
//! `DMS_WRITE_ICON=1 cargo test -p dms-ui --test icon -- --ignored` writes
//! `apps/dms/assets/icon.png` (1024 × 1024), the master for `cargo xtask bundle-mac`.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test tool")]

use dms_ui::icon::paint_app_icon;
use egui::{Rect, Sense, vec2};
use egui_kittest::Harness;
use std::path::Path;

#[test]
#[ignore = "writes apps/dms/assets/icon.png when DMS_WRITE_ICON is set"]
fn render_app_icon() {
    if std::env::var_os("DMS_WRITE_ICON").is_none() {
        return;
    }
    let mut harness = Harness::builder()
        .with_size(vec2(1024.0, 1024.0))
        .with_pixels_per_point(1.0)
        .build_ui(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(1024.0, 1024.0), Sense::hover());
            let rect = Rect::from_min_size(rect.min, rect.size());
            paint_app_icon(ui.painter(), rect);
        });
    // Transparent around the rounded square.
    harness.ctx.set_theme(egui::Theme::Light);
    harness.ctx.style_mut_of(egui::Theme::Light, |style| {
        style.visuals.panel_fill = egui::Color32::TRANSPARENT;
        style.visuals.window_fill = egui::Color32::TRANSPARENT;
    });
    harness.run();
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/dms/assets/icon.png");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let mut image = harness.render().unwrap();
    // The renderer clears to an opaque colour: make everything outside the rounded square
    // transparent, with antialiased edges (macOS icon grid: body 100..924, radius 185).
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let coverage = rounded_square_coverage(x as f32 + 0.5, y as f32 + 0.5);
        pixel.0[3] = (f32::from(pixel.0[3]) * coverage).round() as u8;
    }
    image.save(&out).unwrap();
    println!("wrote {}", out.display());
}

/// How much of the pixel centred at (x, y) lies inside the icon body (0 outside, 1 inside).
fn rounded_square_coverage(x: f32, y: f32) -> f32 {
    let (lo, hi, radius) = (100.0_f32, 924.0_f32, 185.0_f32);
    let cx = x.clamp(lo + radius, hi - radius);
    let cy = y.clamp(lo + radius, hi - radius);
    let inside_corner_box = (x - cx).abs() > 0.0 && (y - cy).abs() > 0.0;
    let distance = if inside_corner_box {
        ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - radius
    } else {
        (lo - x).max(x - hi).max(lo - y).max(y - hi)
    };
    (0.5 - distance).clamp(0.0, 1.0)
}
