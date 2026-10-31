use crate::geometry::{Monitor, Point, Rect};
use anyhow::Result;
use cairo::{Context as Cairo, Format, ImageSurface};
use gdk::prelude::*;
use gdk_pixbuf::Pixbuf;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const UNDO_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Tool {
    Move,
    Select,
    Pen,
    Line,
    Arrow,
    Rectangle,
    Ellipse,
    Highlight,
    Redact,
    Text,
    Image(u64),
    Marquee,
}

impl Tool {
    pub const ALL: [(Tool, &'static str, &'static str); 11] = [
        (Self::Move, "Objekt bearbeiten", "V"),
        (Self::Select, "Zuschneiden", "1"),
        (Self::Pen, "Stift", "2"),
        (Self::Line, "Linie", "3"),
        (Self::Arrow, "Pfeil", "4"),
        (Self::Rectangle, "Rechteck", "5"),
        (Self::Ellipse, "Ellipse", "6"),
        (Self::Highlight, "Marker", "7"),
        (Self::Redact, "Schwärzen", "8"),
        (Self::Text, "Text", "9"),
        (Self::Marquee, "Auswahl", "M"),
    ];
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Annotation {
    #[serde(default)]
    pub effects: crate::effects::ImageEffects,
    /// A retained crop mask, transformed together with the object.
    #[serde(default)]
    pub clip: Option<Rect>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub font_size: Option<f64>,
    pub tool: Tool,
    pub points: Vec<Point>,
    pub color: (f64, f64, f64),
    pub width: f64,
    pub text: String,
    pub visible: bool,
}

impl Annotation {
    pub fn text_size(&self) -> f64 {
        self.font_size.unwrap_or(self.width * 4. + 12.)
    }
    pub fn name(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        if self.tool == Tool::Text {
            return self
                .text
                .lines()
                .next()
                .unwrap_or(crate::i18n::tr("Text"))
                .chars()
                .take(32)
                .collect();
        }
        Tool::ALL
            .iter()
            .find(|(tool, _, _)| *tool == self.tool)
            .map(|(_, name, _)| crate::i18n::tr(name).to_string())
            .unwrap_or_default()
    }
    pub fn bounds(&self) -> Rect {
        let Some(first) = self.points.first() else {
            return Rect::default();
        };
        if self.tool == Tool::Text {
            let size = self.text_size();
            let mut width: f64 = 1.0;
            if let Ok(surface) = ImageSurface::create(Format::ARgb32, 1, 1) {
                if let Ok(cr) = Cairo::new(&surface) {
                    cr.select_font_face(
                        "sans-serif",
                        cairo::FontSlant::Normal,
                        cairo::FontWeight::Bold,
                    );
                    cr.set_font_size(size);
                    for line in self.text.lines() {
                        if let Ok(e) = cr.text_extents(line) {
                            width = width.max(e.x_advance());
                        }
                    }
                }
            }
            return Rect::new(
                first.x,
                first.y,
                width,
                size * 1.2 * self.text.lines().count().max(1) as f64,
            );
        }
        let mut r = Rect::new(first.x, first.y, 0.0, 0.0);
        let mut right = first.x;
        let mut bottom = first.y;
        for p in &self.points {
            r.x = r.x.min(p.x);
            r.y = r.y.min(p.y);
            right = right.max(p.x);
            bottom = bottom.max(p.y);
        }
        r.w = right - r.x;
        r.h = bottom - r.y;
        r
    }
    pub fn translated(&self, dx: f64, dy: f64) -> Self {
        let mut a = self.clone();
        for p in &mut a.points {
            p.x += dx;
            p.y += dy;
        }
        if let Some(r) = &mut a.clip {
            r.x += dx;
            r.y += dy;
        }
        a
    }
    pub fn resized(&self, width: f64, height: f64) -> Self {
        let bounds = self.bounds();
        let mut a = self.clone();
        if self.tool == Tool::Text {
            let ratio = height / bounds.h.max(1.0);
            a.font_size = Some((self.text_size() * ratio).clamp(6., 512.));
        } else {
            for p in &mut a.points {
                let original = *p;
                if bounds.w > 0.0 {
                    p.x = bounds.x + (p.x - bounds.x) * width / bounds.w;
                } else if bounds.h > 0.0 {
                    p.x = bounds.x + (original.y - bounds.y) * width / bounds.h;
                }
                if bounds.h > 0.0 {
                    p.y = bounds.y + (p.y - bounds.y) * height / bounds.h;
                } else if bounds.w > 0.0 {
                    p.y = bounds.y + (original.x - bounds.x) * height / bounds.w;
                }
            }
        }
        if let Some(r) = &mut a.clip {
            let sx = width / bounds.w.max(1.);
            let sy = height / bounds.h.max(1.);
            *r = Rect::new(
                bounds.x + (r.x - bounds.x) * sx,
                bounds.y + (r.y - bounds.y) * sy,
                r.w * sx,
                r.h * sy,
            );
        }
        a
    }
    pub fn resized_from(&self, handle: crate::selection::Handle, dx: f64, dy: f64) -> Self {
        if dx == 0. && dy == 0. {
            return self.clone();
        }
        let r = self.bounds();
        let target = crate::selection::resize_object(
            r,
            handle,
            dx,
            dy,
            matches!(self.tool, Tool::Image(_) | Tool::Text),
        );
        if target == r {
            return self.clone();
        }
        let (width, height) = if self.tool == Tool::Text {
            let size = (self.text_size() * target.h / r.h).clamp(6., 512.);
            let ratio = size / self.text_size();
            (r.w * ratio, r.h * ratio)
        } else {
            (target.w, target.h)
        };
        let resized = self.resized(width, height);
        // Text has font-size limits and measured glyph bounds. Anchor the actual
        // result so even a clamped font size cannot move the opposite edge.
        let actual = resized.bounds();
        let anchored = crate::selection::anchored_size(r, actual.w, actual.h, handle);
        resized.translated(anchored.x - actual.x, anchored.y - actual.y)
    }
    fn hit(&self, p: Point, tolerance: f64) -> bool {
        if !self.visible
            || self
                .clip
                .is_some_and(|r| r.w <= 0. || r.h <= 0. || !r.contains(p))
        {
            return false;
        }
        if matches!(self.tool, Tool::Pen | Tool::Line | Tool::Arrow) {
            let distance = |a: Point, b: Point| {
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy).max(0.001))
                    .clamp(0.0, 1.0);
                (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
            };
            return self
                .points
                .windows(2)
                .any(|v| distance(v[0], v[1]) <= tolerance + self.width / 2.0)
                || self
                    .points
                    .first()
                    .is_some_and(|v| (p.x - v.x).hypot(p.y - v.y) <= tolerance);
        }
        let r = self.bounds();
        p.x >= r.x - tolerance
            && p.x <= r.right() + tolerance
            && p.y >= r.y - tolerance
            && p.y <= r.bottom() + tolerance
    }
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Snapshot {
    #[serde(skip)]
    id: u64,
    annotations: Vec<Annotation>,
    selection: Option<Rect>,
    #[serde(default)]
    canvas: Option<[i32; 2]>,
}

struct ImageAsset {
    surface: ImageSurface,
    png: Arc<String>,
}

pub struct Document {
    images: BTreeMap<u64, ImageAsset>,
    // Keep the background in Cairo's native format. Converting a multi-4K
    // Pixbuf on every pointer motion would copy hundreds of MB per frame.
    image: ImageSurface,
    pub monitors: Vec<Monitor>,
    current: Snapshot,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    revision: u64,
}

impl Document {
    pub fn new(image: Pixbuf, monitors: Vec<Monitor>) -> Result<Self> {
        let surface = ImageSurface::create(Format::ARgb32, image.width(), image.height())?;
        let cr = Cairo::new(&surface)?;
        cr.set_source_pixbuf(&image, 0.0, 0.0);
        cr.paint()?;
        Ok(Self::from_surface(surface, monitors))
    }
    fn from_surface(image: ImageSurface, monitors: Vec<Monitor>) -> Self {
        Self {
            image,
            images: BTreeMap::new(),
            monitors,
            current: Snapshot::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
        }
    }
    pub fn width(&self) -> i32 {
        self.current
            .canvas
            .map_or(self.image.width(), |size| size[0])
    }
    pub fn height(&self) -> i32 {
        self.current
            .canvas
            .map_or(self.image.height(), |size| size[1])
    }
    pub fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width() as f64, self.height() as f64)
    }
    pub fn is_composition(&self) -> bool {
        self.current.canvas.is_some()
    }
    pub fn monitors(&self) -> &[Monitor] {
        if self.is_composition() {
            &[]
        } else {
            &self.monitors
        }
    }
    /// Keep raster data outside snapshots: moving/undoing only copies geometry.
    pub fn insert_image(&mut self, image: Pixbuf, rect: Rect, label: String) -> Result<usize> {
        validate_canvas(i64::from(image.width()), i64::from(image.height()))?;
        crate::i18n::ensure!(
            [rect.x, rect.y, rect.w, rect.h]
                .iter()
                .all(|v| v.is_finite())
                && rect.w > 0.
                && rect.h > 0.,
            "Ungültige Ebenengröße"
        );
        self.ensure_image_capacity(i64::from(image.width()) * i64::from(image.height()))?;
        let asset = ImageAsset::new(image)?;
        self.checkpoint();
        let id = self.insert_asset(asset);
        self.current
            .annotations
            .push(image_annotation(id, label, rect));
        Ok(self.current.annotations.len() - 1)
    }

    pub fn copy_region_to_layer(&mut self, rect: Rect) -> Result<usize> {
        let (x, y, w, h) = rect
            .clamp(self.selection().unwrap_or(self.bounds()))
            .pixel_bounds(self.width(), self.height());
        let region = Rect::new(f64::from(x), f64::from(y), f64::from(w), f64::from(h));
        let image = self.export_region(region)?;
        self.insert_image(
            image,
            region,
            crate::i18n::tr("Kopierter Ausschnitt").into(),
        )
    }

    pub fn append_image_right(&mut self, image: Pixbuf, label: String) -> Result<usize> {
        let (x, y, w, h) = self
            .selection()
            .unwrap_or(self.bounds())
            .pixel_bounds(self.width(), self.height());
        let width = i64::from(w) + 24 + i64::from(image.width());
        let height = h.max(image.height());
        validate_canvas(width, i64::from(height))?;
        let base_pixels = if self.is_composition() {
            0
        } else {
            i64::from(w) * i64::from(h)
        };
        self.ensure_image_capacity(
            base_pixels + i64::from(image.width()) * i64::from(image.height()),
        )?;
        let view = Rect::new(x as f64, y as f64, w as f64, h as f64);
        // Prepare all fallible work before mutating history or the document.
        let asset = ImageAsset::new(image)?;
        let base = if self.is_composition() {
            None
        } else {
            let surface = ImageSurface::create(Format::ARgb32, w, h)?;
            let cr = Cairo::new(&surface)?;
            cr.set_source_surface(&self.image, -f64::from(x), -f64::from(y))?;
            cr.paint()?;
            Some(ImageAsset::new(surface_to_pixbuf(&surface)?)?)
        };
        let mut annotations = self.current.annotations.clone();
        for a in &mut annotations {
            a.clip = Some(a.clip.unwrap_or(view).clamp(view));
            *a = a.translated(-f64::from(x), -f64::from(y));
        }
        self.checkpoint();
        if let Some(base) = base {
            let id = self.insert_asset(base);
            annotations.insert(
                0,
                image_annotation(
                    id,
                    crate::i18n::tr("Ausgangsbild").into(),
                    Rect::new(0., 0., w as f64, h as f64),
                ),
            );
        }
        let rect = Rect::new(
            f64::from(w + 24),
            0.,
            asset.surface.width() as f64,
            asset.surface.height() as f64,
        );
        let id = self.insert_asset(asset);
        annotations.push(image_annotation(id, label, rect));
        let index = annotations.len() - 1;
        self.current.annotations = annotations;
        self.current.canvas = Some([width as i32, height]);
        self.current.selection = None;
        Ok(index)
    }
    fn ensure_image_capacity(&self, additional_pixels: i64) -> Result<()> {
        let retained: i64 = self
            .images_after_checkpoint()
            .into_iter()
            .filter_map(|id| self.images.get(&id))
            .map(|a| i64::from(a.surface.width()) * i64::from(a.surface.height()))
            .sum();
        crate::i18n::ensure!(retained + additional_pixels <= 128_000_000,
            "Zu viele Bildebenen im Speicher. Bitte das Projekt speichern und erneut öffnen, um den Rückgängig-Verlauf freizugeben.");
        Ok(())
    }

    fn insert_asset(&mut self, asset: ImageAsset) -> u64 {
        let id = (1..=self.images.len() as u64 + 1)
            .find(|id| !self.images.contains_key(id))
            .expect("a finite asset map always has a free identifier");
        self.images.insert(id, asset);
        id
    }
    pub fn content_id(&self) -> u64 {
        self.current.id
    }
    pub fn selection(&self) -> Option<Rect> {
        self.current.selection
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    // A new edit discards redo and may expire the oldest undo state. Use the
    // same prospective set for admission and cleanup; inspection itself must
    // preserve all assets until fallible image preparation has succeeded.
    fn images_after_checkpoint(&self) -> BTreeSet<u64> {
        self.undo
            .iter()
            .skip(self.undo.len().saturating_sub(UNDO_LIMIT - 1))
            .chain(std::iter::once(&self.current))
            .flat_map(|snapshot| snapshot.annotations.iter())
            .filter_map(|a| {
                if let Tool::Image(id) = a.tool {
                    Some(id)
                } else {
                    None
                }
            })
            .collect()
    }
    fn checkpoint(&mut self) {
        let used = self.images_after_checkpoint();
        self.revision += 1;
        if self.undo.len() >= UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(self.current.clone());
        self.current.id = self.revision;
        self.redo.clear();
        self.images.retain(|id, _| used.contains(id));
    }
    pub fn select(&mut self, rect: Option<Rect>) {
        let rect = rect
            .map(|r| r.clamp(self.bounds()))
            .filter(|r| r.w >= 1.0 && r.h >= 1.0);
        if rect != self.current.selection {
            self.checkpoint();
            self.current.selection = rect;
        }
    }
    pub fn add(&mut self, annotation: Annotation) {
        self.checkpoint();
        self.current.annotations.push(annotation);
    }
    pub fn annotations(&self) -> &[Annotation] {
        &self.current.annotations
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn replace_many(&mut self, replacements: Vec<(usize, Annotation)>) {
        let valid: Vec<_> = replacements
            .into_iter()
            .filter(|(i, a)| self.current.annotations.get(*i).is_some_and(|old| old != a))
            .collect();
        if valid.is_empty() {
            return;
        }
        self.checkpoint();
        for (i, a) in valid {
            self.current.annotations[i] = a;
        }
    }
    /// Continue a single property edit only while no other document action has
    /// occurred. Every preview gets a fresh revision for rendering and recovery.
    pub fn replace_property(
        &mut self,
        index: usize,
        annotation: Annotation,
        previous: Option<u64>,
    ) -> Option<u64> {
        let Some(old) = self.current.annotations.get(index) else {
            return None;
        };
        if *old == annotation {
            return previous;
        }
        if previous == Some(self.revision) && !self.undo.is_empty() && old.tool == annotation.tool {
            self.revision += 1;
            self.current.id = self.revision;
            self.redo.clear();
        } else {
            self.checkpoint();
        }
        self.current.annotations[index] = annotation;
        if self.undo.last().is_some_and(|before| {
            before.annotations == self.current.annotations
                && before.selection == self.current.selection
                && before.canvas == self.current.canvas
        }) {
            self.current = self.undo.pop().unwrap();
            return None;
        }
        Some(self.revision)
    }
    pub fn remove_many(&mut self, indices: &[usize]) {
        let mut indices: Vec<_> = indices
            .iter()
            .copied()
            .filter(|i| *i < self.current.annotations.len())
            .collect();
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return;
        }
        self.checkpoint();
        for i in indices.into_iter().rev() {
            self.current.annotations.remove(i);
        }
    }
    pub fn duplicate_many(&mut self, indices: &[usize]) -> Vec<usize> {
        let mut indices = indices.to_vec();
        indices.sort_unstable();
        indices.dedup();
        let copies: Vec<_> = indices
            .iter()
            .filter_map(|i| self.current.annotations.get(*i))
            .map(|a| {
                let mut copy = a.translated(12., 12.);
                copy.label = crate::i18n::format!("{} · Kopie", a.name());
                copy
            })
            .collect();
        if copies.is_empty() {
            return Vec::new();
        }
        self.checkpoint();
        let start = self.current.annotations.len();
        self.current.annotations.extend(copies);
        (start..self.current.annotations.len()).collect()
    }
    pub fn align(&mut self, indices: &[usize], alignment: Alignment) {
        let items: Vec<_> = indices
            .iter()
            .filter_map(|i| self.current.annotations.get(*i).map(|a| (*i, a.clone())))
            .collect();
        if items.len() < 2 {
            return;
        }
        let bounds = items
            .iter()
            .map(|(_, a)| a.bounds())
            .reduce(Rect::union)
            .unwrap();
        self.replace_many(
            items
                .into_iter()
                .map(|(i, a)| {
                    let r = a.bounds();
                    let (dx, dy) = match alignment {
                        Alignment::Left => (bounds.x - r.x, 0.),
                        Alignment::Center => (bounds.x + bounds.w / 2. - r.x - r.w / 2., 0.),
                        Alignment::Right => (bounds.right() - r.right(), 0.),
                        Alignment::Top => (0., bounds.y - r.y),
                        Alignment::Middle => (0., bounds.y + bounds.h / 2. - r.y - r.h / 2.),
                        Alignment::Bottom => (0., bounds.bottom() - r.bottom()),
                    };
                    (i, a.translated(dx, dy))
                })
                .collect(),
        );
    }
    pub fn hit_test(&self, point: Point, tolerance: f64) -> Option<usize> {
        self.current
            .annotations
            .iter()
            .rposition(|a| a.hit(point, tolerance))
    }
    pub fn replace(&mut self, index: usize, annotation: Annotation) {
        if self
            .current
            .annotations
            .get(index)
            .is_some_and(|a| *a != annotation)
        {
            self.checkpoint();
            self.current.annotations[index] = annotation;
        }
    }
    pub fn reorder(&mut self, index: usize, up: bool) -> usize {
        let target = if up {
            index.saturating_add(1)
        } else {
            index.saturating_sub(1)
        };
        if target < self.current.annotations.len() && target != index {
            self.checkpoint();
            self.current.annotations.swap(index, target);
            return target;
        }
        index
    }
    /// Move one layer to a gap in the original, bottom-to-top stack.
    pub fn move_layer(&mut self, index: usize, gap: usize) -> Option<usize> {
        let count = self.current.annotations.len();
        if index >= count || gap > count {
            return None;
        }
        let target = gap - usize::from(index < gap);
        if target != index {
            self.checkpoint();
            let layer = self.current.annotations.remove(index);
            self.current.annotations.insert(target, layer);
        }
        Some(target)
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.revision += 1;
            self.redo
                .push(std::mem::replace(&mut self.current, previous));
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.revision += 1;
            self.undo.push(std::mem::replace(&mut self.current, next));
        }
    }
    pub fn paint(&self, cr: &Cairo) -> Result<()> {
        self.paint_preview(cr, &[])
    }
    pub fn paint_preview(&self, cr: &Cairo, replacements: &[(usize, Annotation)]) -> Result<()> {
        cr.save()?;
        cr.rectangle(0.0, 0.0, self.bounds().w, self.bounds().h);
        cr.clip();
        self.paint_background(cr)?;
        for (index, original) in self.current.annotations.iter().enumerate() {
            let a = replacements
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, a)| a)
                .unwrap_or(original);
            self.paint_layer(cr, a)?;
        }
        cr.restore()?;
        Ok(())
    }
    pub fn paint_background(&self, cr: &Cairo) -> Result<()> {
        if !self.is_composition() {
            cr.set_source_surface(&self.image, 0., 0.)?;
            cr.paint()?;
        }
        Ok(())
    }
    pub fn paint_layer(&self, cr: &Cairo, a: &Annotation) -> Result<()> {
        if let Tool::Image(id) = a.tool {
            if a.visible {
                let asset = self
                    .images
                    .get(&id)
                    .ok_or_else(|| crate::i18n::anyhow!("Bildebene fehlt"))?;
                let r = a.bounds();
                cr.save()?;
                if let Some(clip) = a.clip {
                    cr.rectangle(clip.x, clip.y, clip.w, clip.h);
                    cr.clip();
                }
                a.effects.paint_shadow(cr, r)?;
                cr.save()?;
                cr.rectangle(r.x, r.y, r.w, r.h);
                cr.clip();
                cr.translate(r.x, r.y);
                cr.scale(
                    r.w / asset.surface.width() as f64,
                    r.h / asset.surface.height() as f64,
                );
                cr.set_source_surface(&asset.surface, 0., 0.)?;
                cr.source().set_extend(cairo::Extend::Pad);
                cr.source().set_filter(cairo::Filter::Bilinear);
                cr.paint()?;
                cr.restore()?;
                a.effects.paint_border(cr, r)?;
                cr.restore()?;
            }
        } else {
            paint_annotation(cr, a)?;
        }
        Ok(())
    }

    /// A bounded thumbnail of one layer, independent of other layers and visibility.
    pub fn layer_thumbnail(&self, index: usize, device_scale: i32) -> Result<ImageSurface> {
        crate::i18n::ensure!(
            (1..=8).contains(&device_scale),
            "Ungültige Vorschauskalierung"
        );
        let mut a = self
            .current
            .annotations
            .get(index)
            .ok_or_else(|| crate::i18n::anyhow!("Ebene fehlt"))?
            .clone();
        a.visible = true;
        // A thumbnail shows the object, independent of its canvas position.
        let origin = a.bounds();
        a = a.translated(-origin.x, -origin.y);
        let surface = ImageSurface::create(Format::ARgb32, 44 * device_scale, 34 * device_scale)?;
        surface.set_device_scale(f64::from(device_scale), f64::from(device_scale));
        let cr = Cairo::new(&surface)?;
        // Neutral checkerboard makes transparency and white objects visible.
        cr.set_source_rgb(0.94, 0.94, 0.94);
        cr.paint()?;
        cr.set_source_rgb(0.80, 0.80, 0.80);
        for y in 0..6 {
            for x in 0..8 {
                if (x + y) % 2 == 0 {
                    cr.rectangle(f64::from(x * 6), f64::from(y * 6), 6., 6.);
                }
            }
        }
        cr.fill()?;
        let mut r = a.bounds();
        if !matches!(a.tool, Tool::Image(_) | Tool::Text) {
            let padding = a.width / 2.
                + if a.tool == Tool::Arrow {
                    (a.width * 4.).max(12.) / 2.
                } else {
                    0.
                };
            r = Rect::new(
                r.x - padding,
                r.y - padding,
                r.w + padding * 2.,
                r.h + padding * 2.,
            );
        }
        if matches!(a.tool, Tool::Image(_)) {
            r = a.effects.bounds(r);
        }
        if let Some(clip) = a.clip {
            r = r.clamp(clip);
        }
        if r.w > 0. && r.h > 0. {
            let scale = (38. / r.w).min(28. / r.h);
            cr.save()?;
            cr.rectangle(2., 2., 40., 30.);
            cr.clip();
            cr.translate((44. - r.w * scale) / 2., (34. - r.h * scale) / 2.);
            cr.scale(scale, scale);
            cr.translate(-r.x, -r.y);
            self.paint_layer(&cr, &a)?;
            cr.restore()?;
        }
        cr.set_source_rgba(0., 0., 0., 0.25);
        cr.set_line_width(1.);
        cr.rectangle(0.5, 0.5, 43., 33.);
        cr.stroke()?;
        Ok(surface)
    }

    pub fn export(&self) -> Result<Pixbuf> {
        self.export_region(self.selection().unwrap_or(self.bounds()))
    }
    pub fn export_region(&self, region: Rect) -> Result<Pixbuf> {
        let (x, y, w, h) = region.pixel_bounds(self.width(), self.height());
        crate::i18n::ensure!(w > 0 && h > 0, "Leere Auswahl");
        let surface = ImageSurface::create(Format::ARgb32, w, h)?;
        let cr = Cairo::new(&surface)?;
        cr.translate(-f64::from(x), -f64::from(y));
        self.paint(&cr)?;
        surface_to_pixbuf(&surface)
    }

    pub fn write_project(&self, writer: impl std::io::Write) -> Result<()> {
        use base64::Engine;
        let mut png = Vec::new();
        crate::pixels::write_png(&surface_to_pixbuf(&self.image)?, &mut png)?;
        self.recovery_edits().write(
            &base64::engine::general_purpose::STANDARD.encode(png),
            writer,
        )?;
        Ok(())
    }
    pub fn recovery_edits(&self) -> SavedEdits {
        SavedEdits {
            images: self
                .current
                .annotations
                .iter()
                .filter_map(|a| {
                    if let Tool::Image(id) = a.tool {
                        self.images.get(&id).map(|asset| (id, asset.png.clone()))
                    } else {
                        None
                    }
                })
                .collect(),
            snapshot: self.current.clone(),
            monitors: self.monitors.clone(),
        }
    }
    pub fn raw_background(&self) -> Result<RawBackground> {
        let image = surface_to_pixbuf(&self.image)?;
        Ok(RawBackground {
            width: image.width() as u32,
            height: image.height() as u32,
            pixels: image.read_pixel_bytes().as_ref().to_vec(),
        })
    }
    pub fn paint_loupe(
        &self,
        cr: &Cairo,
        point: Point,
        x: f64,
        y: f64,
        accent: &gdk::RGBA,
    ) -> Result<()> {
        cr.save()?;
        cr.rectangle(x, y, 130., 130.);
        cr.clip();
        cr.translate(x + 65., y + 65.);
        cr.scale(10., 10.);
        cr.translate(-point.x.floor() - 0.5, -point.y.floor() - 0.5);
        self.paint(cr)?;
        cr.restore()?;
        cr.set_line_width(2.);
        cr.set_source_rgb(accent.red(), accent.green(), accent.blue());
        cr.rectangle(x, y, 130., 130.);
        cr.stroke()?;
        cr.set_source_rgba(0., 0., 0., 0.7);
        cr.rectangle(x, y + 130., 130., 25.);
        cr.fill()?;
        cr.set_source_rgb(1., 1., 1.);
        cr.set_line_width(1.);
        cr.rectangle(x + 60., y + 60., 10., 10.);
        cr.stroke()?;
        cr.set_font_size(11.);
        cr.move_to(x + 8., y + 147.);
        cr.show_text(&crate::i18n::format!(
            "X {}   Y {} · 10×",
            point.x.floor(),
            point.y.floor()
        ))?;
        Ok(())
    }
    pub fn read_project(reader: impl std::io::Read) -> Result<Self> {
        use base64::Engine;
        let project: Project = serde_json::from_reader(reader.take(512 * 1024 * 1024))?;
        crate::i18n::ensure!(
            matches!(project.version, 1..=4),
            "Unbekannte Projektversion"
        );
        for a in &project.snapshot.annotations {
            crate::i18n::ensure!(
                !a.points.is_empty()
                    && a.effects.valid()
                    && a.font_size.map_or(true, |size| size.is_finite()
                        && (6. ..=512.).contains(&size))
                    && a.width.is_finite()
                    && (1.0..=100.0).contains(&a.width)
                    && a.points.iter().all(|p| p.x.is_finite() && p.y.is_finite())
                    && [a.color.0, a.color.1, a.color.2]
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    && a.clip
                        .map_or(true, |r| [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite())
                            && r.w >= 0.
                            && r.h >= 0.)
                    && !a.text.contains('\0')
                    && !a.label.contains('\0')
                    && !matches!(a.tool, Tool::Move | Tool::Select | Tool::Marquee),
                "Ungültige Annotation im Projekt"
            );
        }
        crate::i18n::ensure!(
            project.monitors.iter().all(|m| !m.name.contains('\0')
                && [m.rect.x, m.rect.y, m.rect.w, m.rect.h, m.scale]
                    .iter()
                    .all(|v| v.is_finite())
                && m.rect.w > 0.
                && m.rect.h > 0.
                && m.scale > 0.),
            "Ungültige Monitorgeometrie im Projekt"
        );
        let bytes = base64::engine::general_purpose::STANDARD.decode(project.background)?;
        let mut doc = Self::new(crate::pixels::from_bytes(&bytes)?, project.monitors)?;
        if let Some([w, h]) = project.snapshot.canvas {
            validate_canvas(i64::from(w), i64::from(h))?;
        }
        let mut pixels = 0_u64;
        for (id, png) in project.images {
            let bytes = base64::engine::general_purpose::STANDARD.decode(&png)?;
            let rgba = crate::pixels::decode_layer(&bytes)?;
            pixels += u64::from(rgba.width()) * u64::from(rgba.height());
            crate::i18n::ensure!(pixels <= 128_000_000, "Bildebenen im Projekt sind zu groß");
            let asset = ImageAsset::from_encoded(crate::pixels::from_rgba(rgba)?, Arc::new(png))?;
            doc.images.insert(id, asset);
        }
        for a in &project.snapshot.annotations {
            if let Tool::Image(id) = a.tool {
                crate::i18n::ensure!(
                    doc.images.contains_key(&id)
                        && a.points.len() == 2
                        && a.bounds().w > 0.
                        && a.bounds().h > 0.
                        && a.bounds().w.is_finite()
                        && a.bounds().h.is_finite(),
                    "Ungültige Bildebene im Projekt"
                );
            }
        }
        doc.current = project.snapshot;
        if let Some(r) = doc.current.selection {
            crate::i18n::ensure!(
                [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()),
                "Ungültiger Bildausschnitt"
            );
            doc.current.selection = Some(r.clamp(doc.bounds())).filter(|r| r.w >= 1. && r.h >= 1.);
        }
        Ok(doc)
    }
}

#[derive(Clone, Copy)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
}
pub struct RawBackground {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
pub struct SavedEdits {
    images: BTreeMap<u64, Arc<String>>,
    snapshot: Snapshot,
    monitors: Vec<Monitor>,
}
impl SavedEdits {
    pub fn write(&self, background: &str, writer: impl std::io::Write) -> Result<()> {
        #[derive(serde::Serialize)]
        struct RefProject<'a> {
            images: BTreeMap<u64, &'a str>,
            version: u32,
            background: &'a str,
            snapshot: &'a Snapshot,
            monitors: &'a [Monitor],
        }
        serde_json::to_writer(
            writer,
            &RefProject {
                images: self
                    .images
                    .iter()
                    .map(|(id, png)| (*id, png.as_str()))
                    .collect(),
                version: 4,
                background,
                snapshot: &self.snapshot,
                monitors: &self.monitors,
            },
        )?;
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Project {
    #[serde(default)]
    images: BTreeMap<u64, String>,
    version: u32,
    background: String,
    snapshot: Snapshot,
    monitors: Vec<Monitor>,
}

// Cairo uses native-endian, premultiplied ARGB. Pixbuf expects straight RGBA.
// This conversion works without a display connection and preserves transparency.
fn surface_to_pixbuf(surface: &ImageSurface) -> Result<Pixbuf> {
    let w = surface.width();
    let h = surface.height();
    let stride = surface.stride() as usize;
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    surface.with_data(|data| {
        for y in 0..h as usize {
            for x in 0..w as usize {
                let offset = y * stride + x * 4;
                let pixel = u32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap());
                let alpha = pixel >> 24;
                let out = &mut rgba[(y * w as usize + x) * 4..][..4];
                for (i, shift) in [16, 8, 0].into_iter().enumerate() {
                    let channel = (pixel >> shift) & 255;
                    out[i] = if alpha == 0 {
                        0
                    } else {
                        ((channel * 255 + alpha / 2) / alpha).min(255) as u8
                    };
                }
                out[3] = alpha as u8;
            }
        }
    })?;
    Ok(Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        w,
        h,
        w * 4,
    ))
}

pub fn paint_annotation(cr: &Cairo, a: &Annotation) -> Result<()> {
    if !a.visible {
        return Ok(());
    }
    let Some(first) = a.points.first().copied() else {
        return Ok(());
    };
    let last = *a.points.last().unwrap();
    let r = Rect::between(first, last);
    cr.save()?;
    if let Some(r) = a.clip {
        cr.rectangle(r.x, r.y, r.w, r.h);
        cr.clip();
    }
    cr.set_source_rgb(a.color.0, a.color.1, a.color.2);
    cr.set_line_width(a.width);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    match a.tool {
        Tool::Pen => {
            cr.move_to(first.x, first.y);
            for p in &a.points {
                cr.line_to(p.x, p.y);
            }
            if a.points.len() == 1 {
                cr.line_to(first.x + 0.01, first.y);
            }
            cr.stroke()?;
        }
        Tool::Line | Tool::Arrow => {
            cr.move_to(first.x, first.y);
            cr.line_to(last.x, last.y);
            cr.stroke()?;
            if a.tool == Tool::Arrow {
                let angle = (last.y - first.y).atan2(last.x - first.x);
                let length = (a.width * 4.0)
                    .max(12.0)
                    .min(((last.x - first.x).hypot(last.y - first.y)) * 0.6);
                for offset in [-0.5f64, 0.5] {
                    cr.move_to(last.x, last.y);
                    cr.line_to(
                        last.x - length * (angle + offset).cos(),
                        last.y - length * (angle + offset).sin(),
                    );
                    cr.stroke()?;
                }
            }
        }
        Tool::Rectangle => {
            cr.rectangle(r.x, r.y, r.w, r.h);
            cr.stroke()?;
        }
        Tool::Ellipse if r.w > 0.0 && r.h > 0.0 => {
            cr.save()?;
            cr.translate(r.x + r.w / 2.0, r.y + r.h / 2.0);
            cr.scale(r.w / 2.0, r.h / 2.0);
            cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
            cr.restore()?;
            cr.stroke()?;
        }
        Tool::Highlight => {
            cr.set_source_rgba(a.color.0, a.color.1, a.color.2, 0.3);
            cr.rectangle(r.x, r.y, r.w, r.h);
            cr.fill()?;
        }
        Tool::Redact => {
            // Opaque, pixel-aligned fill: no blur that could reveal the source.
            cr.set_antialias(cairo::Antialias::None);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.rectangle(
                r.x.floor(),
                r.y.floor(),
                r.right().ceil() - r.x.floor(),
                r.bottom().ceil() - r.y.floor(),
            );
            cr.fill()?;
        }
        Tool::Text => {
            cr.select_font_face(
                "sans-serif",
                cairo::FontSlant::Normal,
                cairo::FontWeight::Bold,
            );
            let size = a.text_size();
            cr.set_font_size(size);
            for (i, line) in a.text.lines().enumerate() {
                cr.move_to(first.x, first.y + size + (i as f64) * size * 1.2);
                cr.show_text(line)?;
            }
        }
        _ => {}
    }
    cr.restore()?;
    Ok(())
}

pub fn demo() -> Result<Document> {
    let surface = ImageSurface::create(Format::ARgb32, 2400, 1100)?;
    let cr = Cairo::new(&surface)?;
    cr.set_source_rgb(0.08, 0.12, 0.19);
    cr.paint()?;
    cr.set_source_rgb(0.12, 0.2, 0.25);
    cr.rectangle(1440.0, 0.0, 960.0, 1100.0);
    cr.fill()?;
    cr.select_font_face(
        "sans-serif",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Bold,
    );
    for (x, title, subtitle) in [
        (
            80.0,
            crate::i18n::tr("Display 1"),
            crate::i18n::tr("1440 × 900 · Querformat"),
        ),
        (
            1510.0,
            crate::i18n::tr("Display 2"),
            crate::i18n::tr("960 × 1100 · Hochformat"),
        ),
    ] {
        cr.set_source_rgb(0.45, 0.85, 0.75);
        cr.set_font_size(52.0);
        cr.move_to(x, 150.0);
        cr.show_text(title)?;
        cr.set_source_rgb(0.75, 0.8, 0.88);
        cr.set_font_size(25.0);
        cr.move_to(x, 205.0);
        cr.show_text(subtitle)?;
        for row in 0..6 {
            cr.set_source_rgb(0.2, 0.28, 0.37);
            cr.rectangle(
                x,
                290.0 + row as f64 * 70.0,
                550.0 - row as f64 * 50.0,
                24.0,
            );
            cr.fill()?;
        }
    }
    cr.set_source_rgb(0.03, 0.04, 0.06);
    cr.rectangle(0.0, 900.0, 1440.0, 200.0);
    cr.fill()?;
    Ok(Document::from_surface(
        surface,
        vec![
            Monitor {
                name: crate::i18n::tr("Demo links").into(),
                rect: Rect::new(0.0, 0.0, 1440.0, 900.0),
                scale: 1.0,
            },
            Monitor {
                name: crate::i18n::tr("Demo rechts").into(),
                rect: Rect::new(1440.0, 0.0, 960.0, 1100.0),
                scale: 2.0,
            },
        ],
    ))
}

fn validate_canvas(width: i64, height: i64) -> Result<()> {
    crate::i18n::ensure!(
        width > 0
            && height > 0
            && width <= 32767
            && height <= 32767
            && width * height <= 64_000_000,
        "Die Arbeitsfläche wird zu groß (maximal 32.767 Pixel pro Seite und 64 Megapixel)"
    );
    Ok(())
}
impl ImageAsset {
    fn new(image: Pixbuf) -> Result<Self> {
        use base64::Engine;
        let mut png = Vec::new();
        crate::pixels::write_png(&image, &mut png)?;
        crate::i18n::ensure!(png.len() <= 128 * 1024 * 1024, "Die Bildebene benötigt als PNG mehr als 128 MiB. Bitte einen kleineren Ausschnitt wählen.");
        Self::from_encoded(
            image,
            Arc::new(base64::engine::general_purpose::STANDARD.encode(png)),
        )
    }
    fn from_encoded(image: Pixbuf, png: Arc<String>) -> Result<Self> {
        let surface = ImageSurface::create(Format::ARgb32, image.width(), image.height())?;
        let cr = Cairo::new(&surface)?;
        cr.set_source_pixbuf(&image, 0., 0.);
        cr.paint()?;
        Ok(Self { surface, png })
    }
}
fn image_annotation(id: u64, label: String, rect: Rect) -> Annotation {
    Annotation {
        effects: Default::default(),
        clip: None,
        label,
        font_size: None,
        tool: Tool::Image(id),
        points: vec![
            Point::new(rect.x, rect.y),
            Point::new(rect.right(), rect.bottom()),
        ],
        color: (0., 0., 0.),
        width: 1.,
        text: String::new(),
        visible: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: i32, h: i32, rgba: u32) -> Pixbuf {
        let image = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, w, h).unwrap();
        image.fill(rgba);
        image
    }
    fn pixel(image: &Pixbuf, x: usize, y: usize) -> Vec<u8> {
        let offset = y * image.rowstride() as usize + x * image.n_channels() as usize;
        image.read_pixel_bytes().as_ref()[offset..offset + 4].to_vec()
    }

    #[test]
    fn property_previews_share_undo_but_preserve_other_actions_and_saved_ids() {
        let mut doc = Document::new(solid(100, 80, 0xffffffff), vec![]).unwrap();
        doc.insert_image(
            solid(10, 10, 0xff0000ff),
            Rect::new(10., 10., 10., 10.),
            "Detail".into(),
        )
        .unwrap();
        let original = doc.annotations()[0].clone();
        let saved = doc.content_id();
        let mut token = None;
        for value in [5., 10., 18.] {
            let mut a = original.clone();
            a.effects.shadow_blur = value;
            token = doc.replace_property(0, a, token);
        }
        let edited = doc.annotations()[0].clone();
        doc.undo();
        assert_eq!(doc.annotations()[0], original);
        assert_eq!(doc.content_id(), saved);
        doc.redo();
        assert_eq!(doc.annotations()[0], edited);
        // A stale continuation may not consume intervening actions, including undo.
        doc.select(Some(Rect::new(0., 0., 50., 40.)));
        let mut a = edited.clone();
        a.effects.shadow_blur = 24.;
        doc.replace_property(0, a, token);
        doc.undo();
        assert_eq!(doc.annotations()[0], edited);
        assert!(doc.selection().is_some());
        doc.undo();
        assert!(doc.selection().is_none());
        // Returning to the starting value removes the empty edit and restores its ID.
        let before = doc.content_id();
        let mut a = edited.clone();
        a.effects.border_width = 9.;
        let token = doc.replace_property(0, a, None);
        assert!(doc.replace_property(0, edited, token).is_none());
        assert_eq!(doc.content_id(), before);
        doc.undo();
        assert_eq!(doc.annotations()[0], original);
    }

    #[test]
    fn copied_region_keeps_canvas_crop_and_original_layers() {
        let mut doc = Document::new(solid(80, 60, 0xffffffff), Vec::new()).unwrap();
        let crop = Rect::new(5., 5., 60., 50.);
        doc.select(Some(crop));
        doc.insert_image(
            solid(12, 10, 0xff0000ff),
            Rect::new(20., 15., 12., 10.),
            "Original".into(),
        )
        .unwrap();
        let mut hidden = doc.annotations()[0].clone();
        hidden.visible = false;
        hidden.points = vec![Point::new(0., 0.), Point::new(80., 60.)];
        doc.add(hidden);
        let originals = doc.annotations().to_vec();
        let before = doc.export().unwrap().read_pixel_bytes();
        let region = Rect::new(17., 12., 20., 18.);
        let index = doc.copy_region_to_layer(region).unwrap();
        assert_eq!(index, 2);
        assert_eq!(&doc.annotations()[..2], originals);
        assert_eq!(doc.annotations()[index].bounds(), region);
        assert_eq!(
            (doc.width(), doc.height(), doc.selection()),
            (80, 60, Some(crop))
        );
        assert!(!doc.is_composition());
        assert_eq!(doc.export().unwrap().read_pixel_bytes(), before);
        let Tool::Image(id) = doc.annotations()[index].tool else {
            panic!("image layer expected")
        };
        let pixels = surface_to_pixbuf(&doc.images[&id].surface).unwrap();
        assert_eq!((pixels.width(), pixels.height()), (20, 18));
        assert_eq!(pixel(&pixels, 1, 1), vec![255, 255, 255, 255]);
        assert_eq!(pixel(&pixels, 6, 6), vec![255, 0, 0, 255]);
        doc.undo();
        assert_eq!(doc.annotations(), originals);
        doc.redo();
        assert_eq!(doc.annotations().len(), 3);
        assert_eq!(doc.export().unwrap().read_pixel_bytes(), before);
        let revision = doc.revision();
        assert!(doc
            .copy_region_to_layer(Rect::new(90., 90., 10., 10.))
            .is_err());
        assert_eq!(doc.revision(), revision);
    }

    #[test]
    fn layer_effects_render_persist_and_undo_without_changing_raster() {
        let mut doc = Document::new(solid(100, 80, 0x00000000), Vec::new()).unwrap();
        doc.insert_image(
            solid(30, 20, 0xff0000ff),
            Rect::new(20., 20., 30., 20.),
            "Detail".into(),
        )
        .unwrap();
        let original = doc.annotations()[0].clone();
        let plain = doc.export().unwrap().read_pixel_bytes();
        let mut styled = original.clone();
        styled.effects.border = true;
        styled.effects.border_color = (0., 0., 1.);
        styled.effects.shadow = true;
        doc.replace(0, styled.clone());
        let rendered = doc.export().unwrap();
        assert_eq!(pixel(&rendered, 20, 25), vec![0, 0, 255, 255]);
        assert_eq!(pixel(&rendered, 30, 30), vec![255, 0, 0, 255]);
        let shadow_alpha = pixel(&rendered, 55, 35)[3];
        assert!(shadow_alpha > 0 && shadow_alpha < 255);
        assert_eq!(pixel(&rendered, 90, 70)[3], 0);
        let decorated = rendered.read_pixel_bytes();
        let mut project = Vec::new();
        doc.write_project(&mut project).unwrap();
        let loaded = Document::read_project(project.as_slice()).unwrap();
        assert_eq!(loaded.annotations(), doc.annotations());
        assert_eq!(loaded.export().unwrap().read_pixel_bytes(), decorated);
        doc.undo();
        assert_eq!(doc.annotations()[0], original);
        assert_eq!(doc.export().unwrap().read_pixel_bytes(), plain);
        doc.redo();
        assert_eq!(doc.export().unwrap().read_pixel_bytes(), decorated);
        // A retained crop masks decorations too; composition must not reveal clipped pixels.
        styled.clip = Some(Rect::new(22., 22., 10., 10.));
        doc.replace(0, styled);
        assert_eq!(pixel(&doc.export().unwrap(), 55, 35)[3], 0);
        // Existing formats without effects remain editable; invalid settings are rejected.
        let mut json: serde_json::Value = serde_json::from_slice(&project).unwrap();
        json["version"] = serde_json::json!(3);
        json["snapshot"]["annotations"][0]
            .as_object_mut()
            .unwrap()
            .remove("effects");
        let legacy = Document::read_project(serde_json::to_vec(&json).unwrap().as_slice()).unwrap();
        assert_eq!(legacy.annotations()[0].effects, Default::default());
        json["snapshot"]["annotations"][0]["effects"] = serde_json::json!({"shadow_blur": 1e20});
        assert!(Document::read_project(serde_json::to_vec(&json).unwrap().as_slice()).is_err());
    }
    #[test]
    fn image_capacity_ignores_discarded_redo_without_mutating_it() {
        let mut doc = Document::new(solid(32, 32, 0xffffffff), vec![]).unwrap();
        doc.insert_image(
            solid(16, 16, 0xff0000ff),
            Rect::new(0., 0., 16., 16.),
            "Rot".into(),
        )
        .unwrap();
        let pixels = doc.export().unwrap().read_pixel_bytes();
        doc.undo();
        let revision = doc.revision();
        // Exercise the production budget check without allocating 128 million pixels.
        assert!(
            doc.ensure_image_capacity(128_000_000).is_ok(),
            "a new image must not be charged for the redo branch it replaces"
        );
        assert!(doc.ensure_image_capacity(128_000_001).is_err());
        assert_eq!(doc.revision(), revision);
        assert!(doc.can_redo(), "capacity checks cannot consume undo/redo");
        assert_eq!(doc.images.len(), 1);
        doc.redo();
        assert_eq!(doc.export().unwrap().read_pixel_bytes(), pixels);
        assert!(doc.ensure_image_capacity(128_000_000 - 256).is_ok());
        assert!(
            doc.ensure_image_capacity(128_000_000 - 255).is_err(),
            "the current layer must still count toward the limit"
        );
        doc.undo();
        assert!(doc
            .insert_image(
                solid(16, 16, 0x00ff00ff),
                Rect::new(0., 0., -1., 16.),
                "Ungültig".into()
            )
            .is_err());
        assert!(
            doc.can_redo(),
            "failed imports preserve the existing redo branch"
        );
        doc.insert_image(
            solid(16, 16, 0x00ff00ff),
            Rect::new(0., 0., 16., 16.),
            "Grün".into(),
        )
        .unwrap();
        assert!(!doc.can_redo());
        assert_eq!(
            doc.images.len(),
            1,
            "discarded raster data is released on commit"
        );
        assert_eq!(pixel(&doc.export().unwrap(), 5, 5), [0, 255, 0, 255]);
    }

    #[test]
    fn image_capacity_releases_only_expiring_undo_assets() {
        let mut doc = Document::new(solid(32, 32, 0xffffffff), vec![]).unwrap();
        doc.insert_image(
            solid(16, 16, 0xff0000ff),
            Rect::new(0., 0., 16., 16.),
            "Rot".into(),
        )
        .unwrap();
        doc.remove_many(&[0]);
        // Keep a different image active; duplicates must share its pixel budget.
        doc.insert_image(
            solid(8, 8, 0x0000ffff),
            Rect::new(20., 20., 8., 8.),
            "Blau".into(),
        )
        .unwrap();
        doc.duplicate_many(&[0]);
        for i in 0..97 {
            let mut layer = doc.annotations()[0].clone();
            layer.label = format!("Schritt {i}");
            doc.replace(0, layer);
        }
        assert_eq!(doc.undo.len(), 100);
        assert_eq!(doc.images.len(), 2);
        assert!(
            doc.ensure_image_capacity(128_000_000 - 64).is_ok(),
            "the oldest image leaves undo at the next checkpoint"
        );
        assert!(doc.ensure_image_capacity(128_000_000 - 63).is_err());
        assert_eq!(
            doc.images.len(),
            2,
            "a check does not discard recoverable images"
        );
        doc.undo();
        assert!(
            doc.ensure_image_capacity(128_000_000 - 64).is_err(),
            "undo made the old image reachable again in the next checkpoint"
        );
        doc.redo();
        doc.insert_image(
            solid(4, 4, 0x00ff00ff),
            Rect::new(0., 0., 4., 4.),
            "Grün".into(),
        )
        .unwrap();
        assert_eq!(doc.images.len(), 2);
        assert_eq!(doc.undo.len(), 100);
        let expected = doc.export().unwrap().read_pixel_bytes();
        for _ in 0..100 {
            doc.undo();
            doc.export().unwrap();
        }
        assert!(!doc.can_undo());
        for _ in 0..100 {
            doc.redo();
            doc.export().unwrap();
        }
        assert!(!doc.can_redo());
        assert_eq!(doc.export().unwrap().read_pixel_bytes(), expected);
        let mut project = Vec::new();
        doc.write_project(&mut project).unwrap();
        let loaded = Document::read_project(project.as_slice()).unwrap();
        assert_eq!(loaded.annotations(), doc.annotations());
        assert_eq!(loaded.export().unwrap().read_pixel_bytes(), expected);
    }

    #[test]
    fn image_layers_preserve_crop_editability_and_undo() {
        let mut doc = Document::new(
            solid(100, 80, 0xcc3333ff),
            vec![Monitor {
                name: "Original monitor".into(),
                rect: Rect::new(0., 0., 100., 80.),
                scale: 1.,
            }],
        )
        .unwrap();
        let mut mark = image_annotation(0, "Markierung".into(), Rect::new(25., 25., 15., 10.));
        mark.tool = Tool::Redact;
        doc.add(mark.clone());
        // This mark is outside the crop, exactly where the new gap will be.
        doc.add(mark.translated(40., 0.));
        doc.select(Some(Rect::new(20., 20., 40., 30.)));
        let before = doc.export().unwrap();
        let before_id = doc.content_id();
        let index = doc
            .append_image_right(solid(30, 50, 0x2255ffff), "Nächster Schritt".into())
            .unwrap();
        assert_eq!(index, 3);
        assert_eq!((doc.width(), doc.height()), (94, 50));
        assert!(doc.monitors().is_empty());
        assert_eq!(doc.annotations()[1].tool, Tool::Redact);
        let output = doc.export().unwrap();
        for y in 0..30 {
            for x in 0..40 {
                assert_eq!(pixel(&before, x, y), pixel(&output, x, y));
            }
        }
        assert_eq!(pixel(&output, 10, 40), [0, 0, 0, 0]);
        assert_eq!(pixel(&output, 50, 10), [0, 0, 0, 0]);
        assert_eq!(pixel(&output, 70, 10), [34, 85, 255, 255]);
        let image = doc.annotations()[index].clone();
        doc.replace(index, image.resized(15., 25.).translated(-5., 10.));
        assert_eq!(pixel(&doc.export().unwrap(), 70, 20), [34, 85, 255, 255]);
        let duplicates = doc.duplicate_many(&[index]);
        assert_eq!(doc.annotations()[duplicates[0]].tool, image.tool);
        assert_eq!(doc.images.len(), 2, "duplicates share the decoded image");
        doc.undo();
        doc.undo();
        doc.undo();
        assert_eq!(doc.content_id(), before_id);
        assert_eq!(doc.selection(), Some(Rect::new(20., 20., 40., 30.)));
        assert_eq!(doc.monitors().len(), 1);
        assert_eq!(doc.annotations().len(), 2);
        assert_eq!(
            doc.export().unwrap().read_pixel_bytes(),
            before.read_pixel_bytes()
        );
        doc.redo();
        assert_eq!(
            doc.export().unwrap().read_pixel_bytes(),
            output.read_pixel_bytes()
        );
        // A second append after another crop must not reveal either old raster.
        doc.select(Some(Rect::new(64., 10., 20., 20.)));
        doc.append_image_right(solid(10, 30, 0x00ff00ff), "Dritter Schritt".into())
            .unwrap();
        let flow = doc.export().unwrap();
        assert_eq!((flow.width(), flow.height()), (54, 30));
        assert_eq!(pixel(&flow, 2, 2), [34, 85, 255, 255]);
        assert_eq!(pixel(&flow, 2, 25), [0, 0, 0, 0]);
        assert_eq!(pixel(&flow, 30, 5), [0, 0, 0, 0]);
        assert_eq!(pixel(&flow, 45, 5), [0, 255, 0, 255]);
    }
    #[test]
    fn image_layers_project_recovery_and_validation() {
        let mut doc = Document::new(solid(12, 10, 0xff0000ff), Vec::new()).unwrap();
        let index = doc
            .append_image_right(solid(8, 15, 0x0000ffff), "Screenshot.png".into())
            .unwrap();
        let duplicates = doc.duplicate_many(&[index]);
        let mut hidden = doc.annotations()[duplicates[0]].clone();
        hidden.visible = false;
        doc.replace(duplicates[0], hidden);
        let mut bytes = Vec::new();
        doc.write_project(&mut bytes).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["images"].as_object().unwrap().len(), 2);
        let mut recovery = Vec::new();
        doc.recovery_edits()
            .write(json["background"].as_str().unwrap(), &mut recovery)
            .unwrap();
        for encoded in [&bytes, &recovery] {
            let loaded = Document::read_project(encoded.as_slice()).unwrap();
            assert_eq!(loaded.annotations(), doc.annotations());
            assert_eq!(
                loaded.export().unwrap().read_pixel_bytes(),
                doc.export().unwrap().read_pixel_bytes()
            );
        }
        for field in ["images", "canvas", "clip"] {
            let mut invalid = json.clone();
            match field {
                "images" => invalid["images"] = serde_json::json!({}),
                "canvas" => invalid["snapshot"]["canvas"] = serde_json::json!([32768, 1]),
                _ => {
                    invalid["snapshot"]["annotations"][0]["clip"] =
                        serde_json::json!({"x":0,"y":0,"w":-1,"h":2})
                }
            }
            assert!(
                Document::read_project(serde_json::to_vec(&invalid).unwrap().as_slice()).is_err()
            );
        }
        let before = doc.content_id();
        assert!(doc
            .append_image_right(solid(32750, 1, 0xff0000ff), "Too wide".into())
            .is_err());
        assert_eq!(doc.content_id(), before);
        doc.remove_many(&[index]);
        assert_eq!(pixel(&doc.export().unwrap(), 38, 2), [0, 0, 0, 0]);
        doc.undo();
        assert_eq!(pixel(&doc.export().unwrap(), 38, 2), [0, 0, 255, 255]);
    }

    #[test]
    fn text_size_is_independent_and_old_projects_remain_readable() {
        let mut doc = demo().unwrap();
        let mut annotation = Annotation {
            effects: Default::default(),
            clip: None,
            label: String::new(),
            font_size: None,
            tool: Tool::Text,
            points: vec![Point::new(20., 30.)],
            color: (1., 0., 0.),
            width: 5.,
            text: "Text".into(),
            visible: true,
        };
        assert_eq!(annotation.text_size(), 32.);
        annotation.font_size = Some(48.);
        let bounds = annotation.bounds();
        annotation.width = 19.;
        assert_eq!(annotation.bounds(), bounds);
        let resized = annotation.resized(bounds.w, bounds.h * 2.);
        assert_eq!(resized.text_size(), 96.);
        assert_eq!(resized.width, 19.);
        doc.add(annotation);
        let mut bytes = Vec::new();
        doc.write_project(&mut bytes).unwrap();
        assert_eq!(
            Document::read_project(bytes.as_slice())
                .unwrap()
                .annotations()[0]
                .text_size(),
            48.
        );
        let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        legacy["version"] = serde_json::json!(1);
        legacy["snapshot"]["annotations"][0]
            .as_object_mut()
            .unwrap()
            .remove("font_size");
        let loaded =
            Document::read_project(serde_json::to_vec(&legacy).unwrap().as_slice()).unwrap();
        assert_eq!(loaded.annotations()[0].text_size(), 88.);
    }

    #[test]
    fn group_edits_duplicate_align_and_delete_are_single_undo_steps() {
        let mut doc = demo().unwrap();
        doc.add(rectangle((1., 0., 0.)));
        doc.add(rectangle((0., 1., 0.)).translated(80., 40.));
        let before = doc.annotations().to_vec();
        let revision = doc.revision();
        let copies = doc.duplicate_many(&[0, 1]);
        assert_eq!(copies, vec![2, 3]);
        assert_eq!(doc.annotations().len(), 4);
        assert_eq!(
            doc.annotations()[2].name(),
            crate::i18n::format!("{} · Kopie", doc.annotations()[0].name())
        );
        doc.undo();
        assert_eq!(doc.annotations(), before);
        assert!(doc.revision() > revision);
        doc.redo();
        doc.align(&copies, Alignment::Left);
        assert_eq!(
            doc.annotations()[2].bounds().x,
            doc.annotations()[3].bounds().x
        );
        doc.undo();
        assert_ne!(
            doc.annotations()[2].bounds().x,
            doc.annotations()[3].bounds().x
        );
        doc.replace_many(vec![
            (2, doc.annotations()[2].translated(1., 2.)),
            (3, doc.annotations()[3].translated(1., 2.)),
        ]);
        doc.undo();
        doc.remove_many(&copies);
        assert_eq!(doc.annotations().len(), 2);
        doc.undo();
        assert_eq!(doc.annotations().len(), 4);
        let mut renamed = doc.annotations()[0].clone();
        renamed.label = "Hinweis".into();
        doc.replace(0, renamed);
        assert_eq!(doc.annotations()[0].name(), "Hinweis");
    }

    fn rectangle(color: (f64, f64, f64)) -> Annotation {
        Annotation {
            effects: Default::default(),
            clip: None,
            label: String::new(),
            font_size: None,
            tool: Tool::Rectangle,
            points: vec![Point::new(100., 100.), Point::new(200., 160.)],
            color,
            width: 4.,
            text: String::new(),
            visible: true,
        }
    }

    #[test]
    fn layer_thumbnails_isolate_content_preserve_crop_and_show_hidden_layers() {
        let mut doc = Document::new(solid(8, 4, 0xff0000ff), Vec::new()).unwrap();
        let split = image::RgbaImage::from_fn(8, 4, |x, _| {
            image::Rgba(if x < 4 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        });
        let index = doc
            .append_image_right(crate::pixels::from_rgba(split).unwrap(), "Bild".into())
            .unwrap();
        let mut layer = doc.annotations()[index].clone();
        let r = layer.bounds();
        layer.clip = Some(Rect::new(r.x + 4., r.y, 4., 4.));
        doc.replace(index, layer.clone());
        let base = surface_to_pixbuf(&doc.layer_thumbnail(0, 1).unwrap()).unwrap();
        assert_eq!(pixel(&base, 22, 17), [255, 0, 0, 255]);
        let visible = surface_to_pixbuf(&doc.layer_thumbnail(index, 1).unwrap()).unwrap();
        assert_eq!((visible.width(), visible.height()), (44, 34));
        assert_eq!(pixel(&visible, 22, 17), [0, 0, 255, 255]);
        layer.visible = false;
        doc.replace(index, layer);
        let hidden = surface_to_pixbuf(&doc.layer_thumbnail(index, 1).unwrap()).unwrap();
        assert_eq!(hidden.read_pixel_bytes(), visible.read_pixel_bytes());
        assert_eq!(
            pixel(&doc.export().unwrap(), (r.x + 6.) as usize, 2),
            [0, 0, 0, 0]
        );
        let hires = doc.layer_thumbnail(index, 2).unwrap();
        assert_eq!((hires.width(), hires.height()), (88, 68));
        for tool in [Tool::Text, Tool::Rectangle, Tool::Arrow] {
            let mut a = rectangle((1., 0., 0.));
            a.tool = tool;
            a.text = "A".into();
            doc.add(a);
            let preview =
                surface_to_pixbuf(&doc.layer_thumbnail(doc.annotations().len() - 1, 1).unwrap())
                    .unwrap();
            assert!(
                preview
                    .read_pixel_bytes()
                    .chunks_exact(4)
                    .any(|p| p[0] > 180 && p[1] < 100 && p[2] < 100),
                "{tool:?} must appear in the thumbnail"
            );
        }
    }

    #[test]
    fn handle_scaling_preserves_image_ratio_text_limits_and_clip() {
        use crate::selection::{handles, Handle};
        let mut image = rectangle((1., 0., 0.));
        image.tool = Tool::Image(1);
        image.clip = Some(Rect::new(110., 110., 50., 30.));
        let r = image.bounds();
        for (_, handle) in handles(r) {
            let scaled = image.resized_from(handle, 20., -10.);
            let b = scaled.bounds();
            assert!((b.w / b.h - r.w / r.h).abs() < 1e-9);
            let clip = scaled.clip.unwrap();
            assert!(((clip.x - b.x) / b.w - 0.1).abs() < 1e-9);
            assert!((clip.w / b.w - 0.5).abs() < 1e-9);
            assert_eq!(image.resized_from(handle, 0., 0.), image);
        }
        let mut text = rectangle((1., 0., 0.));
        text.tool = Tool::Text;
        text.font_size = Some(28.);
        text.text = "Text bleibt proportional".into();
        let r = text.bounds();
        for dx in [-100000., 100000.] {
            let scaled = text.resized_from(Handle::Nw, dx, dx);
            let b = scaled.bounds();
            assert!((b.right() - r.right()).abs() < 1e-8);
            assert!((b.bottom() - r.bottom()).abs() < 1e-8);
            assert!([6., 512.].contains(&scaled.text_size()));
        }
        assert!(text.resized_from(Handle::E, r.w, 0.).text_size() > text.text_size());
        // Horizontal/vertical strokes can acquire a nonzero second dimension.
        let mut line = rectangle((1., 0., 0.));
        line.tool = Tool::Line;
        line.points = vec![Point::new(100., 100.), Point::new(200., 100.)];
        assert_eq!(
            line.resized_from(Handle::N, 0., -20.).bounds(),
            Rect::new(100., 80., 100., 20.)
        );
        line.points = vec![Point::new(100., 100.), Point::new(100., 200.)];
        assert_eq!(
            line.resized_from(Handle::W, -20., 0.).bounds(),
            Rect::new(80., 100., 20., 100.)
        );
    }

    #[test]
    fn drag_reordering_preserves_layers_and_is_one_undo_step() {
        let image = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 8, 8).unwrap();
        image.fill(0xffffffff);
        let mut doc = Document::new(image, vec![]).unwrap();
        for i in 0..5 {
            let mut a = rectangle((1., 0., 0.));
            a.label = format!("Layer {i}");
            a.visible = i != 2;
            doc.add(a);
        }
        let original = doc.annotations().to_vec();
        for source in 0..5 {
            for gap in 0..=5 {
                let revision = doc.revision();
                let target = doc.move_layer(source, gap).unwrap();
                let mut expected = original.clone();
                let moved = expected.remove(source);
                expected.insert(target, moved);
                assert_eq!(doc.annotations(), expected);
                if source == target {
                    assert_eq!(
                        doc.revision(),
                        revision,
                        "a drop beside the same layer is a no-op"
                    );
                } else {
                    doc.undo();
                    assert_eq!(doc.annotations(), original);
                    doc.redo();
                    assert_eq!(doc.annotations(), expected);
                    doc.undo();
                }
            }
        }
        let revision = doc.revision();
        assert_eq!(doc.move_layer(5, 0), None);
        assert_eq!(doc.move_layer(0, 6), None);
        assert_eq!(doc.revision(), revision);
    }
    #[test]
    fn layers_move_resize_hide_reorder_and_undo() {
        let mut doc = demo().unwrap();
        doc.add(rectangle((1., 0., 0.)));
        doc.add(rectangle((0., 1., 0.)));
        assert_eq!(doc.hit_test(Point::new(150., 120.), 2.), Some(1));
        let mut a = doc.annotations()[1]
            .translated(40., 20.)
            .resized(200., 120.);
        assert_eq!(a.bounds(), Rect::new(140., 120., 200., 120.));
        doc.replace(1, a.clone());
        doc.undo();
        assert_eq!(
            doc.annotations()[1].bounds(),
            Rect::new(100., 100., 100., 60.)
        );
        doc.redo();
        a.visible = false;
        doc.replace(1, a);
        assert_eq!(doc.hit_test(Point::new(150., 130.), 2.), Some(0));
        assert_eq!(doc.reorder(0, true), 1);
        assert_eq!(doc.hit_test(Point::new(150., 130.), 2.), Some(1));
        doc.remove_many(&[1]);
        assert_eq!(doc.hit_test(Point::new(150., 130.), 2.), None);
        doc.undo();
        assert_eq!(doc.annotations().len(), 2);
    }

    #[test]
    fn project_preserves_original_editable_layers_and_crop() {
        let mut doc = demo().unwrap();
        doc.add(rectangle((1., 0., 0.)));
        let mut hidden = rectangle((0., 1., 0.));
        hidden.visible = false;
        doc.add(hidden);
        doc.select(Some(Rect::new(90., 90., 130., 90.)));
        let expected = doc.export().unwrap().read_pixel_bytes();
        let mut project = Vec::new();
        doc.write_project(&mut project).unwrap();
        let mut reopened = Document::read_project(&project[..]).unwrap();
        assert_eq!(reopened.annotations(), doc.annotations());
        assert_eq!(reopened.selection(), doc.selection());
        assert_eq!(reopened.monitors, doc.monitors);
        assert_eq!(
            reopened.export().unwrap().read_pixel_bytes().as_ref(),
            expected.as_ref()
        );
        reopened.remove_many(&[0]);
        assert_ne!(
            reopened.export().unwrap().read_pixel_bytes().as_ref(),
            expected.as_ref()
        );
    }

    #[test]
    fn imported_transparency_survives_cairo_and_png_round_trip() {
        let bytes = glib::Bytes::from_owned(vec![255u8, 0, 0, 128, 0, 0, 0, 0]);
        let original = Pixbuf::from_bytes(&bytes, gdk_pixbuf::Colorspace::Rgb, true, 8, 2, 1, 8);
        let doc = Document::new(original, Vec::new()).unwrap();
        let rendered = doc.export().unwrap();
        let mut encoded = Vec::new();
        crate::pixels::write_png(&rendered, &mut encoded).unwrap();
        let decoded = crate::pixels::from_bytes(&encoded).unwrap();
        assert_eq!(
            decoded.read_pixel_bytes().as_ref(),
            &[255, 0, 0, 128, 0, 0, 0, 0]
        );
    }

    #[test]
    fn crop_spans_monitors_and_undo_redo_restore_edits() {
        let mut doc = demo().unwrap();
        doc.select(Some(Rect::new(1300.0, 50.0, 400.0, 400.0)));
        let cropped = doc.export().unwrap();
        assert_eq!((cropped.width(), cropped.height()), (400, 400));
        doc.undo();
        assert_eq!(doc.selection(), None);
        doc.redo();
        assert_eq!(doc.selection().unwrap().w, 400.0);
        doc.undo();
        doc.select(Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert!(!doc.can_redo());
    }

    #[test]
    fn redaction_is_opaque_in_export_and_reversible() {
        let mut doc = demo().unwrap();
        doc.select(Some(Rect::new(0.0, 0.0, 20.0, 20.0)));
        doc.add(Annotation {
            effects: Default::default(),
            clip: None,
            label: String::new(),
            font_size: None,
            tool: Tool::Redact,
            points: vec![Point::new(0.0, 0.0), Point::new(20.0, 20.0)],
            color: (1.0, 0.0, 0.0),
            width: 3.0,
            text: String::new(),
            visible: true,
        });
        let image = doc.export().unwrap();
        let data = image.read_pixel_bytes();
        assert_eq!(&data.as_ref()[0..4], &[0, 0, 0, 255]);
        doc.undo();
        let original = doc.export().unwrap().read_pixel_bytes();
        assert_ne!(&original.as_ref()[0..3], &[0, 0, 0]);
    }
}
