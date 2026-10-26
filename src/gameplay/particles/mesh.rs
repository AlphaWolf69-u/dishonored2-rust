//! Ordered PMODEL emission (1f96c0). Geometry selection uses the live spawn
//! generator; scale, normal velocity and rotation use a copy of the cycle seed.
use super::{parameters::VectorParameter, spawn::rotation_basis};
use crate::{
    assets::particle_model::{ParticleModel, ParticleModelVertex},
    float::{add, divide, multiply, subtract},
    gameplay::effects::normalize_small,
    random::{lcg_step, lcg_unit},
};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MeshSampling {
    Vertex,
    VertexSequential,
    Surface,
    SurfaceCenterSequential,
    SurfaceRandomSequential,
    OriginExtrusion,
    BetweenSurfaces,
}

pub struct SpawnMesh {
    pub source: Option<String>,
    pub model: Option<Arc<ParticleModel>>,
    pub sampling: MeshSampling,
    pub scale: VectorParameter,
    pub rotation: VectorParameter,
    pub rotation_order: usize,
    pub normal_velocity: VectorParameter,
    pub worldspace: bool,
}

impl Default for SpawnMesh {
    /// 1f8cb0 replaces the rotation's unit curve with zero-to-one keys.
    fn default() -> Self {
        let mut rotation = VectorParameter::constant([0.; 3]);
        rotation.curve.maximum = Default::default();
        rotation.curve.maximum.append_key(0., [0.; 3], 4);
        rotation.curve.maximum.append_key(1., [1.; 3], 4);
        Self {
            source: None,
            model: None,
            sampling: MeshSampling::Surface,
            scale: VectorParameter::constant([1.; 3]),
            rotation,
            rotation_order: 0,
            normal_velocity: VectorParameter::constant([0.; 3]),
            worldspace: false,
        }
    }
}

pub struct MeshSample {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub colour: [f32; 4],
}

fn normalize(mut value: [f32; 3]) -> [f32; 3] {
    super::orientation::normalize(&mut value);
    value
}

fn normal(vertex: &ParticleModelVertex) -> [f32; 3] {
    normalize(vertex.packed_normal().map(|v| divide(f32::from(v), 127.)))
}

fn colour(vertex: &ParticleModelVertex) -> [f32; 4] {
    crate::colour::unpack_rgba8(u32::from_le_bytes(vertex.colour()))
}

/// 1f8410/1f73f0/1f6dc0 share the same ordered three-way quad blend.
fn blend<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
    std::array::from_fn(|i| {
        add(
            multiply(subtract(1., t), normalize_small(a[i])),
            multiply(normalize_small(b[i]), t),
        )
    })
}

fn quad(model: &ParticleModel, first: usize, weights: [f32; 3]) -> Result<MeshSample, String> {
    let vertices = model
        .vertices
        .get(first..first + 4)
        .ok_or("particle emission quad exceeds PMODEL vertices")?;
    let [u, v, w] = weights;
    let position = blend(
        blend(vertices[0].position, vertices[1].position, u),
        blend(vertices[3].position, vertices[2].position, w),
        v,
    );
    let velocity = normalize(blend(
        blend(normal(&vertices[0]), normal(&vertices[1]), u),
        blend(normal(&vertices[3]), normal(&vertices[2]), w),
        v,
    ));
    let colour = blend(
        blend(colour(&vertices[0]), colour(&vertices[1]), u),
        blend(colour(&vertices[3]), colour(&vertices[2]), w),
        v,
    );
    Ok(MeshSample {
        position,
        velocity,
        colour,
    })
}

/// 1f66e0 stores only the face-selection RNG advance. Its three weights are
/// derived from a local copy, deliberately leaving the live seed unchanged.
fn random_quad(model: &ParticleModel, seed: &mut u32) -> Result<MeshSample, String> {
    let count = model.vertices.len() / 4;
    let first = if count == 0 {
        0
    } else {
        ((lcg_step(seed) >> 10 & 0x7fff) as usize % count) * 4
    };
    quad(model, first, weights(*seed))
}

fn weights(mut seed: u32) -> [f32; 3] {
    let first = lcg_unit(&mut seed);
    let second = lcg_unit(&mut seed);
    let third = subtract(1., lcg_unit(&mut seed));
    [first, second, third]
}

impl SpawnMesh {
    pub fn sample(
        &self,
        cursor: &mut usize,
        cycle_seed: u32,
        seed: &mut u32,
        fraction: f32,
    ) -> Result<Option<MeshSample>, String> {
        let Some(model) = &self.model else {
            return if self.source.is_none() {
                Ok(None)
            } else {
                Err("particle emission model was not loaded".into())
            };
        };
        // 1f93d0: an empty cache does not contribute spawn geometry or RNG.
        if model.vertices.is_empty() {
            return Ok(None);
        }
        let mut result = match self.sampling {
            MeshSampling::Vertex | MeshSampling::VertexSequential => {
                let index = if self.sampling == MeshSampling::Vertex {
                    (lcg_step(seed) >> 10 & 0x7fff) as usize % model.vertices.len()
                } else {
                    let index = *cursor;
                    *cursor += 1;
                    if *cursor >= model.vertices.len() {
                        *cursor = 0;
                    }
                    index
                };
                let vertex = model
                    .vertices
                    .get(index)
                    .ok_or("particle emission vertex cursor exceeds PMODEL")?;
                MeshSample {
                    position: vertex.position,
                    velocity: normal(vertex),
                    colour: colour(vertex),
                }
            }
            MeshSampling::SurfaceCenterSequential | MeshSampling::SurfaceRandomSequential => {
                let index = *cursor;
                *cursor += 4;
                if *cursor >= model.vertices.len() {
                    *cursor = 0;
                }
                let weights = if self.sampling == MeshSampling::SurfaceCenterSequential {
                    [0.5; 3]
                } else {
                    weights(*seed)
                };
                quad(model, index, weights)?
            }
            MeshSampling::Surface => random_quad(model, seed)?,
            MeshSampling::OriginExtrusion => {
                let mut sample = random_quad(model, seed)?;
                let t = lcg_unit(seed);
                sample.position = sample.position.map(|v| multiply(t, v));
                sample
            }
            MeshSampling::BetweenSurfaces => {
                let first = random_quad(model, seed)?;
                let second = random_quad(model, seed)?;
                let t = lcg_unit(seed);
                MeshSample {
                    position: blend(first.position, second.position, t),
                    velocity: normalize(blend(first.velocity, second.velocity, t)),
                    colour: blend(first.colour, second.colour, t),
                }
            }
        };
        let mut local_seed = cycle_seed;
        let scale = self.scale.sample(fraction, &mut local_seed)?;
        result.position = std::array::from_fn(|i| multiply(scale[i], result.position[i]));
        let speed = self.normal_velocity.sample(fraction, &mut local_seed)?;
        result.velocity = std::array::from_fn(|i| multiply(speed[i], result.velocity[i]));
        let angles = self.rotation.sample(fraction, &mut local_seed)?;
        let basis = rotation_basis(
            angles,
            self.rotation_order,
            crate::gameplay::reference_frame::ReferenceFrame::default().basis,
        )?;
        let rotate = |value: [f32; 3]| {
            std::array::from_fn(|i| {
                add(
                    add(
                        multiply(value[1], basis[1][i]),
                        multiply(value[0], basis[0][i]),
                    ),
                    multiply(value[2], basis[2][i]),
                )
            })
        };
        result.position = rotate(result.position);
        result.velocity = rotate(result.velocity);
        Ok(Some(result))
    }
}
