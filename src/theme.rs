//! Resolve colours from GTK's active theme; GNOME/Yaru supplies the chosen
//! accent and light/dark variant. Never change the user's desktop settings.
use gtk::prelude::*;

pub struct Palette {
    pub background: gdk::RGBA,
    pub foreground: gdk::RGBA,
    pub base: gdk::RGBA,
    pub accent: gdk::RGBA,
    pub border: gdk::RGBA,
}
impl Palette {
    pub fn for_widget(widget: &impl IsA<gtk::Widget>) -> Self {
        let context = widget.style_context();
        let foreground = context.color(gtk::StateFlags::NORMAL);
        let background = context
            .lookup_color("theme_bg_color")
            .unwrap_or_else(|| gdk::RGBA::new(0.95, 0.95, 0.95, 1.));
        Self {
            base: context
                .lookup_color("theme_base_color")
                .unwrap_or(background),
            accent: context
                .lookup_color("accent_bg_color")
                .or_else(|| context.lookup_color("theme_selected_bg_color"))
                .unwrap_or(foreground),
            border: context.lookup_color("borders").unwrap_or(foreground),
            foreground,
            background,
        }
    }
}
pub fn source(cr: &cairo::Context, color: &gdk::RGBA, opacity: f64) {
    cr.set_source_rgba(
        color.red(),
        color.green(),
        color.blue(),
        color.alpha() * opacity,
    );
}
