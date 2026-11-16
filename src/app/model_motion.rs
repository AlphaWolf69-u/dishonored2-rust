//! Publish one physics pose consistently to color geometry and both shadow paths.
use super::{
    initial_placed_model_query, placed_transform, render_bounds, render_model,
    shadow_updates::ModelShadowSources, PlacedModelQueries, RenderSourcePlacements,
    SceneRenderQueue, ShadowModelBoundsSources,
};
use bevy::prelude::*;
use hashbrown::HashMap;
use crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches;

#[path = "model_shader_data.rs"]
mod shader_data;
pub(super) use shader_data::{ModelShaderData, publish_shader_clocks, set_shader_clock, set_shader_clock_with_live, set_in_water};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ModelKey { Placed(usize), Actor(crate::gameplay::entity_lifetime::EntityId) }

/// Source Hide/Show changes actual renderer visibility without removing the
/// rigid body or reacquiring a model. Placed sources retain their map owner.
pub(super) fn set_entity_visible(world: &mut World,
    live: &crate::gameplay::entity_lifetime::LiveEntities,
    id: crate::gameplay::entity_lifetime::EntityId, visible: bool) -> Result<(), String> {
    let (name, _) = live.get(id).ok_or("model visibility targeted a stale entity generation")?;
    if super::world_models::set_visible(world, id, visible)? { return Ok(()) }
    let placements = world.get_resource::<super::RenderSourcePlacements>().map(|placements|
        placements.0.iter().enumerate().filter_map(|(index, placement)|
            placement.id.eq_ignore_ascii_case(name).then_some(index)).collect::<Vec<_>>()).unwrap_or_default();
    if placements.is_empty() { return Ok(()) } // Actual non-render entity.
    {
        let mut hidden = world.resource_mut::<super::interaction_focus::RetiredPlacements>();
        for &placement in &placements { if visible { hidden.0.remove(&placement); } else { hidden.0.insert(placement); } }
    }
    if !visible {
        let entities = if let Some(mut queue) = world.get_resource_mut::<super::SceneRenderQueue>() {
            queue.instances.iter_mut().filter_map(|instance| {
                let source = instance.source?;
                let placement = match source.surface {
                    super::RenderModelSurface::Bmodel { placement, .. } | super::RenderModelSurface::Bmd6 { placement, .. } => placement,
                    _ => return None,
                };
                if !placements.contains(&placement) { return None }
                instance.visible = false;
                instance.entity
            }).collect::<Vec<_>>()
        } else { Vec::new() };
        for entity in entities { if let Some(mut visibility) = world.get_mut::<Visibility>(entity) { *visibility = Visibility::Hidden; } }
    }
    // Source masks, shadow bounds and Tome/color publication all consume the
    // existing placed-hidden set; Show restores the next real query result.
    Ok(())
}

///87b6b0→873340→87e040/87e0a0. The wrapper's local cast bit is
///instance state, not a change to shared mesh/material declarations. Pending
///world models retain the same request until their real upload is admitted.
pub(super) fn set_entity_shadow_casting(
    world:&mut World, live:&crate::gameplay::entity_lifetime::LiveEntities,
    root:crate::gameplay::entity_lifetime::EntityId, value:bool,
)->Result<(),String>{
    let keys=shader_data::keys(world,live,root)?.into_iter().filter(|&key|shadow_depth_input(world,key).is_some()).collect::<Vec<_>>();
    if keys.is_empty(){return Ok(())} //No actual render-owner wrapper.
    let first=shadow_depth_input(world,keys[0]).ok_or("shadow property lost its actual model owner")?;
    let local=world.get_resource::<ModelShaderData>().and_then(|data|data.local_cast_shadows(keys[0])).unwrap_or(first.0);
    if local==value{return Ok(())} //87b6b0 refreshes only on local-bit change.
    for key in keys{
        if let ModelKey::Actor(id)=key{
            if super::world_models::set_local_shadow_casting(world,id,value){continue}
        }
        world.resource_mut::<ModelShaderData>().set_local_cast_shadows(key,value);
    }
    let parent=live.parent(root)?;
    let parent_model=parent.map(|parent|->Result<_,String>{
        let keys=shader_data::keys(world,live,parent)?;
        let model=keys.first().and_then(|&key|shadow_depth_input(world,key)).map(|(_,flags,depth,_)|(flags,depth));
        Ok((0,model))
    }).transpose()?;
    let inherited=render_model::parent_model_depth_state(parent_model);
    let mut stack=vec![(root,inherited)];
    let mut visited=std::collections::HashSet::new();
    while let Some((id,inherited))=stack.pop(){
        if !visited.insert(id){return Err("cyclic live model-depth hierarchy".into())}
        let keys=shader_data::keys(world,live,id)?.into_iter().filter(|&key|shadow_depth_input(world,key).is_some()).collect::<Vec<_>>();
        let input=keys.first().and_then(|&key|shadow_depth_input(world,key));
        let local=keys.first().and_then(|&key|world.get_resource::<ModelShaderData>().and_then(|data|data.local_cast_shadows(key)))
            .unwrap_or_else(||input.map_or(true,|input|input.0));
        let local_depth=input.map_or(0.,|input|input.3);
        let resolved=render_model::resolve_inherited_model_depth_state(inherited,local_depth,0,u8::from(local));
        for key in keys{publish_shadow_depth(world,key,resolved)?;}
        //Source ordered recursion propagates through model-less children too.
        for &child in live.children(id)?.iter().rev(){stack.push((child,resolved));}
    }
    Ok(())
}

///(constructor local bit, actual flags18bc, resolved depth, local depth).
fn shadow_depth_input(world:&World,key:ModelKey)->Option<(bool,u8,f32,f32)>{
    let (local,mut flags,mut depth,local_depth)=match key{
        ModelKey::Placed(placement)=>{
            let source=world.get_resource::<ModelShadowSources>()?.0.iter().find(|source|source.placement==placement)?;
            //874c50's local depth starts at zero; these placed source models
            //are constructed with INITIAL_MODEL_CONTEXT_FLAGS_18BC.
            (true,source.flags_18bc,0.,0.)
        }
        ModelKey::Actor(id)=>{
            if let Some((local,flags))=super::world_models::shadow_state(world,id){(local,flags,0.,0.)}
            else{
                let arms=world.get_resource::<super::player_arms::PlayerArms>()?;
                let(flags,depth)=arms.managed_model_depth_state(id)?;
                (false,flags,depth,arms.managed_model_local_depth_hack(id))
            }
        }
    };
    if let Some(data)=world.get_resource::<ModelShaderData>(){
        if let Some(casts)=data.cast_shadows(key){flags=(flags&!8)|(u8::from(casts)<<3);}
        if let Some(value)=data.depth_hack(key){depth=value;}
    }
    Some((local,flags,depth,local_depth))
}

fn publish_shadow_depth(world:&mut World,key:ModelKey,resolved:render_model::InheritedModelDepthState)->Result<(),String>{
    let Some((_,before,mut depth,_))=shadow_depth_input(world,key)else{return Ok(())};
    let previous_depth=depth;
    let mut flags=before;
    render_model::publish_model_depth_state(&mut flags,&mut depth,resolved);
    let casts=flags&8!=0;
    let changed=(before^flags)&8!=0;
    let dynamic=match key{
        ModelKey::Placed(placement)=>{
            if let Some(mut sources)=world.get_resource_mut::<ModelShadowSources>(){
                for source in sources.0.iter_mut().filter(|source|source.placement==placement){
                    render_model::publish_model_depth_state(&mut source.flags_18bc,&mut depth,resolved);
                }
            }
            if changed{
                let entities=world.get_resource::<SceneRenderQueue>().map(|queue|queue.instances.iter().filter_map(|instance|{
                    let matches=instance.source.is_some_and(|source|match source.surface{
                        super::RenderModelSurface::Bmodel{placement:actual,..}|super::RenderModelSurface::Bmd6{placement:actual,..}=>actual==placement,
                        _=>false,
                    });
                    matches.then_some(instance.entity).flatten()
                }).collect::<Vec<_>>()).unwrap_or_default();
                for entity in entities{if let Ok(mut entity)=world.get_entity_mut(entity){
                    if casts{entity.remove::<bevy::light::NotShadowCaster>();}else{entity.insert(bevy::light::NotShadowCaster);}
                }}
            }
            render_model::model_is_dynamic_caster(flags)
        }
        ModelKey::Actor(id)=>{
            if super::world_models::publish_shadow_state(world,id,resolved)?.is_none(){
                let entities=world.get_resource::<super::player_arms::PlayerArms>().map(|arms|arms.managed_model_shadow_entities(id)).unwrap_or_default();
                if let Some(mut arms)=world.get_resource_mut::<super::player_arms::PlayerArms>(){
                    if arms.attachments.has(id){arms.attachments.set_shadow_casting(id,casts)?;}
                }
                if changed{for entity in entities{if let Ok(mut entity)=world.get_entity_mut(entity){
                    if casts{entity.remove::<bevy::light::NotShadowCaster>();}else{entity.insert(bevy::light::NotShadowCaster);}
                }}}
            }
            //WorldModels already gathers runtime instances in its dynamic
            //bucket, including rigid actors; preserve that host contract.
            true
        }
    };
    if changed||depth!=previous_depth{
        let mut data=world.resource_mut::<ModelShaderData>();
        data.publish_cast_shadows(key,casts);
        data.publish_depth_hack(key,depth);
    }
    if let Some(mut controls)=world.get_resource_mut::<super::local_shadow_updates::LocalShadowViewControls>(){
        controls.0.actor_caster_changed(changed,dynamic);
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct ModelPose {
    pub placement: usize,
    /// Z-up rigid world delta from the body's initial pose. Geometry may have
    /// additional authored scale, shear or a blueprint offset from that body.
    pub delta: [[f32; 4]; 3],
}

#[derive(Resource, Default)]
pub(super) struct ModelPoseUpdates(pub Vec<ModelPose>);

/// Entities placed from a bind parent every frame (1407506d0): the child
/// keeps its spawn transform relative to the parent (or the parent's tag
/// joint, rigid with a rigidly moved parent), so it takes the parent's world
/// delta. Children of a child follow transitively.
#[derive(Resource, Default)]
pub(super) struct BoundModels {
    binds: Vec<crate::entities::EntityBind>,
    /// Bound child placements by parent placement, resolved on first use.
    children: Option<HashMap<usize, Vec<usize>>>,
    /// The same rigid owner deltas, including bound entities without meshes.
    entity_deltas: HashMap<String, [[f32; 4]; 3]>,
}

impl BoundModels {
    pub(super) fn new(binds: Vec<crate::entities::EntityBind>) -> Self {
        Self { binds, children: None, entity_deltas: HashMap::new() }
    }

    pub(super) fn entity_delta(&self, id: &str) -> Option<&[[f32; 4]; 3]> { self.entity_deltas.get(id) }

    ///Claimed once during level preparation, after actual actor admission.
    ///808880's canonical binding owner publishes these final sampled frames;
    ///7506d0's legacy parent-delta adapter must not publish them as well.
    pub(super) fn exclude_canonical_bindings<'a>(&mut self, entities: impl Iterator<Item = &'a str>) {
        let entities = entities.collect::<hashbrown::HashSet<_>>();
        self.binds.retain(|binding| !entities.contains(binding.entity_id.as_str()));
    }

    fn children(&mut self, placements: &[crate::entities::PlacedModel]) -> &HashMap<usize, Vec<usize>> {
        self.children.get_or_insert_with(|| {
            let mut by_id = HashMap::<&str, Vec<usize>>::new();
            for (index, placement) in placements.iter().enumerate() {
                by_id.entry(placement.id.as_str()).or_default().push(index);
            }
            let mut children = HashMap::<usize, Vec<usize>>::new();
            for bind in &self.binds {
                let (Some(parents), Some(bound)) = (by_id.get(bind.parent.as_str()), by_id.get(bind.entity_id.as_str()))
                else {
                    continue;
                };
                for &parent in parents {
                    children.entry(parent).or_default().extend(bound);
                }
            }
            children
        })
    }

    /// Add the poses of children whose nearest posed ancestor moved.
    pub(super) fn propagate(&mut self, poses: &mut Vec<ModelPose>, placements: &[crate::entities::PlacedModel]) {
        // 7506d0 carries the parent's rigid world delta to bound children.
        // Retain it by entity ID as well as placement so non-render owners
        // (including audio targets) participate in the same propagation.
        let mut entity_stack = poses.iter().filter_map(|pose| placements.get(pose.placement)
            .map(|placement| (placement.id.clone(), pose.delta))).collect::<Vec<_>>();
        let mut moved = entity_stack.iter().map(|(id, _)| id.clone()).collect::<hashbrown::HashSet<_>>();
        while let Some((id, delta)) = entity_stack.pop() {
            self.entity_deltas.insert(id.clone(), delta);
            for bind in self.binds.iter().filter(|bind| bind.parent == id) {
                if moved.insert(bind.entity_id.clone()) { entity_stack.push((bind.entity_id.clone(), delta)); }
            }
        }
        let children = self.children(placements);
        let mut posed: hashbrown::HashSet<usize> = poses.iter().map(|p| p.placement).collect();
        let mut stack: Vec<ModelPose> = poses.clone();
        while let Some(pose) = stack.pop() {
            for &child in children.get(&pose.placement).into_iter().flatten() {
                if posed.insert(child) {
                    let child = ModelPose { placement: child, delta: pose.delta };
                    poses.push(child);
                    stack.push(child);
                }
            }
        }
    }
}

struct SurfaceBinding {
    index: usize,
    /// World-baked vertices retain their initial pose. Only their rigid delta
    /// changes: no new meshes, vertex copies or per-frame asset allocations.
    baked: bool,
    initial_transform: Transform,
    initial_bounds: bevy::camera::primitives::Aabb,
}

struct Binding {
    initial_position: [f32; 3],
    initial_rotation: [[f32; 3]; 3],
    source: usize,
    shadow_bounds: Option<usize>,
    query: Option<usize>,
    surfaces: Vec<SurfaceBinding>,
    lighting: crate::probe_lighting::ProbeQuery,
    /// The last query's position bits and result, kept only when that query
    /// left `lighting` unchanged: the same position from the same state
    /// gives the same result, so a model that has not moved skips the walk.
    lighting_memo: Option<([u32; 3], crate::probe_lighting::ProbeQuery,
        Option<crate::probe_lighting::InterpolatedProbeLighting>)>,
}

#[derive(Resource)]
pub(super) struct ModelProbeLighting {
    handle: Handle<super::ShaderBuffer>,
    first: usize,
    records: Option<Vec<ProbeRecord>>,
    next: usize,
    dynamic_start: usize,
    capacity: usize,
    returned: std::sync::Arc<std::sync::Mutex<Vec<u32>>>,
    free: Vec<u32>,
    shader_clocks: HashMap<u32,[f32;2]>,
    default_shader_clock: Option<[f32;2]>,
}

/// Host SH-bank allocation corresponding to a model's shader-bank entry
/// (31ad30/329120). The owning draw survives in RetiredModel until GPU ack;
/// only then may a different actor reuse this row.
pub(super) struct ProbeSlot {
    index: u32,
    returned: std::sync::Arc<std::sync::Mutex<Vec<u32>>>,
}
impl ProbeSlot { pub(super) fn index(&self) -> u32 { self.index } }
impl Drop for ProbeSlot {
    fn drop(&mut self) { self.returned.lock().unwrap().push(self.index); }
}

type ProbeRecord = [u8; 9 * 16];

impl ModelProbeLighting {
    /// F8-only view of the latest CPU publication, not a GPU readback.
    pub(super) fn diagnostic_record(&self, buffers: &Assets<super::ShaderBuffer>, index: u32) -> Option<[[f32; 4]; 9]> {
        let index = index as usize;
        let bytes: &[u8] = if let Some(records) = &self.records {
            records.get(index.checked_sub(self.first)?)?
        } else {
            buffers.get(&self.handle)?.data.as_deref()?.get(index * 144..index * 144 + 144)?
        };
        Some(std::array::from_fn(|row| std::array::from_fn(|column| {
            let at = row * 16 + column * 4;
            f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
        })))
    }
    pub(super) fn asset_id(&self) -> AssetId<super::ShaderBuffer> { self.handle.id() }

    pub(super) fn new(handle: Handle<super::ShaderBuffer>, first: usize, buffer: &mut super::ShaderBuffer) -> Self {
        let retain_full = std::env::var_os("DH2_DEBUG_RETAIN_PROBE_RECORDS").is_some_and(|value| value == "1");
        // bevy_backend: only the final unused SH alpha lane holds model time.
        // Other alpha lanes retain reflection and rigid-water inputs. NaN is
        // the host default-clock selector; no lighting RGB component changes.
        for record in buffer.data.as_mut().expect("instance probe upload prepared")[first*144..].chunks_exact_mut(144) {
            record[140..144].copy_from_slice(&0x7fc00000u32.to_le_bytes());
        }
        let bytes = buffer.data.as_deref().expect("instance probe upload prepared");
        let records = (!retain_full).then(|| {
            bytes[first * size_of::<ProbeRecord>()..].chunks_exact(size_of::<ProbeRecord>())
                .map(|record| record.try_into().expect("probe record extent"))
                .collect::<Vec<ProbeRecord>>()
        });
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            let mut crc = flate2::Crc::new();
            crc.update(bytes);
            let retained = records.as_ref().map_or(bytes.len(), |records| records.capacity() * size_of::<ProbeRecord>());
            eprintln!("INSTANCE_PROBE_RECORDS bytes={} retained_cpu_bytes={retained} crc32={:08x}", bytes.len(), crc.sum());
        }
        let next = bytes.len() / size_of::<ProbeRecord>();
        if records.is_some() {
            buffer.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
        }
        Self { handle, first, records, next, dynamic_start: next, capacity: next,
            returned: Default::default(), free: Vec::new(), shader_clocks:HashMap::new(), default_shader_clock:None }
    }

    /// Allocate only at admission. Growth copies the existing GPU bank; it
    /// does not reconstruct static records or re-upload their CPU payload.
    fn allocate(&mut self, buffers: &mut Assets<super::ShaderBuffer>,
        patches: &mut ShaderBufferPatches, mut record: ProbeRecord) -> Result<(ProbeSlot, bool), String> {
        self.free.extend(self.returned.lock().unwrap().drain(..));
        let index = if let Some(index) = self.free.pop() { index as usize } else {
            let index = self.next;
            if index >= 0x8000_0000 { return Err("model probe bank exhausted its tag index".into()); }
            self.next += 1;
            if let Some(records) = &mut self.records { records.push([0; 144]); }
            index
        };
        self.shader_clocks.remove(&(index as u32));
        record[140..144].copy_from_slice(&0x7fc00000u32.to_le_bytes());
        let grew = self.next > self.capacity;
        if grew {
            // bevy_backend: amortized capacity, preserving the game's one bank
            // and the exact per-draw record contents across host buffer changes.
            let dynamic = (self.next - self.dynamic_start).checked_next_power_of_two().ok_or("model probe bank size overflow")?;
            self.capacity = self.dynamic_start.checked_add(dynamic).ok_or("model probe bank size overflow")?;
            let bytes = self.capacity.checked_mul(size_of::<ProbeRecord>()).ok_or("model probe bank byte overflow")?;
            buffers.get_mut(&self.handle).ok_or("model probe buffer missing")?.resize_in_place(bytes as u64);
        }
        if let Some(records) = &mut self.records { records[index - self.first] = record; }
        else if let Some(mut buffer) = buffers.get_mut(&self.handle) {
            if let Some(data) = &mut buffer.data { data[index * 144..index * 144 + 144].copy_from_slice(&record); }
        }
        patches.enqueue(self.handle.id(), index * 144, &record);
        Ok((ProbeSlot { index: index as u32, returned: self.returned.clone() }, grew))
    }

    pub(super) fn allocate_draws(world: &mut World, records: Vec<ProbeRecord>) -> Result<Vec<ProbeSlot>, String> {
        let (slot, grew, handle) = world.resource_scope(|world, mut bank: Mut<Self>| {
            world.resource_scope(|world, mut buffers: Mut<Assets<super::ShaderBuffer>>| {
                let mut patches = world.resource_mut::<ShaderBufferPatches>();
                let mut slots = Vec::with_capacity(records.len());
                let mut grew = false;
                for record in records {
                    let (slot, growth) = bank.allocate(&mut buffers, &mut patches, record)?;
                    slots.push(slot);
                    grew |= growth;
                }
                Ok::<_, String>((slots, grew, bank.handle.id()))
            })
        })?;
        if grew {
            // The storage handle stays stable, but its GPU buffer changed.
            // Bevy material extraction must rebuild bind groups referencing it.
            let mut probes = world.resource_mut::<Assets<super::ProbeMaterial>>();
            let ids = probes.iter().filter_map(|(id, material)| (material.extension.sh.id() == handle).then_some(id)).collect::<Vec<_>>();
            for id in ids { if let Some(material) = probes.get_mut(id) { let _ = material.into_inner(); } }
            drop(probes);
            let mut decals = world.resource_mut::<Assets<super::DecalMaterial>>();
            let ids = decals.iter().filter_map(|(id, material)| (material.extension.sh.id() == handle).then_some(id)).collect::<Vec<_>>();
            for id in ids { if let Some(material) = decals.get_mut(id) { let _ = material.into_inner(); } }
        }
        Ok(slot)
    }

    pub(super) fn shader_clock(&self,index:u32,fallback:[f32;2])->[f32;2]{self.shader_clocks.get(&index).copied().unwrap_or(fallback)}
    pub(super) fn publish_default_clock(&mut self,patches:&mut ShaderBufferPatches,time:[f32;2]){
        if self.default_shader_clock.map(|old|old[0].to_bits())!=Some(time[0].to_bits()){
            patches.enqueue(self.handle.id(),140,&time[0].to_le_bytes());
        }
        self.default_shader_clock=Some(time);
    }
    pub(super) fn publish_clock(&mut self,patches:&mut ShaderBufferPatches,index:u32,time:[f32;2]){
        let changed=self.shader_clocks.insert(index,time).is_none_or(|old|old[0].to_bits()!=time[0].to_bits());
        if !changed{return}
        if let Some(records)=&mut self.records{
            if let Some(record)=(index as usize).checked_sub(self.first).and_then(|index|records.get_mut(index)){
                record[140..144].copy_from_slice(&time[0].to_le_bytes());
            }
        }
        patches.enqueue(self.handle.id(),index as usize*144+140,&time[0].to_le_bytes());
    }
    pub(super) fn write(&mut self, patches: &mut ShaderBufferPatches,
        buffers: &mut Assets<super::ShaderBuffer>, index: u32, record: &[u8; 9 * 16]) {
        let mut host_record=*record;
        host_record[140..144].copy_from_slice(&self.shader_clocks.get(&index).map_or(0x7fc00000,|time|time[0].to_bits()).to_le_bytes());
        let record=&host_record;
        let index = index as usize;
        let offset = index * record.len();
        let Some(records) = &mut self.records else {
            if buffers.get(&self.handle).is_some_and(|buffer| buffer.data.is_none()) {
                patches.enqueue(self.handle.id(), offset, record);
            } else {
                patches.write(buffers, &self.handle, offset, record);
            }
            return;
        };
        let Some(target) = index.checked_sub(self.first).and_then(|index| records.get_mut(index)) else { return; };
        if target != record {
            *target = *record;
            patches.enqueue(self.handle.id(), offset, record);
        }
    }
}

/// One object's nine SH coefficient rows as the shared probe buffer stores them.
pub(super) fn probe_record_bytes(record: [[f32; 4]; 9]) -> [u8; 9 * 16] {
    let mut bytes = [0; 9 * 16];
    for (word, value) in bytes.chunks_exact_mut(4).zip(record.iter().flatten()) {
        word.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[derive(Resource, Default)]
pub(super) struct ModelMotionBindings(HashMap<usize, Binding>);

#[cfg(test)]
fn rigid_matrix(position: [f32; 3], rotation: [[f32; 3]; 3]) -> Mat4 {
    let rows = render_model::affine_rows(position, rotation, [1.; 3]);
    super::game_affine_to_view(&std::array::from_fn(|i| rows[i / 4][i % 4])).to_matrix()
}

fn moved_bounds(
    bounds: bevy::camera::primitives::Aabb,
    delta: Mat4,
) -> bevy::camera::primitives::Aabb {
    let center = Vec3::from(bounds.center);
    let half = Vec3::from(bounds.half_extents);
    let corners: [Vec3; 8] = std::array::from_fn(|i| {
        let sign = Vec3::new(
            if i & 1 == 0 { -1. } else { 1. },
            if i & 2 == 0 { -1. } else { 1. },
            if i & 4 == 0 { -1. } else { 1. },
        );
        delta.transform_point3(center + half * sign)
    });
    render_bounds(&corners)
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct ModelPosePublication<'w, 's> {
    pending: ResMut<'w, ModelPoseUpdates>,
    bindings: ResMut<'w, ModelMotionBindings>,
    placements: ResMut<'w, RenderSourcePlacements>,
    models: ResMut<'w, ModelShadowSources>,
    bounds: ResMut<'w, ShadowModelBoundsSources>,
    queries: ResMut<'w, PlacedModelQueries>,
    queue: ResMut<'w, SceneRenderQueue>,
    entities: Query<'w, 's, (&'static mut Transform, &'static mut GlobalTransform)>,
    scene: Option<Res<'w, super::SceneData>>,
    probe_buffer: Option<ResMut<'w, ModelProbeLighting>>,
    buffers: Option<ResMut<'w, Assets<super::ShaderBuffer>>>,
    patches: ResMut<'w, crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches>,
    bound: Option<ResMut<'w, BoundModels>>,
    effect_frames: Option<ResMut<'w, crate::rendering::bevy_backend::world_effects::WorldEffectFrames>>,
    clock: Option<Res<'w, crate::gameplay::game_clock_runtime::ClockRuntime>>,
    model_projections: Option<ResMut<'w, super::projected_decals::ModelProjectionContexts>>,
}

pub(super) fn publish_model_poses(publication: ModelPosePublication<'_, '_>) { publish(publication); }

/// 808880 runs after the final skeletal pose producer. Its changed child
/// deltas use the same publication owner; empty batches touch no asset tables.
pub(super) fn publish_bound_model_poses(publication: ModelPosePublication<'_, '_>) {
    if publication.pending.0.is_empty() { return; }
    publish(publication);
}

fn publish(publication: ModelPosePublication<'_, '_>) {
    let ModelPosePublication { mut pending, mut bindings, mut placements, mut models, mut bounds,
        mut queries, mut queue, mut entities, scene, mut probe_buffer, mut buffers, mut patches,
        bound, mut effect_frames, clock, mut model_projections } = publication;
    if let Some(frames) = effect_frames.as_mut() {
        frames.clock = clock.as_ref().and_then(|clock| clock.clock().ok()).and_then(|clock| {
            Some(crate::rendering::bevy_backend::world_effects::WorldFxClock {
                time: clock.time(3)?, delta: clock.delta(2)?, scale: clock.rates(3)?[1],
                entity_delta: clock.delta(3)?,
            })
        });
    }
    if let Some(mut bound) = bound {
        bound.propagate(&mut pending.0, &placements.0);
    }
    for pose in pending.0.drain(..) {
        let Some(placement) = placements.0.get_mut(pose.placement) else {
            error!(
                "Physics pose references missing placement {}",
                pose.placement
            );
            continue;
        };
        if !bindings.0.contains_key(&pose.placement) {
            let Some(source) = models.0.iter().position(|s| s.placement == pose.placement) else {
                continue;
            };
            let range = models.0[source].instances.clone();
            let surfaces = range
                .clone()
                .map(|index| {
                    let instance = &queue.instances[index];
                    SurfaceBinding {
                        index,
                        baked: instance.source.is_some_and(|source| source.world_baked),
                        initial_transform: instance.transform,
                        initial_bounds: instance.bounds,
                    }
                })
                .collect();
            let mut lighting = crate::probe_lighting::ProbeQuery::default();
            if let Some(probes) = scene.as_ref().and_then(|s| s.environment_probes.as_ref()) {
                let bounds = models.0[source].bounds;
                let _ = probes.interpolate_query_sh(
                    std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5),
                    &mut lighting,
                );
            }
            bindings.0.insert(
                pose.placement,
                Binding {
                    initial_position: placement.position,
                    initial_rotation: placement.rotation,
                    source,
                    shadow_bounds: bounds.0.iter().position(|b| b.placement == pose.placement),
                    query: queries.0.iter().position(|q| q.indices == range),
                    surfaces,
                    lighting,
                    lighting_memo: None,
                },
            );
        }
        let binding = bindings.0.get_mut(&pose.placement).unwrap();
        let rows = std::array::from_fn(|i| pose.delta[i / 4][i % 4]);
        let delta = super::game_affine_to_view(&rows).to_matrix();
        placement.position = std::array::from_fn(|r| {
            pose.delta[r][3]
                + (0..3)
                    .map(|c| pose.delta[r][c] * binding.initial_position[c])
                    .sum::<f32>()
        });
        placement.rotation = std::array::from_fn(|c| {
            std::array::from_fn(|r| {
                (0..3)
                    .map(|i| pose.delta[r][i] * binding.initial_rotation[c][i])
                    .sum::<f32>()
            })
        });
        if let Some(frames) = effect_frames.as_mut() {
            frames.publish_delta(&placement.id, pose.delta);
        }
        let source = &mut models.0[binding.source];
        if let Some(contexts)=model_projections.as_deref_mut(){contexts.dirty(super::projected_decals::ModelKey::Placed(source.placement));}
        source.bounds = render_model::transformed_bounds(
            source.local_bounds,
            placement.position,
            placement.rotation,
            placement.scale,
        );
        source.flags_1c30 = render_model::model_owner_bounds_flags(
            source.flags_1c30,
            [
                source.bounds[0][0],
                source.bounds[0][1],
                source.bounds[0][2],
                0.,
            ],
            [
                source.bounds[1][0],
                source.bounds[1][1],
                source.bounds[1][2],
                0.,
            ],
        );
        render_model::set_model_dynamic_caster_flag(&mut source.flags_18bc, 1);
        let lighting = scene
            .as_ref()
            .and_then(|s| s.environment_probes.as_ref())
            .and_then(|probes| {
                let position: [f32; 3] =
                    std::array::from_fn(|i| (source.bounds[0][i] + source.bounds[1][i]) * 0.5);
                let key = position.map(f32::to_bits);
                if let Some((memo_key, state, lighting)) = &binding.lighting_memo {
                    if *memo_key == key && *state == binding.lighting {
                        return lighting.clone();
                    }
                }
                let before = binding.lighting;
                let lighting = probes.interpolate_query_sh(position, &mut binding.lighting);
                binding.lighting_memo =
                    (binding.lighting == before).then(|| (key, before, lighting.clone()));
                lighting
            });
        if let (Some(lighting), Some(handle), Some(buffers)) =
            (lighting, probe_buffer.as_mut(), buffers.as_mut())
        {
            let record = probe_record_bytes(lighting.object_shader_record());
            for surface in &binding.surfaces {
                let tag = queue.instances[surface.index].probe_index;
                if tag & 0x80000000 == 0 {
                    handle.write(&mut patches, buffers, tag, &record);
                }
            }
        }
        if let Some(index) = binding.shadow_bounds {
            bounds.0[index].bounds = source.bounds;
        }
        if let Some(index) = binding.query {
            if let Some(input) = initial_placed_model_query(placement, source.local_bounds) {
                queries.0[index].input = input;
            }
        }
        for surface in &binding.surfaces {
            let instance = &mut queue.instances[surface.index];
            instance.transform = if surface.baked {
                Transform::from_matrix(delta * surface.initial_transform.to_matrix())
            } else {
                placed_transform(placement)
            };
            instance.bounds = moved_bounds(surface.initial_bounds, delta);
            if let Some(entity) = instance.entity {
                if let Ok((mut transform, mut global)) = entities.get_mut(entity) {
                    *transform = instance.transform;
                    *global = GlobalTransform::from(instance.transform);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_children_take_the_nearest_posed_ancestor_delta() {
        let placement = |id: &str| crate::entities::PlacedModel {
            id: id.into(),
            resource_path: String::new(),
            position: [0.; 3],
            scale: [1.; 3],
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            custom_materials: Vec::new(),
            max_visible_range: None,
        };
        let placements = ["turnbuckle", "speaker", "grille", "lamp", "other"].map(placement);
        let bind = |child: &str, parent: &str| crate::entities::EntityBind {
            entity_id: child.into(),
            parent: parent.into(),
            tag: None,
        };
        let mut bound = BoundModels::new(vec![
            bind("speaker", "turnbuckle"),
            bind("grille", "speaker"),
            bind("lamp", "turnbuckle"),
        ]);
        let moved = |x: f32| [[1., 0., 0., x], [0., 1., 0., 0.], [0., 0., 1., 0.]];
        let mut poses = vec![
            ModelPose { placement: 0, delta: moved(1.) },
            ModelPose { placement: 3, delta: moved(5.) },
        ];
        bound.propagate(&mut poses, &placements);
        let mut got: Vec<_> = poses.iter().map(|p| (p.placement, p.delta[0][3])).collect();
        got.sort_by_key(|p| p.0);
        assert_eq!(got, [(0, 1.), (1, 1.), (2, 1.), (3, 5.)]);
    }

    #[test]
    fn physics_pose_updates_geometry_queries_and_both_shadow_bound_sources() {
        let mut app = App::new();
        let placement = crate::entities::PlacedModel {
            id: "rotor".into(),
            resource_path: "rotor.bmodel".into(),
            position: [3., -4., 2.],
            rotation: [[1., 0., 0.], [0.3, 1., 0.], [0., 0.2, 1.]],
            scale: [1., 2., -1.],
            custom_materials: Vec::new(),
            max_visible_range: Some(50.),
        };
        let local = [[-2., -1., -0.2], [2., 1., 0.2]];
        let input = initial_placed_model_query(&placement, local).unwrap();
        let source = super::super::shadow_updates::ModelShadowSource {
            placement: 0,
            local_bounds: local,
            bounds: input.bounds,
            skinned: false,
            surfaces: Vec::new(),
            instances: 0..1,

            flags_1c30: 0,
            flags_18bc: render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC,
            flags_18bd: 0,
            skinning: None,
        };
        let entity = app
            .world_mut()
            .spawn((Transform::IDENTITY, GlobalTransform::IDENTITY))
            .id();
        let corners: [Vec3; 8] = std::array::from_fn(|i| {
            super::super::placed_vertex(
                &placement,
                std::array::from_fn(|axis| local[(i >> axis) & 1][axis]),
            )
        });
        let world_bounds = render_bounds(&corners);
        let mut queue = SceneRenderQueue::default();
        queue.instances.push(super::super::RenderInstance {
            source: Some(super::super::RenderInstanceSource {
                surface: super::super::RenderModelSurface::Bmodel {
                    placement: 0,
                    surface: 0,
                },
                world_baked: true,
            }),
            mesh: Handle::default(),
            material: super::super::MaterialHandle::Probe(Handle::default()),
            diffuse_image_slot: None,
            probe_index: 0,
            transform: Transform::IDENTITY,
            bounds: world_bounds,

            entity: Some(entity),
            visible: true,
        });
        app.insert_resource(ModelMotionBindings::default())
            .insert_resource(ModelPoseUpdates::default())
            .insert_resource(RenderSourcePlacements(vec![placement.clone()]))
            .insert_resource(ModelShadowSources(vec![source]))
            .insert_resource(ShadowModelBoundsSources(vec![
                super::super::ShadowModelBoundsSource {
                    placement: 0,
                    instances: 0..1,
                    bounds: input.bounds,
                },
            ]))
            .insert_resource(PlacedModelQueries(vec![
                super::super::PlacedModelQueryGroup {
                    indices: 0..1,
                    input,
                },
            ]))
            .insert_resource(queue)
            .add_systems(Update, publish_model_poses);
        let rotation = bevy::math::Mat3::from_rotation_z(0.8).to_cols_array_2d();
        let rows = render_model::affine_rows([1., 2., -3.], rotation, [1.; 3]);
        let delta: [[f32; 4]; 3] = std::array::from_fn(|i| rows[i]);
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<ModelPoseUpdates>()
                .0
                .push(ModelPose {
                    placement: 0,
                    delta,
                });
            app.update();
        }
        let current = &app.world().resource::<RenderSourcePlacements>().0[0];
        let expected_position = std::array::from_fn::<_, 3, _>(|r| {
            delta[r][3]
                + (0..3)
                    .map(|c| delta[r][c] * placement.position[c])
                    .sum::<f32>()
        });
        assert_eq!(current.position, expected_position);
        // The shear must survive rotation: replacing the placement by the
        // body's orthogonal basis would change visible and shadow vertices.
        for c in 0..3 {
            let a = Vec3::from(placement.rotation[c]).length();
            let b = Vec3::from(current.rotation[c]).length();
            assert!((a - b).abs() < 0.00001);
        }
        let bounds = render_model::transformed_bounds(
            local,
            current.position,
            current.rotation,
            current.scale,
        );
        assert_eq!(
            app.world().resource::<ModelShadowSources>().0[0].bounds,
            bounds
        );
        assert_eq!(
            app.world().resource::<ShadowModelBoundsSources>().0[0].bounds,
            bounds
        );
        assert_eq!(
            app.world().resource::<PlacedModelQueries>().0[0]
                .input
                .bounds,
            bounds
        );
        assert!(render_model::model_is_dynamic_caster(
            app.world().resource::<ModelShadowSources>().0[0].flags_18bc
        ));
        let instance = &app.world().resource::<SceneRenderQueue>().instances[0];
        assert_eq!(
            *app.world().get::<Transform>(entity).unwrap(),
            instance.transform
        );
        assert_eq!(
            app.world()
                .get::<GlobalTransform>(entity)
                .unwrap()
                .to_matrix(),
            instance.transform.to_matrix()
        );
        assert!(app.world().resource::<ModelPoseUpdates>().0.is_empty());
    }

    #[test]
    fn baked_and_local_vertices_receive_the_same_rigid_motion() {
        let initial = rigid_matrix([3., -4., 2.], Mat3::from_rotation_z(0.3).to_cols_array_2d());
        let current = rigid_matrix([-2., 8., 1.], Mat3::from_rotation_z(1.4).to_cols_array_2d());
        let delta = current * initial.inverse();
        let scale = Mat4::from_scale(Vec3::new(2., 0.5, -3.));
        let point = Vec3::new(1., 2., -1.);
        let baked = (delta * initial * scale).transform_point3(point);
        let local = (current * scale).transform_point3(point);
        assert!(baked.distance(local) < 0.00001);
        let bounds = render_bounds(&[Vec3::ZERO, Vec3::ONE]);
        let moved = moved_bounds(bounds, delta);
        for i in 0..8 {
            let corner = Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32);
            let p = delta.transform_point3(corner);
            let d = (p - Vec3::from(moved.center)).abs() - Vec3::from(moved.half_extents);
            assert!(d.max_element() < 0.00001);
        }
    }
}
