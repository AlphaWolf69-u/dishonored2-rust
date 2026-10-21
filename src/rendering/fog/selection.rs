//! 324be0's per-object alpha-fog selection and 0fc020's signed line/AABB query.
pub type Bounds = [[f32; 3]; 2];

#[derive(Clone, Copy, Debug)]
pub struct LineIntersection {
    pub hit: bool,
    pub interval: [f32; 2],
}

pub fn intersect_line(bounds: Bounds, origin: [f32; 3], direction: [f32; 3]) -> LineIntersection {
    let limit = f32::from_bits(0x7149f2ca);
    let miss = LineIntersection {
        hit: false,
        interval: [-limit, limit],
    };
    let mut first = -limit;
    let mut last = limit;
    for axis in 0..3 {
        let d = direction[axis];
        if f32::from_bits(d.to_bits() & 0x7fff_ffff) <= f32::from_bits(0x00800000) {
            if origin[axis] < bounds[0][axis]
                || !(origin[axis] < bounds[1][axis] || origin[axis] == bounds[1][axis])
            {
                return miss;
            }
        } else {
            let inverse = 1. / d;
            let mut low = (bounds[0][axis] - origin[axis]) * inverse;
            let mut high = (bounds[1][axis] - origin[axis]) * inverse;
            if high < low {
                std::mem::swap(&mut low, &mut high);
            }
            if first < low {
                first = low;
            }
            if high < last {
                last = high;
            }
            if !(first < last || first == last) {
                return miss;
            }
        }
    }
    LineIntersection {
        hit: true,
        interval: [first, last],
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SelectedVolume {
    pub index: usize,
    /// Entry, exit and object-centre depths in the game camera's view basis.
    pub depths: [f32; 3],
}

/// Bounds arrive in the same far-to-near order as opaque area fog rendering.
/// Intersecting the receiver's AABB wins immediately; otherwise the nearest
/// ray entry to its centre wins, including intersections behind the camera.
pub fn select(
    bounds: &[Bounds],
    eye: [f32; 3],
    center: [f32; 3],
    receiver: Bounds,
    view_z: [f32; 4],
) -> Option<SelectedVolume> {
    let delta = std::array::from_fn::<_, 3, _>(|i| center[i] - eye[i]);
    let length_squared = delta[1] * delta[1] + delta[0] * delta[0] + delta[2] * delta[2];
    let minimum = f32::from_bits(0x00800000);
    let length_squared = if length_squared <= minimum {
        minimum
    } else {
        length_squared
    };
    let inverse = 1. / length_squared.sqrt();
    let direction = delta.map(|v| v * inverse);
    let mut best = f32::MAX;
    let mut selected = None;
    for (index, &volume) in bounds.iter().enumerate() {
        let overlap =
            (0..3).all(|i| receiver[0][i] <= volume[1][i] && volume[0][i] <= receiver[1][i]);
        // The scalar fallback uses COMISS/JA: unordered comparisons are
        // accepted here, unlike the SIMD overlap test above. Do not fold it
        // into the usual ordered AABB predicate.
        let contains = !(receiver[0][0] > receiver[1][0])
            && (0..3).all(|i| !(volume[0][i] > receiver[0][i]) && !(receiver[1][i] > volume[1][i]));
        if overlap || contains {
            selected = Some(index);
            break;
        }
        let interval = intersect_line(volume, eye, direction);
        if interval.hit {
            let difference = std::array::from_fn::<_, 3, _>(|i| {
                direction[i] * interval.interval[0] + eye[i] - center[i]
            });
            let distance = difference[1] * difference[1]
                + difference[0] * difference[0]
                + difference[2] * difference[2];
            if distance < best {
                best = distance;
                selected = Some(index);
            }
        }
    }
    let index = selected?;
    // The source consumes the initialized interval even if this second query
    // fails after the immediate overlap/containment branch.
    let interval = intersect_line(bounds[index], eye, direction).interval;
    let depth = |point: [f32; 3]| {
        -((view_z[0] * point[0] + view_z[1] * point[1]) + view_z[2] * point[2] + view_z[3])
    };
    Some(SelectedVolume {
        index,
        depths: [
            depth(std::array::from_fn(|i| direction[i] * interval[0] + eye[i])),
            depth(std::array::from_fn(|i| direction[i] * interval[1] + eye[i])),
            depth(center),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words(line: &str) -> Vec<u32> {
        line.split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect()
    }
    fn close(actual: f32, expected: f32) {
        assert!(
            actual == expected
                || actual.is_nan() && expected.is_nan()
                || (actual - expected).abs() < 0.00003 * expected.abs().max(1.),
            "{actual} != {expected}"
        );
    }
    #[test]
    fn signed_line_bounds_match_installed_function() {
        for line in include_str!("../../../tests/fixtures/fog_ray_bounds.txt").lines() {
            let w = words(line);
            assert_eq!(w.len(), 15);
            let f: Vec<_> = w.iter().copied().map(f32::from_bits).collect();
            let actual = intersect_line(
                [f[0..3].try_into().unwrap(), f[3..6].try_into().unwrap()],
                f[6..9].try_into().unwrap(),
                f[9..12].try_into().unwrap(),
            );
            assert_eq!(actual.hit, w[12] != 0);
            close(actual.interval[0], f[13]);
            close(actual.interval[1], f[14]);
        }
    }
    #[test]
    fn alpha_volume_selection_matches_installed_function() {
        for (case, line) in include_str!("../../../tests/fixtures/fog_volume_selection.txt")
            .lines()
            .enumerate()
        {
            let w = words(line);
            let count = w[0] as usize;
            assert_eq!(w.len(), 24 + count * 6);
            let f: Vec<_> = w.iter().copied().map(f32::from_bits).collect();
            let bounds: Vec<Bounds> = f[17..17 + count * 6]
                .chunks_exact(6)
                .map(|v| [v[0..3].try_into().unwrap(), v[3..6].try_into().unwrap()])
                .collect();
            let actual = select(
                &bounds,
                f[1..4].try_into().unwrap(),
                f[4..7].try_into().unwrap(),
                [f[7..10].try_into().unwrap(), f[10..13].try_into().unwrap()],
                f[13..17].try_into().unwrap(),
            );
            let start = 17 + count * 6;
            let expected = w[start] as i32;
            assert_eq!(
                actual.map(|v| v.index as i32).unwrap_or(-1),
                expected,
                "selection case{case}"
            );
            if let Some(actual) = actual {
                for (a, b) in actual
                    .depths
                    .into_iter()
                    .zip(f[start + 1..start + 4].iter().copied())
                {
                    close(a, b);
                }
            }
            let types = actual
                .map(|v| [1, (v.index as u32 % 4) & 1, (v.index as u32 % 4) >> 1])
                .unwrap_or([0; 3]);
            assert_eq!(types, w[start + 4..start + 7], "type case{case}");
        }
    }
}
