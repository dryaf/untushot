//! Color selection anchored to its swatch, with explicit apply/cancel semantics.
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

pub struct Picker {
    pub button: gtk::MenuButton,
    value: Rc<Cell<gdk::RGBA>>,
    swatch: gtk::DrawingArea,
    apply: gtk::Button,
}

impl Picker {
    pub fn new(title: &str, initial: gdk::RGBA, side: gtk::PositionType) -> Self {
        let button = gtk::MenuButton::new();
        button.set_tooltip_text(Some(title));
        let value = Rc::new(Cell::new(initial));
        let swatch = gtk::DrawingArea::new();
        swatch.set_size_request(22, 18);
        let color = value.clone();
        swatch.connect_draw(move |area, cr| {
            let c = color.get();
            cr.set_source_rgb(c.red(), c.green(), c.blue());
            cr.rectangle(
                1.,
                1.,
                f64::from(area.allocated_width() - 2),
                f64::from(area.allocated_height() - 2),
            );
            let _ = cr.fill_preserve();
            let palette = crate::theme::Palette::for_widget(area);
            crate::theme::source(cr, &palette.foreground, 0.4);
            cr.set_line_width(1.);
            let _ = cr.stroke();
            glib::Propagation::Stop
        });
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.pack_start(&swatch, true, true, 0);
        if side == gtk::PositionType::Right {
            content.pack_end(
                &gtk::Image::from_icon_name(Some("pan-down-symbolic"), gtk::IconSize::Menu),
                false,
                false,
                0,
            );
        }
        button.add(&content);
        let popover = gtk::Popover::new(Some(&button));
        popover.set_position(side);
        button.set_popover(Some(&popover));
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_border_width(12);
        let heading = gtk::Label::new(Some(title));
        heading.set_xalign(0.);
        heading.style_context().add_class("caption");
        root.pack_start(&heading, false, false, 0);
        let choice = gtk::ColorChooserWidget::new();
        choice.set_use_alpha(false);
        choice.set_rgba(&initial);
        root.pack_start(&choice, false, false, 0);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let cancel = gtk::Button::with_label(crate::i18n::tr("Abbrechen"));
        let apply = gtk::Button::with_label(crate::i18n::tr("Übernehmen"));
        apply.style_context().add_class("suggested-action");
        actions.pack_end(&apply, false, false, 0);
        actions.pack_end(&cancel, false, false, 0);
        root.pack_start(&actions, false, false, 0);
        popover.add(&root);
        root.show_all();
        let color = value.clone();
        let chooser = choice.clone();
        button.connect_toggled(move |b| {
            if b.is_active() {
                chooser.set_rgba(&color.get());
                chooser.set_show_editor(false);
            }
        });
        let weak = popover.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.popdown();
            }
        });
        let color = value.clone();
        let sample = swatch.downgrade();
        let weak = popover.downgrade();
        let chooser = choice.clone();
        apply.connect_clicked(move |_| {
            color.set(chooser.rgba());
            if let Some(sample) = sample.upgrade() {
                sample.queue_draw();
            }
            if let Some(p) = weak.upgrade() {
                p.popdown();
            }
        });
        let weak = apply.downgrade();
        choice.connect_color_activated(move |_, _| {
            if let Some(apply) = weak.upgrade() {
                apply.emit_clicked();
            }
        });
        Self {
            button,
            value,
            swatch,
            apply,
        }
    }

    pub fn rgba(&self) -> gdk::RGBA {
        self.value.get()
    }
    pub fn set_rgba(&self, value: &gdk::RGBA) {
        self.value.set(*value);
        self.swatch.queue_draw();
    }
    pub fn set_sensitive(&self, sensitive: bool) {
        self.button.set_sensitive(sensitive);
    }
    pub fn connect_color_set(&self, f: impl Fn(gdk::RGBA) + 'static) {
        let value = self.value.clone();
        self.apply.connect_clicked(move |_| f(value.get()));
    }
}
