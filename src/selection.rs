use crate::geometry::{Point, Rect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    New,
    Move,
    Nw,
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
}

pub fn handles(r: Rect) -> [(Point, Handle); 8] {
    use Handle::*;
    [
        (Point::new(r.x, r.y), Nw),
        (Point::new(r.right(), r.y), Ne),
        (Point::new(r.right(), r.bottom()), Se),
        (Point::new(r.x, r.bottom()), Sw),
        (Point::new(r.x + r.w / 2., r.y), N),
        (Point::new(r.right(), r.y + r.h / 2.), E),
        (Point::new(r.x + r.w / 2., r.bottom()), S),
        (Point::new(r.x, r.y + r.h / 2.), W),
    ]
}

pub fn hit(r: Rect, p: Point, tolerance: f64) -> Option<Handle> {
    hit_handle(r, p, tolerance).or_else(|| r.contains(p).then_some(Handle::Move))
}

pub fn hit_handle(r: Rect, p: Point, tolerance: f64) -> Option<Handle> {
    handles(r)
        .into_iter()
        .filter(|(v, _)| (v.x - p.x).abs() <= tolerance && (v.y - p.y).abs() <= tolerance)
        .min_by(|(a, _), (b, _)| {
            let distance = |v: Point| (v.x - p.x).powi(2) + (v.y - p.y).powi(2);
            distance(*a).total_cmp(&distance(*b))
        })
        .map(|(_, h)| h)
}

/// Keep the opposite corner or edge midpoint fixed during object scaling.
pub fn anchored_size(r: Rect, width: f64, height: f64, handle: Handle) -> Rect {
    use Handle::*;
    let x = match handle {
        Nw | W | Sw => r.right() - width,
        N | S => r.x + (r.w - width) / 2.,
        _ => r.x,
    };
    let y = match handle {
        Nw | N | Ne => r.bottom() - height,
        E | W => r.y + (r.h - height) / 2.,
        _ => r.y,
    };
    Rect::new(x, y, width, height)
}

/// Object resizing is not clipped to the canvas. The pointer delta preserves
/// the grab offset; crossing the fixed edge stops at one pixel instead of flipping.
pub fn resize_object(r: Rect, handle: Handle, dx: f64, dy: f64, proportional: bool) -> Rect {
    use Handle::*;
    let dw = match handle {
        Nw | W | Sw => -dx,
        Ne | E | Se => dx,
        _ => 0.,
    };
    let dh = match handle {
        Nw | N | Ne => -dy,
        Sw | S | Se => dy,
        _ => 0.,
    };
    let (width, height) = if proportional && r.w > 0. && r.h > 0. {
        let delta = match handle {
            N | S => dh / r.h,
            E | W => dw / r.w,
            _ if (dw / r.w).abs() >= (dh / r.h).abs() => dw / r.w,
            _ => dh / r.h,
        };
        let ratio = (1. + delta).max(1. / r.w).max(1. / r.h);
        (r.w * ratio, r.h * ratio)
    } else {
        (
            if dw == 0. { r.w } else { (r.w + dw).max(1.) },
            if dh == 0. { r.h } else { (r.h + dh).max(1.) },
        )
    };
    anchored_size(r, width, height, handle)
}

pub struct Drag {
    pub original: Rect,
    pub start: Point,
    pub handle: Handle,
    pub preview: Rect,
}
impl Drag {
    pub fn update(&mut self, p: Point, bounds: Rect) {
        use Handle::*;
        let p = Point::new(
            p.x.round().clamp(bounds.x, bounds.right()),
            p.y.round().clamp(bounds.y, bounds.bottom()),
        );
        let dx = (p.x - self.start.x).round();
        let dy = (p.y - self.start.y).round();
        let r = self.original;
        self.preview = match self.handle {
            New => Rect::between(self.start, p).clamp(bounds),
            Move => Rect::new(
                (r.x + dx).clamp(bounds.x, (bounds.right() - r.w).max(bounds.x)),
                (r.y + dy).clamp(bounds.y, (bounds.bottom() - r.h).max(bounds.y)),
                r.w,
                r.h,
            ),
            h => {
                let left = if matches!(h, Nw | W | Sw) {
                    p.x.min(r.right() - 1.)
                } else {
                    r.x
                };
                let right = if matches!(h, Ne | E | Se) {
                    p.x.max(r.x + 1.)
                } else {
                    r.right()
                };
                let top = if matches!(h, Nw | N | Ne) {
                    p.y.min(r.bottom() - 1.)
                } else {
                    r.y
                };
                let bottom = if matches!(h, Sw | S | Se) {
                    p.y.max(r.y + 1.)
                } else {
                    r.bottom()
                };
                Rect::new(left, top, right - left, bottom - top).clamp(bounds)
            }
        };
    }
}

pub fn nudge(r: Rect, dx: f64, dy: f64, resize: bool, bounds: Rect) -> Rect {
    if resize {
        Rect::new(
            r.x,
            r.y,
            (r.w + dx).clamp(1., bounds.right() - r.x),
            (r.h + dy).clamp(1., bounds.bottom() - r.y),
        )
    } else {
        Rect::new(
            (r.x + dx).clamp(bounds.x, bounds.right() - r.w),
            (r.y + dy).clamp(bounds.y, bounds.bottom() - r.h),
            r.w,
            r.h,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_handles_resize_from_the_opposite_edge() {
        use Handle::*;
        let r = Rect::new(100., 80., 200., 100.);
        for (handle, expected) in [
            (Nw, Rect::new(110., 100., 190., 80.)),
            (N, Rect::new(100., 100., 200., 80.)),
            (Ne, Rect::new(100., 100., 210., 80.)),
            (E, Rect::new(100., 80., 210., 100.)),
            (Se, Rect::new(100., 80., 210., 120.)),
            (S, Rect::new(100., 80., 200., 120.)),
            (Sw, Rect::new(110., 80., 190., 120.)),
            (W, Rect::new(110., 80., 190., 100.)),
        ] {
            assert_eq!(
                resize_object(r, handle, 10., 20., false),
                expected,
                "{handle:?}"
            );
        }
        assert_eq!(
            resize_object(r, Nw, 1000., 1000., false),
            Rect::new(299., 179., 1., 1.)
        );
        assert_eq!(
            resize_object(r, Nw, -200., -100., false),
            Rect::new(-100., -20., 400., 200.)
        );
        // Prefer the nearest grip when targets overlap at low zoom.
        assert_eq!(
            hit_handle(Rect::new(0., 0., 10., 10.), Point::new(5., 0.), 8.),
            Some(N)
        );
    }

    #[test]
    fn proportional_handles_keep_corners_or_edge_midpoints_fixed() {
        use Handle::*;
        let r = Rect::new(100., 80., 200., 100.);
        let cases = [
            (Nw, (-40., -10.), Rect::new(60., 60., 240., 120.)),
            (N, (900., -20.), Rect::new(80., 60., 240., 120.)),
            (Ne, (40., -10.), Rect::new(100., 60., 240., 120.)),
            (E, (40., 900.), Rect::new(100., 70., 240., 120.)),
            (Se, (40., 10.), Rect::new(100., 80., 240., 120.)),
            (S, (900., 20.), Rect::new(80., 80., 240., 120.)),
            (Sw, (-40., 10.), Rect::new(60., 80., 240., 120.)),
            (W, (-40., 900.), Rect::new(60., 70., 240., 120.)),
        ];
        for (handle, (dx, dy), expected) in cases {
            assert_eq!(
                resize_object(r, handle, dx, dy, true),
                expected,
                "{handle:?}"
            );
            let min = resize_object(r, handle, -dx * 100., -dy * 100., true);
            assert!(min.w >= 1. && min.h >= 1.);
            assert!((min.w / min.h - 2.).abs() < 1e-9);
        }
    }

    #[test]
    fn resize_all_handles_and_move_without_shrinking() {
        let bounds = Rect::new(0., 0., 400., 300.);
        let r = Rect::new(100., 80., 200., 100.);
        for (start, handle) in handles(r) {
            let mut d = Drag {
                original: r,
                start,
                handle,
                preview: r,
            };
            d.update(Point::new(start.x + 10., start.y + 10.), bounds);
            assert!(d.preview.w >= 1. && d.preview.h >= 1.);
            assert_ne!(d.preview, r);
        }
        let mut d = Drag {
            original: r,
            start: Point::new(120., 100.),
            handle: Handle::Move,
            preview: r,
        };
        d.update(Point::new(0., 0.), bounds);
        assert_eq!(d.preview, Rect::new(0., 0., 200., 100.));
        assert_eq!(
            nudge(r, 1., -1., false, bounds),
            Rect::new(101., 79., 200., 100.)
        );
        assert_eq!(
            nudge(r, 10., 0., true, bounds),
            Rect::new(100., 80., 210., 100.)
        );
    }
    #[test]
    fn reverse_drag_snaps_to_pixels_and_handles_win_over_move() {
        let mut d = Drag {
            original: Rect::default(),
            start: Point::new(300., 200.),
            handle: Handle::New,
            preview: Rect::default(),
        };
        d.update(Point::new(100.3, 49.8), Rect::new(0., 0., 400., 300.));
        assert_eq!(d.preview, Rect::new(100., 50., 200., 150.));
        assert_eq!(hit(d.preview, Point::new(100., 50.), 4.), Some(Handle::Nw));
    }
}
