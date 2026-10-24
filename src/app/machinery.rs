//! Client wiring for archive-defined constrained ambient machinery.
use super::model_motion::{ModelPose, ModelPoseUpdates};
use crate::{
    archive::Catalog,
    entities::{Document, PlacedModel},
    gameplay::{game_clock_runtime::ClockRuntime, machinery_world::MachineryWorld},
};
use bevy::prelude::*;
#[derive(Resource, Default)]
pub(super) struct AmbientMachinery {
    physics: MachineryWorld,
    scenes: crate::gameplay::scene_motion::SceneMotion,
    /// Per body: its owner's render source, resolved once sources exist.
    sources: Option<Vec<Option<usize>>>,
    /// Render +0x1c31 keeps the main view's visibility for two frame parities.
    visible_previous: Vec<bool>,
    skins: Vec<super::physics_skinning::SystemSkin>,
}
impl AmbientMachinery {
    pub(super) fn load(
        catalog: &Catalog,
        document: Option<&Document>,
        placements: &[PlacedModel],
    ) -> Result<Self, String> {
        let Some(document) = document else {
            return Ok(Self::default());
        };
        let placements = super::expanded_model_placements(placements, catalog);
        let recipes = crate::entities::machinery::recipes::load(catalog, document, &placements)?;
        for diagnostic in &recipes.unsupported {
            warn!("Ambient machinery assembly unsupported: {diagnostic}");
        }
        let mut world = MachineryWorld::from_recipes(&recipes)?;
        // 14087caa0 -> 14064c820: owners queue for lod_dist_warmup_framecount.
        let warmup = crate::platform::console::shared()
            .read()
            .cvars
            .int("lod_dist_warmup_framecount")
            .unwrap_or(3);
        for body in &mut world.bodies {
            if body.entity_placement.is_some() {
                body.lod = crate::gameplay::entity_lod::EntityLod::spawned(warmup);
            }
        }
        info!("Ambient machinery: {} assemblies, {} bodies, {} constraints, {} unsupported assemblies",recipes.assemblies.len(),world.bodies.iter().filter(|b|b.placement.is_some()).count(),world.joint_count(),recipes.unsupported.len());
        let graphs = crate::entities::scene_motion::load(catalog, document)?;
        let counter = crate::platform::performance_counter::counter()? as u64;
        let scenes = crate::gameplay::scene_motion::SceneMotion::new(graphs, &placements, counter);
        for diagnostic in &scenes.diagnostics {
            warn!("Ambient scene motion: {diagnostic}");
        }
        info!(
            "Ambient scene motion: {} playing groups",
            scenes.playing_count()
        );
        let visible_previous = vec![false; world.bodies.len()];
        let skins = super::physics_skinning::load_skins(catalog, &world, &placements);
        info!("Physics systems: {} simulated, {} skinned", world.systems.len(), skins.len());
        Ok(Self {
            physics: world,
            scenes,
            sources: None,
            visible_previous,
            skins,
        })
    }
}
/// The top render view for 14064cff0: camera position and the game
/// horizontal field of view of the viewport.
fn lod_view(
    cameras: &Query<(&Camera, &Transform, &Projection), With<Camera3d>>,
) -> Option<crate::gameplay::entity_lod::LodView> {
    let (camera, transform, projection) = cameras.single().ok()?;
    let viewport = camera.physical_viewport_size()?;
    let reference = super::reference_fov_from_projection(projection).ok()?;
    let horizontal_fov =
        crate::tome::render_fov_radians(reference, viewport.x, viewport.y, 1.0).ok()?.0;
    let position = super::view_to_game(transform.translation);
    Some(crate::gameplay::entity_lod::LodView {
        positions: [position; 2],
        horizontal_fov,
    })
}

fn lod_distances() -> crate::gameplay::entity_lod::LodDistances {
    let console = crate::platform::console::shared().read();
    let value = |name: &str| console.cvars.float(name).unwrap_or_default();
    crate::gameplay::entity_lod::LodDistances {
        force_visible: value("lod_forcevisibledistance"),
        levels: std::array::from_fn(|i| value(&format!("lod_dist_level{i}"))),
        far: value("lod_dist_far"),
    }
}

/// 14064cd50 for machinery owners: count the warm-up down, then request
/// 14064c1a0 (render owners) or 14064c3a0 (origin only) levels.
fn update_lod_levels(
    machinery: &mut AmbientMachinery,
    view: &crate::gameplay::entity_lod::LodView,
    sources: &super::shadow_updates::ModelShadowSources,
    scene: &super::SceneRenderQueue,
    placements: &super::RenderSourcePlacements,
) {
    use crate::gameplay::entity_lod::{origin_level, render_level};
    let distances = lod_distances();
    if machinery.sources.is_none() && !sources.0.is_empty() {
        machinery.sources = Some(
            machinery
                .physics
                .bodies
                .iter()
                .map(|body| {
                    let placement = body.entity_placement?;
                    sources.0.iter().position(|s| s.placement == placement)
                })
                .collect(),
        );
    }
    let mut changes = Vec::new();
    for index in 0..machinery.physics.bodies.len() {
        let Some(placement) = machinery.physics.bodies[index].entity_placement else {
            continue;
        };
        let source = machinery
            .sources
            .as_ref()
            .and_then(|sources| sources[index])
            .and_then(|i| sources.0.get(i));
        let visible_now = source.is_some_and(|source| {
            scene.instances[source.instances.clone()].iter().any(|i| i.visible)
        });
        let visible = visible_now || machinery.visible_previous[index];
        machinery.visible_previous[index] = visible_now;
        let body = &mut machinery.physics.bodies[index];
        if !body.lod.listed() {
            continue;
        }
        let level = match source {
            Some(source) => {
                render_level(source.bounds, source.flags_1c30, visible, false, view, &distances)
            }
            None => match placements.0.get(placement) {
                Some(placed) => origin_level(placed.position, view, &distances),
                None => continue,
            },
        };
        if let Some((old, new)) = body.lod.request(level) {
            changes.push((index, old, new));
        }
    }
    for (index, old, new) in changes {
        machinery.physics.entity_level_changed(index, old, new);
    }
}

pub(super) fn update_machinery(
    clock: Res<ClockRuntime>,
    mut machinery: ResMut<AmbientMachinery>,
    mut pending: ResMut<ModelPoseUpdates>,
    world_wind: Res<super::fog_scene::WorldWind>,
    winds: Option<Res<super::vegetation_wind::VegetationWind>>,
    cameras: Query<(&Camera, &Transform, &Projection), With<Camera3d>>,
    sources: Res<super::shadow_updates::ModelShadowSources>,
    scene: Res<super::SceneRenderQueue>,
    placements: Res<super::RenderSourcePlacements>,
) {
    if !clock.advanced() {
        return;
    }
    let Ok(clock) = clock.clock() else {
        return;
    };
    let machinery = &mut *machinery;
    if let Some(view) = lod_view(&cameras) {
        update_lod_levels(machinery, &view, &sources, &scene, &placements);
    }
    machinery.scenes.advance_motors(
        clock.delta(3).expect("five clock groups"),
        &mut machinery.physics,
    );
    let manager = winds.as_deref().map(|winds| winds.manager());
    // 7b8340 -> 807680, before the physics step: a body inside trigger winds
    // uses the first (if started); otherwise the registered world wind.
    let actor_seconds = clock.delta(3).expect("five clock groups");
    if actor_seconds > 0. {
        for index in 0..machinery.physics.bodies.len() {
            let body = &machinery.physics.bodies[index];
            let Some(state) = &body.wind else {
                continue;
            };
            let world = world_wind.0.as_ref();
            let wind = if !state.triggers.is_empty() || world.is_none() {
                state.triggers.first().and_then(|&index| {
                    let manager = manager?;
                    manager.trigger_shapes[index].1.then(|| &manager.triggers[index])
                })
            } else {
                world
            };
            if let Some(wind) = wind {
                let velocity = wind.velocity(Vec3::from_array(body.mass_center()));
                machinery.physics.apply_wind(index, velocity.to_array(), actor_seconds);
            }
        }
    }
    machinery.physics.step(std::array::from_fn(|i| {
        clock.delta(i).expect("five clock groups")
    }));
    if let Some(manager) = manager {
        for body in &mut machinery.physics.bodies {
            body.update_wind_triggers(&manager.trigger_shapes);
        }
    }
    pending
        .0
        .extend(machinery.physics.bodies.iter().filter_map(|body| {
            body.placement.map(|placement| ModelPose {
                placement,
                delta: body.delta(),
            })
        }));
    machinery.scenes.step(
        clock.delta(3).expect("five clock groups"),
        |placement, delta| pending.0.push(ModelPose { placement, delta }),
    );
}

/// After update_machinery: skin simulated physics-system placements.
pub(super) fn pose_physics_systems(
    mut machinery: ResMut<AmbientMachinery>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<super::ShaderBuffer>>,
    mut queue: ResMut<super::SceneRenderQueue>,
    mut sources: ResMut<super::shadow_updates::ModelShadowSources>,
    placements: Res<super::RenderSourcePlacements>,
    mut mesh_components: Query<&mut Mesh3d>,
) {
    let machinery = &mut *machinery;
    if machinery.skins.is_empty() {
        return;
    }
    super::physics_skinning::pose_physics_systems(
        &mut machinery.skins,
        &machinery.physics,
        &mut meshes,
        &mut buffers,
        &mut queue,
        &mut sources,
        &placements,
        &mut mesh_components,
    );
}
