//! Prepare world-program particle geometry for startup or runtime publication.
use crate::assets::particle_model::{ParticleModel, ParticleModelVertex};
use crate::rendering::bevy_backend::surface_gpu::ImageRole;
use bevy::prelude::*;
use std::sync::Arc;

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

/// A shared sprite grid (1dc5b0) as a surface: its positions and half UVs
/// with idDrawVert::Clear's remaining attributes (14000b0e0: zero second UV,
/// normal bytes 00 00 00 55, zero tangent, white colour) and its indices.
pub(super) fn prepare_sprite_grid(level: u32) -> Mesh {
    let grid = crate::rendering::particles::geometry::SpriteGrid::new(level);
    let positions: Vec<[f32; 3]> = grid.vertices.iter().map(|v| v.position).collect();
    let uvs: Vec<[f32; 2]> = grid.vertices.iter()
        .map(|v| v.uv.map(|x| half::f16::from_bits(crate::float::pack_half_truncated(x)).to_f32())).collect();
    let packed = vec![[0u8, 0, 0, 0, 0, 0, 0, 0x55, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]; positions.len()];
    super::view_geometry::source_mesh(&positions, &uvs, &grid.indices, &packed, false)
}

/// The worker owns these CPU-side results until the current level accepts
/// them. No world entities are reserved while a request can still fail.
pub(super) struct PreparedSurface {
    pub source: String,
    pub material: String,
    pub mesh: Handle<Mesh>,
    pub handle: Handle<super::ProbeMaterial>,
    pub images: Vec<(usize, ImageRole)>,
    pub alpha: Option<Arc<crate::rendering::bevy_backend::alpha_depth_scene::AlphaSurface>>,
    /// The resolved material owning its shadow programs, when the material
    /// admits shadow casting (caster admission bit 0).
    pub shadow: Option<super::MaterialHandle>,
    /// Game-local bounds of the surface geometry.
    pub bounds: [[f32; 3]; 2],
}

/// The shadow-casting identity of a particle surface proxy: its resolved
/// material (None for a material that casts no shadow) and local bounds.
#[derive(Component, Clone)]
pub(super) struct ParticleSurfaceShadow {
    pub material: Option<super::MaterialHandle>,
    pub bounds: [[f32; 3]; 2],
}

#[derive(Component)]
pub(super) struct ParticleSurfaceAlpha(pub Arc<crate::rendering::bevy_backend::alpha_depth_scene::AlphaSurface>);

pub(super) type AlphaRoutes = std::collections::HashMap<AssetId<super::ProbeMaterial>,
    (Arc<crate::rendering::bevy_backend::alpha_depth_scene::AlphaSurface>, Vec4)>;

/// Game-local positions of a sprite grid (Some(level)) or a PMODEL surface,
/// in the order the prepared mesh stores its vertices.
pub(super) fn local_positions(grid: Option<u32>, model: &ParticleModel) -> Vec<[f32; 3]> {
    match grid {
        Some(level) => crate::rendering::particles::geometry::SpriteGrid::new(level).vertices.iter().map(|v| v.position).collect(),
        None => model.vertices.iter().map(|v| v.position).collect(),
    }
}

/// The packed vertex channels (normal, tangent, colour) the prepared mesh
/// stores, for the vegetation shadow inputs.
pub(super) fn local_packed(grid: Option<u32>, model: &ParticleModel) -> Vec<[u8; 16]> {
    match grid {
        Some(level) => vec![[0u8, 0, 0, 0, 0, 0, 0, 0x55, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
            crate::rendering::particles::geometry::SpriteGrid::new(level).vertices.len()],
        None => model.vertices.iter().map(|v| v.attributes[4..20].try_into().unwrap()).collect(),
    }
}

/// The game-local box of `positions`.
pub(super) fn local_bounds(positions: &[[f32; 3]]) -> [[f32; 3]; 2] {
    let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
    for p in positions {
        for axis in 0..3 {
            bounds[0][axis] = bounds[0][axis].min(p[axis]);
            bounds[1][axis] = bounds[1][axis].max(p[axis]);
        }
    }
    bounds
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
            ParticleSurfaceShadow { material: self.shadow, bounds: self.bounds },
        )
    }

    /// Publish only after the current level accepts the worker's asset batch.
    /// The entity identity is then tracked by the render readiness owner.
    pub fn publish(mut self, world: &mut World) -> Entity {
        let alpha = self.alpha.take();
        let mut entity = world.spawn(self.into_bundle());
        if let Some(alpha) = alpha { entity.insert(ParticleSurfaceAlpha(alpha)); }
        entity.id()
    }
}

pub(super) fn publish(
    commands: &mut super::setup_commands::SetupCommands,
    mut surface: PreparedSurface,
) -> (super::MaterialHandle, Vec<(usize, ImageRole)>) {
    let binding = super::MaterialHandle::Probe(surface.handle.clone());
    let images = std::mem::take(&mut surface.images);
    let alpha = surface.alpha.take();
    let mut entity = commands.spawn(surface.into_bundle());
    if let Some(alpha) = alpha { entity.insert(ParticleSurfaceAlpha(alpha)); }
    (binding, images)
}
