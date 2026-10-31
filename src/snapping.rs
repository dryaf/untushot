//! Alignment of moving objects; all distances are in image coordinates.
use crate::geometry::{Point, Rect};

#[derive(Clone, Copy, Debug)]
pub struct Guide {
    pub vertical: bool,
    pub coordinate: f64,
    pub start: f64,
    pub end: f64,
}

pub fn align(moving: Rect, targets: &[Rect], tolerance: f64) -> (Point, Vec<Guide>) {
    let axis = |vertical: bool| {
        let anchors = |r: Rect| {
            if vertical {
                [r.x, r.x + r.w / 2., r.right()]
            } else {
                [r.y, r.y + r.h / 2., r.bottom()]
            }
        };
        let mut best: Option<(f64, f64, Rect)> = None;
        for target in targets {
            for to in anchors(*target) {
                for from in anchors(moving) {
                    let delta = to - from;
                    if delta.abs() <= tolerance
                        && best.map_or(true, |(distance, _, _)| delta.abs() < distance.abs())
                    {
                        best = Some((delta, to, *target));
                    }
                }
            }
        }
        best
    };
    let (x, y) = (axis(true), axis(false));
    let offset = Point::new(x.map_or(0., |v| v.0), y.map_or(0., |v| v.0));
    let moved = Rect::new(moving.x + offset.x, moving.y + offset.y, moving.w, moving.h);
    let mut guides = Vec::new();
    for (vertical, best) in [(true, x), (false, y)] {
        if let Some((_, coordinate, target)) = best {
            let span = moved.union(target);
            guides.push(Guide {
                vertical,
                coordinate,
                start: if vertical { span.y } else { span.x },
                end: if vertical {
                    span.bottom()
                } else {
                    span.right()
                },
            });
        }
    }
    (offset, guides)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_edges_and_centers_snap_independently() {
        let canvas = Rect::new(10., 20., 1000., 700.);
        let (offset, guides) = align(Rect::new(13., 22., 100., 80.), &[canvas], 6.);
        assert_eq!(offset, Point::new(-3., -2.));
        assert_eq!(guides.len(), 2);
        let (offset, guides) = align(Rect::new(457., 332., 100., 80.), &[canvas], 6.);
        assert_eq!(offset, Point::new(3., -2.));
        assert!(guides.iter().any(|g| g.vertical && g.coordinate == 510.));
        let other = Rect::new(300., 100., 60., 80.);
        let (offset, _) = align(Rect::new(262., 101., 40., 30.), &[canvas, other], 6.);
        assert_eq!(offset, Point::new(-2., -1.));
    }
    #[test]
    fn tolerance_uses_screen_pixels_and_ties_are_stable() {
        let moving = Rect::new(3., 13., 20., 20.);
        let canvas = Rect::new(0., 0., 100., 100.);
        assert_eq!(align(moving, &[canvas], 6.).0.x, -3.);
        assert_eq!(align(moving, &[canvas], 6. / 4.).0.x, 0.);
        let a = Rect::new(1., 100., 100., 50.);
        let b = Rect::new(5., 200., 100., 50.);
        assert_eq!(align(moving, &[a, b], 6.).0.x, -2.);
        assert!(align(moving, &[], 6.).1.is_empty());
    }
}
