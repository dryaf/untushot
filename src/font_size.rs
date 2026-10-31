//! Anchored text-size presets. Browsing previews never edits the document.
use gtk::{pango, prelude::*};

const PRESETS: [u32; 16] = [6, 8, 10, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 64, 72, 96];

pub fn chooser(size: &gtk::SpinButton) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_focus_on_click(false);
    button.set_widget_name("font-size-presets");
    button.set_tooltip_text(Some(crate::i18n::tr(
        "Schriftgröße auswählen · Sample-Vorschau",
    )));
    button.set_image(Some(&gtk::Image::from_icon_name(
        Some("pan-down-symbolic"),
        gtk::IconSize::Menu,
    )));
    let popover = gtk::Popover::new(Some(&button));
    button.set_popover(Some(&popover));
    gtk::prelude::MenuButtonExt::set_direction(&button, gtk::ArrowType::Right);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_border_width(10);
    let title = gtk::Label::new(Some(&crate::i18n::format!(
        "Schriftgröße · {} px",
        size.value_as_int()
    )));
    title.set_xalign(0.);
    title.style_context().add_class("caption");
    root.pack_start(&title, false, false, 0);
    let hint = gtk::Label::new(Some(crate::i18n::tr(
        "Vorschau 75 % · Größen in Bildpixeln",
    )));
    hint.set_xalign(0.);
    hint.style_context().add_class("muted");
    root.pack_start(&hint, false, false, 0);

    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(280);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    for value in PRESETS {
        let row = gtk::ListBoxRow::new();
        row.set_widget_name(&crate::i18n::format!("font-size-{value}"));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.set_border_width(4);
        let label = gtk::Label::new(Some(&crate::i18n::format!("{value} px")));
        label.set_width_chars(4);
        label.set_xalign(1.);
        content.pack_start(&label, false, false, 0);
        let sample = gtk::Label::new(Some(crate::i18n::tr("Sample")));
        sample.set_xalign(0.);
        let mut font = pango::FontDescription::from_string("sans-serif Bold");
        font.set_absolute_size(f64::from(value) * 0.75 * f64::from(pango::SCALE));
        let attributes = pango::AttrList::new();
        attributes.insert(pango::AttrFontDesc::new(&font));
        sample.set_attributes(Some(&attributes));
        content.pack_start(&sample, true, true, 0);
        row.add(&content);
        list.add(&row);
    }
    let sync_selection = |list: &gtk::ListBox, size: f64| {
        let row = PRESETS
            .iter()
            .position(|v| f64::from(*v) == size)
            .and_then(|index| list.row_at_index(index as i32));
        list.select_row(row.as_ref());
    };
    sync_selection(&list, size.value());
    let weak_list = list.downgrade();
    size.connect_value_changed(move |size| {
        title.set_text(&crate::i18n::format!(
            "Schriftgröße · {} px",
            size.value_as_int()
        ));
        if let Some(list) = weak_list.upgrade() {
            sync_selection(&list, size.value());
        }
    });
    let weak_size = size.downgrade();
    let weak_popover = popover.downgrade();
    list.connect_row_activated(move |_, row| {
        if let Some(size) = weak_size.upgrade() {
            size.set_value(f64::from(PRESETS[row.index() as usize]));
        }
        if let Some(popover) = weak_popover.upgrade() {
            popover.popdown();
        }
    });
    let weak_list = list.downgrade();
    let weak_scroll = scroll.downgrade();
    let weak_size = size.downgrade();
    popover.connect_map(move |popover| {
        let weak_list = weak_list.clone();
        let weak_scroll = weak_scroll.clone();
        let weak_size = weak_size.clone();
        let allocated = std::cell::Cell::new(false);
        // Wait for the popover's final allocation before centering the current
        // preset. A focus request during map alone can leave it offscreen.
        popover.add_tick_callback(move |_, _| {
            if !allocated.replace(true) {
                return glib::ControlFlow::Continue;
            }
            if let (Some(list), Some(scroll), Some(size)) = (
                weak_list.upgrade(),
                weak_scroll.upgrade(),
                weak_size.upgrade(),
            ) {
                let index = PRESETS
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        (f64::from(**a) - size.value())
                            .abs()
                            .total_cmp(&(f64::from(**b) - size.value()).abs())
                    })
                    .map(|(i, _)| i)
                    .unwrap();
                if let Some(row) = list.row_at_index(index as i32) {
                    // Custom values remain unselected: focus must not imply
                    // that a neighboring preset is the actual font size.
                    if list.selected_row().as_ref() == Some(&row) {
                        row.grab_focus();
                    }
                    let a = scroll.vadjustment();
                    let center =
                        f64::from(row.allocation().y()) + f64::from(row.allocated_height()) / 2.;
                    a.set_value(
                        (center - a.page_size() / 2.)
                            .clamp(a.lower(), (a.upper() - a.page_size()).max(a.lower())),
                    );
                }
            }
            glib::ControlFlow::Break
        });
    });
    scroll.add(&list);
    root.pack_start(&scroll, true, true, 0);
    popover.add(&root);
    root.show_all();
    button
}
