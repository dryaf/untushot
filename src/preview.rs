//! Viewport-sized raster cache. Pointer motion only redraws crop/tool overlays;
//! originals and export keep their full resolution, including on mixed-DPI desktops.
use crate::{
    document::{Annotation, Document},
    geometry::Rect,
};
use anyhow::Result;
use cairo::{Context, Format, ImageSurface};

pub fn relative_layer(a: &Annotation) -> Annotation {
    let bounds = a.bounds();
    a.translated(-bounds.x, -bounds.y)
}

#[derive(Default)]
pub struct LayerPreviews {
    scale: i32,
    entries: Vec<Option<(Annotation, ImageSurface)>>,
    #[cfg(test)]
    pub builds: usize,
}
impl LayerPreviews {
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn truncate(&mut self, count: usize) {
        self.entries.truncate(count);
    }
    pub fn get(&mut self, doc: &Document, index: usize, scale: i32) -> Result<ImageSurface> {
        if self.scale != scale {
            self.clear();
            self.scale = scale;
        }
        self.entries.resize_with(doc.annotations().len(), || None);
        let mut key = relative_layer(&doc.annotations()[index]);
        key.visible = true;
        key.label.clear();
        let entry = &mut self.entries[index];
        if entry.as_ref().map(|(a, _)| a) != Some(&key) {
            *entry = Some((key, doc.layer_thumbnail(index, scale)?));
            #[cfg(test)]
            {
                self.builds += 1;
            }
        }
        Ok(entry.as_ref().unwrap().1.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub revision: u64,
    pub width: i32,
    pub height: i32,
    pub device_scale: i32,
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
    pub viewport: Rect,
}

#[derive(Default)]
pub struct Preview {
    cached: Option<(Key, ImageSurface)>,
    dynamic: Option<DynamicPreview>,
    #[cfg(test)]
    pub builds: usize,
}
struct DynamicPreview {
    key: Key,
    indices: Vec<usize>,
    parts: Vec<PreviewPart>,
}
enum PreviewPart {
    Fixed(ImageSurface),
    Edited(usize),
}
impl Preview {
    pub fn clear(&mut self) {
        self.cached = None;
        self.dynamic = None;
    }
    pub fn paint(&mut self, cr: &Context, doc: &Document, key: Key) -> Result<()> {
        self.dynamic = None;
        if self.cached.as_ref().map(|(k, _)| *k) != Some(key) {
            let surface = ImageSurface::create(
                Format::ARgb32,
                key.width.max(1) * key.device_scale,
                key.height.max(1) * key.device_scale,
            )?;
            surface.set_device_scale(key.device_scale as f64, key.device_scale as f64);
            let canvas = Context::new(&surface)?;
            canvas.translate(key.x, key.y);
            canvas.scale(key.zoom, key.zoom);
            let r = key.viewport;
            canvas.rectangle(r.x, r.y, r.w, r.h);
            canvas.clip();
            doc.paint(&canvas)?;
            self.cached = Some((key, surface));
            #[cfg(test)]
            {
                self.builds += 1;
            }
        }
        cr.save()?;
        cr.set_source_surface(&self.cached.as_ref().unwrap().1, 0., 0.)?;
        cr.source().set_filter(cairo::Filter::Nearest);
        cr.paint()?;
        cr.restore()?;
        Ok(())
    }

    /// Cache runs of unchanged layers, preserving their order around edited
    /// layers. Moving one object no longer resamples the whole 4K background.
    pub fn paint_dynamic(
        &mut self,
        cr: &Context,
        doc: &Document,
        edits: &[(usize, Annotation)],
        key: Key,
    ) -> Result<()> {
        let mut indices: Vec<_> = edits
            .iter()
            .map(|(i, _)| *i)
            .filter(|i| *i < doc.annotations().len())
            .collect();
        // Cairo's subpixel text compositing depends on the destination pixels.
        // Paint text onto the final image so its edges do not change during a drag.
        indices.extend(
            doc.annotations()
                .iter()
                .enumerate()
                .filter_map(|(i, a)| (a.tool == crate::document::Tool::Text).then_some(i)),
        );
        indices.sort_unstable();
        indices.dedup();
        let bytes = i64::from(key.width.max(1))
            * i64::from(key.height.max(1))
            * i64::from(key.device_scale).pow(2)
            * 4;
        // Keep the extra cache bounded even for scattered groups on HiDPI screens.
        // Erasures must composite with every preceding layer, including edited ones.
        if doc.has_erasure() || bytes * (indices.len() as i64 + 1) > 64 * 1024 * 1024 {
            self.dynamic = None;
            cr.save()?;
            transform(cr, key, doc.bounds());
            doc.paint_preview(cr, edits)?;
            cr.restore()?;
            return Ok(());
        }
        if !self
            .dynamic
            .as_ref()
            .is_some_and(|d| d.key == key && d.indices == indices)
        {
            self.dynamic = None;
            let mut parts = Vec::new();
            let mut start = 0;
            for &end in indices
                .iter()
                .chain(std::iter::once(&doc.annotations().len()))
            {
                if end > start {
                    let surface = ImageSurface::create(
                        Format::ARgb32,
                        key.width.max(1) * key.device_scale,
                        key.height.max(1) * key.device_scale,
                    )?;
                    surface
                        .set_device_scale(f64::from(key.device_scale), f64::from(key.device_scale));
                    let canvas = Context::new(&surface)?;
                    transform(&canvas, key, doc.bounds());
                    for a in &doc.annotations()[start..end] {
                        doc.paint_layer(&canvas, a)?;
                    }
                    parts.push(PreviewPart::Fixed(surface));
                }
                if end < doc.annotations().len() {
                    parts.push(PreviewPart::Edited(end));
                }
                start = end + 1;
            }
            self.dynamic = Some(DynamicPreview {
                key,
                indices,
                parts,
            });
        }
        for part in &self.dynamic.as_ref().unwrap().parts {
            cr.save()?;
            match part {
                PreviewPart::Fixed(surface) => {
                    cr.set_source_surface(surface, 0., 0.)?;
                    cr.paint()?;
                }
                PreviewPart::Edited(index) => {
                    transform(cr, key, doc.bounds());
                    let a = edits
                        .iter()
                        .find(|(i, _)| i == index)
                        .map(|(_, a)| a)
                        .unwrap_or(&doc.annotations()[*index]);
                    doc.paint_layer(cr, a)?;
                }
            }
            cr.restore()?;
        }
        Ok(())
    }
}

fn transform(cr: &Context, key: Key, bounds: Rect) {
    cr.translate(key.x, key.y);
    cr.scale(key.zoom, key.zoom);
    let r = key.viewport.clamp(bounds);
    cr.rectangle(r.x, r.y, r.w, r.h);
    cr.clip();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn erasures_clear_document_pixels_without_clearing_the_ui_during_drag() {
        let bg = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 20, 20).unwrap();
        bg.fill(0xff0000ff);
        let mut doc = Document::new(bg.clone(), vec![]).unwrap();
        doc.insert_image(bg, Rect::new(0., 0., 10., 10.), "image".into())
            .unwrap();
        doc.delete_region(Rect::new(3., 3., 4., 4.));
        let key = Key {
            revision: doc.revision(),
            width: 20,
            height: 20,
            device_scale: 1,
            x: 0.,
            y: 0.,
            zoom: 1.,
            viewport: doc.bounds(),
        };
        let mut preview = Preview::default();
        // Exercise both moving a layer below the erasure and moving the erasure.
        for index in [1, 2] {
            for dynamic in [false, true] {
                let surface = ImageSurface::create(Format::ARgb32, 20, 20).unwrap();
                let cr = Context::new(&surface).unwrap();
                cr.set_source_rgb(0., 0., 1.);
                cr.paint().unwrap();
                if dynamic {
                    let edits = [(index, doc.annotations()[index].translated(1., 0.))];
                    preview.paint_dynamic(&cr, &doc, &edits, key).unwrap();
                } else {
                    preview.paint(&cr, &doc, key).unwrap();
                }
                surface
                    .with_data(|bytes| {
                        let pixel = |x: usize, y: usize| {
                            let offset = y * surface.stride() as usize + x * 4;
                            u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
                        };
                        assert_eq!(
                            pixel(5, 5),
                            0xff0000ff,
                            "deleted space must show the UI behind it"
                        );
                        assert_eq!(pixel(1, 1), 0xffff0000);
                    })
                    .unwrap();
            }
        }
    }

    #[test]
    fn layer_cache_reuses_visibility_and_name_changes_but_updates_content_and_dpi() {
        use crate::{document::Tool, geometry::Point};
        let mut doc = crate::document::demo().unwrap();
        doc.add(Annotation {
            effects: Default::default(),
            tool: Tool::Rectangle,
            points: vec![Point::new(0., 0.), Point::new(100., 50.)],
            color: (1., 0., 0.),
            width: 4.,
            visible: true,
            label: String::new(),
            text: String::new(),
            font_size: None,
            clip: None,
        });
        let mut cache = LayerPreviews::default();
        cache.get(&doc, 1, 1).unwrap();
        let mut a = doc.annotations()[1].clone();
        a.visible = false;
        a.label = "Neuer Name".into();
        doc.replace(1, a.clone());
        cache.get(&doc, 1, 1).unwrap();
        assert_eq!(cache.builds, 1);
        a = a.translated(200., 80.);
        doc.replace(1, a.clone());
        cache.get(&doc, 1, 1).unwrap();
        assert_eq!(cache.builds, 1, "moving a layer must reuse its thumbnail");
        a.color = (0., 0., 1.);
        doc.replace(1, a);
        cache.get(&doc, 1, 1).unwrap();
        assert_eq!(cache.builds, 2);
        doc.undo();
        cache.get(&doc, 1, 1).unwrap();
        assert_eq!(cache.builds, 3);
        assert_eq!(cache.get(&doc, 1, 2).unwrap().width(), 88);
        assert_eq!(cache.builds, 4);
        cache.truncate(0);
        assert!(cache.entries.is_empty());
        cache.get(&doc, 1, 2).unwrap();
        assert_eq!(cache.builds, 5);
    }
    #[test]
    fn edited_layers_match_direct_render_with_overlap_effects_zoom_and_crop() {
        use crate::document::Tool;
        use crate::geometry::Point;
        let bg = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 400, 300).unwrap();
        bg.fill(0x80a0c0cc);
        let mut doc = Document::new(bg, vec![]).unwrap();
        for i in 0..4 {
            let image =
                gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 200, 180).unwrap();
            image.fill(0x3080a088 + i * 0x20101000);
            let index = doc
                .insert_image(
                    image,
                    Rect::new(
                        20. + 25. * f64::from(i),
                        15. + 30. * f64::from(i),
                        200.,
                        180.,
                    ),
                    format!("Image {i}"),
                )
                .unwrap();
            let mut a = doc.annotations()[index].clone();
            a.effects.border = true;
            a.effects.shadow = true;
            a.visible = i != 2;
            if i == 1 {
                a.clip = Some(Rect::new(65., 70., 140., 100.));
            }
            doc.replace(index, a);
        }
        doc.add(Annotation {
            tool: Tool::Text,
            points: vec![Point::new(60., 100.), Point::new(180., 150.)],
            color: (0.8, 0., 0.),
            width: 4.,
            text: "Overlap".into(),
            font_size: Some(24.),
            visible: true,
            label: String::new(),
            clip: None,
            effects: Default::default(),
        });
        let mut cache = Preview::default();
        for dpi in [1, 2] {
            for zoom in [0.5, 1.25] {
                for indices in [vec![1], vec![2, 4], vec![0, 1, 2, 3, 4, 5]] {
                    // The large virtual viewport exercises the memory-bounded fallback.
                    for width in [320, 100_000] {
                        let mut key = Key {
                            revision: doc.revision(),
                            width,
                            height: 240,
                            device_scale: dpi,
                            x: -12.25,
                            y: 7.5,
                            zoom,
                            viewport: Rect::new(10., 20., 360., 260.),
                        };
                        for (offset, changed) in [(0., false), (17., false), (17., true)] {
                            if changed {
                                let index = (1..5).find(|i| !indices.contains(i)).unwrap_or(1);
                                let mut a = doc.annotations()[index].clone();
                                a.effects.border_width = if a.effects.border_width == 3. {
                                    11.
                                } else {
                                    3.
                                };
                                doc.replace(index, a);
                                key.revision = doc.revision();
                            }
                            let edits: Vec<_> = indices
                                .iter()
                                .map(|&i| (i, doc.annotations()[i].translated(offset, offset / 2.)))
                                .collect();
                            let render = |cached: bool, cache: &mut Preview| {
                                let surface =
                                    ImageSurface::create(Format::ARgb32, 320 * dpi, 240 * dpi)
                                        .unwrap();
                                surface.set_device_scale(f64::from(dpi), f64::from(dpi));
                                let cr = Context::new(&surface).unwrap();
                                if cached {
                                    cache.paint_dynamic(&cr, &doc, &edits, key).unwrap();
                                } else {
                                    transform(&cr, key, doc.bounds());
                                    doc.paint_preview(&cr, &edits).unwrap();
                                }
                                drop(cr);
                                let mut bytes = vec![];
                                surface.with_data(|v| bytes.extend_from_slice(v)).unwrap();
                                bytes
                            };
                            let direct = render(false, &mut cache);
                            let cached = render(true, &mut cache);
                            let error = direct
                                .iter()
                                .zip(&cached)
                                .map(|(a, b)| a.abs_diff(*b))
                                .max()
                                .unwrap();
                            // Grouping translucent layers can round an 8-bit channel differently.
                            assert!(error <= 2, "pixel error {error}, dpi={dpi}, zoom={zoom}, layers={indices:?}, offset={offset}, width={width}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn cached_view_matches_direct_render_and_invalidates_after_changes() {
        let mut doc = crate::document::demo().unwrap();
        let mut cache = Preview::default();
        for dpi in [1, 2] {
            for zoom in [0.25, 0.5] {
                let key = Key {
                    revision: doc.revision(),
                    width: 600,
                    height: 350,
                    device_scale: dpi,
                    x: -35.,
                    y: 12.,
                    zoom,
                    viewport: doc.bounds(),
                };
                let cached = ImageSurface::create(Format::ARgb32, 600 * dpi, 350 * dpi).unwrap();
                cached.set_device_scale(dpi as f64, dpi as f64);
                cache
                    .paint(&Context::new(&cached).unwrap(), &doc, key)
                    .unwrap();
                let count = cache.builds;
                for _ in 0..10 {
                    let cr = Context::new(&cached).unwrap();
                    cr.set_operator(cairo::Operator::Clear);
                    cr.paint().unwrap();
                    cr.set_operator(cairo::Operator::Over);
                    cache.paint(&cr, &doc, key).unwrap();
                }
                assert_eq!(cache.builds, count);
                let direct = ImageSurface::create(Format::ARgb32, 600 * dpi, 350 * dpi).unwrap();
                direct.set_device_scale(dpi as f64, dpi as f64);
                let cr = Context::new(&direct).unwrap();
                cr.translate(key.x, key.y);
                cr.scale(zoom, zoom);
                doc.paint(&cr).unwrap();
                drop(cr);
                let mut bytes = Vec::new();
                cached.with_data(|v| bytes.extend_from_slice(v)).unwrap();
                direct
                    .with_data(|v| {
                        let difference = v
                            .iter()
                            .zip(&bytes)
                            .enumerate()
                            .find(|(_, (a, b))| a != b)
                            .map(|(i, (a, b))| (i, *a, *b));
                        assert_eq!(
                            difference, None,
                            "cached raster differs: dpi={dpi}, zoom={zoom}"
                        );
                    })
                    .unwrap();
            }
        }
        doc.select(Some(Rect::new(100., 100., 300., 200.)));
        let key = Key {
            revision: doc.revision(),
            width: 600,
            height: 350,
            device_scale: 1,
            x: 5.,
            y: 9.,
            zoom: 1.,
            viewport: doc.selection().unwrap(),
        };
        let output = ImageSurface::create(Format::ARgb32, 600, 350).unwrap();
        let cr = Context::new(&output).unwrap();
        let before = cache.builds;
        cache.paint(&cr, &doc, key).unwrap();
        assert_eq!(cache.builds, before + 1);
        cache.clear();
        cache.paint(&cr, &doc, key).unwrap();
        assert_eq!(cache.builds, before + 2);
    }
    #[test]
    #[ignore = "manual rendering benchmark; synthetic image, no desktop access"]
    fn large_desktop_render_timing() {
        let image =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 7680, 2160).unwrap();
        image.fill(0x507090ff);
        let doc = Document::new(image, Vec::new()).unwrap();
        let surface = ImageSurface::create(Format::ARgb32, 1280, 720).unwrap();
        let cr = Context::new(&surface).unwrap();
        let key = Key {
            revision: doc.revision(),
            width: 1280,
            height: 720,
            device_scale: 1,
            x: 16.,
            y: 100.,
            zoom: 0.1625,
            viewport: doc.bounds(),
        };
        let mut cache = Preview::default();
        cache.paint(&cr, &doc, key).unwrap();
        let frames = 100;
        let before = std::time::Instant::now();
        for _ in 0..frames {
            cr.save().unwrap();
            cr.translate(key.x, key.y);
            cr.scale(key.zoom, key.zoom);
            doc.paint(&cr).unwrap();
            cr.restore().unwrap();
        }
        let direct = before.elapsed().as_secs_f64() * 1000. / frames as f64;
        let before = std::time::Instant::now();
        for _ in 0..frames {
            cache.paint(&cr, &doc, key).unwrap();
        }
        let cached = before.elapsed().as_secs_f64() * 1000. / frames as f64;
        println!("7680×2160 → 1280×720, {frames} frames: direct {direct:.2} ms/frame, cached {cached:.2} ms/frame ({:.1}×)",direct/cached);
        assert_eq!(cache.builds, 1);
    }
}
