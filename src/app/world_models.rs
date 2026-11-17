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
    ///Actual render-model+18bc, independent of the shared declaration bit.
    flags_18bc: u8,
    local_cast_shadows: bool,
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
    owners: std::collections::HashSet<Arc<str>>,
    lease: Arc<()>,
}
#[derive(Resource, Default)]
pub(super) struct WorldModels {
    entries: HashMap<EntityId, Entry>,
    prototypes: HashMap<usize, Prototype>,
    models: Models,
    retiring: HashMap<EntityId, RetiredModel>,
    retiring_resources: Vec<RetiredModel>,
    closing: bool,
}
impl WorldModels {
    /// Borrow the actual renderer pose and draw entities for action diagnostics.
    /// Requested Entry.frame alone does not prove the surface publisher ran.
    pub(super) fn publication_pose(&self, id: EntityId) -> Option<(ReferenceFrame, [f32; 3], bool)> {
        let entry = self.entries.get(&id)?;
        Some((self.models.frame(id)?, entry.scale,
            matches!(&entry.descriptor.geometry, crate::assets::equipment::attachment::Geometry::Rigid(_))))
    }
    pub(super) fn publication_draws(&self, id: EntityId) -> Option<impl Iterator<Item = Entity> + '_> {
        Some(self.models.projection_draws(id)?.map(|(entity, _, _, _, _)| entity))
    }
    pub(super) fn publication_visibility(&self, id: EntityId) -> Option<(bool, bool, Option<f32>)> {
        let entry = self.entries.get(&id)?;
        self.models.has(id).then_some((entry.visible, self.models.visible(id), entry.area))
    }
    pub(super) fn interaction_bounds(&self, id: EntityId) -> Option<[[f32; 3]; 2]> {
        let entry = self.entries.get(&id)?;
        Some(crate::render_model::transformed_bounds(entry.descriptor.geometry.bounds(), entry.frame.position, entry.frame.basis, entry.scale))
    }
    pub(super) fn interaction_visible(&self, id: EntityId) -> bool {
        self.entries.get(&id).is_some_and(|entry| entry.visible && !entry.render.hidden && self.models.has(id))
    }
    pub(super) fn publish_clock(&self,id:EntityId,bank:&mut super::model_motion::ModelProbeLighting,patches:&mut crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches,time:[f32;2]){
        if let Some(draws)=self.models.projection_draws(id){for (_,_,_,tag,_) in draws{if tag&0x80000000==0{bank.publish_clock(patches,tag,time);}}}
    }
    pub(super) fn projection_keys(&self)->impl Iterator<Item=EntityId>+'_ {self.entries.keys().copied()}
    pub(super) fn projection_input(&self,id:EntityId)->Option<([[f32;4];4],Vec<(usize,[[f32;3];2])>)>{
        let entry=self.entries.get(&id)?;
        let affine=crate::render_model::affine_rows(entry.frame.position,entry.frame.basis,entry.scale);
        let model=[affine[0],affine[1],affine[2],[0.,0.,0.,1.]];
        let local=match &entry.descriptor.geometry{
            crate::assets::equipment::attachment::Geometry::Rigid(source)=>source.surfaces.iter().map(|surface|surface.authored_bounds).collect::<Vec<_>>(),
            crate::assets::equipment::attachment::Geometry::Animated{model,..}=>model.surfaces.iter().map(|surface|{
                let mut bounds=[[f32::INFINITY;3],[f32::NEG_INFINITY;3]];
                for position in &surface.positions{for axis in 0..3{bounds[0][axis]=bounds[0][axis].min(position[axis]);bounds[1][axis]=bounds[1][axis].max(position[axis]);}}bounds
            }).collect(),
        };
        let surfaces=local.into_iter().enumerate().map(|(surface,bounds)|(surface,crate::render_model::transformed_bounds(bounds,entry.frame.position,entry.frame.basis,entry.scale))).collect();
        Some((model,surfaces))
    }
    pub(super) fn projection_receivers(&self,layouts:&std::collections::HashMap<bevy::asset::AssetId<Mesh>,u32>,materials:&super::ShadowMaterials,
        visibility:&Query<&ViewVisibility>,contexts:&super::projected_decals::ModelProjectionContexts,
        output:&mut Vec<crate::rendering::bevy_backend::projected_decals::Receiver>){
        for (&id,entry) in &self.entries{
            if !entry.visible{continue}
            let Some(draws)=self.models.projection_draws(id)else{continue};
            let bounds=crate::render_model::transformed_bounds(entry.descriptor.geometry.bounds(),entry.frame.position,entry.frame.basis,entry.scale);
            let affine=crate::render_model::affine_rows(entry.frame.position,entry.frame.basis,entry.scale);
            let rigid=super::view_geometry::game_affine_to_view(&std::array::from_fn(|i|affine[i/4][i%4])).to_matrix();
            let rows=contexts.rows(super::projected_decals::ModelKey::Actor(id)).cloned();
            for (entity,mesh,surface,probe,material) in draws{
                if !visibility.get(entity).is_ok_and(|visibility| visibility.get()){continue}
                let Some(&layout)=layouts.get(&mesh.id())else{continue};
                let Some(Ok(material))=materials.0.get(material)else{continue};
                output.push(crate::rendering::bevy_backend::projected_decals::Receiver{
                    entity:bevy::render::sync_world::MainEntity::from(entity),mesh:mesh.id(),
                    model:if matches!(&entry.descriptor.geometry,crate::assets::equipment::attachment::Geometry::Animated{..}){Mat4::IDENTITY}else{rigid},
                    probe,world_instance:None,opaque:material.source.main_blend_state&1==0,flags48:0,bounds,layout,surface,model_projections:rows.clone(),
                });
            }
        }
    }
    pub(super) fn shadow_bounds(&self) -> impl Iterator<Item = [[f32; 3]; 2]> + '_ {
        self.entries.values().filter(|entry| entry.visible && entry.flags_18bc&8!=0).map(|entry|
            crate::render_model::transformed_bounds(entry.descriptor.geometry.bounds(), entry.frame.position, entry.frame.basis, entry.scale))
    }
    pub(super) fn collect_shadow_casters(&self, materials: &super::ShadowMaterials, shader_data:&super::model_motion::ModelShaderData,
        inverse_view: &[[f32; 4]; 4], clip: &[f32; 16], cull_distance: f32,
        output: &mut Vec<super::shadow_updates::RuntimeSurfaceCaster>) -> Result<(), String> {
        for (&id, entry) in &self.entries {
            // Actual hidden state, independent of the main camera's visibility.
            if !entry.visible || entry.flags_18bc&8==0 { continue; }
            let Some((bounds, mut surfaces)) = self.models.shadow_surfaces(id, materials)? else { continue };
            let clock=shader_data.clock(super::model_motion::ModelKey::Actor(id));
            for surface in &mut surfaces{surface.clock=Some(clock);}
            let flags = crate::render_model::model_owner_bounds_flags(0,
                [bounds[0][0], bounds[0][1], bounds[0][2], 0.],
                [bounds[1][0], bounds[1][1], bounds[1][2], 0.]);
            let info = crate::render_model::prepare_model_caster_info(bounds, flags,
                entry.flags_18bc, 0,
                surfaces.iter().any(|surface| surface.skin.is_some()),
                surfaces.iter().map(|_| (1, 1)));
            let Some(sphere) = info.sphere else { continue };
            let Some(sort_distance) = crate::render_model::model_caster_sort_distance(info.flags,
                sphere, inverse_view, cull_distance) else { continue };
            output.push(super::shadow_updates::RuntimeSurfaceCaster {
                bounds, flags: info.flags, sort_distance,
                camera_clip_mask: crate::render_model::transformed_aabb_clip_cull(clip, bounds[0], bounds[1]).output_byte,
                surfaces,
                // bevy_backend: runtime actor poses are not kept in the baked
                // WORLD atlas cache. This prevents stale poses/deleted actors;
                // both buckets otherwise use identical source draw programs.
                dynamic: true,
            });
        }
        Ok(())
    }
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
    app.init_resource::<super::model_motion::ModelShaderData>();
    app.init_resource::<WorldModels>()
        .init_resource::<Uploads>()
        .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            Uploads,
        >::default());
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.add_systems(bevy::render::Render,
            storage_bindings_ready
                .after(bevy::render::render_asset::prepare_assets::<bevy::render::storage::GpuShaderBuffer>)
                .before(bevy::render::erased_render_asset::prepare_erased_assets::<MeshMaterial3d<super::ProbeMaterial>>)
                .before(bevy::render::erased_render_asset::prepare_erased_assets::<MeshMaterial3d<super::DecalMaterial>>));
        render.add_systems(
            bevy::render::Render,
            acknowledge.in_set(bevy::render::RenderSystems::PrepareResources),
        );
    }
}
// bevy_backend: storage replacement must precede dependent material binding.
// The two generic asset plugins otherwise have no dependency on each other.
fn storage_bindings_ready() {}
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if let Some(owner) = world.get_resource::<WorldModels>() {
        if !owner.entries.is_empty()
            || !owner.retiring.is_empty()
            || !owner.retiring_resources.is_empty()
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
            owners: Default::default(),
            lease: Arc::new(()),
        },
    );
    drop(owner);
    super::runtime_materials::invalidate_world_models(world);
    Ok(())
}
pub(super) fn prewarm_for(world: &mut World, scope: Arc<str>, descriptor: Arc<Descriptor>) -> Result<(), String> {
    prewarm(world, descriptor.clone())?;
    world.resource_mut::<WorldModels>().prototypes.get_mut(&key(&descriptor)).unwrap().owners.insert(scope);
    Ok(())
}
pub(super) fn release_prewarm_for(world: &mut World, scope: &str, descriptor: &Arc<Descriptor>) -> Result<(), String> {
    if let Some(mut owner) = world.get_resource_mut::<WorldModels>() {
        if let Some(prototype) = owner.prototypes.get_mut(&key(descriptor)) { prototype.owners.remove(scope); }
    }
    prune(world);
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
    request_validated(world, id, descriptor, frame)
}

/// The normal entity constructor can already hold the canonical live owner.
/// Validate that generation directly instead of moving it back into World.
pub(super) fn request_with_live(
    world: &mut World,
    live: &LiveEntities,
    id: EntityId,
    descriptor: Arc<Descriptor>,
    frame: ReferenceFrame,
) -> Result<(), String> {
    if live.get(id).is_none() {
        return Err("world model request has no live managed generation".into());
    }
    request_validated(world, id, descriptor, frame)
}

fn request_validated(
    world: &mut World,
    id: EntityId,
    descriptor: Arc<Descriptor>,
    frame: ReferenceFrame,
) -> Result<(), String> {
    prewarm(world, descriptor.clone())?;
    let mut owner = world.resource_mut::<WorldModels>();
    if let Some(entry) = owner.entries.get(&id) {
        if Arc::ptr_eq(&entry.descriptor, &descriptor) {
            return Ok(());
        }
        return Err("world generation owns another model descriptor".into());
    }
    let scale = descriptor.placement.scale;
    let local_cast_shadows=descriptor.cast_shadows;
    let flags_18bc=(crate::render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC&!8)|(u8::from(descriptor.cast_shadows)<<3);
    owner.entries.insert(
        id,
        Entry {
            descriptor,
            flags_18bc,
            local_cast_shadows,
            frame,
            scale,
            visible: true,
            area: None,
            render: RenderState::default(),
        },
    );
    drop(owner);
    if let Some(mut contexts)=world.get_resource_mut::<super::projected_decals::ModelProjectionContexts>(){contexts.dirty(super::projected_decals::ModelKey::Actor(id));}
    advance(world)
}
///2679a0 ->87bcb0/873350 replaces only the actual render model. Source
///prototypes are prepared before the firing callback; old draws retain their
///exact handles/SH/skin leases through the normal submitted-work receipt.
pub(super) fn replace(world:&mut World,id:EntityId,descriptor:Arc<Descriptor>,frame:ReferenceFrame)->Result<bool,String>{
    if world.get_resource::<LiveEntities>().and_then(|live|live.get(id)).is_none(){return Err("model replacement entity generation stale".into())}
    if !prototype_ready(world,&descriptor)?{return Ok(false)}
    let mut owner=world.remove_resource::<WorldModels>().ok_or("world model owner absent")?;
    let mut commands=super::setup_commands::SetupCommands::prepared(world);
    let result=(||{
        let entry=owner.entries.get(&id).ok_or("model replacement has no actual render owner")?;
        if Arc::ptr_eq(&entry.descriptor,&descriptor){
            owner.entries.get_mut(&id).unwrap().frame=frame;
            owner.models.set_world_frame(id,frame)?;
            return Ok(true)
        }
        let visible=entry.visible;let scale=entry.scale;let casts=entry.flags_18bc&8!=0;
        let template=owner.prototypes.get(&key(&descriptor)).and_then(|prototype|prototype.model.clone()).ok_or("ready replacement template absent")?;
        let mut prepared=template.instantiate(&mut commands)?;
        prepared.assign_probe_slots(None,world,&mut commands)?;
        let old=owner.models.remove(id,&mut commands).ok_or("replacement old draw generation absent")?;
        owner.retiring_resources.push(old);
        owner.models.publish_world(id,prepared,frame,visible)?;
        owner.models.set_shadow_casting(id,casts)?;
        for entity in owner.models.shadow_entities(id){commands.queue(move|world:&mut World|{
            if let Ok(mut entity)=world.get_entity_mut(entity){
                if casts{entity.remove::<bevy::light::NotShadowCaster>();}else{entity.insert(bevy::light::NotShadowCaster);}
            }
        });}
        owner.models.set_world_scale(id,scale)?;
        let entry=owner.entries.get_mut(&id).unwrap();
        entry.descriptor=descriptor;entry.frame=frame;entry.area=None;
        entry.render=RenderState{hidden:!visible,current:false,previous:false};
        //873350 copies18a0/18b0/depth/cast fields, not local projection rows
        //or1c2c/1c31 shader time. The fresh constructor owns their defaults.
        if let Some(mut contexts)=world.get_resource_mut::<super::projected_decals::ModelProjectionContexts>(){
            contexts.remove(super::model_motion::ModelKey::Actor(id));contexts.dirty(super::model_motion::ModelKey::Actor(id));
        }
        if let Some(mut data)=world.get_resource_mut::<super::model_motion::ModelShaderData>(){data.replace_model(super::model_motion::ModelKey::Actor(id));}
        Ok(true)
    })();
    if result.is_ok(){commands.publish(world);}
    world.insert_resource(owner);
    result
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
    let changed=self::frame(world,id).is_none_or(|old|old.position!=frame.position || old.basis!=frame.basis);
    if changed{if let Some(mut contexts)=world.get_resource_mut::<super::projected_decals::ModelProjectionContexts>(){contexts.dirty(super::projected_decals::ModelKey::Actor(id));}}
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
pub(super) fn set_visible(world: &mut World, id: EntityId, visible: bool) -> Result<bool, String> {
    let Some(frame) = self::frame(world, id) else { return Ok(false) };
    update(world, id, frame, visible)?;
    Ok(true)
}
///873340's wrapper and87e0a0's actual model flags remain per generation;
///the immutable descriptor supplies only the constructor's local default.
pub(super) fn shadow_state(world:&World,id:EntityId)->Option<(bool,u8)>{
    let entry=world.get_resource::<WorldModels>()?.entries.get(&id)?;
    Some((entry.local_cast_shadows,entry.flags_18bc))
}
pub(super) fn set_local_shadow_casting(world:&mut World,id:EntityId,value:bool)->bool{
    let Some(mut owner)=world.get_resource_mut::<WorldModels>()else{return false};
    let Some(entry)=owner.entries.get_mut(&id)else{return false};
    entry.local_cast_shadows=value;true
}
pub(super) fn publish_shadow_state(world:&mut World,id:EntityId,resolved:crate::render_model::InheritedModelDepthState)->Result<Option<bool>,String>{
    let Some(mut owner)=world.get_resource_mut::<WorldModels>()else{return Ok(None)};
    let Some(entry)=owner.entries.get_mut(&id)else{return Ok(None)};
    let before=entry.flags_18bc;
    let mut depth=0.;
    crate::render_model::publish_model_depth_state(&mut entry.flags_18bc,&mut depth,resolved);
    let casts=entry.flags_18bc&8!=0;
    let changed=(before^entry.flags_18bc)&8!=0;
    let entities=if owner.models.has(id){
        owner.models.set_shadow_casting(id,casts)?;
        if changed{owner.models.shadow_entities(id)}else{Vec::new()}
    }else{Vec::new()};
    drop(owner);
    for entity in entities{if let Ok(mut entity)=world.get_entity_mut(entity){
        if casts{entity.remove::<bevy::light::NotShadowCaster>();}else{entity.insert(bevy::light::NotShadowCaster);}
    }}
    Ok(Some(changed))
}
pub(super) fn set_scale(world: &mut World, id: EntityId, scale: [f32; 3]) -> Result<(), String> {
    if let Some(mut contexts)=world.get_resource_mut::<super::projected_decals::ModelProjectionContexts>(){contexts.dirty(super::projected_decals::ModelKey::Actor(id));}
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
/// Actual renderer model resource and current entity scale for tag-property
///selection (784e30/879890). No rig lookup, conversion or tag alias fallback.
pub(super) fn descriptor(world:&World,id:EntityId)->Option<Arc<Descriptor>>{
    Some(world.get_resource::<WorldModels>()?.entries.get(&id)?.descriptor.clone())
}
pub(super) fn tag_model<'w>(world:&'w World,live:&LiveEntities,id:EntityId)->Result<Option<(&'w str,[f32;3])>,String>{
    let (name,_)=live.get(id).ok_or("tag model entity generation stale")?;
    if let Some(entry)=world.get_resource::<WorldModels>().and_then(|owner|owner.entries.get(&id)){
        return Ok(Some((entry.descriptor.placement.resource_path.as_str(),entry.scale)));
    }
    let Some(placements)=world.get_resource::<super::RenderSourcePlacements>()else{return Ok(None)};
    let mut matches=placements.0.iter().filter(|placement|placement.id.eq_ignore_ascii_case(name));
    let Some(placement)=matches.next()else{return Ok(None)};
    if matches.any(|other|other.resource_path!=placement.resource_path || other.scale!=placement.scale){return Err("entity has multiple incompatible current tag model sources".into())}
    Ok(Some((placement.resource_path.as_str(),placement.scale)))
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
///878230/738bf0 query actual render-model world bounds independently of
///camera visibility. Placed resources use their existing authored model bounds
///and current placement transform, including physics/animation publication.
pub(super) fn entity_bounds(world: &World, live: &LiveEntities, id: EntityId) -> Result<Option<[[f32; 3]; 2]>, String> {
    if let Some(bounds)=self::bounds(world,id){return Ok(Some(bounds));}
    let placements=world.get_resource::<super::RenderSourcePlacements>().ok_or("entity bounds require the canonical placed model owner")?;
    entity_bounds_from_parts(live,id,None,world.get_resource::<super::shadow_updates::ModelShadowSources>(),placements)
}
///Same8780d0 owner when a gameplay/UI caller already borrows its actual parts.
pub(super) fn entity_bounds_from_parts(live:&LiveEntities,id:EntityId,runtime:Option<&WorldModels>,
    sources:Option<&super::shadow_updates::ModelShadowSources>,placements:&super::RenderSourcePlacements)
    ->Result<Option<[[f32;3];2]>,String> {
    if let Some(bounds)=runtime.and_then(|owner|owner.interaction_bounds(id)){return Ok(Some(bounds));}
    let (name, _) = live.get(id).ok_or("entity bounds generation is stale")?;
    if !placements.0.iter().any(|placement| placement.id.eq_ignore_ascii_case(name)) {
        return Ok(None); // Actual source entity has no authored render model.
    }
    let models=sources.ok_or("entity bounds require prepared model resource bounds")?;
    let mut merged = None::<[[f32; 3]; 2]>;
    for source in &models.0 {
        let Some(placement) = placements
            .0
            .get(source.placement)
            .filter(|placement| placement.id.eq_ignore_ascii_case(name))
        else {
            continue;
        };
        let bounds = crate::render_model::transformed_bounds(
            source.local_bounds,
            placement.position,
            placement.rotation,
            placement.scale,
        );
        if let Some(current) = &mut merged {
            for axis in 0..3 {
                current[0][axis] = current[0][axis].min(bounds[0][axis]);
                current[1][axis] = current[1][axis].max(bounds[1][axis]);
            }
        } else {
            merged = Some(bounds);
        }
    }
    if merged.is_none() { return Err("authored render model has no prepared resource bounds".into()); }
    Ok(merged)
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
pub(super) fn publish(world: &mut World, mut prepared: Prepared) -> Result<(), String> {
    validate(world, &prepared)?;
    let assets = prepared.model.uploads();
    let meshes = prepared.model.receiver_meshes();
    let lease = Arc::downgrade(&world.resource::<WorldModels>().prototypes[&prepared.key].lease);
    super::projected_decals::register_model_layouts(world,&meshes,lease);
    let mut owner = world.resource_mut::<WorldModels>();
    let prototype = owner.prototypes.get_mut(&prepared.key).unwrap();
    let complete = prototype.uploaded.clone();
    prepared.model.retain_lease(prototype.lease.clone());
    prototype.model = Some(Arc::new(prepared.model));
    drop(owner);
    Arc::make_mut(&mut world.resource_mut::<Uploads>().0).push(Upload { assets, complete });
    Ok(())
}
pub(super) fn add_leases(
    world: &World,
    keys: impl Iterator<Item = usize>,
    leases: &mut Vec<Weak<()>>,
) {
    let Some(owner) = world.get_resource::<WorldModels>() else {
        return;
    };
    for key in keys {
        if let Some(prototype) = owner.prototypes.get(&key) {
            if !leases
                .iter()
                .any(|lease| lease.ptr_eq(&Arc::downgrade(&prototype.lease)))
            {
                leases.push(Arc::downgrade(&prototype.lease));
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
pub(super) fn advance_after_materials(world: &mut World) {
    if let Err(error) = advance(world) {
        error!("World model admission failed: {error}");
        crate::platform::client_diagnostics::record(&format!("World model admission failed: {error}"));
        world.write_message(AppExit::error());
    }
    prune(world);
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
            let mut prepared = template.instantiate(&mut commands)?;
            prepared.assign_probe_slots(None, world, &mut commands)?;
            models.publish_world(id, prepared, entry.frame, entry.visible)?;
            let casts=entry.flags_18bc&8!=0;
            models.set_shadow_casting(id,casts)?;
            for entity in models.shadow_entities(id){commands.queue(move|world:&mut World|{
                if let Ok(mut entity)=world.get_entity_mut(entity){
                    if casts{entity.remove::<bevy::light::NotShadowCaster>();}else{entity.insert(bevy::light::NotShadowCaster);}
                }
            });}
            models.set_world_scale(id, entry.scale)?;
        }
        Ok(())
    })();
    commands.publish(world);
    world.insert_resource(owner);
    result
}
pub(super) fn remove(world: &mut World, id: EntityId) -> Result<(), String> {
    super::flash_surface_movie::retire_model(world,id)?;
    if let Some(mut contexts)=world.get_resource_mut::<super::projected_decals::ModelProjectionContexts>(){contexts.retire(super::projected_decals::ModelKey::Actor(id));}
    if let Some(mut data)=world.get_resource_mut::<super::model_motion::ModelShaderData>(){data.remove(super::model_motion::ModelKey::Actor(id));}
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
/// Retire an actual dynamic or map-placed renderer generation. Placed source
/// geometry remains owned by the map's scene registry; only the actor's draw
/// entities disappear, and the queue receipt gates deletion of its managed ID.
pub(super) fn remove_entity(world: &mut World, live: &LiveEntities, id: EntityId) -> Result<(), String> {
    if world.get_resource::<WorldModels>().is_some_and(|owner| owner.entries.contains_key(&id)) {
        return remove(world, id);
    }
    if world.get_resource::<WorldModels>().is_some_and(|owner| owner.retiring.contains_key(&id)) { return Ok(()) }
    super::flash_surface_movie::retire_model(world,id)?;
    let (name, _) = live.get(id).ok_or("render retirement targeted a stale entity generation")?;
    let name = name.to_owned();
    super::model_motion::set_entity_visible(world, live, id, false)?;
    let placements = world.resource::<super::RenderSourcePlacements>().0.iter().enumerate()
        .filter_map(|(index, placement)| placement.id.eq_ignore_ascii_case(&name).then_some(index)).collect::<Vec<_>>();
    if let Some(mut contexts)=world.get_resource_mut::<super::projected_decals::ModelProjectionContexts>(){
        for &placement in &placements{contexts.retire(super::projected_decals::ModelKey::Placed(placement));}
    }
    if let Some(mut data)=world.get_resource_mut::<super::model_motion::ModelShaderData>(){for &placement in &placements{data.remove(super::model_motion::ModelKey::Placed(placement));}}
    let mut entities = Vec::new();
    let mut handles = Vec::new();
    for instance in &mut world.resource_mut::<super::SceneRenderQueue>().instances {
        let Some(source) = instance.source else { continue };
        let placement = match source.surface {
            super::RenderModelSurface::Bmodel { placement, .. } | super::RenderModelSurface::Bmd6 { placement, .. } => placement,
            _ => continue,
        };
        if !placements.contains(&placement) { continue }
        if let Some(entity) = instance.entity.take() {
            entities.push(entity);
            handles.push((instance.mesh.clone(), instance.material.clone()));
        }
    }
    for entity in entities { if world.entities().contains(entity) { world.despawn(entity); } }
    if !handles.is_empty() {
        world.resource_mut::<WorldModels>().retiring.insert(id, RetiredModel::new(handles));
    }
    Ok(())
}
pub(super) fn retired_entity(world: &mut World, id: EntityId) -> bool { retired(world, id) }
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
        .chain(owner.retiring_resources.iter())
        .cloned()
}
pub(super) fn prune(world: &mut World) {
    let Some(mut owner) = world.remove_resource::<WorldModels>() else {
        return;
    };
    let before = owner.retiring_resources.len();
    owner
        .retiring_resources
        .retain(|receipt| !receipt.complete());
    let mut changed = before != owner.retiring_resources.len();
    let dead = owner
        .prototypes
        .iter()
        .filter_map(|(&key, prototype)| {
            (prototype.owners.is_empty() && !owner.entries.values().any(|entry| self::key(&entry.descriptor) == key))
                .then_some(key)
        })
        .collect::<Vec<_>>();
    for key in dead {
        if let Some(prototype) = owner.prototypes.remove(&key) {
            Arc::make_mut(&mut world.resource_mut::<Uploads>().0)
                .retain(|upload| !Arc::ptr_eq(&upload.complete, &prototype.uploaded));
            if let Some(model) = prototype.model {
                owner.retiring_resources.push(RetiredModel::new(model));
            }
            changed = true;
        }
    }
    world.insert_resource(owner);
    super::projected_decals::prune_model_layouts(world);
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
            owner.retiring_resources.push(RetiredModel::new(model));
        }
    }
    owner.retiring.retain(|_, receipt| !receipt.complete());
    owner
        .retiring_resources
        .retain(|receipt| !receipt.complete());
    let done = owner.retiring.is_empty() && owner.retiring_resources.is_empty();
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
    world_poses: Option<Res<super::physics_scene::PhysicsScene>>,
    mut model_bounds: Query<&mut bevy::camera::primitives::Aabb, With<Mesh3d>>,
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
        world_poses.as_deref(),
    ) {
        error!("World model pose publication failed: {error}");
        crate::platform::client_diagnostics::record(&format!(
            "World model pose publication failed: {error}"
        ));
        commands.write_message(AppExit::error());
    }
    owner.models.publish_animated_bounds(&mut model_bounds, &mut commands);
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
