use crate::geometry::{Layout, Monitor, Point, Rect};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone)]
pub struct MonitorMap {
    pub area: gtk::DrawingArea,
    monitors: Rc<RefCell<Vec<Monitor>>>,
    active: Rc<Cell<Option<usize>>>,
}

pub fn projected(monitors: &[Monitor], width: f64, height: f64) -> Vec<Rect> {
    let Some(layout) = Layout::new(monitors.to_vec()) else {
        return Vec::new();
    };
    let b = layout.bounds;
    let scale = ((width - 24.) / b.w).min((height - 24.) / b.h).max(0.001);
    let x = (width - b.w * scale) / 2.;
    let y = (height - b.h * scale) / 2.;
    monitors
        .iter()
        .map(|m| {
            Rect::new(
                x + (m.rect.x - b.x) * scale,
                y + (m.rect.y - b.y) * scale,
                m.rect.w * scale,
                m.rect.h * scale,
            )
        })
        .collect()
}

impl MonitorMap {
    pub fn monitors(&self) -> Vec<Monitor> {
        self.monitors.borrow().clone()
    }
    pub fn new(monitors: Vec<Monitor>) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_size_request(300, 130);
        area.set_hexpand(true);
        area.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
        area.set_tooltip_text(Some(crate::i18n::tr(
            "Bildschirm anklicken · Anordnung entspricht deinem Desktop",
        )));
        let map = Self {
            area,
            monitors: Rc::new(RefCell::new(monitors)),
            active: Rc::new(Cell::new(None)),
        };
        let state = map.clone();
        map.area.connect_draw(move |area, cr| {
            let monitors = state.monitors.borrow();
            let rects = projected(
                &monitors,
                area.allocated_width() as f64,
                area.allocated_height() as f64,
            );
            let palette = crate::theme::Palette::for_widget(area);
            crate::theme::source(cr, &palette.base, 1.);
            let _ = cr.paint();
            for (i, r) in rects.iter().enumerate() {
                let selected = state.active.get().is_none() || state.active.get() == Some(i);
                if selected {
                    crate::theme::source(cr, &palette.accent, 0.22);
                } else {
                    crate::theme::source(cr, &palette.foreground, 0.06);
                }
                cr.rectangle(r.x + 1., r.y + 1., (r.w - 2.).max(1.), (r.h - 2.).max(1.));
                let _ = cr.fill_preserve();
                crate::theme::source(
                    cr,
                    if selected {
                        &palette.accent
                    } else {
                        &palette.border
                    },
                    1.,
                );
                cr.set_line_width(if selected { 2. } else { 1. });
                let _ = cr.stroke();
                crate::theme::source(cr, &palette.foreground, 1.);
                cr.set_font_size(18.);
                let label = (i + 1).to_string();
                let ext = cr.text_extents(&label).ok();
                cr.move_to(
                    r.x + r.w / 2. - ext.map(|e| e.width() / 2.).unwrap_or(5.),
                    r.y + r.h / 2. + 6.,
                );
                let _ = cr.show_text(&label);
            }
            if rects.is_empty() {
                crate::theme::source(cr, &palette.foreground, 0.7);
                cr.set_font_size(12.);
                cr.move_to(12., 32.);
                let _ = cr.show_text(crate::i18n::tr(
                    "Keine eindeutige Monitorzuordnung vorhanden",
                ));
            }
            glib::Propagation::Stop
        });
        map
    }
    pub fn set_monitors(&self, monitors: Vec<Monitor>) {
        *self.monitors.borrow_mut() = monitors;
        self.active.set(None);
        self.area.queue_draw();
    }
    pub fn select(&self, index: Option<usize>) {
        self.active.set(index);
        self.area.queue_draw();
    }
    pub fn connect_selected(&self, callback: impl Fn(usize) + 'static) {
        let map = self.clone();
        self.area.connect_button_press_event(move |area, event| {
            if event.button() != 1 {
                return glib::Propagation::Proceed;
            }
            let (x, y) = event.position();
            let rects = projected(
                &map.monitors.borrow(),
                area.allocated_width() as f64,
                area.allocated_height() as f64,
            );
            if let Some(index) = rects.iter().position(|r| r.contains(Point::new(x, y))) {
                map.select(Some(index));
                callback(index);
            }
            glib::Propagation::Stop
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projects_negative_and_portrait_layout_without_reordering() {
        let m = vec![
            Monitor {
                name: "L".into(),
                rect: Rect::new(-1080., -400., 1080., 1920.),
                scale: 1.,
            },
            Monitor {
                name: "R".into(),
                rect: Rect::new(0., 0., 1920., 1080.),
                scale: 2.,
            },
        ];
        let r = projected(&m, 600., 200.);
        assert!(r[0].x < r[1].x && r[0].y < r[1].y);
        assert!(r[0].h > r[0].w && r[1].w > r[1].h);
        assert!((r[0].right() - r[1].x).abs() < 1e-8);
    }
}
