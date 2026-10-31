//! Native stroke-width chooser. Samples share a fixed 1:2 scale, independent
//! of canvas zoom. Hovering never mutates the document.
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

const PRESETS: [u32; 12] = [1, 2, 4, 6, 8, 12, 16, 24, 32, 48, 64, 100];

fn sample(width: impl Fn() -> f64 + 'static) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_size_request(110, 54);
    area.connect_draw(move |area, cr| {
        let palette = crate::theme::Palette::for_widget(area);
        crate::theme::source(cr, &palette.foreground, 1.);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_width(width() / 2.);
        let y = area.allocated_height() as f64 / 2.;
        cr.move_to(30., y);
        cr.line_to(area.allocated_width() as f64 - 30., y);
        let _ = cr.stroke();
        glib::Propagation::Stop
    });
    area
}

pub fn chooser(width: &gtk::SpinButton) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_widget_name("stroke-width");
    button.set_tooltip_text(Some(crate::i18n::tr(
        "Linienstärke auswählen · Vorschau und exakter Pixelwert",
    )));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let current = gtk::Label::new(Some(&crate::i18n::format!("{} px", width.value_as_int())));
    content.pack_start(&current, true, true, 0);
    content.pack_end(
        &gtk::Image::from_icon_name(Some("pan-down-symbolic"), gtk::IconSize::Menu),
        false,
        false,
        0,
    );
    button.add(&content);
    let popover = gtk::Popover::new(Some(&button));
    popover.set_position(gtk::PositionType::Right);
    button.set_popover(Some(&popover));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_border_width(12);
    let title = gtk::Label::new(Some(crate::i18n::tr("Linienstärke")));
    title.set_xalign(0.);
    title.style_context().add_class("caption");
    root.pack_start(&title, false, false, 0);
    let hint = gtk::Label::new(Some(crate::i18n::tr("Vorschau 1:2 · Werte in Bildpixeln")));
    hint.set_xalign(0.);
    hint.style_context().add_class("muted");
    root.pack_start(&hint, false, false, 0);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(6);
    grid.set_row_spacing(6);
    let mut choices = Vec::new();
    // GTK also emits clicked when set_active changes a toggle programmatically.
    let syncing = Rc::new(Cell::new(false));
    for (i, value) in PRESETS.into_iter().enumerate() {
        let choice = gtk::ToggleButton::new();
        choice.set_widget_name(&crate::i18n::format!("stroke-{value}"));
        choice.style_context().add_class("flat");
        choice.style_context().add_class("stroke-choice");
        choice.set_active(width.value() == value as f64);
        choice.set_tooltip_text(Some(&crate::i18n::format!("{value} Bildpixel")));
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tile.pack_start(&sample(move || value as f64), false, false, 0);
        tile.pack_start(
            &gtk::Label::new(Some(&crate::i18n::format!("{value} px"))),
            false,
            false,
            0,
        );
        choice.add(&tile);
        let weak_width = width.downgrade();
        let weak_popover = popover.downgrade();
        let guard = syncing.clone();
        choice.connect_clicked(move |choice| {
            if guard.replace(true) {
                return;
            }
            if let Some(width) = weak_width.upgrade() {
                width.set_value(value as f64);
                choice.set_active(true);
            }
            guard.set(false);
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
        });
        grid.attach(&choice, i as i32 % 3, i as i32 / 3, 1, 1);
        choices.push((choice, value));
    }
    root.pack_start(&grid, false, false, 0);
    root.pack_start(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        false,
        false,
        4,
    );
    let custom = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(crate::i18n::tr("Eigene Stärke · px")));
    label.set_xalign(0.);
    custom.pack_start(&label, true, true, 0);
    custom.pack_end(width, false, false, 0);
    root.pack_start(&custom, false, false, 0);
    let weak_width = width.downgrade();
    let live = sample(move || weak_width.upgrade().map_or(1., |w| w.value()));
    root.pack_start(&live, false, false, 0);
    width.connect_value_changed(move |width| {
        let previous = syncing.replace(true);
        current.set_text(&crate::i18n::format!("{} px", width.value_as_int()));
        for (choice, value) in &choices {
            choice.set_active(width.value() == *value as f64);
        }
        syncing.set(previous);
        live.queue_draw();
    });
    popover.add(&root);
    root.show_all();
    button
}

#[cfg(test)]
pub fn preset(button: &gtk::MenuButton, value: u32) -> gtk::ToggleButton {
    let root = button
        .popover()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::Box>()
        .unwrap();
    let grid = root
        .children()
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Grid>().ok())
        .unwrap();
    grid.children()
        .into_iter()
        .find(|w| w.widget_name() == crate::i18n::format!("stroke-{value}"))
        .unwrap()
        .downcast()
        .unwrap()
}
