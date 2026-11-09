//! The app ships no fonts (docs/gui/DESIGN.md §11.4): every glyph comes from the system through
//! eframe's system font provider. This renders English, Czech, Chinese and the menu symbols with
//! no bundled fonts at all; kittest panics on any glyph nothing can draw. Needs a desktop OS with
//! its standard fonts (macOS, Windows).
#![allow(clippy::unwrap_used, reason = "test assertions")]

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::Arc;

const TEXT: &str = "Run Stream · Výběr příjmu ≥ 18 · Žluťoučký kůň · 数据挖掘 · ⌘⇧⌥⌃ · 1 234,5";

#[test]
fn system_fonts_cover_the_interface_text() {
    // Nothing is drawn until the fonts are set up (`show` stays false).
    let mut harness = Harness::builder()
        .with_size(vec2(900.0, 120.0))
        .build_ui_state(
            |ui, show: &mut bool| {
                if *show {
                    ui.label(TEXT);
                    ui.heading("Data Mining");
                    ui.monospace("  Age <= 30 → yes (n = 120, 85.0%)");
                }
            },
            false,
        );
    harness.ctx.set_fonts(egui::FontDefinitions::empty());
    harness.ctx.set_font_providers(vec![Arc::new(
        egui_system_font_provider::SystemFontProvider::new(),
    )]);
    // The provider loads system fonts in the background: draw boxes until they arrive (as the
    // app does), then require every glyph.
    harness
        .ctx
        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    *harness.state_mut() = true;
    for _ in 0..200 {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    harness
        .ctx
        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
    harness.run();
    harness.get_by_label(TEXT);
}
