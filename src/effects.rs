//! Editable image-layer decoration, rendered identically in previews and exports.
use crate::geometry::Rect;
use anyhow::Result;
use cairo::{Context, LinearGradient, RadialGradient};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ImageEffects {
    pub border: bool,
    pub border_width: f64,
    pub border_color: (f64, f64, f64),
    pub shadow: bool,
    pub shadow_blur: f64,
    pub shadow_offset: f64,
    pub shadow_opacity: f64,
}
impl Default for ImageEffects {
    fn default() -> Self {
        Self {
            border: false,
            border_width: 3.,
            border_color: (1., 1., 1.),
            shadow: false,
            shadow_blur: 12.,
            shadow_offset: 8.,
            shadow_opacity: 0.3,
        }
    }
}
impl ImageEffects {
    pub fn valid(&self) -> bool {
        [
            self.border_width,
            self.shadow_blur,
            self.shadow_offset,
            self.shadow_opacity,
            self.border_color.0,
            self.border_color.1,
            self.border_color.2,
        ]
        .iter()
        .all(|v| v.is_finite())
            && (1. ..=40.).contains(&self.border_width)
            && (0. ..=80.).contains(&self.shadow_blur)
            && (-100. ..=100.).contains(&self.shadow_offset)
            && (0. ..=1.).contains(&self.shadow_opacity)
            && [
                self.border_color.0,
                self.border_color.1,
                self.border_color.2,
            ]
            .iter()
            .all(|v| (0. ..=1.).contains(v))
    }
    pub fn bounds(&self, r: Rect) -> Rect {
        if !self.shadow {
            return r;
        }
        let b = self.shadow_blur;
        r.union(Rect::new(
            r.x + self.shadow_offset - b,
            r.y + self.shadow_offset - b,
            r.w + 2. * b,
            r.h + 2. * b,
        ))
    }
    pub fn paint_shadow(&self, cr: &Context, r: Rect) -> Result<()> {
        if !self.shadow || self.shadow_opacity == 0. || r.w <= 0. || r.h <= 0. {
            return Ok(());
        }
        let r = Rect::new(r.x + self.shadow_offset, r.y + self.shadow_offset, r.w, r.h);
        let alpha = self.shadow_opacity;
        cr.set_source_rgba(0., 0., 0., alpha);
        cr.rectangle(r.x, r.y, r.w, r.h);
        cr.fill()?;
        let b = self.shadow_blur;
        if b == 0. {
            return Ok(());
        }
        // Nine-patch gradients keep soft rectangular shadows resolution-independent
        // without allocating a raster the size of the original screenshot.
        let stops = [(0., 1.), (0.25, 0.72), (0.5, 0.32), (0.75, 0.08), (1., 0.)];
        for (area, from, to) in [
            (Rect::new(r.x, r.y - b, r.w, b), (r.x, r.y), (r.x, r.y - b)),
            (
                Rect::new(r.x, r.bottom(), r.w, b),
                (r.x, r.bottom()),
                (r.x, r.bottom() + b),
            ),
            (Rect::new(r.x - b, r.y, b, r.h), (r.x, r.y), (r.x - b, r.y)),
            (
                Rect::new(r.right(), r.y, b, r.h),
                (r.right(), r.y),
                (r.right() + b, r.y),
            ),
        ] {
            let gradient = LinearGradient::new(from.0, from.1, to.0, to.1);
            for (offset, opacity) in stops {
                gradient.add_color_stop_rgba(offset, 0., 0., 0., alpha * opacity);
            }
            cr.set_source(&gradient)?;
            cr.rectangle(area.x, area.y, area.w, area.h);
            cr.fill()?;
        }
        for (cx, cy, x, y) in [
            (r.x, r.y, r.x - b, r.y - b),
            (r.right(), r.y, r.right(), r.y - b),
            (r.x, r.bottom(), r.x - b, r.bottom()),
            (r.right(), r.bottom(), r.right(), r.bottom()),
        ] {
            let gradient = RadialGradient::new(cx, cy, 0., cx, cy, b);
            for (offset, opacity) in stops {
                gradient.add_color_stop_rgba(offset, 0., 0., 0., alpha * opacity);
            }
            cr.set_source(&gradient)?;
            cr.rectangle(x, y, b, b);
            cr.fill()?;
        }
        Ok(())
    }
    pub fn paint_border(&self, cr: &Context, r: Rect) -> Result<()> {
        if !self.border || r.w <= 0. || r.h <= 0. {
            return Ok(());
        }
        let w = self.border_width.min(r.w).min(r.h);
        cr.set_source_rgb(
            self.border_color.0,
            self.border_color.1,
            self.border_color.2,
        );
        if w >= r.w || w >= r.h {
            cr.rectangle(r.x, r.y, r.w, r.h);
            cr.fill()?;
            return Ok(());
        }
        cr.set_line_width(w);
        cr.set_line_join(cairo::LineJoin::Miter);
        cr.rectangle(
            r.x + w / 2.,
            r.y + w / 2.,
            (r.w - w).max(0.),
            (r.h - w).max(0.),
        );
        cr.stroke()?;
        Ok(())
    }
}
