//! Expanding-polytope penetration search and deterministic retries (c231b0).
use super::{
    distance::ConvexDistanceSolution,
    faces::reciprocal_sqrt_estimate,
    hull::{ExpansionOutcome, PenetrationHull},
    penetration::PenetrationSupport,
    seed::{cross, dot, inverse_length, subtract, PenetrationSeed, SeedOutcome},
};
use crate::gameplay::{
    constraint_solver::reciprocal,
    shape_transform::rotation::{axis_angle_basis, compose_basis},
};

pub struct PenetrationResult {
    pub status: u32,
    pub solution: Option<ConvexDistanceSolution>,
}

fn weights(points: [[f32; 4]; 3]) -> [f32; 3] {
    let [a, b, c] = points;
    let n = cross(subtract(b, a), subtract(c, a));
    let triple = |u: [f32; 4], v: [f32; 4]| {
        ((u[2] * n[0] - u[0] * n[2]) * v[1] + (u[1] * n[2] - u[2] * n[1]) * v[0])
            + (u[0] * n[1] - u[1] * n[0]) * v[2]
    };
    let w = [triple(c, b), triple(a, c), triple(b, a)];
    let sum = (w[1] + w[0]) + w[2];
    let inverse = if sum == 0. { 0. } else { reciprocal(sum) };
    w.map(|w| inverse * w)
}
fn face_points(hull: &PenetrationHull, index: usize) -> [[f32; 4]; 3] {
    hull.faces[index]
        .vertices
        .map(|i| hull.vertices[i].difference)
}

/// Edge-plane score used when the nearest face's barycentric weights are negative.
fn fallback_score(points: [[f32; 4]; 3], normal: [f32; 4]) -> f32 {
    let [a, b, c] = points;
    let e1 = subtract(b, a);
    let e2 = subtract(c, b);
    let n = cross(e1, e2);
    let edges = [cross(e1, n), cross(e2, n), cross(subtract(a, c), n)];
    let score = std::array::from_fn::<_, 3, _>(|i| {
        let r = reciprocal_sqrt_estimate(dot(edges[i], edges[i]));
        let product: [f32; 4] = std::array::from_fn(|j| (r * edges[i][j]) * points[i][j]);
        (product[1] + product[0]) + product[2]
    });
    let min = |a: f32, b: f32| if a < b { a } else { b };
    min(min(score[0], score[1]), min(score[2], 0.)) - dot(a, normal)
}

pub fn solve_penetration(
    seed: &mut PenetrationSeed,
    support: &mut PenetrationSupport<'_>,
    retries: &mut u32,
    epsilon: [f32; 4],
    tolerance: [f32; 4],
) -> Result<PenetrationResult, String> {
    loop {
        match seed.build(support, epsilon[0], tolerance[0])? {
            SeedOutcome::Contact(solution) => {
                return Ok(PenetrationResult {
                    status: 1,
                    solution: Some(solution),
                })
            }
            SeedOutcome::Failed => {}
            SeedOutcome::Ready => {
                let mut hull = PenetrationHull::new(seed.points);
                let (mut nearest, status) = loop {
                    let mut nearest = 0;
                    for i in 1..hull.faces.len() {
                        if hull.faces[i].distance < hull.faces[nearest].distance {
                            nearest = i;
                        }
                    }
                    let direction = hull.faces[nearest].normal;
                    let candidate = hull.vertex_count;
                    hull.vertex_count += 1;
                    hull.vertices.push(support.point(direction)?);
                    match hull.expand(candidate, Some(nearest), direction, tolerance)? {
                        ExpansionOutcome::Stopped(status) => break (nearest, status),
                        ExpansionOutcome::Continue if hull.vertex_count > 49 => break (nearest, 3),
                        ExpansionOutcome::Continue => {}
                    }
                };
                if status != 7 {
                    let mut barycentric = weights(face_points(&hull, nearest));
                    if barycentric.iter().any(|&v| v < 0.) {
                        let mut maximum = -f32::from_bits(0x7f7f_ffee);
                        for i in 0..hull.faces.len() {
                            let score = fallback_score(face_points(&hull, i), hull.faces[i].normal);
                            if maximum < score {
                                nearest = i;
                                maximum = score;
                            }
                        }
                        barycentric = weights(face_points(&hull, nearest));
                    }
                    let face = &hull.faces[nearest];
                    let packets = face.vertices.map(|i| hull.vertices[i]);
                    let witness = std::array::from_fn(|i| {
                        (barycentric[1] * packets[1].query[i]
                            + barycentric[0] * packets[0].query[i])
                            + barycentric[2] * packets[2].query[i]
                    });
                    for i in 0..3 {
                        seed.points[i].query = packets[i].query;
                        seed.points[i].target = packets[i].target;
                    }
                    seed.count = 3;
                    let solution = ConvexDistanceSolution {
                        status: status as i32,
                        distance: [0. - face.distance; 4],
                        normal: face
                            .normal
                            .map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000)),
                        witness,
                    };
                    return Ok(PenetrationResult {
                        status,
                        solution: Some(solution),
                    });
                }
            }
        }
        *retries = retries.wrapping_add(1);
        if *retries == 1 {
            seed.count = 1;
            continue;
        }
        if (*retries as i32) >= 20 {
            return Ok(PenetrationResult {
                status: 7,
                solution: None,
            });
        }
        seed.count = 0;
        let random =
            (0x3039_u32.wrapping_sub(retries.wrapping_mul(0x3e39_b193)) & 0x7fff_ffff) as f32;
        let unit = random * f32::from_bits(0x3000_0000);
        let displacement = (unit - f32::from_bits(0x38d1_b717)) * f32::from_bits(0x3951_b717);
        for axis in 0..4 {
            support.relative[3][axis] += if axis == 3 { 0. } else { displacement };
        }
        let component = (unit - 1.) * 2.;
        let axis = [component, component, component, 0.];
        let squared = dot(axis, axis);
        let axis = if squared < f32::from_bits(0x2880_0000) {
            [1., 0., 0., 0.]
        } else {
            let factor = inverse_length(squared);
            axis.map(|v| factor * v)
        };
        let basis = axis_angle_basis(axis, unit * f32::from_bits(0x38d1_b717));
        compose_basis(&mut support.relative, basis);
    }
}
