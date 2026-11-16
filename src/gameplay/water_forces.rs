//! Fluid velocity changes from 4d5d50, over the shape owner's generated key-1
//! geometry. This kernel borrows the real body/motion data and retains only
//! reusable distance scratch; it acquires no resources and publishes no body.
use super::{
    aerodynamics::{self, MotionMass},
    constraint_solver::{inverse_length, reciprocal},
    shape_transform::ShapeMatrix,
};
use std::sync::Arc;

/// 08b830's generated arkPhysicsShapeProperty, shared by wind and water.
/// Face normals/areas live with the aerodynamic owner; this kernel reads the
/// original face's three vertex indices (+20/+24/+28) only.
#[derive(Debug)]
pub struct Geometry {
    pub vertices: Arc<[[f32; 4]]>,
    pub triangles: Arc<[[u32; 3]]>,
    pub volume: [f32; 4],
}
pub struct Input<'a> {
    pub geometry: &'a Geometry,
    pub world_plane: [f32; 4],
    /// Actual hknpBody columns, including its cached local COM in the W lanes.
    pub body_frame: &'a ShapeMatrix,
    /// 651a40: authored density override, or actual body mass / key-1 volume.
    pub body_density: [f32; 4],
    pub water_flow_and_density: [f32; 4],
    /// World body-mass sidecar +50, not inverse motion mass.
    pub body_mass: f32,
    pub gravity: [f32; 4],
    pub motion: &'a MotionMass,
    /// PhysicsBase collision-userdata +40/+50, in world space.
    pub linear: [f32; 4],
    pub angular: [f32; 4],
    pub delta: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct Change {
    pub linear: [f32; 4],
    pub angular: [f32; 4],
    pub submerged_center: [f32; 4],
}
#[derive(Default)]
pub struct Scratch {
    // Host: each native distance vector contains the same scalar four times.
    // Keeping one lane preserves every operation while removing duplication.
    distances: Vec<f32>,
}
fn dot3(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn cross(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
        a[3] * b[3] - a[3] * b[3],
    ]
}
fn combine(c: &ShapeMatrix, v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| (v[2] * c[2][i] + v[1] * c[1][i]) + v[0] * c[0][i])
}
fn minimum(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
fn maximum(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
impl Scratch {
    /// Source positive-volume result is None when no submerged tetrahedra
    /// exist. Malformed geometry is an asset error, never a substitute force.
    pub fn calculate(&mut self, input: &Input<'_>) -> Result<Option<Change>, String> {
        let c = input.body_frame;
        let n = input.world_plane;
        let plane = [
            dot3(n, c[0]),
            dot3(n, c[1]),
            dot3(n, c[2]),
            (c[3][2] * n[2] + c[3][1] * n[1]) + c[3][0] * n[0] + n[3],
        ];
        let center = [c[0][3], c[1][3], c[2][3], c[2][3]];
        // Source chooses an integration origin on the water plane, closest
        // to the body's cached COM. Its fourth lane is deliberately preserved.
        let projected =
            (center[0] * plane[0] + center[2] * plane[2]) + (center[1] * plane[1] + plane[3]);
        let origin: [f32; 4] = std::array::from_fn(|i| center[i] + plane[i] * (0. - projected));
        self.distances.clear();
        self.distances
            .try_reserve(input.geometry.vertices.len())
            .map_err(|e| format!("fluid distance scratch: {e}"))?;
        let mut minimum_depth = 0.;
        for vertex in input.geometry.vertices.iter() {
            let distance = (plane[0] * vertex[0] + plane[2] * vertex[2])
                + (plane[1] * vertex[1] + plane[3] * vertex[3]);
            self.distances.push(distance);
            minimum_depth = minimum(minimum_depth, distance);
        }
        let mut six_volume = 0.;
        let mut centroid_sum = [0.; 4];
        for triangle in input.geometry.triangles.iter() {
            let mut vertices = [[0.; 4]; 3];
            let mut distances = [0.; 3];
            for i in 0..3 {
                let index = triangle[i] as usize;
                vertices[i] = *input
                    .geometry
                    .vertices
                    .get(index)
                    .ok_or("fluid triangle vertex is outside its source geometry")?;
                distances[i] = *self
                    .distances
                    .get(index)
                    .ok_or("fluid distance vertex is outside its source geometry")?;
            }
            if !(distances[0] < 0. || distances[1] < 0. || distances[2] < 0.) {
                continue;
            }
            let mut clipped = [[0.; 4]; 6];
            let mut count = 0;
            for current in 0..3 {
                let previous = (current + 2) % 3;
                let before = distances[previous];
                let after = distances[current];
                //1423cce30 is f32::EPSILON, read from the installed executable.
                if before * after + f32::from_bits(0x3400_0000) < 0. {
                    let factor = before * reciprocal(before - after);
                    clipped[count] = std::array::from_fn(|lane| {
                        factor * (vertices[current][lane] - vertices[previous][lane])
                            + vertices[previous][lane]
                            - origin[lane]
                    });
                    count += 1;
                }
                if after < 0. {
                    clipped[count] =
                        std::array::from_fn(|lane| vertices[current][lane] - origin[lane]);
                    count += 1;
                }
            }
            if count < 3 {
                continue;
            }
            let a = clipped[0];
            for i in 1..count - 1 {
                let b = clipped[i];
                let next = clipped[i + 1];
                let x = next[0] * (b[2] * a[1] - a[2] * b[1]);
                let y = next[1] * (b[0] * a[2] - a[0] * b[2]);
                let z = next[2] * (b[1] * a[0] - a[1] * b[0]);
                let determinant = z + (y + x);
                six_volume = (six_volume + z) + (y + x);
                for lane in 0..4 {
                    centroid_sum[lane] += determinant * ((b[lane] + a[lane]) + next[lane]);
                }
            }
        }
        if !(six_volume > 0.) {
            return Ok(None);
        }
        //1423ccd40=.25;1423ccd60=1/6. Keep source operation ordering.
        let inverse_volume = reciprocal(six_volume);
        let submerged: [f32; 4] =
            std::array::from_fn(|i| inverse_volume * 0.25 * centroid_sum[i] + origin[i]);
        let geometry_inverse = input.geometry.volume.map(reciprocal);
        let density_inverse = input.body_density.map(reciprocal);
        let gravity_local = [
            (input.gravity[2] * c[0][2] + input.gravity[1] * c[0][1]) + input.gravity[0] * c[0][0],
            (input.gravity[2] * c[1][2] + input.gravity[1] * c[1][1]) + input.gravity[0] * c[1][0],
            (input.gravity[2] * c[2][2] + input.gravity[1] * c[2][1]) + input.gravity[0] * c[2][0],
            (input.gravity[2] * c[2][3] + input.gravity[1] * c[2][3]) + input.gravity[0] * c[2][1],
        ];
        let local_force: [f32; 4] = std::array::from_fn(|i| {
            (((((0. - input.water_flow_and_density[3]) * input.body_mass * geometry_inverse[i])
                * density_inverse[i])
                * six_volume)
                * f32::from_bits(0x3e2a_aaab))
                * gravity_local[i]
        });
        let local_offset = std::array::from_fn(|i| submerged[i] - center[i]);
        let local_torque = cross(local_offset, local_force);
        let force = combine(c, local_force);
        let torque = combine(c, local_torque);
        let (mut linear_delta, mut angular_delta) =
            aerodynamics::velocity_change(input.motion, force, torque, input.delta);
        let world_center: [f32; 4] = std::array::from_fn(|i| {
            ((c[3][i] + submerged[1] * c[1][i]) + submerged[0] * c[0][i]) + submerged[2] * c[2][i]
        });
        let world_offset = std::array::from_fn(|i| world_center[i] - input.motion.center[i]);
        let old_cross = cross(input.angular, world_offset);
        let old_point: [f32; 4] = std::array::from_fn(|i| old_cross[i] + input.linear[i]);
        let mut next_linear: [f32; 4] = std::array::from_fn(|i| linear_delta[i] + input.linear[i]);
        let mut next_angular: [f32; 4] =
            std::array::from_fn(|i| angular_delta[i] + input.angular[i]);
        let next_cross = cross(next_angular, world_offset);
        let next_point: [f32; 4] = std::array::from_fn(|i| next_cross[i] + next_linear[i]);
        let next_normal = (n[2] * next_point[2] + n[1] * next_point[1]) + n[0] * next_point[0];
        let speed_limit = (0. - minimum_depth) * reciprocal(input.delta);
        if speed_limit < next_normal {
            let target: [f32; 4] = std::array::from_fn(|i| {
                n[i] * (speed_limit * reciprocal(next_normal) * next_point[i] - old_point[i])
            });
            let correction = maximum((target[2] + target[1]) + target[0], 0.);
            let squared = linear_delta[2] * linear_delta[2]
                + (linear_delta[1] * linear_delta[1] + linear_delta[0] * linear_delta[0]);
            let scale = inverse_length(squared) * correction;
            for i in 0..4 {
                linear_delta[i] *= scale;
                angular_delta[i] *= scale;
                next_linear[i] = linear_delta[i] + input.linear[i];
                next_angular[i] = angular_delta[i] + input.angular[i];
            }
        }
        let mut change = Change {
            linear: [0.; 4],
            angular: [0.; 4],
            submerged_center: world_center,
        };
        for i in 0..4 {
            let ratio = maximum(input.water_flow_and_density[3] * density_inverse[i], 1.);
            let denominator =
                geometry_inverse[i] * six_volume * f32::from_bits(0x3e2a_aaab) * ratio * 0.125 + 1.;
            let keep = reciprocal(denominator);
            change.linear[i] = (keep * next_linear[i]
                + (1. - keep) * input.water_flow_and_density[i])
                - input.linear[i];
            change.angular[i] = keep * next_angular[i] - input.angular[i];
        }
        Ok(Some(change))
    }
}
