//! Particle renderers whose material has a world MAIN program. 1401fbad0
//! publishes every renderer as an ordinary model surface (material =
//! desc+0x278), so these draw through the material's own passes with its
//! direct-instancing variant: 140319900 -> 1402bb550 -> 1402bace0 binds the
//! renderer's instance count, matrices, particle parameters and the system's
//! shared indirect-lighting record. Each (PMODEL, material) pair has one
//! proxy entity carrying the world material; its instances come from the
//! FX update's publication (1eaa40) of this frame.
use super::camera_scene::FxPublication;
use bevy::{
    pbr::DirectInstanceDraws,
    prelude::*,
    render::{
        extract_component::ExtractComponent,
        render_resource::{binding_types::storage_buffer_read_only_sized, *},
        renderer::{RenderDevice, RenderQueue, ViewQuery},
        sync_world::MainEntity,
    },
};
use std::collections::HashMap;

/// Main-world marker of a world-material particle surface.
#[derive(Component, Clone)]
pub struct ParticleMeshProxy {
    pub source: String,
    pub material: String,
}

impl bevy::render::sync_component::SyncComponent for ParticleMeshProxy {
    type Target = Self;
}

impl ExtractComponent for ParticleMeshProxy {
    type QueryData = &'static Self;
    type QueryFilter = ();
    type Out = Self;
    fn extract_component(item: &Self) -> Option<Self::Out> {
        Some(item.clone())
    }
}

/// Group 4 of a direct-instanced surface pipeline: instance records and
/// this frame's per-system lighting records.
pub fn instance_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "particle surface instances",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                storage_buffer_read_only_sized(false, None),
                storage_buffer_read_only_sized(false, None),
            ),
        ),
    )
}

/// Immediates of DrawMesh's direct-instancing branch: {mesh_index, first}.
pub const IMMEDIATE_SIZE: u32 = 8;

const INSTANCE_BYTES: usize = 64;
const LIGHTING_BYTES: usize = 9 * 16;

#[derive(Resource, Default)]
pub(super) struct ParticleMeshBuffers {
    instances: Option<Buffer>,
    lighting: Option<Buffer>,
    instance_bytes: Vec<u8>,
    lighting_bytes: Vec<u8>,
}

/// The backend's view axes for game space (view_geometry::game_affine_to_view).
fn game_to_view() -> Mat4 {
    Mat4::from_cols(Vec4::X, Vec4::new(0.0, 0.0, -1.0, 0.0), Vec4::new(0.0, 1.0, 0.0, 0.0), Vec4::W)
}

/// Upload this frame's world-mesh publication and list every proxy's draw.
/// Runs after the FX update and before any pass that draws the proxies.
pub(super) fn prepare_particle_meshes(
    publication: Res<FxPublication>,
    water: Res<super::water_scene::WaterSceneFrame>,
    proxies: Query<(&MainEntity, &ParticleMeshProxy)>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    mut buffers: ResMut<ParticleMeshBuffers>,
    mut draws: ResMut<DirectInstanceDraws>,
    view: ViewQuery<&MainEntity>,
    mut reported: Local<Option<String>>,
    mut reported_active: Local<bool>,
    mut previous: Local<Vec<MainEntity>>,
) {
    // Every proxy is listed before any pass can draw it.
    // Each owner replaces its own entries; decal receiver draws share this
    // service and are published later in the same FX preparation interval.
    for entity in previous.drain(..) { draws.entities.remove(&entity); }
    for (entity, _) in &proxies {
        previous.push(*entity);
        draws.entities.insert(*entity, (0, 0).into());
    }
    let Some(water) = water.0.as_ref() else {
        return;
    };
    if view.into_inner().id() != water.camera {
        return;
    }
    let published = publication.0
        .as_ref()
        .filter(|published| published.camera == water.camera && published.frame == water.matrices.frame)
        .and_then(|published| published.particles.as_ref());
    let mut by_key: HashMap<(&str, &str), Vec<&super::particle_system::WorldMeshDraw>> = HashMap::new();
    for draw in published.iter().flat_map(|published| &published.world_meshes) {
        by_key.entry((draw.source.as_str(), draw.material.as_str())).or_default().push(draw);
    }
    let axes = game_to_view();
    let inverse = axes.inverse();
    let buffers = &mut *buffers;
    buffers.instance_bytes.clear();
    buffers.lighting_bytes.clear();
    let mut lighting_index = 0u32;
    for (entity, proxy) in &proxies {
        let first = (buffers.instance_bytes.len() / INSTANCE_BYTES) as u32;
        let mut sample = None;
        for draw in by_key.remove(&(proxy.source.as_str(), proxy.material.as_str())).unwrap_or_default() {
            for coefficient in draw.lighting {
                buffers.lighting_bytes.extend(coefficient.into_iter().flat_map(f32::to_le_bytes));
            }
            for instance in &draw.instances {
                sample.get_or_insert((instance.position, Vec3::from_array(instance.axes[0]).length()));
                let model = Mat4::from_cols(
                    Vec3::from_array(instance.axes[0]).extend(0.0),
                    Vec3::from_array(instance.axes[1]).extend(0.0),
                    Vec3::from_array(instance.axes[2]).extend(0.0),
                    Vec3::from_array(instance.position).extend(1.0),
                );
                let rows = (axes * model * inverse).transpose();
                for row in [rows.x_axis, rows.y_axis, rows.z_axis] {
                    buffers.instance_bytes.extend(row.to_array().into_iter().flat_map(f32::to_le_bytes));
                }
                for word in [
                    instance.parameters.colour,
                    instance.parameters.flipbook_seed_uv,
                    instance.parameters.size,
                    lighting_index,
                ] {
                    buffers.instance_bytes.extend(word.to_le_bytes());
                }
            }
            lighting_index += 1;
        }
        let count = (buffers.instance_bytes.len() / INSTANCE_BYTES) as u32 - first;
        draws.entities.insert(*entity, (first, count).into());
        if count != 0 && !*reported_active {
            info!(source = proxy.source, material = proxy.material, count, sample = ?sample,
                "Particle surface submission active (first instance game position, axis-0 scale)");
            *reported_active = true;
        }
    }
    // A publication without a proxy is a definition the level setup did not
    // prepare a surface for: report it instead of dropping it silently.
    if let Some(((source, material), _)) = by_key.iter().next() {
        let error = format!("particle surface {source} / {material} has no prepared proxy");
        if reported.as_ref() != Some(&error) {
            warn!("{error}");
            *reported = Some(error);
        }
    }
    let mut changed = false;
    for (buffer, bytes, label) in [
        (&mut buffers.instances, &buffers.instance_bytes, "particle surface instances"),
        (&mut buffers.lighting, &buffers.lighting_bytes, "particle surface lighting"),
    ] {
        let size = (bytes.len().max(LIGHTING_BYTES) as u64).next_power_of_two();
        if buffer.as_ref().is_none_or(|buffer| buffer.size() < size) {
            *buffer = Some(device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            changed = true;
        }
        if !bytes.is_empty() {
            queue.write_buffer(buffer.as_ref().unwrap(), 0, bytes);
        }
    }
    if changed || draws.bind_group.is_none() {
        draws.bind_group = Some(device.create_bind_group(
            Some("particle surface instances"),
            &cache.get_bind_group_layout(&instance_layout()),
            &BindGroupEntries::sequential((
                buffers.instances.as_ref().unwrap().as_entire_binding(),
                buffers.lighting.as_ref().unwrap().as_entire_binding(),
            )),
        ));
    }
}
