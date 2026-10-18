//! Native CPU Voronoi selection: 382d90,383d80,383750,382f90.
use super::ProbeLightingSet;
#[derive(Clone, Debug)]
pub struct CellGeometry {
    pub center: [f32; 3],
    pub quads: Vec<Quad>,
}
#[derive(Clone, Debug)]
pub struct Quad {
    position: [f32; 3],
    normal: [f32; 3],
    u: [f32; 3],
    v: [f32; 3],
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn geometry(
    center: [f32; 3],
    samples: &[crate::bprobe::Sample],
    indices: &[usize],
) -> Result<CellGeometry, String> {
    let mut quads = Vec::new();
    for &i in indices {
        let sample = samples.get(i).ok_or("Voronoi sample index outside probe")?;
        if sample.values[0] < 0.001 {
            continue;
        }
        let normal = sample.direction;
        let t = cross(sub(center, sample.position), normal);
        let l2 = dot(t, t);
        let (t, b) = if l2 <= 1e-6 {
            // FUN_140111840's pole-safe orthogonal basis.
            if normal[2] < -0.9999998807907104 {
                ([0., -1., 0.], [-1., 0., 0.])
            } else {
                let a = 1. / (normal[2] + 1.);
                let xy = -normal[0] * normal[1] * a;
                (
                    [1. - normal[0] * normal[0] * a, xy, -normal[0]],
                    [xy, 1. - normal[1] * normal[1] * a, -normal[1]],
                )
            }
        } else {
            let t = t.map(|v| v / l2.sqrt());
            (t, cross(normal, t))
        };
        // Native r_ILBuild_VoronoiSampleRadiusGrowFactor defaults to1.5.
        quads.push(Quad {
            position: sample.position,
            normal,
            u: t.map(|v| v / (1.5 * sample.values[0])),
            v: b.map(|v| v / (1.5 * sample.values[1])),
        });
    }
    Ok(CellGeometry { center, quads })
}
fn classify(cell: &crate::pnetwork::Cell, g: &CellGeometry, p: [f32; 3]) -> (usize, f32) {
    let mut inside = true;
    let mut score = f32::MAX;
    let mut sum = 0.;
    let mut count = 0;
    for &(origin, normal, skip) in &cell.planes {
        if skip {
            continue;
        }
        let d = dot(sub(p, origin), normal);
        if d >= -0.0 {
            sum += d;
            count += 1;
        } else {
            score = score.min(-d);
            inside = false;
        }
    }
    if !inside {
        return (2, score);
    }
    let delta = sub(p, g.center);
    let distance = dot(delta, delta).sqrt();
    let mut hit = -1.;
    if distance >= 1e-6 {
        let ray = delta.map(|v| v / distance);
        for q in &g.quads {
            let denom = dot(q.normal, ray);
            if denom >= -1e-6 {
                continue;
            }
            let t = dot(q.normal, sub(q.position, g.center)) / denom;
            let d = std::array::from_fn(|a| g.center[a] + ray[a] * t - q.position[a]);
            if dot(d, q.u).abs() < 1. && dot(d, q.v).abs() < 1. {
                hit = f32::max(hit, t);
                if distance < t {
                    sum += t - distance;
                    count += 1;
                }
            }
        }
    }
    if hit > 0. && hit < distance {
        return (1, distance - hit);
    }
    (0, -sum / if count > 1 { count as f32 } else { 1. })
}
fn tetra_distance(p: [f32; 3], v: [[f32; 3]; 4]) -> f32 {
    // FUN_140382130's four oriented unit face normals.
    let faces = [(0, 1, 3), (1, 0, 2), (2, 3, 1), (3, 2, 0)];
    let mut d = -f32::MAX;
    for (a, b, c) in faces {
        let n = cross(sub(v[b], v[a]), sub(v[c], v[a]));
        let n = n.map(|x| x / dot(n, n).max(f32::MIN_POSITIVE).sqrt());
        d = d.max(dot(sub(p, v[a]), n));
    }
    d
}
pub fn select(set: &ProbeLightingSet, p: [f32; 3]) -> Option<usize> {
    let mut nearest = [(f32::MAX, usize::MAX); 4];
    for (i, g) in set.cell_geometry.iter().enumerate() {
        let delta = sub(p, g.center);
        let d = dot(delta, delta);
        if d < nearest[3].0 {
            nearest[3] = (d, i);
            for k in (1..4).rev() {
                if nearest[k].0 < nearest[k - 1].0 {
                    nearest.swap(k, k - 1);
                }
            }
        }
    }
    let mut best = [(f32::MAX, usize::MAX); 3];
    for (_, i) in nearest {
        if i == usize::MAX {
            continue;
        }
        let (kind, score) = classify(&set.spatial.cells[i], &set.cell_geometry[i], p);
        if score < best[kind].0 {
            best[kind] = (score, i);
        }
    }
    let cell = best.into_iter().find(|v| v.1 != usize::MAX)?.1;
    let cell = &set.spatial.cells[cell];
    if cell.tetrahedron_count == 0 {
        return None;
    }
    let mut chosen = cell.first_tetrahedron;
    let mut score = f32::MAX;
    // The first tetrahedron is the cell's fallback; regular tetrahedra start+1.
    for i in 1..cell.tetrahedron_count {
        let index = cell.first_tetrahedron + i;
        let d = tetra_distance(p, set.spatial.runtime_tetrahedra[index].positions);
        if d < score {
            score = d;
            chosen = index;
            if d < 0. {
                break;
            }
        }
    }
    Some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visibility_aware_cell_ranking_matches_original_executable() {
        for (i, line) in include_str!("query_cases.txt").lines().enumerate() {
            let mut fields = line.split_whitespace();
            let expected_kind = fields.next().unwrap().parse::<usize>().unwrap();
            let expected = f32::from_bits(u32::from_str_radix(fields.next().unwrap(), 16).unwrap());
            let planes = (0..6)
                .map(|j| {
                    let mut p = [0.; 3];
                    let mut n = [0.; 3];
                    let sign = if j % 2 == 0 { 1. } else { -1. };
                    p[j / 2] = -2. * sign;
                    n[j / 2] = sign;
                    (p, n, i & 8 != 0 && j == 5)
                })
                .collect();
            let cell = crate::pnetwork::Cell {
                probe_name: String::new(),
                first_tetrahedron: 0,
                tetrahedron_count: 0,
                planes,
                occluder_samples: vec![],
            };
            let g = CellGeometry {
                center: [0.; 3],
                quads: if i & 1 != 0 {
                    vec![Quad {
                        position: [0.5, 0., 0.],
                        normal: [-1., 0., 0.],
                        u: [0., 1., 0.],
                        v: [0., 0., 1.],
                    }]
                } else {
                    vec![]
                },
            };
            let p = [
                (i % 8) as f32 * 0.75 - 2.625,
                ((i / 8) % 4) as f32 * 0.5 - 0.75,
                (i / 32) as f32 * 0.5 - 0.25,
            ];
            let (kind, score) = classify(&cell, &g, p);
            assert_eq!(kind, expected_kind, "case{i}");
            assert!(
                (score - expected).abs() < 1e-6,
                "case{i}: {score} != {expected}"
            );
        }
    }
}
