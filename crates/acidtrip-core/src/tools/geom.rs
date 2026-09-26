//! Shape rasterization shared by the cell and half-block pixel tools.

use std::collections::BTreeMap;

/// Outline of the ellipse inscribed in the inclusive box (x0,y0)-(x1,y1),
/// using Zingl's integer midpoint algorithm, which handles even and odd
/// diameters exactly. Points may repeat.
pub(crate) fn ellipse_outline(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<(i64, i64)> {
    let (mut x0, mut x1) = (x0.min(x1), x0.max(x1));
    let (top, bottom) = (y0.min(y1), y0.max(y1));
    if x0 == x1 || top == bottom {
        // Zero-diameter axis: the ellipse is a straight line.
        return (top..=bottom).flat_map(|y| (x0..=x1).map(move |x| (x, y))).collect();
    }
    let a = x1 - x0;
    let b = bottom - top;
    let b1 = b & 1;
    let mut dx = 4 * (1 - a) * b * b;
    let mut dy = 4 * (b1 + 1) * a * a;
    let mut err = dx + dy + b1 * a * a;
    let mut y0 = top + (b + 1) / 2;
    let mut y1 = y0 - b1;
    let (a8, b8) = (8 * a * a, 8 * b * b);
    let mut pts = Vec::new();
    loop {
        pts.extend([(x1, y0), (x0, y0), (x0, y1), (x1, y1)]);
        let e2 = 2 * err;
        if e2 <= dy {
            y0 += 1;
            y1 -= 1;
            dy += a8;
            err += dy;
        }
        if e2 >= dx || 2 * err > dy {
            x0 += 1;
            x1 -= 1;
            dx += b8;
            err += dx;
        }
        if x0 > x1 {
            break;
        }
    }
    // Flat ellipses stop early; finish the tips.
    while y0 - y1 <= b {
        pts.extend([(x0 - 1, y0), (x1 + 1, y0), (x0 - 1, y1), (x1 + 1, y1)]);
        y0 += 1;
        y1 -= 1;
    }
    pts
}

/// Horizontal spans (y, x_min, x_max) covering the filled ellipse.
pub(crate) fn ellipse_spans(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<(i64, i64, i64)> {
    let mut rows: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
    for (x, y) in ellipse_outline(x0, y0, x1, y1) {
        let e = rows.entry(y).or_insert((x, x));
        e.0 = e.0.min(x);
        e.1 = e.1.max(x);
    }
    rows.into_iter().map(|(y, (a, b))| (y, a, b)).collect()
}

/// Integer line, both endpoints included. Uses the midpoint variant of
/// Bresenham, which centers the steps of shallow lines (plain `Bresenham` in
/// `line_drawing` bunches them at one end).
pub(crate) fn line(x0: i64, y0: i64, x1: i64, y1: i64) -> impl Iterator<Item = (i64, i64)> {
    line_drawing::Midpoint::<f64, i64>::new((x0 as f64, y0 as f64), (x1 as f64, y1 as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn set(pts: Vec<(i64, i64)>) -> HashSet<(i64, i64)> {
        pts.into_iter().collect()
    }

    #[test]
    fn outline_stays_in_box_and_touches_all_edges() {
        for (w, h) in SIZES {
            let p = set(ellipse_outline(0, 0, w - 1, h - 1));
            assert!(p.iter().all(|&(x, y)| (0..w).contains(&x) && (0..h).contains(&y)), "{w}x{h} escapes box");
            assert!(p.iter().any(|&(x, _)| x == 0), "{w}x{h} left");
            assert!(p.iter().any(|&(x, _)| x == w - 1), "{w}x{h} right");
            assert!(p.iter().any(|&(_, y)| y == 0), "{w}x{h} top");
            assert!(p.iter().any(|&(_, y)| y == h - 1), "{w}x{h} bottom");
        }
    }

    const SIZES: [(i64, i64); 16] = [
        (1, 1),
        (1, 5),
        (5, 1),
        (2, 2),
        (2, 7),
        (7, 2),
        (3, 3),
        (4, 3),
        (10, 6),
        (7, 12),
        (31, 9),
        (3, 20),
        (20, 3),
        (80, 25),
        (2, 1),
        (1, 2),
    ];

    #[test]
    fn outline_rows_are_covered_without_gaps() {
        for (w, h) in SIZES {
            let p = set(ellipse_outline(0, 0, w - 1, h - 1));
            for y in 0..h {
                assert!(p.iter().any(|&(_, py)| py == y), "{w}x{h} misses row {y}");
            }
            // 8-connected: every point has a neighbor (unless it is the only point).
            if p.len() > 1 {
                for &(x, y) in &p {
                    let n = (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (x + dx, y + dy)));
                    assert!(n.filter(|q| *q != (x, y)).any(|q| p.contains(&q)), "{w}x{h} isolated {x},{y}");
                }
            }
        }
    }

    #[test]
    fn outline_is_symmetric() {
        for (w, h) in [(6, 4), (7, 5), (12, 7), (9, 10)] {
            let p = set(ellipse_outline(0, 0, w - 1, h - 1));
            for &(x, y) in &p {
                assert!(p.contains(&(w - 1 - x, y)), "{w}x{h} not x-symmetric at {x},{y}");
                assert!(p.contains(&(x, h - 1 - y)), "{w}x{h} not y-symmetric at {x},{y}");
            }
        }
    }

    #[test]
    fn degenerate_boxes_are_lines() {
        assert_eq!(set(ellipse_outline(2, 3, 2, 3)), set(vec![(2, 3)]));
        assert_eq!(set(ellipse_outline(0, 0, 0, 3)), set((0..4).map(|y| (0, y)).collect()));
        assert_eq!(set(ellipse_outline(0, 0, 3, 0)), set((0..4).map(|x| (x, 0)).collect()));
    }

    #[test]
    fn spans_cover_every_row() {
        let s = ellipse_spans(0, 0, 9, 5);
        assert_eq!(s.len(), 6);
        assert!(s.iter().all(|&(_, a, b)| a <= b));
    }

    #[test]
    fn line_includes_endpoints() {
        let v: Vec<_> = line(0, 0, 3, -2).collect();
        assert_eq!(v.first(), Some(&(0, 0)));
        assert_eq!(v.last(), Some(&(3, -2)));
        assert_eq!(v.len(), 4);
    }
}
