//! Runtime world render actors (874c50/329120/31ad30). Prototypes prepare
//! shared geometry/materials before input without allocating managed actors.
//! Real entity generations instantiate draws only after GPU upload ack.
use super::player_arms::{
    attachment::{Models, Template},
    RetiredModel,
};
use crate::{
    assets::equipment::attachment::Descriptor,
    gameplay::{
        entity_lifetime::{EntityId, LiveEntities},
        reference_frame::ReferenceFrame,
    },
};
use bevy::prelude::*;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
};
pub(super) struct Request {
    pub key: usize,
    pub descriptor: Arc<Descriptor>,
}
pub(super) struct Prepared {
    pub key: usize,
    pub model: Template,
}
#[derive(Clone, Copy, Default)]
pub(super) struct RenderState {
    pub hidden: bool,
    pub current: bool,
    pub previous: bool,
}
struct Entry {
    descriptor: Arc<Descriptor>,
    frame: ReferenceFrame,
    scale: [f32; 3],
    visible: bool,
    area: Option<f32>,
    render: RenderState,
}
struct Prototype {
    descriptor: Weak<Descriptor>,
    model: Option<Arc<Template>>,
    uploaded: Arc<AtomicBool>,
    error: Option<String>,
}
#[derive(Resource, Default)]
pub(super) struct WorldModels {
    entries: HashMap<EntityId, Entry>,
    prototypes: HashMap<usize, Prototype>,
    models: Models,
    retiring: HashMap<EntityId, RetiredModel>,
    retiring_prototypes: Vec<RetiredModel>,
    closing: bool,
}
impl WorldModels {
    pub(super) fn shown_materials(&self) -> impl Iterator<Item = &super::MaterialHandle> {
        self.models.shown_materials()
    }
    pub(super) fn submit_texture_demands(
        &self,
        _state: Option<&crate::visibility::tome::RenderQueryState>,
        bindings: &bevy::platform::collections::HashMap<super::MaterialHandle, Vec<usize>>,
        demands: &[crate::texture::ProjectedAreaDemand],
    ) -> Result<(), String> {
        for (&id, entry) in &self.entries {
            self.models
                .submit_area(id, entry.area.unwrap_or(1.0), bindings, demands)?;
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Upload {
    assets: Vec<(bevy::asset::AssetId<Mesh>, bevy::asset::UntypedAssetId)>,
    complete: Arc<AtomicBool>,
}
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
struct Uploads(Arc<Vec<Upload>>);
fn key(descriptor: &Arc<Descriptor>) -> usize {
    Arc::as_ptr(descriptor) as usize
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<WorldModels>()
        .init_resource::<Uploads>()
        .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            Uploads,
        >::default());
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.add_systems(
            bevy::render::Render,
            acknowledge.in_set(bevy::render::RenderSystems::PrepareResources),
        );
    }
}
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if let Some(owner) = world.get_resource::<WorldModels>() {
        if !owner.entries.is_empty()
            || !owner.retiring.is_empty()
            || !owner.retiring_prototypes.is_empty()
        {
            return Err("previous world models still own live/GPU generations".into());
        }
    }
    world.insert_resource(WorldModels::default());
    world.insert_resource(Uploads::default());
    Ok(())
}
fn acknowledge(
    uploads: Res<Uploads>,
    meshes: Res<bevy::render::render_asset::RenderAssets<bevy::render::mesh::RenderMesh>>,
    materials: Res<
        bevy::render::erased_render_asset::ErasedRenderAssets<bevy::pbr::PreparedMaterial>,
    >,
) {
    for upload in uploads
        .0
        .iter()
        .filter(|upload| !upload.complete.load(Ordering::Acquire))
    {
        if upload.assets.iter().all(|(mesh, material)| {
            meshes.get(*mesh).is_some() && materials.get(*material).is_some()
        }) {
            upload.complete.store(true, Ordering::Release);
        }
    }
}
pub(super) fn prewarm(world: &mut World, descriptor: Arc<Descriptor>) -> Result<(), String> {
    let mut owner = world
        .get_resource_mut::<WorldModels>()
        .ok_or("world model owner is absent")?;
    if owner.closing {
        return Err("world model acquisition is closed".into());
    }
    if let Some(prototype) = owner.prototypes.get(&key(&descriptor)) {
        return prototype.error.clone().map_or(Ok(()), Err);
    }
    owner.prototypes.insert(
        key(&descriptor),
        Prototype {
            descriptor: Arc::downgrade(&descriptor),
            model: None,
            uploaded: Arc::default(),
            error: None,
        },
    );
    drop(owner);
    super::runtime_materials::invalidate_world_models(world);
    Ok(())
}
pub(super) fn prototype_ready(world: &World, descriptor: &Arc<Descriptor>) -> Result<bool, String> {
    let Some(prototype) = world
        .get_resource::<WorldModels>()
        .and_then(|owner| owner.prototypes.get(&key(descriptor)))
    else {
        return Ok(false);
    };
    if let Some(error) = &prototype.error {
        return Err(error.clone());
    }
    Ok(prototype.model.is_some() && prototype.uploaded.load(Ordering::Acquire))
}
pub(super) fn request(
    world: &mut World,
    id: EntityId,
    descriptor: Arc<Descriptor>,
    frame: ReferenceFrame,
) -> Result<(), String> {
    if world
        .get_resource::<LiveEntities>()
        .and_then(|live| live.get(id))
        .is_none()
    {
        return Err("world model request has no live managed generation".into());
    }
    prewarm(world, descriptor.clone())?;
    let mut owner = world.resource_mut::<WorldModels>();
    if let Some(entry) = owner.entries.get(&id) {
        if Arc::ptr_eq(&entry.descriptor, &descriptor) {
            return Ok(());
        }
        return Err("world generation owns another model descriptor".into());
    }
    let scale = descriptor.placement.scale;
    owner.entries.insert(
        id,
        Entry {
            descriptor,
            frame,
            scale,
            visible: true,
            area: None,
            render: RenderState::default(),
        },
    );
    drop(owner);
    advance(world)
}
pub(super) fn ready(world: &World, id: EntityId) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<WorldModels>() else {
        return Ok(false);
    };
    let Some(entry) = owner.entries.get(&id) else {
        return Ok(false);
    };
    Ok(prototype_ready(world, &entry.descriptor)? && owner.models.has(id))
}
pub(super) fn update(
    world: &mut World,
    id: EntityId,
    frame: ReferenceFrame,
    visible: bool,
) -> Result<(), String> {
    let mut owner = world
        .get_resource_mut::<WorldModels>()
        .ok_or("world model owner is absent")?;
    let entry = owner
        .entries
        .get_mut(&id)
        .ok_or("world model generation is absent")?;
    entry.frame = frame;
    entry.visible = visible;
    entry.render.hidden = !visible;
    if owner.models.has(id) {
        owner.models.set_world_frame(id, frame)?;
        owner.models.set_visible(id, visible)?;
    }
    Ok(())
}
pub(super) fn set_scale(world: &mut World, id: EntityId, scale: [f32; 3]) -> Result<(), String> {
    let mut owner = world
        .get_resource_mut::<WorldModels>()
        .ok_or("world model owner is absent")?;
    owner
        .entries
        .get_mut(&id)
        .ok_or("world model generation is absent")?
        .scale = scale;
    if owner.models.has(id) {
        owner.models.set_world_scale(id, scale)?;
    }
    Ok(())
}
pub(super) fn render_state(world: &World, id: EntityId) -> Option<RenderState> {
    let owner = world.get_resource::<WorldModels>()?;
    owner
        .models
        .has(id)
        .then(|| owner.entries.get(&id).map(|entry| entry.render))
        .flatten()
}
pub(super) fn frame(world: &World, id: EntityId) -> Option<ReferenceFrame> {
    world
        .get_resource::<WorldModels>()?
        .entries
        .get(&id)
        .map(|entry| entry.frame)
}
pub(super) fn bounds(world:&World,id:EntityId)->Option<[[f32;3];2]>{
    let entry=world.get_resource::<WorldModels>()?.entries.get(&id)?;
    Some(crate::render_model::transformed_bounds(entry.descriptor.geometry.bounds(),entry.frame.position,entry.frame.basis,entry.scale))
}
pub(super) fn requested(world: &World) -> Vec<Request> {
    world
        .get_resource::<WorldModels>()
        .map_or_else(Vec::new, |owner| {
            owner
                .prototypes
                .iter()
                .filter(|(_, prototype)| prototype.model.is_none() && prototype.error.is_none())
                .filter_map(|(&key, prototype)| {
                    prototype
                        .descriptor
                        .upgrade()
                        .map(|descriptor| Request { key, descriptor })
                })
                .collect()
        })
}
pub(super) fn validate(world: &World, prepared: &Prepared) -> Result<(), String> {
    let prototype = world
        .get_resource::<WorldModels>()
        .and_then(|owner| owner.prototypes.get(&prepared.key))
        .ok_or("world model prototype retired during preparation")?;
    if !prototype
        .descriptor
        .ptr_eq(&Arc::downgrade(prepared.model.descriptor()))
    {
        return Err("world model prototype generation changed".into());
    }
    Ok(())
}
pub(super) fn publish(world: &mut World, prepared: Prepared) -> Result<(), String> {
    validate(world, &prepared)?;
    let assets = prepared.model.uploads();
    let mut owner = world.resource_mut::<WorldModels>();
    let prototype = owner.prototypes.get_mut(&prepared.key).unwrap();
    let complete = prototype.uploaded.clone();
    prototype.model = Some(Arc::new(prepared.model));
    drop(owner);
    Arc::make_mut(&mut world.resource_mut::<Uploads>().0).push(Upload { assets, complete });
    Ok(())
}
pub(super) fn add_leases(
    world: &World,
    keys: impl Iterator<Item = usize>,
    leases: &mut Vec<Weak<Descriptor>>,
) {
    let Some(owner) = world.get_resource::<WorldModels>() else {
        return;
    };
    for key in keys {
        if let Some(prototype) = owner.prototypes.get(&key) {
            if !leases
                .iter()
                .any(|lease| lease.ptr_eq(&prototype.descriptor))
            {
                leases.push(prototype.descriptor.clone());
            }
        }
    }
}
pub(super) fn failed(world: &mut World, error: &str) {
    if let Some(mut owner) = world.get_resource_mut::<WorldModels>() {
        for prototype in owner
            .prototypes
            .values_mut()
            .filter(|prototype| prototype.model.is_none())
        {
            prototype.error = Some(error.to_owned());
        }
    }
}
pub(super) fn advance(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<WorldModels>() else {
        return Ok(());
    };
    let mut commands = super::setup_commands::SetupCommands::prepared(world);
    let result = (|| {
        let WorldModels {
            entries,
            models,
            prototypes,
            ..
        } = &mut owner;
        for (&id, entry) in entries {
            if models.has(id) {
                continue;
            }
            let Some(prototype) = prototypes
                .get(&key(&entry.descriptor))
                .filter(|prototype| prototype.uploaded.load(Ordering::Acquire))
            else {
                continue;
            };
            let Some(template) = &prototype.model else {
                continue;
            };
            let prepared = template.instantiate(&mut commands)?;
            models.publish_world(id, prepared, entry.frame, entry.visible)?;
            models.set_world_scale(id, entry.scale)?;
        }
        Ok(())
    })();
    commands.publish(world);
    world.insert_resource(owner);
    result
}
pub(super) fn remove(world: &mut World, id: EntityId) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<WorldModels>() else {
        return Ok(());
    };
    let mut commands = super::setup_commands::SetupCommands::prepared(world);
    if let Some(receipt) = owner.models.remove(id, &mut commands) {
        owner.retiring.insert(id, receipt);
    }
    owner.entries.remove(&id);
    commands.publish(world);
    world.insert_resource(owner);
    Ok(())
}
pub(super) fn retired(world: &mut World, id: EntityId) -> bool {
    let Some(mut owner) = world.get_resource_mut::<WorldModels>() else {
        return true;
    };
    if owner.entries.contains_key(&id)
        || owner
            .retiring
            .get(&id)
            .is_some_and(|receipt| !receipt.complete())
    {
        return false;
    }
    let changed = owner.retiring.remove(&id).is_some();
    drop(owner);
    if changed {
        super::runtime_materials::invalidate_world_models(world);
    }
    true
}
pub(super) fn retirement_receipts(owner: &WorldModels) -> impl Iterator<Item = RetiredModel> + '_ {
    owner
        .retiring
        .values()
        .chain(owner.retiring_prototypes.iter())
        .cloned()
}
pub(super) fn prune(world: &mut World) {
    let Some(mut owner) = world.remove_resource::<WorldModels>() else {
        return;
    };
    let before = owner.retiring_prototypes.len();
    owner
        .retiring_prototypes
        .retain(|receipt| !receipt.complete());
    let mut changed = before != owner.retiring_prototypes.len();
    let dead = owner
        .prototypes
        .iter()
        .filter_map(|(&key, prototype)| {
            (prototype.descriptor.strong_count() <= usize::from(prototype.model.is_some()))
                .then_some(key)
        })
        .collect::<Vec<_>>();
    for key in dead {
        if let Some(prototype) = owner.prototypes.remove(&key) {
            Arc::make_mut(&mut world.resource_mut::<Uploads>().0)
                .retain(|upload| !Arc::ptr_eq(&upload.complete, &prototype.uploaded));
            if let Some(model) = prototype.model {
                owner.retiring_prototypes.push(RetiredModel::new(model));
            }
            changed = true;
        }
    }
    world.insert_resource(owner);
    if changed {
        super::runtime_materials::invalidate_world_models(world);
    }
}
pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    let Some(mut owner) = world.get_resource_mut::<WorldModels>() else {
        return Ok(true);
    };
    owner.closing = true;
    let ids = owner.entries.keys().copied().collect::<Vec<_>>();
    drop(owner);
    for id in ids {
        remove(world, id)?;
    }
    let mut owner = world.resource_mut::<WorldModels>();
    let prototypes = std::mem::take(&mut owner.prototypes);
    for (_, prototype) in prototypes {
        if let Some(model) = prototype.model {
            owner.retiring_prototypes.push(RetiredModel::new(model));
        }
    }
    owner.retiring.retain(|_, receipt| !receipt.complete());
    owner
        .retiring_prototypes
        .retain(|receipt| !receipt.complete());
    let done = owner.retiring.is_empty() && owner.retiring_prototypes.is_empty();
    drop(owner);
    world.resource_mut::<Uploads>().0 = Arc::default();
    super::runtime_materials::invalidate_world_models(world);
    Ok(done)
}
pub(super) fn publish_poses(
    mut owner: ResMut<WorldModels>,
    scene: Option<Res<super::SceneData>>,
    queue: Option<Res<super::SceneRenderQueue>>,
    mut visibility: Query<&mut Visibility>,
    mut transforms: Query<
        (&mut Transform, &mut GlobalTransform),
        (With<Mesh3d>, Without<Camera3d>),
    >,
    mut lighting: Option<ResMut<super::model_motion::ModelProbeLighting>>,
    mut patches: ResMut<crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches>,
    mut buffers: ResMut<Assets<super::ShaderBuffer>>,
    mut commands: Commands,
    mut inverse: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    mut joints: Query<&mut GlobalTransform, Without<Mesh3d>>,
    clock: Option<Res<crate::gameplay::game_clock_runtime::ClockRuntime>>,
) {
    let state = queue
        .as_ref()
        .and_then(|queue| queue.shadow_ordinary_query.as_ref())
        .and_then(|query| query.state.as_ref());
    let query = state
        .map(crate::visibility::tome::RenderAabbQuery::from_state)
        .transpose();
    let result = (|| -> Result<(), String> {
        let query = query?;
        let WorldModels {
            entries, models, ..
        } = &mut *owner;
        for (&id, entry) in entries {
            if !models.has(id) {
                continue;
            }
            let mut visible = entry.visible;
            if let (Some(query), Some(state)) = (query.as_ref(), state) {
                let descriptor = &entry.descriptor;
                let bounds = crate::render_model::transformed_bounds(
                    descriptor.geometry.bounds(),
                    entry.frame.position,
                    entry.frame.basis,
                    entry.scale,
                );
                let mut input = super::initial_placed_model_query(
                    &descriptor.placement,
                    descriptor.geometry.bounds(),
                )
                .ok_or("world model has no prepared bounds query")?;
                input.bounds = bounds;
                input.region = crate::render_model::range_region(
                    bounds,
                    descriptor
                        .placement
                        .max_visible_range
                        .unwrap_or(crate::render_model::INITIAL_MODEL_MAX_VISIBLE_RANGE),
                );
                let viewport = [
                    (state.viewport[2] - state.viewport[0]) as u32,
                    (state.viewport[3] - state.viewport[1]) as u32,
                ];
                entry.area = crate::render_model::model_visible_area(
                    query,
                    state.camera_point,
                    viewport,
                    input,
                )?;
                visible &= entry.area.is_some();
            }
            models.set_visible(id, visible)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        error!("World model visibility publication failed: {error}");
        commands.write_message(AppExit::error());
        return;
    }
    if let Err(error) = owner.models.update(
        &[],
        &mut visibility,
        &mut transforms,
        scene
            .as_ref()
            .and_then(|scene| scene.environment_probes.as_deref()),
        lighting.as_deref_mut(),
        &mut patches,
        &mut buffers,
        &mut commands,
        &mut inverse,
        &mut joints,
        None,
        clock.as_deref(),
    ) {
        error!("World model pose publication failed: {error}");
        crate::platform::client_diagnostics::record(&format!(
            "World model pose publication failed: {error}"
        ));
        commands.write_message(AppExit::error());
    }
}
pub(super) fn observe_visibility(mut owner: ResMut<WorldModels>, views: Query<&ViewVisibility>) {
    let WorldModels {
        models, entries, ..
    } = &mut *owner;
    for (id, entry) in entries {
        entry.render.previous = entry.render.current;
        entry.render.current = models.view_visible(*id, &views);
        entry.render.hidden = !entry.visible;
    }
}
