//! Fill tessellation for Iggy edge paths.
//!
//! Iggy flattens curves (`FUN_1410ad260`) and triangulates each fill region
//! at runtime (`FUN_1410aaff0`, swf_draw_triangulate.c). A fill region is
//! bounded by the edges that have the fill on exactly one side (SWF edge
//! model), so an even-odd decomposition of those edges covers exactly the
//! region Iggy fills. The triangulation here is a scanline trapezoid
//! decomposition; it differs from Iggy's triangle layout but not from its
//! covered area. Anti-aliasing of edges is not reproduced.
use crate::assets::iggy::shape::{Layer, Segment};

/// Curve flattening tolerance in device pixels. Iggy derives its tolerance
/// from the draw scale and player+0x1e80 (`FUN_1410aaff0`); that value is
/// not read yet.
const APPROXIMATE_TOLERANCE: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub a: [f32; 2],
    pub b: [f32; 2],
}

/// Line segments of `layer` bordering fill `fill` (1-based), flattened for a
/// drawing scale of `scale` device pixels per shape unit.
pub fn fill_edges(layer: &Layer, fill: u16, scale: f32) -> Vec<Edge> {
    let mut out = Vec::new();
    let mut pen = [0f32; 2];
    let tolerance = APPROXIMATE_TOLERANCE / scale.max(1e-6);
    for p in &layer.points {
        let end = [p.x, p.y];
        let borders = (p.fill0 == fill) != (p.fill1 == fill);
        match p.segment {
            Segment::MoveTo => {}
            Segment::LineTo => {
                if borders {
                    out.push(Edge { a: pen, b: end });
                }
            }
            Segment::CurveTo => {
                if borders {
                    flatten(pen, [p.cx, p.cy], end, tolerance, &mut out);
                }
            }
        }
        pen = end;
    }
    out
}

/// Quadratic Bézier flattening with a segment count from the control-point
/// deviation (the curve's maximum distance from its chord is 1/4 of it).
pub fn flatten(a: [f32; 2], c: [f32; 2], b: [f32; 2], tolerance: f32, out: &mut Vec<Edge>) {
    let dx = a[0] - 2.0 * c[0] + b[0];
    let dy = a[1] - 2.0 * c[1] + b[1];
    let deviation = (dx * dx + dy * dy).sqrt() / 4.0;
    let steps = ((deviation / tolerance).sqrt().ceil() as usize).clamp(1, 64);
    let mut prev = a;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0 - t;
        let q = [u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0], u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1]];
        out.push(Edge { a: prev, b: q });
        prev = q;
    }
}

/// Even-odd triangulation of a set of edges: scanline bands between edge
/// endpoints, paired crossings per band, two triangles per trapezoid.
pub fn triangulate(edges: &[Edge]) -> Vec<[f32; 2]> {
    let mut ys: Vec<f32> = edges.iter().flat_map(|e| [e.a[1], e.b[1]]).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let mut out = Vec::new();
    for band in ys.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        if y1 - y0 < 1e-6 {
            continue;
        }
        let ym = 0.5 * (y0 + y1);
        // Edges spanning the band, with their x at the band's top and bottom.
        let mut spans: Vec<(f32, f32, f32)> = edges
            .iter()
            .filter(|e| (e.a[1] <= ym) != (e.b[1] <= ym))
            .map(|e| {
                let at = |y: f32| e.a[0] + (y - e.a[1]) * (e.b[0] - e.a[0]) / (e.b[1] - e.a[1]);
                (at(ym), at(y0), at(y1))
            })
            .collect();
        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for pair in spans.chunks_exact(2) {
            let (l, r) = (pair[0], pair[1]);
            let (tl, tr, bl, br) = ([l.1, y0], [r.1, y0], [l.2, y1], [r.2, y1]);
            out.extend([tl, tr, br, tl, br, bl]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::iggy::shape::{Fill, PathPoint};

    fn point(segment: Segment, x: f32, y: f32, fill1: u16) -> PathPoint {
        PathPoint { x, y, cx: 0.0, cy: 0.0, segment, fill0: 0, fill1, line: 0 }
    }

    fn area(tris: &[[f32; 2]]) -> f32 {
        tris.chunks_exact(3)
            .map(|t| ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])).abs() / 2.0)
            .sum()
    }

    #[test]
    fn square_fill_covers_its_area() {
        let layer = Layer {
            bounds: [0.0; 4],
            points: vec![
                point(Segment::MoveTo, -108.0, -108.0, 0),
                point(Segment::LineTo, 108.0, -108.0, 1),
                point(Segment::LineTo, 108.0, 108.0, 1),
                point(Segment::LineTo, -108.0, 108.0, 1),
                point(Segment::LineTo, -108.0, -108.0, 1),
            ],
            fills: vec![Fill::Solid([255; 4])],
            lines: Vec::new(),
            fill_bounds: Vec::new(),
            no_strokes: true,
        };
        let tris = triangulate(&fill_edges(&layer, 1, 1.0));
        assert!((area(&tris) - 216.0 * 216.0).abs() < 1e-2);
    }

    #[test]
    fn even_odd_hole_is_excluded() {
        let square = |s: f32| {
            vec![
                Edge { a: [-s, -s], b: [s, -s] },
                Edge { a: [s, -s], b: [s, s] },
                Edge { a: [s, s], b: [-s, s] },
                Edge { a: [-s, s], b: [-s, -s] },
            ]
        };
        let mut edges = square(10.0);
        edges.extend(square(5.0));
        assert!((area(&triangulate(&edges)) - (400.0 - 100.0)).abs() < 1e-3);
    }
}
