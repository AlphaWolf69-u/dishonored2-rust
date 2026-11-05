//! Prepare world-program particle geometry for startup or runtime publication.
use crate::assets::particle_model::{ParticleModel, ParticleModelVertex};
use crate::rendering::bevy_backend::surface_gpu::ImageRole;
use bevy::prelude::*;

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

/// The worker owns these CPU-side results until the current level accepts
/// them. No world entities are reserved while a request can still fail.
pub(super) struct PreparedSurface {
    pub source: String,
    pub material: String,
    pub mesh: Handle<Mesh>,
    pub handle: Handle<super::ProbeMaterial>,
    pub images: Vec<(usize, ImageRole)>,
}

impl PreparedSurface {
    fn into_bundle(self) -> impl Bundle {
        (
            Mesh3d(self.mesh),
            MeshMaterial3d(self.handle),
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
            Visibility::Visible,
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
            crate::rendering::bevy_backend::particle_meshes::ParticleMeshProxy {
                source: self.source,
                material: self.material,
            },
        )
    }

    /// Publish only after the current level accepts the worker's asset batch.
    /// The entity identity is then tracked by the render readiness owner.
    pub fn publish(self, world: &mut World) -> Entity {
        world.spawn(self.into_bundle()).id()
    }
}

pub(super) fn publish(
    commands: &mut super::setup_commands::SetupCommands,
    mut surface: PreparedSurface,
) -> (super::MaterialHandle, Vec<(usize, ImageRole)>) {
    let binding = super::MaterialHandle::Probe(surface.handle.clone());
    let images = std::mem::take(&mut surface.images);
    commands.spawn(surface.into_bundle());
    (binding, images)
}
