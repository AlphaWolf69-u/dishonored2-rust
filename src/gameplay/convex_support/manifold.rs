//! Face-clip candidate geometry from 1412abc30, after its GSK/face virtuals.
//! The caller retains shapes, face selection and cache; this workspace retains
//! only the source's 48 candidate slots and two projected 16-vertex polygons.
use super::seed::{cross, dot, perpendicular, subtract};
use crate::gameplay::{constraint_solver::reciprocal, physics_world::contacts::ManifoldCandidate};

#[derive(Clone, Copy)]
pub(crate) struct ClipFace<'a> {
    /// Actual face vertices in the shape virtual's cyclic order, common frame.
    pub vertices: &'a [[f32; 4]],
    /// Outward plane: dot(XYZ, point) + W = 0, in that same frame.
    pub plane: [f32; 4],
}
pub(crate) struct FaceClipInput<'a> {
    pub a: ClipFace<'a>,
    pub b: ClipFace<'a>,
    /// GSK separating normal, directed from B towards A.
    pub normal: [f32; 4],
    pub radii: [f32; 2],
    pub tolerance: f32,
    /// Collision settings+90, used by the source's depth-weighted reduction.
    pub reduction_tolerance: f32,
}
pub(crate) struct ClippedManifold<'a> {
    /// Includes inactive slots: correspondence indexes these by source ID.
    pub candidates: &'a [ManifoldCandidate],
    pub selected: &'a [u8],
    pub normal: [f32; 4],
}

const EMPTY: ManifoldCandidate = ManifoldCandidate {
    id: 0,
    projected_xy: [0.; 2],
    point_on_b: [0.; 4],
    distance: 0.,
};
pub(crate) struct ManifoldWorkspace {
    candidates: [ManifoldCandidate; 48],
    active: [bool; 48],
    a: [[f32; 2]; 16],
    b: [[f32; 2]; 16],
    selected: [u8; 4],
}
impl Default for ManifoldWorkspace {
    fn default() -> Self {
        Self {
            candidates: [EMPTY; 48],
            active: [false; 48],
            a: [[0.; 2]; 16],
            b: [[0.; 2]; 16],
            selected: [0; 4],
        }
    }
}

/// Source SSE extrema retain the first winning index within each lane.
/// Deepest/farthest then reduce 0/2, 1/3, 0/1 with strict comparisons;
/// signed area reduces adjacent lanes with <=, choosing the later tied lane.
fn extreme(
    count: usize,
    active: &[bool; 48],
    initial: f32,
    later_ties: bool,
    score: impl Fn(usize) -> f32,
) -> Option<usize> {
    let mut values = [initial; 4];
    let mut indices = [None; 4];
    for i in 0..count {
        let lane = i & 3;
        let value = score(i);
        if active[i] && values[lane] < value {
            values[lane] = value;
            indices[lane] = Some(i);
        }
    }
    let choose = |a: usize, b: usize| {
        if values[a] < values[b] || (later_ties && values[a] == values[b]) { b } else { a }
    };
    let lane = if later_ties {
        choose(choose(0, 1), choose(2, 3))
    } else {
        choose(choose(0, 2), choose(1, 3))
    };
    indices[lane]
}

impl ManifoldWorkspace {
    pub(crate) fn clip(&mut self, input: FaceClipInput<'_>) -> Result<ClippedManifold<'_>, String> {
        let na = input.a.vertices.len();
        let nb = input.b.vertices.len();
        if !(3..=16).contains(&na) || !(3..=16).contains(&nb) {
            return Err("face clip requires the shape virtual's 3..16 vertex face polygons".into());
        }
        let pa = (na + 3) & !3;
        let pb = (nb + 3) & !3;
        let count = pa + 2 * pb;
        self.active.fill(false);
        self.selected = [0; 4];

        // 1412ac8xx: near-perpendicular face planes perturb only the clip
        // direction, at most ten times. The published contact normal stays GSK's.
        let mut direction = input.normal;
        let mut denominators = [0.; 2];
        let mut usable = false;
        for attempt in 0..10 {
            denominators = [dot(direction, input.a.plane), dot(direction, input.b.plane)];
            if denominators.iter().all(|d| d * d >= f32::from_bits(0x3400_0000)) {
                usable = true;
                break;
            }
            if attempt != 9 {
                direction = normalize(std::array::from_fn(|i| {
                    direction[i] + input.a.plane[i] * 0.001 + input.b.plane[i] * 0.0015
                }));
            }
        }
        let inverses = if usable {
            denominators.map(reciprocal)
        } else {
            direction = input.normal;
            [0.; 2]
        };
        let perpendicular = perpendicular(direction);
        let factor = normalization_factor(dot(perpendicular, perpendicular));
        let tangent = perpendicular.map(|v| v * factor);
        let bitangent = cross(direction, perpendicular).map(|v| v * factor);
        let origin = input.a.vertices[0];
        let project = |point| {
            let delta = subtract(point, origin);
            [dot(delta, tangent), dot(delta, bitangent)]
        };
        for i in 0..pa {
            self.a[i] = project(input.a.vertices[i.min(na - 1)]);
        }
        for i in 0..pb {
            self.b[i] = project(input.b.vertices[i.min(nb - 1)]);
        }
        let area_a = winding(&self.a[..na]);
        let area_b = winding(&self.b[..nb]);
        // Source SIMD padding repeats the final vertex. Its candidate IDs stay
        // addressable too; exact duplicate XY suppression happens in reduction.
        for i in 0..pa {
            self.candidates[i].projected_xy = self.a[i];
            self.active[i] = inside(self.a[i], &self.b[..nb], area_b);
        }
        for i in 0..pb {
            let start = self.b[i];
            let end = self.b[if i + 1 < pb { i + 1 } else { 0 }];
            let mut lower = 0.0f32;
            let mut upper = 1.0f32;
            for edge in 0..na {
                let a = self.a[edge];
                let b = self.a[(edge + 1) % na];
                let delta = [b[0] - a[0], b[1] - a[1]];
                let side = |p: [f32; 2]| area_a * (delta[0] * (p[1] - a[1]) - delta[1] * (p[0] - a[0]));
                let s0 = side(start);
                let change = side(end) - s0;
                // 1412adxxx uses the source's finite zero-denominator sentinel.
                let inverse = if change == 0. { f32::from_bits(0x5f7f_fff0) } else { reciprocal(change) };
                let t = -s0 * inverse;
                if change < 0. {
                    upper = upper.min(t);
                } else {
                    lower = lower.max(t);
                }
            }
            lower = lower.min(f32::from_bits(0x3f80_0054)); // 141f68840: 1+1e-5
            upper = upper.max(f32::from_bits(0xb727_c5ac)); // 141f68830: -1e-5
            let point = |t| [start[0] + (end[0] - start[0]) * t, start[1] + (end[1] - start[1]) * t];
            self.candidates[pa + i].projected_xy = point(lower);
            self.candidates[pa + pb + i].projected_xy = point(upper);
            // Enter includes zero; exit excludes one. Shared vertices therefore
            // belong to the following edge's entry rather than two exit slots.
            self.active[pa + i] = 0. <= lower && lower <= upper;
            self.active[pa + pb + i] = lower <= upper && upper < 1.;
        }
        let projection_dot = dot(input.normal, direction);
        let mut minimum = f32::from_bits(0x7f7f_ffee);
        for i in 0..count {
            let candidate = &mut self.candidates[i];
            candidate.id = i as u8;
            let [x, y] = candidate.projected_xy;
            let base = std::array::from_fn(|j| (origin[j] + tangent[j] * x) + bitangent[j] * y);
            let heights = [
                -(dot(input.a.plane, base) + input.a.plane[3]) * inverses[0],
                -(dot(input.b.plane, base) + input.b.plane[3]) * inverses[1],
            ];
            candidate.point_on_b = std::array::from_fn(|j| {
                (base[j] + direction[j] * heights[1]) + input.normal[j] * input.radii[1]
            });
            candidate.distance = (heights[0] - heights[1]) * projection_dot - (input.radii[0] + input.radii[1]);
            if self.active[i] { minimum = minimum.min(candidate.distance); }
        }
        // Geometry admission is manifold-wide; sloped face points remain in the
        // candidate packet even when individually above the contact tolerance.
        let selected_count = if minimum < input.tolerance { self.reduce(count, input.reduction_tolerance) } else { 0 };
        Ok(ClippedManifold {
            candidates: &self.candidates[..count],
            selected: &self.selected[..selected_count],
            normal: input.normal,
        })
    }

    fn reduce(&mut self, count: usize, tolerance: f32) -> usize {
        let lowest = -f32::from_bits(0x7f7f_ffee);
        let Some(first) = extreme(count, &self.active, lowest, false, |i| -self.candidates[i].distance) else { return 0 };
        let a = self.candidates[first];
        let Some(second) = extreme(count, &self.active, lowest, false, |i| {
            let p = self.candidates[i].projected_xy;
            let dy = p[1] - a.projected_xy[1];
            let dx = p[0] - a.projected_xy[0];
            dy * dy + dx * dx
        }) else { return 0 };
        self.selected[0] = first as u8;
        if second == first { return 1; }
        self.selected[1] = second as u8;
        let b = self.candidates[second];
        let area = |i: usize| {
            let p = self.candidates[i];
            let dy = p.projected_xy[1] - a.projected_xy[1];
            let dx = p.projected_xy[0] - a.projected_xy[0];
            let numerator = dy * (a.projected_xy[0] - b.projected_xy[0])
                - dx * (a.projected_xy[1] - b.projected_xy[1]);
            // 1412aecxx: depth-weighted signed area, using unrefined RCPPS.
            let denominator = (p.distance - a.distance) + tolerance * 2.;
            numerator * reciprocal_estimate(denominator)
        };
        let positive = extreme(count, &self.active, 0., true, area).unwrap_or(first);
        let negative = extreme(count, &self.active, 0., true, |i| -area(i)).unwrap_or(first);
        let mut n = 2;
        for index in [positive, negative] {
            if !self.selected[..n].contains(&(index as u8)) {
                self.selected[n] = index as u8;
                n += 1;
            }
        }
        // Source fills collapsed extrema by ascending source slot, rejecting
        // exact XY duplicates rather than a heuristic spatial threshold.
        for i in 0..count {
            if n == 4 { break; }
            if self.active[i] && !self.selected[..n].iter().any(|&old| {
                old as usize == i || self.candidates[old as usize].projected_xy == self.candidates[i].projected_xy
            }) {
                self.selected[n] = i as u8;
                n += 1;
            }
        }
        n
    }
}

fn reciprocal_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    { 1. / value }
}

fn normalization_factor(squared: f32) -> f32 {
    let r = super::faces::reciprocal_sqrt_estimate(squared);
    ((3. - r * squared * r) * r) * 0.5
}
fn normalize(v: [f32; 4]) -> [f32; 4] {
    let factor = normalization_factor(dot(v, v));
    v.map(|x| x * factor)
}

fn winding(vertices: &[[f32; 2]]) -> f32 {
    let mut area = 0.;
    for i in 0..vertices.len() {
        let p = vertices[i];
        let q = vertices[(i + 1) % vertices.len()];
        area += p[0] * q[1] - p[1] * q[0];
    }
    if area < 0. { -1. } else { 1. }
}
fn inside(point: [f32; 2], vertices: &[[f32; 2]], winding: f32) -> bool {
    vertices.iter().enumerate().all(|(i, a)| {
        let b = vertices[(i + 1) % vertices.len()];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let squared = dy * dy + dx * dx;
        let inverse = if squared <= 0. { 0. } else { super::faces::reciprocal_sqrt_estimate(squared) };
        // 1412adxxx uses unrefined RSQRTPS for normalized clip edge tests.
        let outside = winding * ((point[0] - a[0]) * (dy * inverse) - (point[1] - a[1]) * (dx * inverse));
        outside < 0.001
    })
}
