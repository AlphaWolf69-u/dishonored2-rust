//! WeaponThrowable633520/632f70: the equipped source component and actual
//! spawned grenade keep distinct managed identities and shared resource leases.
use bevy::{prelude::*, tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once}};
use std::{collections::{HashMap, VecDeque}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use crate::{assets::{equipment::Descriptor, runtime_entity::Definition, wwise::SoundEventDeclaration},
    gameplay::{entity_lifetime::EntityId, player::{AnimationEvent, weapons::throwable::{Effect, GrenadeState, Source}},
        reference_frame::ReferenceFrame, sound::{Emitter, Request}}};

struct Prepared {
    index: usize,
    entity: Option<Arc<Definition>>,
    bounds: Option<[[f32;3];2]>,
    sounds: HashMap<String, Arc<SoundEventDeclaration>>,
    target_flags:u32,
}
struct Pending {
    cancel: Arc<AtomicBool>,
    task: Mutex<Task<Result<Prepared,String>>>,
}
struct Owned {
    id: EntityId,
    identity: Arc<str>,
    scope: Arc<str>,
    descriptor: Arc<Descriptor>,
    selected: usize,
    prepared: Vec<Option<Arc<Definition>>>,
    bounds: Vec<Option<[[f32;3];2]>>,
    acquired: Vec<bool>,
    flags:Vec<u32>,
    requested: Vec<bool>,
    sounds: HashMap<String,Arc<SoundEventDeclaration>>,
    audio_reserved: bool,
    held_admitted: bool,
    held_prepared: bool,
    pending: Option<Pending>,
    ready: bool,
    retired: bool,
    failure: Option<String>,
}
struct Delivery { id: EntityId, effect: Effect, born: Option<EntityId> }
#[derive(Resource,Default)]
pub(super) struct Throwing { owned: HashMap<EntityId,Owned>, deliveries: VecDeque<Delivery>, generation:u64, error:Option<String> }

impl Throwing {
    pub(super) fn write_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(report, "  throwable owners={} callbacks={} front_callback={:?} error={:.240}", self.owned.len(), self.deliveries.len(), self.deliveries.front().map(|delivery| (delivery.id, delivery.born)), self.error.as_deref().unwrap_or("<none>"));
        for entry in self.owned.values().take(4) {
            let _ = writeln!(report, "    entity={:?} ammo={} ready={} preparing={} retired={} held_admitted={} held_prepared={} selected_prepared={} acquired={:?} requested={:?} failure={:.240}", entry.id, entry.selected, entry.ready, entry.pending.is_some(), entry.retired, entry.held_admitted, entry.held_prepared, entry.prepared.get(entry.selected).is_some_and(Option::is_some), entry.acquired.get(entry.selected), entry.requested.get(entry.selected), entry.failure.as_deref().unwrap_or("<none>"));
        }
    }
}

pub(in crate::app) fn request_owned(world:&mut World, identity:Arc<str>, descriptor:Arc<Descriptor>)->Result<(),String>{
    let Some(settings)=descriptor.throwable.as_ref() else { return Ok(()); };
    let id=super::equipment_identity::id(world,&identity).ok_or("throwable has no actual managed equipment")?;
    if !world.contains_resource::<Throwing>() { world.insert_resource(Throwing::default()); }
    let mut owner=world.resource_mut::<Throwing>();
    if owner.owned.contains_key(&id) { return Ok(()); }
    owner.generation=owner.generation.wrapping_add(1);
    let scope=Arc::from(format!("throwable:{}:{}",owner.generation,identity));
    let count=settings.ammunition.len().max(1);
    owner.owned.insert(id,Owned{id,identity,scope,descriptor,selected:0,
        prepared:vec![None;count],bounds:vec![None;count],acquired:vec![false;count],flags:vec![0;count],requested:vec![false;count],
        sounds:HashMap::new(),audio_reserved:false,held_admitted:false,held_prepared:false,pending:None,
        ready:false,retired:false,failure:None});
    Ok(())
}
pub(in crate::app) fn remove_owned(world:&mut World,identity:&str){
    if let Some(mut owner)=world.get_resource_mut::<Throwing>(){
        for entry in owner.owned.values_mut().filter(|entry|entry.identity.as_ref()==identity){
            entry.retired=true;entry.ready=false;
            if let Some(pending)=&entry.pending{pending.cancel.store(true,Ordering::Release);}
        }
    }
}
pub(in crate::app) fn owns_weapon(world:&World,id:EntityId)->bool{
    world.get_resource::<Throwing>().is_some_and(|owner|owner.owned.get(&id)
        .is_some_and(|entry|entry.descriptor.throwable.is_some()&&entry.held_admitted))
}
pub(in crate::app) fn request_ammunition(world:&mut World,identity:&str,item:&str)->Result<(),String>{
    let mut owner=world.get_resource_mut::<Throwing>().ok_or("throwable admission is absent")?;
    let entry=owner.owned.values_mut().find(|entry|entry.identity.as_ref()==identity).ok_or("owned WeaponThrowable is absent")?;
    let definition=entry.descriptor.throwable.as_ref().ok_or("WeaponThrowable declaration absent")?;
    let index=definition.ammunition.iter().position(|ammo|ammo.item==item).ok_or_else(||format!("{identity}: no authored ammunition {item}"))?;
    if entry.selected!=index{entry.selected=index;entry.ready=false;}
    Ok(())
}
fn acquire(world:&World,entry:&mut Owned)->Result<(),String>{
    let catalog=world.resource::<super::SceneData>().catalog.clone().ok_or("throwable has no catalog")?;
    let settings=entry.descriptor.throwable.clone().ok_or("throwable source definition absent")?;
    let index=entry.selected;
    let upgrades=super::equipment_attachment_admission::upgrade_definitions(world,entry.id)
        .ok_or("throwable upgrade admission is pending")?;
    let mut variants=Vec::new();
    if let Some(ammo)=settings.ammunition.get(index){
        for upgrade in upgrades {
            for record in upgrade.ammunition.iter().filter(|record|record.item==ammo.item){
                if record.grenade.is_some(){variants.insert(0,record.clone());}else{variants.push(record.clone());}
            }
        }
    }
    let cancel=Arc::new(AtomicBool::new(false));let cancellation=cancel.clone();
    let task=IoTaskPool::get().spawn(async move{
        let cancelled=||cancellation.load(Ordering::Acquire);
        if cancelled(){return Err("throwable preparation cancelled".into());}
        let mut bounds=None;
        let entity=settings.ammunition.get(index).and_then(|ammo|ammo.projectile.as_deref()).map(|path|{
            let base=Definition::acquire(&catalog,path,Some(&cancelled))?;
            bounds=base.model.as_ref().map(|model|model.geometry.bounds());
            crate::assets::runtime_entity::weapon_variant(&catalog,base,&variants,Some(&cancelled))
        }).transpose()?;
        let mut sounds=HashMap::new();
        for stage in settings.cooking_stages.iter(){
            for path in [stage.cooking_sound.as_ref(),stage.cooked_sound.as_ref()].into_iter().flatten(){
                if cancelled(){return Err("throwable preparation cancelled".into());}
                if !sounds.contains_key(path){
                    let event=crate::assets::wwise::event_resources::acquire_with(&catalog,path,
                        |path|crate::assets::declaration_texts::read_common_or_catalog(&catalog,path))?;
                    sounds.insert(path.clone(),event);
                }
            }
        }
        let target_flags=variants.iter().fold(0,|flags,variant|flags|variant.flags);
        Ok(Prepared{index,entity,bounds,sounds,target_flags})
    });
    entry.pending=Some(Pending{cancel,task:Mutex::new(task)});Ok(())
}
fn advance_owned(world:&mut World,entry:&mut Owned)->Result<(),String>{
    if let Some(pending)=&mut entry.pending{
        if let Some(result)=block_on(poll_once(pending.task.get_mut().unwrap_or_else(|e|e.into_inner()))){
            entry.pending=None;
            if !entry.retired{
                let prepared=result?;
                entry.prepared[prepared.index]=prepared.entity;entry.bounds[prepared.index]=prepared.bounds;
                entry.flags[prepared.index]=prepared.target_flags;
                entry.acquired[prepared.index]=true;entry.sounds.extend(prepared.sounds);
            }
        }
    }
    if entry.retired{return Ok(());}
    if let Some(error)=&entry.failure{return Err(error.clone());}
    if !entry.held_admitted{
        let definition=entry.descriptor.grenade.clone().ok_or("WeaponThrowable requires its actual Grenade component")?;
        let scheduling=entry.descriptor.grenade_scheduling.clone().ok_or("held Grenade scheduling is absent")?;
        super::runtime_entities::with_scene(world,|world,scene,live,events|{
            scene.admit_held_grenade(world,live,events,entry.id,definition,scheduling,&entry.descriptor).map(|_|())
        })?;
        entry.held_admitted=true;
    }
    if !entry.held_prepared{
        super::runtime_entities::with_scene(world,|world,scene,live,_|{
            let handle=scene.grenade_component(live,entry.id)?.ok_or("held Grenade payload is absent")?;
            scene.prepare_grenade_resources(world,handle.handle)
        })?;
        entry.held_prepared=true;
    }
    if !entry.acquired[entry.selected] && entry.pending.is_none(){
        //The actual equipment upgrade source is initialized before its ammo
        //definition. A pending owner is a readiness boundary, not absence.
        if super::equipment_attachment_admission::upgrade_definitions(world,entry.id).is_some(){acquire(world,entry)?;}
    }
    if !entry.audio_reserved && entry.acquired[entry.selected]{
        super::audio::reserve_events(world,entry.scope.clone(),entry.sounds.values().cloned())?;
        entry.audio_reserved=true;
    }
    if entry.acquired[entry.selected] && !entry.requested[entry.selected]{
        if let Some(definition)=&entry.prepared[entry.selected]{
            super::runtime_entities::request_for(world,Arc::from(format!("{}:ammo:{}",entry.scope,entry.selected)),definition.clone())?;
        }
        entry.requested[entry.selected]=true;
    }
    let held_ready=super::runtime_entities::with_scene(world,|world,scene,live,_|{
        let handle=scene.grenade_component(live,entry.id)?.ok_or("held Grenade payload is absent")?;
        scene.grenade_resources_ready(world,handle.handle)
    })?;
    entry.ready=held_ready && entry.acquired[entry.selected] && entry.audio_reserved
        && super::audio::events_ready(world,&entry.scope)?
        && entry.prepared[entry.selected].as_ref().map_or(Ok(true),|source|super::runtime_entities::ready(world,source))?;
    Ok(())
}

///Source633520 caches the actual equipped grenade; every pre-frame snapshot
///comes from that same component. The native fuse owner keeps running through
///cooking, while a cold source prevents CanFire before input is accepted.
pub(in crate::app) fn prepare(world:&mut World){
    let Some(mut owner)=world.remove_resource::<Throwing>() else{return;};
    let Some(mut state)=world.remove_resource::<super::player::PlayerState>() else{world.insert_resource(owner);return;};
    let result=(||{
        for entry in owner.owned.values_mut(){advance_owned(world,entry)?;}
        let Some(player)=state.player.as_mut() else{return Ok(());};
        player.publish_throwable_magazine();
        let (eye,camera_axis)=player.published_camera_frame();
        let hit=player.ranged_target_hit();let attacker=player.managed_entity();
        let Some(runtime)=player.throwable.as_mut() else{return Ok(());};
        runtime.ready=false;runtime.source=None;
        runtime.entity_delta=world.resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .entity_delta(runtime.entity).map_err(str::to_owned)?;
        let Some(entry)=owner.owned.get_mut(&runtime.entity) else{return Ok(());};
        if runtime.ammunition_index>=entry.prepared.len(){return Err("throwable ammunition selection is outside its declaration".into());}
        if entry.selected!=runtime.ammunition_index{return Ok(());}
        runtime.grenade_state=super::runtime_entities::with_scene(world,|_,scene,live,_|{
            scene.grenade_flags(live,entry.id).map(|flags|flags.map(|flags|GrenadeState{armed:flags&1!=0,exploded:flags&8!=0}))
        })?;
        if !entry.ready || entry.retired{return Ok(());}
        runtime.target_flags=entry.flags[entry.selected];
        if let Some(ammo)=runtime.ammunition(){
            use crate::gameplay::player::weapons::AmmunitionInventory;
            runtime.total_ammo=world.resource::<super::interaction_focus::DiscoveredDocuments>().quantity(&ammo.item);
        }
        let definition=entry.prepared.get(entry.selected).and_then(Clone::clone);
        let Some((frame,_))=super::equipment_identity::frame(world,entry.id) else{return Ok(());};
        let settings=entry.descriptor.throwable.as_ref().ok_or("throwable declaration absent")?;
        let aim=hit.unwrap_or(eye+camera_axis.x_axis*settings.aim.ideal_distance);
        let weapon_axis=Mat3::from_cols_array_2d(&frame.basis);
        let muzzle=Vec3::from_array(frame.position);
        runtime.source=Some(Source{definition,attacker:attacker.ok_or("throwable attacker absent")?,origin:muzzle,eye,aim,
            weapon_axis,weapon_position:muzzle,camera_axis,failure_chance:player.attributes.get("ARK_ATTRIBUTE_PLAYER_GRENADE_THROW_FAIL_CHANCE")});
        runtime.ready=true;Ok::<_,String>(())
    })();
    if let Err(error)=result{owner.error=Some(error);}
    world.insert_resource(state);world.insert_resource(owner);
}

pub(in crate::app) fn handle_animation_event(world:&mut World,event:&AnimationEvent,equipment:[Option<EntityId>;2])->Result<(),String>{
    if !matches!(event.name.as_str(),"ARK_MDNT_MSG_WEAPON_FIRE"|"ARK_MDNT_MSG_WEAPON_RELOAD"){return Ok(());}
    if equipment[0].is_none_or(|id|!owns_weapon(world,id)){return Ok(());}
    //638ae0 executes at Fire, not once per idle/cooking frame. The checked
    //event still owns the actual selected component before spending ammo.
    let origin=if event.name=="ARK_MDNT_MSG_WEAPON_FIRE"{
        let id=equipment[0].expect("validated equipment");
        let source=world.get_resource::<super::player::PlayerState>().and_then(|state|state.player.as_ref())
            .and_then(|player|player.throwable_fire_source(id));
        let bounds=world.resource::<Throwing>().owned.get(&id).and_then(|entry|entry.bounds[entry.selected]);
        if let (Some((source,deviation)),Some(bounds))=(source,bounds){
            Some((id,super::weapon_firing::spawn_origin(world,source.eye,source.camera_axis,
                source.weapon_position,source.weapon_axis,bounds,deviation,source.aim)?))
        }else{None}
    }else{None};
    let Some(mut state)=world.remove_resource::<super::player::PlayerState>() else{return Ok(());};
    let Some(mut inventory)=world.remove_resource::<super::interaction_focus::DiscoveredDocuments>() else{world.insert_resource(state);return Ok(());};
    let result=state.player.as_mut().map_or(Ok(()),|player|{
        if let Some((id,origin))=origin{player.set_throwable_origin(id,origin);}
        player.handle_throwable_animation_event(event,equipment[0],&mut inventory)?;
        player.publish_throwable_magazine();
        Ok::<_, String>(())
    });
    world.insert_resource(inventory);world.insert_resource(state);
    result?;
    //The native C8 receiver finishes632f70/490f70 before the PlayerState's
    //next Update. Publish the accepted receipt here, then refresh its same
    //component snapshot so an already thrown grenade is not still "armed".
    publish(world);
    if let Some(id)=equipment[0]{
        let flags=super::runtime_entities::with_scene(world,|_,scene,live,_|scene.grenade_flags(live,id))?;
        if let Some(mut state)=world.get_resource_mut::<super::player::PlayerState>(){
            if let Some(runtime)=state.player.as_mut().and_then(|player|player.throwable.as_mut()).filter(|runtime|runtime.entity==id){
                runtime.grenade_state=flags.map(|flags|GrenadeState{armed:flags&1!=0,exploded:flags&8!=0});
            }
        }
    }
    Ok(())
}
fn dispatch(world:&mut World,entry:&Owned,delivery:&mut Delivery)->Result<bool,String>{
    match &delivery.effect{
        Effect::Arm{in_hand,bypass_hit_floor}=>super::runtime_entities::with_scene(world,|world,scene,live,events|
            scene.grenade_arm(world,live,events,entry.id,*in_hand,*bypass_hit_floor)).map(|_|true),
        Effect::Disarm=>super::runtime_entities::with_scene(world,|world,scene,live,events|scene.grenade_disarm(world,live,events,entry.id)).map(|_|true),
        Effect::Reset=>super::runtime_entities::with_scene(world,|world,scene,live,events|scene.grenade_reset(world,live,events,entry.id)).map(|_|true),
        Effect::ConsumeAmmo(item)=>{
            use crate::gameplay::player::weapons::AmmunitionInventory;
            world.resource_mut::<super::interaction_focus::DiscoveredDocuments>().consume(item,1);Ok(true)
        }
        Effect::FiredNotice{attacker,position}=>{
            use crate::gameplay::entity_events::{Event,Payload};
            let mut bytes=Vec::with_capacity(16);bytes.extend_from_slice(&0x17u32.to_le_bytes());
            for lane in position.to_array(){bytes.extend_from_slice(&lane.to_le_bytes());}
            world.resource_mut::<crate::gameplay::entity_events::EntityEvents>().post(Event{id:0x73,target:Some(*attacker),payload:Payload::Bytes(bytes)});Ok(true)
        }
        Effect::Sound(path)=>{
            let event=entry.sounds.get(path).ok_or_else(||format!("throwable cooking sound source not prepared: {path}"))?;
            world.write_message(super::audio::SoundEvent(Request::Post{event:event.id,emitter:Emitter::Equipment{entity:entry.id,part:8}}));Ok(true)
        }
        Effect::Fire{source,linear,angular,target_flags}=>{
            let Some(definition)=&source.definition else{return Ok(true);};
            let born=match delivery.born{Some(id)=>id,None=>{
                let id=super::runtime_entities::spawn_grenade(world,definition.clone(),super::runtime_entities::SpawnOptions{
                    frame:ReferenceFrame{position:source.origin.to_array(),basis:source.weapon_axis.to_cols_array_2d()},
                    scale:None,hidden_disabled:false},source.attacker,*linear,*angular,entry.id,*target_flags)?;
                delivery.born=Some(id);id
            }};
            super::runtime_entities::finish_grenade_birth(world,born)
        }
    }
}
pub(in crate::app) fn publish(world:&mut World){
    let Some(mut owner)=world.remove_resource::<Throwing>() else{return;};
    if let Some(mut state)=world.get_resource_mut::<super::player::PlayerState>(){
        if let Some(player)=state.player.as_mut(){
            owner.deliveries.extend(player.drain_throwable_effects().map(|(id,effect)|Delivery{id,effect,born:None}));
        }
    }
    while let Some(mut delivery)=owner.deliveries.pop_front(){
        let result=owner.owned.get(&delivery.id).ok_or_else(||"throwable callback source retired before acknowledgement".to_owned())
            .and_then(|entry|dispatch(world,entry,&mut delivery));
        match result{
            Ok(true)=>{},
            Ok(false)=>{owner.deliveries.push_front(delivery);break;}
            Err(error)=>{
                //Retain the actual constructor failure before another shared
                //startup pass can report a secondary error from its entered
                //component. One receipt per failed action, not per retry.
                if owner.error.is_none(){
                    crate::platform::client_diagnostics::record(&format!("Throwable callback failed for {:?}: {error}",delivery.id));
                    owner.error=Some(error);
                }
                owner.deliveries.push_front(delivery);break;
            }
        }
    }
    if let Err(error)=prune(world,&mut owner){owner.error=Some(error);}
    world.insert_resource(owner);
}
fn prune(world:&mut World,owner:&mut Throwing)->Result<(),String>{
    let mut retired=Vec::new();
    for (id,entry) in &mut owner.owned{
        if entry.retired && entry.pending.is_none() && !owner.deliveries.iter().any(|delivery|delivery.id==*id){
            release(world,entry)?;retired.push(*id);
        }
    }
    for id in retired{owner.owned.remove(&id);}Ok(())
}
pub(in crate::app) fn startup_pending(world:&World)->bool{
    world.get_resource::<Throwing>().is_some_and(|owner|owner.owned.values().any(|entry|!entry.retired&&!entry.ready))
}
pub(in crate::app) fn debug_startup_pending(world:&World)->String {
    use std::fmt::Write;
    let Some(owner)=world.get_resource::<Throwing>()else{return "throwable owner=absent".into();};
    let mut report=String::new();owner.write_diagnostics(&mut report);
    for entry in owner.owned.values(){
        let _=writeln!(report,"    identity={} upgrades={:?} audio={:?}",entry.identity,
            super::equipment_attachment_admission::upgrade_definitions(world,entry.id).map(|upgrades|upgrades.len()),
            super::audio::events_ready(world,&entry.scope));
        if let (Some(scene),Some(live))=(world.get_resource::<super::physics_scene::PhysicsScene>(),world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()) {
            match scene.grenade_component(live,entry.id){
                Ok(Some(handle))=>{let _=writeln!(report,"    {}",scene.debug_grenade_resources(world,handle.handle));},
                result=>{let _=writeln!(report,"    Grenade component={result:?}");},
            }
        }
        if let Some(source)=&entry.prepared[entry.selected]{let _=writeln!(report,"    {}",super::runtime_entities::debug_readiness(world,source));}
    }report
}
pub(in crate::app) fn ready_owned(world:&World,identity:&str)->Result<bool,String>{
    let Some(owner)=world.get_resource::<Throwing>()else{return Ok(false);};
    if let Some(error)=&owner.error{return Err(error.clone());}
    let Some(entry)=owner.owned.values().find(|entry|entry.identity.as_ref()==identity)else{return Ok(false);};
    if let Some(error)=&entry.failure{return Err(error.clone());}
    Ok(entry.ready&&!entry.retired)
}
pub(in crate::app) fn startup_error(world:&World)->Option<&str>{world.get_resource::<Throwing>().and_then(|owner|owner.error.as_deref())}
pub(in crate::app) fn retired(world:&World,identity:&str)->bool{
    world.get_resource::<Throwing>().is_none_or(|owner|!owner.owned.values().any(|entry|entry.identity.as_ref()==identity))
}
fn release(world:&mut World,entry:&mut Owned)->Result<(),String>{
    for index in 0..entry.requested.len(){
        if entry.requested[index]{
            if let Some(definition)=&entry.prepared[index]{super::runtime_entities::release_for(world,&format!("{}:ammo:{index}",entry.scope),definition)?;}
            entry.requested[index]=false;
        }
    }
    if entry.audio_reserved{super::audio::release_events(world,&entry.scope)?;entry.audio_reserved=false;}
    Ok(())
}
///No worker is detached: close cancels its acquisition, waits for its result,
///and only then returns source leases to their canonical owners. Accepted
///spawn/transfer journals remain until their real callbacks acknowledge.
pub(in crate::app) fn shutdown(world:&mut World)->Result<bool,String>{
    //Accepted source callbacks precede component Stop/Deinit, including when
    //a level exit is requested in the same frame as its Fire key.
    super::runtime_entities::advance(world)?;
    publish(world);
    if world.get_resource::<Throwing>().is_some_and(|owner|!owner.deliveries.is_empty()) {
        //LevelLifetime retains ordinary resource preparation and its actual
        //GPU view until this accepted-operation fence completes.
        publish(world);
        if let Some(owner)=world.get_resource::<Throwing>() {
            if let Some(error)=&owner.error{return Err(error.clone());}
            if !owner.deliveries.is_empty(){return Ok(false);}
        }
    }
    let Some(mut owner)=world.remove_resource::<Throwing>() else{return Ok(true);};
    let result=(||{
        for entry in owner.owned.values_mut(){
            entry.retired=true;entry.ready=false;
            if let Some(pending)=&entry.pending{pending.cancel.store(true,Ordering::Release);}
            advance_owned(world,entry)?;
        }
        let mut done=Vec::new();
        for (id,entry) in &mut owner.owned{
            if entry.pending.is_none()&&!owner.deliveries.iter().any(|delivery|delivery.id==*id){release(world,entry)?;done.push(*id);}
        }
        for id in done{owner.owned.remove(&id);}
        Ok(owner.owned.is_empty()&&owner.deliveries.is_empty())
    })();
    if !matches!(result,Ok(true)){world.insert_resource(owner);}
    result
}
