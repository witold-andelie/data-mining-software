//! Modal lifecycle decisions. The caller owns the pending engine command.
use crate::i18n::{tr, tr_fmt};
use egui::{Context, Id, Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardChoice {
    Save,
    Discard,
    Cancel,
}

pub fn confirm_discard(ctx: &Context, name: &str) -> Option<DiscardChoice> {
    let mut choice = None;
    let response = egui::Modal::new(Id::unique("confirm-discard")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.heading(tr_fmt(
            "Do you want to save the changes you made to {name}?",
            &[("name", name)],
        ));
        ui.label(tr("Your changes will be lost if you don't save them."));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(tr("Don't Save")).clicked() {
                choice = Some(DiscardChoice::Discard);
            }
            if ui.button(tr("Cancel")).clicked() {
                choice = Some(DiscardChoice::Cancel);
            }
            if ui
                .add(egui::Button::new(tr("Save")).fill(ui.visuals().selection.bg_fill))
                .clicked()
            {
                choice = Some(DiscardChoice::Save);
            }
        });
    });
    if response.is_top_modal {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            choice = Some(DiscardChoice::Cancel);
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            choice = Some(DiscardChoice::Save);
        }
    }
    choice
}
