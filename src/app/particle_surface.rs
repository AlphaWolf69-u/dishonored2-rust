//! Prepare world-program particle geometry for startup or runtime publication.
use bevy::prelude::Mesh;
use crate::assets::particle_model::{ParticleModel, ParticleModelVertex};

/// 1402320b0 uses the shared quad indices over 140232480's PMODEL vertices.
/// This CPU-only conversion can run before the main-world asset publication.
pub(super) fn prepare_mesh(model: &ParticleModel) -> Result<Mesh, &'static str> {
    if model.vertices.is_empty() || model.vertices.len() % 4 != 0 || model.vertices.len() > 65536 {
        return Err("must contain complete u16-indexed quads");
    }
    let positions: Vec<[f32; 3]> = model.vertices.iter().map(|v| v.position).collect();
    let half = |v: &ParticleModelVertex, at: usize| {
        half::f16::from_bits(u16::from_le_bytes([v.attributes[at], v.attributes[at + 1]])).to_f32()
    };
    let uvs: Vec<[f32; 2]> = model.vertices.iter().map(|v| [half(v, 0), half(v, 2)]).collect();
    let packed: Vec<[u8; 16]> = model.vertices.iter()
        .map(|v| v.attributes[4..20].try_into().unwrap()).collect();
    let indices: Vec<u16> = (0..model.vertices.len()).step_by(4)
        .flat_map(|base| [0, 2, 3, 0, 3, 1].map(|i| (base + i) as u16)).collect();
    Ok(super::view_geometry::source_mesh(&positions, &uvs, &indices, &packed, false))
}
