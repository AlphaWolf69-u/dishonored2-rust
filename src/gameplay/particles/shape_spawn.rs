//! m_physicsShapesSpawn: spawning on the owner entity's physics shapes.
//! arkCpntFX (1404b1d80) gives an FX record whose declaration starts a
//! system with this spawn (FX +0x68, 1401c5c10) the list of its owner
//! entity's hknp bodies (14067a340: those of the physics component whose
//! shape is a convex polytope, shape flag 2); 140227b90 copies it into the
//! system (+0x1d78) and 140679ed0 drops removed bodies every tick. Kiscule
//! FX nodes and FX entities never build one, and without a list the stage
//! spawns as usual.
use crate::assets::physworld::polytope::ConvexPolytope;
use crate::float::{add, multiply, subtract};
use crate::gameplay::shape_transform::ShapeMatrix;
use std::sync::Arc;

/// The owner's bodies in physics-component order: shape and hknpBody
/// transform.
#[derive(Clone, Default)]
pub struct OwnerShapes {
    pub bodies: Vec<(Arc<ConvexPolytope>, ShapeMatrix)>,
}

fn draw(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(0x19660d).wrapping_add(0x3c6e_f35f);
    (*seed >> 10) & 0x7fff
}

fn pick(count: u32, seed: &mut u32) -> u32 {
    if count == 0 { 0 } else { draw(seed) % count }
}

/// 14067a050: a random face, a point on the segment between points on two
/// of its edges, and the face plane (shape space).
pub fn face_point(shape: &ConvexPolytope, seed: &mut u32) -> Result<([f32; 4], [f32; 4]), String> {
    let unit = f32::from_bits(0x3800_0100);
    let face_index = pick(shape.faces.len() as u32, seed) as usize;
    let normal = *shape.planes.get(face_index).ok_or("polytope face without a plane")?;
    let face = shape.faces.get(face_index).ok_or("polytope without faces")?;
    let count = u32::from(face.num_indices);
    let first = usize::from(face.first_index);
    let corner = |k: u32| -> Result<[f32; 4], String> {
        let index = *shape.indices.get(first + k as usize).ok_or("polytope face index out of range")?;
        shape.vertices.get(usize::from(index)).copied().ok_or_else(|| "polytope vertex out of range".into())
    };
    let a = pick(count, seed);
    let (a0, a1) = (corner(a)?, corner((a + 1) % count)?);
    let mut b = pick(count, seed);
    if b == a {
        b = count - a - 1;
    }
    let (b0, b1) = (corner(b)?, corner((b + 1) % count)?);
    let t1 = multiply(draw(seed) as f32, unit);
    let t2 = multiply(draw(seed) as f32, unit);
    let t3 = multiply(draw(seed) as f32, unit);
    let p: [f32; 4] = std::array::from_fn(|i| add(multiply(t1, subtract(a1[i], a0[i])), a0[i]));
    let q: [f32; 4] = std::array::from_fn(|i| add(multiply(t2, subtract(b1[i], b0[i])), b0[i]));
    let point = std::array::from_fn(|i| add(multiply(t3, subtract(q[i], p[i])), p[i]));
    Ok((point, normal))
}

/// 140679fe0 -> 14067a270: a random body's face point and normal in world
/// axes.
pub fn sample(owner: &OwnerShapes, seed: &mut u32) -> Result<([f32; 3], [f32; 3]), String> {
    let index = pick(owner.bodies.len() as u32, seed) as usize;
    let (shape, x) = owner.bodies.get(index).ok_or("owner physics shape list is empty")?;
    let (p, n) = face_point(shape, seed)?;
    let point = std::array::from_fn(|i| add(add(add(multiply(p[1], x[1][i]), multiply(p[0], x[0][i])), multiply(p[2], x[2][i])), x[3][i]));
    let normal = std::array::from_fn(|i| add(add(multiply(n[1], x[1][i]), multiply(n[0], x[0][i])), multiply(n[2], x[2][i])));
    Ok((point, normal))
}

/// 1401dce70: the point moved along its normal by m_normalOffset, and the
/// normal times m_normalToVelocity.
pub fn spawn(
    spawn: &crate::assets::fx::particle_stage::PhysicsShapesSpawn,
    owner: &OwnerShapes,
    fraction: f32,
    seed: &mut u32,
) -> Result<([f32; 3], [f32; 3]), String> {
    let (point, normal) = sample(owner, seed)?;
    let offset = spawn.normal_offset.sample(fraction, seed)?;
    let position = std::array::from_fn(|i| add(point[i], multiply(normal[i], offset)));
    let speed = spawn.normal_to_velocity.sample(fraction, seed)?;
    Ok((position, normal.map(|n| multiply(n, speed))))
}

/// Runs tests/fixtures/polytope_face_points.txt (14067a050 executed by
/// analysis/probe_polytope_face_points.py); returns (cases, mismatches).
#[doc(hidden)]
pub fn compare_face_point_fixture(text: &str) -> (usize, Vec<String>) {
    use crate::assets::physworld::polytope::PolytopeFace;
    let mut failures = Vec::new();
    let mut cases = 0;
    for line in text.lines().filter(|line| !line.starts_with('#') && !line.is_empty()) {
        let w: Vec<u32> = line.split_whitespace().map(|word| u32::from_str_radix(word, 16).unwrap()).collect();
        let (vertices, faces, indices) = (w[0] as usize, w[1] as usize, w[2] as usize);
        let mut at = 3;
        let mut take = |n: usize| { let s = w[at..at + n].to_vec(); at += n; s };
        let v = take(vertices * 4);
        let pl = take(faces * 4);
        let fc = take(faces);
        let ix = take(indices);
        let seed_in = take(1)[0];
        let expected = take(9);
        let shape = ConvexPolytope {
            vertices: v.chunks(4).map(|c| std::array::from_fn(|i| f32::from_bits(c[i]))).collect(),
            planes: pl.chunks(4).map(|c| std::array::from_fn(|i| f32::from_bits(c[i]))).collect(),
            faces: fc.iter().map(|&f| PolytopeFace { first_index: (f & 0xffff) as u16, num_indices: (f >> 16) as u8, min_half_angle: 0 }).collect(),
            indices: ix.iter().map(|&i| i as u8).collect(),
            radius: 0.,
            connectivity: None,
        };
        let mut seed = seed_in;
        cases += 1;
        match face_point(&shape, &mut seed) {
            Ok((p, n)) => {
                let got: Vec<u32> = p.iter().chain(&n).map(|x| x.to_bits()).chain([seed]).collect();
                if got != expected {
                    failures.push(format!("case {cases}: got {got:08x?} want {expected:08x?}"));
                }
            }
            Err(error) => failures.push(format!("case {cases}: {error}")),
        }
    }
    (cases, failures)
}

#[cfg(test)]
mod tests {
    #[test]
    fn face_points_match_executed_original() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/polytope_face_points.txt")).unwrap();
        let (cases, failures) = super::compare_face_point_fixture(&text);
        assert!(cases > 0 && failures.is_empty(), "{} of {cases}: {:?}", failures.len(), &failures[..failures.len().min(3)]);
    }
}
