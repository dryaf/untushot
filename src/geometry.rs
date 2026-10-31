//! All editing coordinates are image pixels. Desktop coordinates may be negative.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn between(a: Point, b: Point) -> Self {
        Self::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }

    pub fn right(self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(self) -> f64 {
        self.y + self.h
    }

    pub fn clamp(self, bounds: Self) -> Self {
        let x = self.x.max(bounds.x).min(bounds.right());
        let y = self.y.max(bounds.y).min(bounds.bottom());
        Self::new(
            x,
            y,
            (self.right().min(bounds.right()) - x).max(0.0),
            (self.bottom().min(bounds.bottom()) - y).max(0.0),
        )
    }

    pub fn pixel_bounds(self, width: i32, height: i32) -> (i32, i32, i32, i32) {
        let r = self.clamp(Self::new(0.0, 0.0, width as f64, height as f64));
        let x = r.x.floor() as i32;
        let y = r.y.floor() as i32;
        (
            x,
            y,
            r.right().ceil() as i32 - x,
            r.bottom().ceil() as i32 - y,
        )
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Monitor {
    pub name: String,
    pub rect: Rect,
    pub scale: f64,
}

/// Resolve a saved choice only when its identity is unambiguous. Indices alone
/// are not stable when displays are disconnected or the compositor reorders them.
pub fn monitor_index(monitors: &[Monitor], selected: &Monitor) -> Option<usize> {
    let mut matches = monitors.iter().enumerate().filter(|(_, m)| *m == selected);
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub monitors: Vec<Monitor>,
    pub bounds: Rect,
}

impl Layout {
    pub fn new(monitors: Vec<Monitor>) -> Option<Self> {
        if monitors.is_empty()
            || monitors.iter().any(|m| {
                ![m.rect.x, m.rect.y, m.rect.w, m.rect.h, m.scale]
                    .iter()
                    .all(|v| v.is_finite())
                    || m.rect.w <= 0.
                    || m.rect.h <= 0.
                    || m.scale <= 0.
            })
        {
            return None;
        }
        let x = monitors
            .iter()
            .map(|m| m.rect.x)
            .fold(f64::INFINITY, f64::min);
        let y = monitors
            .iter()
            .map(|m| m.rect.y)
            .fold(f64::INFINITY, f64::min);
        let right = monitors
            .iter()
            .map(|m| m.rect.right())
            .fold(f64::NEG_INFINITY, f64::max);
        let bottom = monitors
            .iter()
            .map(|m| m.rect.bottom())
            .fold(f64::NEG_INFINITY, f64::max);
        if ![right - x, bottom - y]
            .iter()
            .all(|v| v.is_finite() && *v > 0.)
        {
            return None;
        }
        Some(Self {
            monitors,
            bounds: Rect::new(x, y, right - x, bottom - y),
        })
    }

    /// Map a known full-layout capture (grim) into its common pixel grid.
    /// An aspect ratio alone is not evidence for mapping a portal image.
    pub fn image_monitors(&self, width: i32, height: i32) -> Option<Vec<Monitor>> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let sx = width as f64 / self.bounds.w;
        let sy = height as f64 / self.bounds.h;
        if (self.bounds.h * sx - height as f64).abs() > 2.0 {
            return None;
        }
        Some(
            self.monitors
                .iter()
                .map(|m| Monitor {
                    name: m.name.clone(),
                    rect: Rect::new(
                        (m.rect.x - self.bounds.x) * sx,
                        (m.rect.y - self.bounds.y) * sy,
                        m.rect.w * sx,
                        m.rect.h * sy,
                    ),
                    scale: m.scale,
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: f64, y: f64, w: f64, h: f64, scale: f64) -> Monitor {
        Monitor {
            name: "Test".into(),
            rect: Rect::new(x, y, w, h),
            scale,
        }
    }

    #[test]
    fn mixed_dpi_negative_origin_and_portrait() {
        let layout = Layout::new(vec![
            monitor(-1080.0, -400.0, 1080.0, 1920.0, 1.0),
            monitor(0.0, 0.0, 1920.0, 1080.0, 2.0),
        ])
        .unwrap();
        assert_eq!(layout.bounds, Rect::new(-1080.0, -400.0, 3000.0, 1920.0));
        let mapped = layout.image_monitors(6000, 3840).unwrap();
        assert_eq!(mapped[1].rect, Rect::new(2160.0, 800.0, 3840.0, 2160.0));
        assert_eq!(mapped[0].rect, Rect::new(0.0, 0.0, 2160.0, 3840.0));
    }

    #[test]
    fn rejects_single_monitor_portal_image_for_wide_layout() {
        let layout = Layout::new(vec![
            monitor(0.0, 0.0, 1920.0, 1080.0, 1.0),
            monitor(1920.0, 0.0, 1920.0, 1080.0, 1.0),
        ])
        .unwrap();
        assert!(layout.image_monitors(1920, 1080).is_none());
        assert!(layout.image_monitors(4800, 1350).is_some());
    }

    #[test]
    fn reverse_drag_across_monitor_seam_preserves_pixel_coverage() {
        let r = Rect::between(Point::new(2200.2, 800.4), Point::new(1800.7, 40.1));
        assert_eq!(r.pixel_bounds(3840, 1080), (1800, 40, 401, 761));
        assert_eq!(
            Rect::new(-10.0, -20.0, 60.0, 70.0).pixel_bounds(100, 100),
            (0, 0, 50, 50)
        );
    }

    #[test]
    fn offset_dual_4k_portrait_maps_to_known_capture_grid() {
        // Read-only GNOME/GDK inspection: two 4K displays at 150%, rotated
        // right display, GTK render scale 2. No desktop image is captured here.
        let layout = Layout::new(vec![
            monitor(2560., 0., 1440., 2560., 2.),
            monitor(0., 752., 2560., 1440., 2.),
        ])
        .unwrap();
        assert_eq!(layout.bounds, Rect::new(0., 0., 4000., 2560.));
        let grid = layout.image_monitors(8000, 5120).unwrap();
        assert_eq!(grid[0].rect, Rect::new(5120., 0., 2880., 5120.));
        assert_eq!(grid[1].rect, Rect::new(0., 1504., 5120., 2880.));
        let grid = layout.image_monitors(6000, 3840).unwrap();
        assert_eq!(grid[0].rect.pixel_bounds(6000, 3840), (3840, 0, 2160, 3840));
        assert_eq!(grid[1].rect.pixel_bounds(6000, 3840), (0, 1128, 3840, 2160));
        assert!(layout.image_monitors(3840, 2160).is_none());
    }

    #[test]
    fn transient_invalid_geometry_is_not_mapped() {
        for bad in [
            monitor(0., 0., 0., 1080., 1.),
            monitor(0., 0., 1920., -1., 1.),
            monitor(f64::NAN, 0., 1920., 1080., 1.),
            monitor(0., 0., 1920., 1080., 0.),
            monitor(0., 0., f64::INFINITY, 1080., 1.),
        ] {
            assert!(Layout::new(vec![bad]).is_none());
        }
        let layout = Layout::new(vec![monitor(0., 0., 1920., 1080., 1.)]).unwrap();
        assert!(layout.image_monitors(0, 0).is_none());
        assert!(layout.image_monitors(-1920, -1080).is_none());
    }
}
