//! Point/target feature reductions in141212780's count1/2/3/4 branches.
use super::super::seed::{cross, dot, subtract};
use crate::gameplay::constraint_solver::reciprocal;

fn reciprocal_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        unimplemented!("point segment reduction requires x86-64 RCPPS")
    }
}

pub(super) fn reduce(
    point: [f32; 4],
    vertices: &mut [[f32; 4]; 4],
    count: &mut usize,
) -> Result<([f32; 4], bool), String> {
    if *count == 4 && !tetrahedron(point, vertices) {
        return Ok(([0.; 4], true));
    }
    if *count == 4 {
        *count = 3;
    }
    if *count == 3 {
        if let Some(direction) = triangle(point, vertices, count) {
            return Ok((direction, false));
        }
    }
    if *count == 2 {
        return Ok((segment(point, vertices, count), false));
    }
    if *count == 1 {
        return Ok((subtract(point, vertices[0]), false));
    }
    Err("invalid point distance feature count".into())
}

fn segment(point: [f32; 4], v: &mut [[f32; 4]; 4], count: &mut usize) -> [f32; 4] {
    let edge = subtract(v[1], v[0]);
    let a = dot(subtract(v[1], point), edge);
    let b = dot(edge, subtract(v[0], point));
    if b * a < 0. {
        *count = 2;
        let numerator =
            std::array::from_fn(|i| a * (v[0][i] - point[i]) - b * (v[1][i] - point[i]));
        let n = cross(edge, numerator);
        let result = cross(edge, n);
        let inverse = reciprocal_estimate(dot(edge, edge));
        return result.map(|v| v * inverse);
    }
    *count = 1;
    if a <= 0. {
        v[0] = v[1];
    }
    subtract(point, v[0])
}

/// Returns a face normal for an interior projection, or reduces to an edge.
fn triangle(point: [f32; 4], v: &mut [[f32; 4]; 4], count: &mut usize) -> Option<[f32; 4]> {
    let [a, b, c, _] = *v;
    let ac = subtract(a, c);
    let cb = subtract(c, b);
    let mut n = cross(cb, ac);
    let numerators = [
        dot(cross(subtract(point, b), cb), n),
        dot(cross(subtract(point, c), ac), n),
        dot(cross(subtract(point, a), subtract(b, a)), n),
    ];
    let mask = numerators
        .iter()
        .enumerate()
        .fold(0, |mask, (i, &value)| mask | ((value < 0.) as usize) << i);
    if mask == 7 {
        if dot(subtract(point, a), n) < 0. {
            v.swap(0, 1);
            n = n.map(|v| -v);
        }
        return Some(n);
    }
    let region = [-1_i8, 0, 1, -6, 2, -7, -8, -1][mask];
    if region < 0 {
        let slot = (region + 8) as usize;
        if slot > 2 {
            *count = 1;
            return Some(subtract(point, v[0]));
        }
        v[slot] = v[2];
        *count = 2;
        return None;
    }
    let base = region as usize;
    let first = [2, 0, 1][base];
    let second = [1, 2, 0][base];
    let offset = subtract(point, v[base]);
    let toward_first = dot(subtract(v[first], v[base]), offset);
    let toward_second = dot(subtract(v[second], v[base]), offset);
    if toward_first >= 0. {
        v[second] = v[2];
        *count = 2;
        return None;
    }
    v[first] = v[2];
    if toward_second < 0. {
        let slot = if second == 2 { first } else { second };
        v[slot] = v[1];
        *count = 1;
        return Some(subtract(point, v[0]));
    }
    *count = 2;
    None
}

fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
fn tetrahedron(point: [f32; 4], v: &mut [[f32; 4]; 4]) -> bool {
    let d = v[3];
    let offset = subtract(point, d);
    let normals = [
        cross(subtract(v[2], v[1]), subtract(d, v[1])),
        cross(subtract(v[0], v[2]), subtract(d, v[2])),
        cross(subtract(v[1], v[0]), subtract(d, v[0])),
    ];
    // Tetrahedron scoring uses Z+X+Y, unlike the support scan's Y+X+Z.
    let project = |a: [f32; 4], b: [f32; 4]| (a[2] * b[2] + a[0] * b[0]) + a[1] * b[1];
    let projections = normals.map(|n| project(offset, n));
    let scores = std::array::from_fn::<_, 3, _>(|i| {
        let squared = project(normals[i], normals[i]);
        if squared == 0. {
            f32::from_bits(0x7f7f_ffee)
        } else {
            projections[i].abs() * projections[i] * reciprocal(squared)
        }
    });
    let maximum = max(scores[2], max(scores[1], scores[0]));
    let mask = scores
        .iter()
        .enumerate()
        .fold(0, |m, (i, &s)| m | ((maximum <= s) as usize) << i);
    let mut selected = [0, 0, 1, 1, 2, 2, 2, 2][mask];
    if !(maximum >= f32::from_bits(0x3586_37be)) {
        return false;
    }
    {
        let mut other = scores;
        other[selected] = -f32::from_bits(0x7f7f_ffee);
        let next = max(other[2], max(other[1], other[0]));
        if maximum <= next * 1.1 {
            let mut positive = projections
                .iter()
                .enumerate()
                .fold(0, |m, (i, &s)| m | ((0. <= s) as usize) << i);
            for (bit, other, k, a, b) in [(1, 2, 2, 0, 1), (2, 4, 0, 1, 2), (4, 1, 1, 2, 0)] {
                if positive & bit != 0 && positive & other != 0 {
                    let left = dot(subtract(d, v[k]), subtract(v[a], v[k]))
                        * dot(subtract(point, v[k]), subtract(v[b], v[k]));
                    let right = dot(subtract(point, v[k]), subtract(v[a], v[k]))
                        * dot(subtract(d, v[k]), subtract(v[b], v[k]));
                    positive &= !if left <= right { bit } else { other };
                }
            }
            let choice = [-1, 0, 1, 0, 2, 0, 1, 0][positive & 7];
            if choice < 0 {
                return false;
            }
            selected = choice as usize;
        }
    }
    v[selected] = d;
    true
}
