//! Native symbolic theme icons, with colour-inheriting vector fallbacks for drawing tools.
use gtk::prelude::*;

const SLOT: i32 = 20;
const GLYPH: i32 = 16;

pub fn widget(name: &'static str) -> gtk::Widget {
    let symbolic = match name {
        "menu" => Some("open-menu-symbolic"),
        "camera" => Some("camera-photo-symbolic"),
        "monitor" => Some("video-display-symbolic"),
        "copy" => Some("edit-copy-symbolic"),
        "paste" => Some("edit-paste-symbolic"),
        "image" => Some("image-x-generic-symbolic"),
        "web" => Some("web-browser-symbolic"),
        "save" => Some("document-save-symbolic"),
        "export" => Some("document-save-as-symbolic"),
        "open" => Some("document-open-symbolic"),
        "undo" => Some("edit-undo-symbolic"),
        "redo" => Some("edit-redo-symbolic"),
        "trash" => Some("user-trash-symbolic"),
        "up" => Some("go-up-symbolic"),
        "down" => Some("go-down-symbolic"),
        "plus" => Some("zoom-in-symbolic"),
        "minus" => Some("zoom-out-symbolic"),
        "fit" => Some("zoom-fit-best-symbolic"),
        "timer" => Some("alarm-symbolic"),
        "close" => Some("window-close-symbolic"),
        "quit" => Some("application-exit-symbolic"),
        "eye" => Some("view-reveal-symbolic"),
        "eye-off" => Some("view-conceal-symbolic"),
        "lock" => Some("changes-prevent-symbolic"),
        "unlock" => Some("changes-allow-symbolic"),
        _ => None,
    };
    if let Some(icon) =
        symbolic.filter(|name| gtk::IconTheme::default().is_some_and(|theme| theme.has_icon(name)))
    {
        let image = gtk::Image::from_icon_name(Some(icon), gtk::IconSize::Menu);
        image.set_pixel_size(GLYPH);
        image.set_size_request(SLOT, SLOT);
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        return image.upcast();
    }

    let area = gtk::DrawingArea::new();
    area.set_size_request(SLOT, SLOT);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.connect_draw(move |area, cr| {
        let color = area.style_context().color(area.state_flags());
        cr.set_source_rgba(color.red(), color.green(), color.blue(), color.alpha());
        cr.set_line_width(1.7);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.translate(
            (area.allocated_width() as f64 - GLYPH as f64) / 2.0,
            (area.allocated_height() as f64 - GLYPH as f64) / 2.0,
        );
        cr.scale(GLYPH as f64 / 20., GLYPH as f64 / 20.);
        let line = |pts: &[(f64, f64)]| {
            if let Some((x, y)) = pts.first() {
                cr.move_to(*x, *y);
                for (x, y) in &pts[1..] {
                    cr.line_to(*x, *y);
                }
                let _ = cr.stroke();
            }
        };
        match name {
            "lock" | "unlock" => {
                cr.rectangle(4., 9., 12., 9.);
                let _ = cr.stroke();
                cr.move_to(6., 9.);
                cr.line_to(6., 6.);
                cr.curve_to(6., 1., 14., 1., 14., 6.);
                if name == "lock" {
                    cr.line_to(14., 9.);
                }
                let _ = cr.stroke();
                line(&[(10., 12.), (10., 15.)]);
            }
            "fill" => {
                line(&[(3., 9.), (10., 2.), (17., 9.), (10., 16.), (3., 9.)]);
                line(&[(6., 2.), (10., 6.)]);
                line(&[(3., 9.), (17., 9.)]);
                cr.move_to(17., 12.);
                cr.curve_to(13., 17., 20., 19., 17., 12.);
                let _ = cr.fill();
            }
            "selection" => {
                cr.set_dash(&[3., 2.], 0.);
                cr.rectangle(2., 3., 16., 14.);
                let _ = cr.stroke();
            }
            "eye" | "eye-off" => {
                cr.move_to(1., 10.);
                cr.curve_to(6., 2., 14., 2., 19., 10.);
                cr.curve_to(14., 18., 6., 18., 1., 10.);
                let _ = cr.stroke();
                cr.arc(10., 10., 2.5, 0., std::f64::consts::TAU);
                let _ = cr.stroke();
                if name == "eye-off" {
                    line(&[(3., 3.), (17., 17.)]);
                }
            }
            "menu" => {
                for y in [5., 10., 15.] {
                    line(&[(3., y), (17., y)]);
                }
            }
            "close" => {
                line(&[(5., 5.), (15., 15.)]);
                line(&[(5., 15.), (15., 5.)]);
            }
            "quit" => {
                line(&[(9., 3.), (3., 3.), (3., 17.), (9., 17.)]);
                line(&[(8., 10.), (18., 10.)]);
                line(&[(14., 6.), (18., 10.), (14., 14.)]);
            }
            "move" => {
                line(&[
                    (4., 2.),
                    (4., 16.),
                    (8., 12.),
                    (11., 18.),
                    (14., 16.),
                    (11., 10.),
                    (17., 10.),
                    (4., 2.),
                ]);
            }
            "crop" => {
                line(&[(5., 2.), (5., 15.), (18., 15.)]);
                line(&[(2., 5.), (15., 5.), (15., 18.)]);
            }
            "pen" => {
                line(&[
                    (3., 16.),
                    (4., 12.),
                    (14., 2.),
                    (18., 6.),
                    (8., 16.),
                    (3., 17.),
                ]);
                line(&[(12., 4.), (16., 8.)]);
            }
            "line" => line(&[(3., 17.), (17., 3.)]),
            "arrow" => {
                line(&[(3., 17.), (17., 3.)]);
                line(&[(8., 3.), (17., 3.), (17., 12.)]);
            }
            "rectangle" => {
                cr.rectangle(3., 4., 14., 12.);
                let _ = cr.stroke();
            }
            "ellipse" => {
                let _ = cr.save();
                cr.translate(10., 10.);
                cr.scale(1., 0.75);
                cr.arc(0., 0., 7., 0., std::f64::consts::TAU);
                let _ = cr.restore();
                let _ = cr.stroke();
            }
            "highlight" => {
                line(&[(5., 12.), (13., 3.), (18., 8.), (10., 16.), (5., 12.)]);
                line(&[(2., 18.), (16., 18.)]);
            }
            "redact" => {
                cr.rectangle(3., 5., 14., 10.);
                let _ = cr.fill();
            }
            "text" => {
                line(&[(3., 5.), (3., 3.), (17., 3.), (17., 5.)]);
                line(&[(10., 3.), (10., 17.)]);
                line(&[(6., 17.), (14., 17.)]);
            }
            "camera" => {
                line(&[
                    (2., 6.),
                    (6., 6.),
                    (8., 3.),
                    (12., 3.),
                    (14., 6.),
                    (18., 6.),
                    (18., 17.),
                    (2., 17.),
                    (2., 6.),
                ]);
                cr.arc(10., 11., 3., 0., std::f64::consts::TAU);
                let _ = cr.stroke();
            }
            "image" => {
                cr.rectangle(3., 4., 14., 12.);
                let _ = cr.stroke();
                line(&[(3., 13.), (7., 9.), (10., 12.), (13., 8.), (17., 13.)]);
                cr.arc(7., 7., 1., 0., std::f64::consts::TAU);
                let _ = cr.fill();
            }
            "monitor" => {
                cr.rectangle(2., 3., 16., 11.);
                let _ = cr.stroke();
                line(&[(10., 14.), (10., 18.)]);
                line(&[(6., 18.), (14., 18.)]);
            }
            "web" => {
                cr.arc(10., 10., 8., 0., std::f64::consts::TAU);
                let _ = cr.stroke();
                line(&[(2., 10.), (18., 10.)]);
                let _ = cr.save();
                cr.translate(10., 10.);
                cr.scale(0.5, 1.);
                cr.arc(0., 0., 8., 0., std::f64::consts::TAU);
                let _ = cr.stroke();
                let _ = cr.restore();
            }
            "paste" => {
                cr.rectangle(4., 4., 12., 14.);
                let _ = cr.stroke();
                cr.rectangle(7., 2., 6., 4.);
                let _ = cr.stroke();
                line(&[(7., 10.), (13., 10.)]);
                line(&[(7., 14.), (13., 14.)]);
            }
            "copy" => {
                cr.rectangle(7., 7., 11., 11.);
                let _ = cr.stroke();
                line(&[(13., 4.), (13., 2.), (2., 2.), (2., 13.), (4., 13.)]);
            }
            "save" => {
                line(&[
                    (3., 2.),
                    (15., 2.),
                    (18., 5.),
                    (18., 18.),
                    (2., 18.),
                    (2., 2.),
                    (3., 2.),
                ]);
                cr.rectangle(6., 2., 7., 5.);
                cr.rectangle(6., 12., 8., 6.);
                let _ = cr.stroke();
            }
            "export" => {
                line(&[(10., 2.), (10., 13.)]);
                line(&[(6., 9.), (10., 13.), (14., 9.)]);
                line(&[(3., 13.), (3., 18.), (17., 18.), (17., 13.)]);
            }
            "open" => {
                line(&[
                    (2., 16.),
                    (2., 4.),
                    (8., 4.),
                    (10., 7.),
                    (18., 7.),
                    (16., 17.),
                    (2., 17.),
                    (5., 9.),
                    (16., 9.),
                ]);
            }
            "undo" => {
                line(&[(7., 3.), (2., 8.), (7., 13.)]);
                cr.move_to(2., 8.);
                cr.curve_to(18., 4., 21., 17., 12., 18.);
                let _ = cr.stroke();
            }
            "redo" => {
                line(&[(13., 3.), (18., 8.), (13., 13.)]);
                cr.move_to(18., 8.);
                cr.curve_to(2., 4., -1., 17., 8., 18.);
                let _ = cr.stroke();
            }
            "layers" => {
                line(&[(2., 6.), (10., 2.), (18., 6.), (10., 10.), (2., 6.)]);
                line(&[(2., 10.), (10., 14.), (18., 10.)]);
                line(&[(2., 14.), (10., 18.), (18., 14.)]);
            }
            "trash" => {
                line(&[(3., 5.), (17., 5.)]);
                line(&[(7., 5.), (7., 2.), (13., 2.), (13., 5.)]);
                line(&[(5., 7.), (6., 18.), (14., 18.), (15., 7.)]);
                line(&[(9., 8.), (9., 15.)]);
                line(&[(12., 8.), (12., 15.)]);
            }
            "up" => {
                line(&[(4., 12.), (10., 6.), (16., 12.)]);
            }
            "down" => {
                line(&[(4., 8.), (10., 14.), (16., 8.)]);
            }
            "plus" => {
                line(&[(3., 10.), (17., 10.)]);
                line(&[(10., 3.), (10., 17.)]);
            }
            "minus" => line(&[(3., 10.), (17., 10.)]),
            "fit" => {
                line(&[(7., 3.), (3., 3.), (3., 7.)]);
                line(&[(13., 3.), (17., 3.), (17., 7.)]);
                line(&[(3., 13.), (3., 17.), (7., 17.)]);
                line(&[(13., 17.), (17., 17.), (17., 13.)]);
            }
            "timer" => {
                cr.arc(10., 11., 7., 0., std::f64::consts::TAU);
                let _ = cr.stroke();
                line(&[(7., 1.), (13., 1.)]);
                line(&[(10., 7.), (10., 11.), (13., 11.)]);
            }
            _ => {}
        }
        glib::Propagation::Stop
    });
    area.upcast()
}

pub fn content(icon: &'static str, label: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);
    row.pack_start(&widget(icon), false, false, 0);
    if !label.is_empty() {
        let text = gtk::Label::new(Some(label));
        text.set_xalign(0.0);
        text.set_valign(gtk::Align::Center);
        row.pack_start(&text, true, true, 0);
    }
    row
}

/// Center the complete icon/label group, including in stretched or linked buttons.
/// List and menu rows use `content` instead so their text columns stay aligned.
pub fn button_content(icon: &'static str, label: &str) -> gtk::Box {
    let row = content(icon, label);
    row.set_halign(gtk::Align::Center);
    row
}

pub fn icon_control(button: &impl IsA<gtk::Button>) {
    let button: &gtk::Button = button.as_ref();
    button.style_context().add_class("image-button");
    button.style_context().add_class("icon-control");
    button.set_valign(gtk::Align::Center);
}

pub fn accessible_name(widget: &impl IsA<gtk::Widget>, name: &str) {
    use gtk::atk::prelude::*;
    if let Some(accessible) = widget.accessible() {
        accessible.set_name(name);
    }
}

/// Composite buttons have no GtkButton label property. Give assistive tools
/// the same text (or tooltip for icon-only actions) that sighted users see.
pub fn name_controls(widget: &gtk::Widget) {
    fn labels(widget: &gtk::Widget) -> Vec<gtk::Label> {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return vec![label.clone()];
        }
        widget
            .downcast_ref::<gtk::Container>()
            .map(|c| c.children().iter().flat_map(labels).collect())
            .unwrap_or_default()
    }
    fn update(button: &gtk::Button) {
        let text = labels(button.upcast_ref())
            .iter()
            .map(|label| label.text().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let name = if text.is_empty() {
            button
                .tooltip_text()
                .map(|s| s.to_string())
                .unwrap_or_default()
        } else {
            text
        };
        if !name.is_empty() {
            accessible_name(button, &name);
        }
    }
    if let Some(button) = widget
        .downcast_ref::<gtk::Button>()
        .filter(|b| b.label().is_none())
    {
        update(button);
        // Native labelled buttons already track their labels. Composite controls
        // need to announce changes too (capture mode, countdown, current size).
        for label in labels(widget) {
            let weak = button.downgrade();
            label.connect_label_notify(move |_| {
                if let Some(button) = weak.upgrade() {
                    update(&button);
                }
            });
        }
        button.connect_tooltip_text_notify(update);
    }
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            name_controls(&child);
        }
    }
}

pub fn tool(tool: crate::document::Tool) -> &'static str {
    use crate::document::Tool::*;
    match tool {
        Marquee => "selection",
        Fill => "fill",
        Erase => "trash",
        Move => "move",
        Select => "crop",
        Pen => "pen",
        Line => "line",
        Arrow => "arrow",
        Rectangle => "rectangle",
        Ellipse => "ellipse",
        Highlight => "highlight",
        Redact => "redact",
        Text => "text",
        Image(_) => "image",
    }
}
