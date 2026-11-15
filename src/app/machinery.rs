//! Client wiring for archive-defined constrained ambient machinery.
use super::model_motion::{ModelPose, ModelPoseUpdates};
use crate::{
    archive::Catalog,
    entities::{Document, PlacedModel},
    gameplay::{game_clock_runtime::ClockRuntime, machinery_world::MachineryWorld},
};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

struct SceneAsset {
    animation: crate::gameplay::scene_animation::SceneAnimation,
    model: crate::bmd6::SkinModel,
}
struct SceneSkin {
    asset: Arc<SceneAsset>,
    publication: super::skin_publication::SkinPublication,
    time: Option<i32>,
    lod: crate::gameplay::entity_lod::EntityLod,
    visible_previous: bool,
    source: Option<usize>,
    physics_skin: Option<usize>,
    failed: bool,
}

#[derive(Resource, Default)]
pub(super) struct AmbientMachinery {
    physics: MachineryWorld,
    scenes: crate::gameplay::scene_motion::SceneMotion,
    /// Per body: its owner's render source, resolved once sources exist.
    sources: Option<Vec<Option<usize>>>,
    /// Render +0x1c31 keeps the main view's visibility for two frame parities.
    visible_previous: Vec<bool>,
    skins: Vec<super::physics_skinning::SystemSkin>,
    scene_skins: HashMap<usize, SceneSkin>,
}
impl AmbientMachinery {
    /// Placements whose MD6 vertices are re-posed from simulated bodies.
    pub(super) fn deformed_placements(&self) -> std::collections::HashSet<usize> {
        self.skins.iter().map(|s| s.placement()).chain(self.scene_skins.keys().copied()).collect()
    }
    pub(super) fn physics_placements(&self) -> std::collections::HashSet<usize> {
        self.skins.iter().map(|s| s.placement()).collect()
    }
    pub(super) fn scene_layer_active(&self, placement: usize) -> bool {
        self.scene_skins.contains_key(&placement)
    }
    pub(super) fn animated_palette(&self, placement: usize, animation: &[super::physics_skinning::Affine],
        joint_frames: Option<&mut super::joint_frames::JointFrames>)
        -> Option<Result<Vec<super::physics_skinning::Affine>, String>> {
        self.skins.iter().find(|skin| skin.placement() == placement)
            .map(|skin| skin.animated_palette(&self.physics, animation, joint_frames))
    }
    pub(super) fn initial_joint_matrix(&self, placement: usize, joint: i16,
        skeleton: &Arc<crate::edge_skeleton::EdgeSkeleton>)
        -> Result<Option<super::physics_skinning::Affine>, String> {
        let Some(skin) = self.skins.iter().find(|skin| skin.placement() == placement) else { return Ok(None); };
        if !Arc::ptr_eq(skin.skeleton(), skeleton) { return Err("attachment and physics pose skeleton owners differ".into()); }
        let current = skin.current_with_animation(&self.physics, None)?;
        Ok(usize::try_from(joint).ok().and_then(|joint| current.get(joint)).copied())
    }
    pub(super) fn skin_models(&self) -> impl Iterator<Item = (usize, &std::sync::Arc<crate::bmd6::ModelInfo>)> {
        self.skins.iter().map(|skin| (skin.placement(), skin.model()))
    }
    #[allow(dead_code)]
    pub(super) fn load(
        catalog: &Catalog,
        document: Option<&Document>,
        placements: &[PlacedModel],
    ) -> Result<Self, String> {
        if document.is_none() {
            return Ok(Self::default());
        }
        let placements = super::expanded_model_placements(placements, catalog);
        let mut sources = crate::physworld::source::ShapeSources::new(catalog);
        Self::load_with_texts(catalog, document, &placements, &mut Default::default(), &mut sources)
    }

    pub(super) fn load_with_texts(
        catalog: &Catalog,
        document: Option<&Document>,
        placements: &[PlacedModel],
        texts: &mut crate::assets::declaration_texts::PhysicsSeed,
        shape_sources: &mut crate::physworld::source::ShapeSources,
    ) -> Result<Self, String> {
        Self::load_with_texts_cancellable(catalog, document, placements, texts, shape_sources, None)
    }

    pub(super) fn load_with_texts_cancellable(
        catalog: &Catalog,
        document: Option<&Document>,
        placements: &[PlacedModel],
        texts: &mut crate::assets::declaration_texts::PhysicsSeed,
        shape_sources: &mut crate::physworld::source::ShapeSources,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, String> {
        let Some(document) = document else {
            return Ok(Self::default());
        };
        let recipes = crate::entities::machinery::recipes::load_with_sources(catalog, document, placements, texts, shape_sources)?;
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
        let graphs = crate::entities::scene_motion::load_cancellable(catalog, document, cancelled)?;
        let counter = crate::platform::performance_counter::counter()? as u64;
        let scenes = crate::gameplay::scene_motion::SceneMotion::new(graphs, placements, counter);
        for diagnostic in &scenes.diagnostics {
            warn!("Ambient scene motion: {diagnostic}");
        }
        info!(
            "Ambient scene motion: {} playing groups",
            scenes.playing_count()
        );
        let visible_previous = vec![false; world.bodies.len()];
        let skins = super::physics_skinning::load_skins(catalog, &world, placements);
        // Only playback admitted by the startup graph acquires skeletal data.
        // Each started instance leases an immutable asset; no scene references
        // alone preload rigs, meshes or compressed animations.
        let mut scene_skins = HashMap::new();
        let mut scene_assets = HashMap::new();
        for (placement, clip, group, _) in scenes.animated_groups() {
            let Some(placed) = placements.get(placement) else { continue };
            let key = (placed.resource_path.clone(), Arc::as_ptr(clip) as usize, group,
                catalog.has_overlays().then_some(placement));
            let asset = scene_assets.entry(key).or_insert_with(|| -> Result<Arc<SceneAsset>, String> {
                let declaration = crate::bmd6::declaration_path(&placed.resource_path)
                    .ok_or("scene model is not a compiled MD6")?;
                let model_name = declaration.strip_prefix("generated/decls/md6def/")
                    .and_then(|name| name.strip_suffix(".decl")).ok_or("scene MD6 path is invalid")?;
                let rig = crate::assets::animation_rig::Rig::acquire(catalog, model_name)?;
                let model = crate::bmd6::ModelSources::acquire_skin_for_rig(catalog, &placed.resource_path, &rig, None)?;
                let uid = clip.motion.groups[group].uid;
                let animation = crate::gameplay::scene_animation::SceneAnimation::load(catalog, rig, Arc::clone(clip), uid)?;
                Ok(Arc::new(SceneAsset { animation, model }))
            });
            match asset {
                Ok(asset) => {
                    if scene_skins.contains_key(&placement) {
                        warn!("Multiple scene animation tracks target placement {placement}");
                        continue;
                    }
                    scene_skins.insert(placement, SceneSkin { asset: Arc::clone(asset), publication: Default::default(),
                        time: None, lod: crate::gameplay::entity_lod::EntityLod::spawned(warmup),
                        visible_previous: false, source: None, physics_skin: skins.iter().position(|skin| skin.placement() == placement), failed: false });
                }
                Err(error) => warn!("Scene animation unavailable for placement {placement}: {error}"),
            }
        }
        info!("Physics systems: {} simulated, {} skinned", world.systems.len(), skins.len());
        Ok(Self {
            physics: world,
            scenes,
            sources: None,
            visible_previous,
            skins,
            scene_skins,
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
    // 54b430 disables the animation tree at LOD >= 6; its scene clock
    // continues so reappearing actors resume at the current scene time.
    for (&placement, skin) in &mut machinery.scene_skins {
        if !skin.source.and_then(|i| sources.0.get(i)).is_some_and(|s| s.placement == placement) {
            skin.source = sources.0.iter().position(|s| s.placement == placement);
        }
        let source = skin.source.and_then(|i| sources.0.get(i));
        let visible_now = source.is_some_and(|source| scene.instances[source.instances.clone()].iter().any(|i| i.visible));
        let visible = visible_now || skin.visible_previous;
        skin.visible_previous = visible_now;
        if !skin.lod.listed() { continue; }
        let level = match source {
            Some(source) => render_level(source.bounds, source.flags_1c30, visible, false, view, &distances),
            None => match placements.0.get(placement) {
                Some(placed) => origin_level(placed.position, view, &distances),
                None => continue,
            },
        };
        skin.lod.request(level);
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
    // 8af460 fills exactly two entries from1422952d8: clock groups3/4.
    // Raw wall/view clocks are not physics timespaces.
    machinery.physics.step(std::array::from_fn(|i| {
        clock.entity_delta(i as u32).expect("two physics timespaces")
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
    animations: Option<Res<super::usable_animation::UsableAnimations>>,
    mut joint_frames: Option<ResMut<super::joint_frames::JointFrames>>,
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
        animations.as_deref(),
        |placement| machinery.scene_skins.contains_key(&placement),
        joint_frames.as_deref_mut(),
    );
}

/// Compose the live lower tree and scene tracks before caster collection.
/// Startup loops own independently prepared meshes until map retirement.
pub(super) fn pose_scene_systems(
    mut machinery: ResMut<AmbientMachinery>,
    animations: Option<Res<super::usable_animation::UsableAnimations>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<super::ShaderBuffer>>,
    mut queue: ResMut<super::SceneRenderQueue>,
    mut sources: ResMut<super::shadow_updates::ModelShadowSources>,
    placements: Res<super::RenderSourcePlacements>,
    mut joint_frames: Option<ResMut<super::joint_frames::JointFrames>>,
) {
    let machinery = &mut *machinery;
    let (scenes, skins, physics_skins, physics) = (&machinery.scenes, &mut machinery.scene_skins,
        &machinery.skins, &machinery.physics);
    for (placement, _, _, time) in scenes.animated_groups() {
        let Some(skin) = skins.get_mut(&placement) else { continue };
        let lower = animations.as_deref().and_then(|a| a.local_pose(placement));
        if skin.failed || skin.lod.level >= 6 || (skin.time == Some(time) && lower.is_none()) { continue; }
        if !skin.source.and_then(|index| sources.0.get(index)).is_some_and(|source| source.placement == placement) {
            skin.source = sources.0.iter().position(|source| source.placement == placement);
        }
        let (Some(source_index), Some(placed)) = (skin.source, placements.0.get(placement)) else { continue };
        let animation = &skin.asset.animation;
        let result = (|| {
            let pose = match lower {
                Some((skeleton, pose)) => {
                    if !Arc::ptr_eq(skeleton, animation.skeleton()) { return Err("scene and lower tree skeleton owners differ".into()); }
                    animation.pose_over(time, pose.clone())?
                }
                None => animation.pose(time)?,
            };
            let matrices = animation.matrices_from_pose(&pose)?;
            if let Some(physics_skin) = skin.physics_skin.and_then(|index| physics_skins.get(index)).filter(|skin| skin.placement() == placement) {
                if !Arc::ptr_eq(physics_skin.skeleton(), animation.skeleton()) {
                    return Err("scene and physics skeleton owners differ".into());
                }
                physics_skin.animated_palette(physics, &matrices, joint_frames.as_deref_mut())
            } else {
                if let Some(frames) = joint_frames.as_deref_mut() { frames.publish(placement, &matrices); }
                // 24c340 uses SIMD-padded banks; this host evaluates only the
                // logical joints. Project the same logical lanes, as usable and
                // physics skin owners do; never invent transforms for padding.
                let inverse = animation.skeleton().inverse_bind.get(..matrices.len())
                    .ok_or("scene skeleton has fewer inverse bind matrices than joints")?;
                crate::edge_skeleton::skin_palette(&matrices, inverse)
            }
        })();
        match result {
            Ok(palette) => {
                super::skin_publication::publish_target(placed, &mut sources.0[source_index], skin.asset.model.surfaces(),
                    palette, &mut skin.publication, &mut meshes, &mut buffers, &mut queue);
                skin.time = Some(time);
            }
            Err(error) => { warn!("Scene pose unavailable for placement {placement}: {error}"); skin.failed = true; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires installed Edge of the World archives"]
    fn installed_edge_machinery_keeps_finite_render_poses() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let level = crate::assets::level::LevelResources::resolve(&catalog, "eotw_p_lowchaos").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let placements = crate::entities::render_placements(&document);
        for seconds in [1. / 60., 0.1] {
            let mut machinery = AmbientMachinery::load(&catalog, Some(&document), &placements).unwrap();
            let mut wind = super::super::fog_scene::WorldWind::load(&catalog, Some(&document)).unwrap();
            for frame in 0..1200 {
                machinery.scenes.advance_motors(seconds, &mut machinery.physics);
                if let Some(wind) = wind.0.as_mut() {
                    wind.advance(seconds);
                    for index in 0..machinery.physics.bodies.len() {
                        let velocity = wind.velocity(Vec3::from_array(machinery.physics.bodies[index].mass_center()));
                        machinery.physics.apply_wind(index, velocity.to_array(), seconds);
                    }
                }
                machinery.physics.step([seconds; 2]);
                for (index, body) in machinery.physics.bodies.iter().enumerate() {
                    assert!(body.delta().iter().flatten().all(|v| v.is_finite()),
                        "dt {seconds} frame {frame} body {index} placement {:?} pose {:?}", body.placement, body.motion.pose);
                }
                machinery.scenes.step(seconds, |placement, delta| {
                    assert!(delta.iter().flatten().all(|v| v.is_finite()), "frame {frame} scene placement {placement}");
                });
            }
        }
    }
}
