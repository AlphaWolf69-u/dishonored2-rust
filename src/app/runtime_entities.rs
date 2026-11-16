//! Runtime entity admission8aecc0/87a4b0. Source prototypes prepare without
//! managed IDs; only accepted Spawn allocates the actual entity/component set.
use bevy::{prelude::*,tasks::{Task,IoTaskPool,block_on,futures_lite::future::poll_once}};
use crate::{assets::{runtime_entity::{Definition,Kind},equipment::attachment::Geometry},gameplay::{entity_lifetime::EntityId,reference_frame::ReferenceFrame}};
use std::{collections::{HashMap,VecDeque},sync::{Arc,Weak,Mutex,atomic::{AtomicBool,Ordering}}};

#[derive(Clone,Copy)]
pub(super) struct SpawnOptions{pub frame:ReferenceFrame,pub scale:f32,pub hidden_disabled:bool}
pub(super) struct BodyDraft{pub component:usize,pub draft:Arc<super::rigid_body::RigidBodyDraft>}
pub(super) struct BodyResources{pub component:usize,pub resources:Arc<super::rigid_body::RigidBodyResources>}
struct SourceWorker{shapes:crate::physworld::source::ShapeSources,texts:crate::assets::declaration_texts::PhysicsSeed}
struct Prototype{definition:Weak<Definition>,resources:Option<Arc<Vec<BodyResources>>>,error:Option<String>,fx_key:Arc<str>,wanted:bool}
struct PreparedDependencies{bodies:Vec<BodyDraft>,items:HashMap<usize,Arc<crate::assets::inventory_item::PickableItem>>}
struct Pending{key:usize,task:Mutex<Task<Result<PreparedDependencies,String>>>}
struct Actor{definition:Arc<Definition>,removing:bool,release:Option<super::physics_scene::runtime_entities::Retirement>}
#[derive(Resource)]
pub(super) struct RuntimeEntities{catalog:Arc<crate::archive::Catalog>,source:Arc<Mutex<SourceWorker>>,prototypes:HashMap<usize,Prototype>,items:HashMap<usize,Arc<HashMap<usize,Arc<crate::assets::inventory_item::PickableItem>>>>,queue:VecDeque<usize>,pending:Option<Pending>,actors:HashMap<EntityId,Actor>,cancelled:Arc<AtomicBool>,active_retirement:Option<EntityId>}
fn key(definition:&Definition)->usize{definition as *const Definition as usize}
pub(super) fn bounds(world:&World,id:EntityId)->Result<Option<[[f32;3];2]>,String>{
    if let Some(bounds)=super::world_models::bounds(world,id){return Ok(Some(bounds));}
    let live=world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>().ok_or("entity bounds require the actual borrowed identity owner")?;
    bounds_for(world,live,id)
}
pub(super) fn bounds_for(world:&World,live:&crate::gameplay::entity_lifetime::LiveEntities,id:EntityId)->Result<Option<[[f32;3];2]>,String>{
    if let Some(bounds)=super::world_models::bounds(world,id){return Ok(Some(bounds));}
    let(name,_)=live.get(id).ok_or("entity bounds generation is stale")?;
    let(Some(placements),Some(models))=(world.get_resource::<super::RenderSourcePlacements>(),world.get_resource::<super::shadow_updates::ModelShadowSources>())else{return Ok(None)};
    let mut merged=None::<[[f32;3];2]>;
    for source in &models.0{let Some(placement)=placements.0.get(source.placement).filter(|placement|placement.id.eq_ignore_ascii_case(name))else{continue};
        let bounds=crate::render_model::transformed_bounds(source.local_bounds,placement.position,placement.rotation,placement.scale);
        if let Some(current)=&mut merged{for axis in 0..3{current[0][axis]=current[0][axis].min(bounds[0][axis]);current[1][axis]=current[1][axis].max(bounds[1][axis]);}}else{merged=Some(bounds);}
    }Ok(merged)
}

pub(super) fn initialize(world:&mut World)->Result<(),String>{
    if world.contains_resource::<RuntimeEntities>(){return Err("runtime entity owner already exists".into());}
    let catalog=world.resource::<super::SceneData>().catalog.clone().ok_or("runtime entity catalog is absent")?;
    let texts=world.remove_resource::<super::level_prepare::PreparedPhysicsTexts>().ok_or("runtime entities require the prepared physics declaration owner")?.0;
    let source=Arc::new(Mutex::new(SourceWorker{shapes:crate::physworld::source::ShapeSources::new(&catalog),texts}));
    world.insert_resource(RuntimeEntities{catalog,source,prototypes:HashMap::new(),items:HashMap::new(),queue:VecDeque::new(),pending:None,actors:HashMap::new(),cancelled:Arc::default(),active_retirement:None});Ok(())
}
fn effect_paths(definition:&Definition)->Vec<String>{
    let mut paths=Vec::new();if let Some(projectile)=&definition.projectile{paths.extend(projectile.effect_paths().map(str::to_owned));}
    if let Some(breakable)=&definition.breakable{paths.extend(breakable.fx_paths().map(str::to_owned));}paths.sort_unstable();paths.dedup();paths
}
pub(super) fn request(world:&mut World,definition:Arc<Definition>)->Result<(),String>{
    if !matches!(definition.class.as_str(),"idEntity"|"arkProjectile"){return Err(format!("runtime actor class {} has no actual constructor adapter",definition.class));}
    let identity=key(&definition);
    if let Some(prototype)=world.get_resource::<RuntimeEntities>().and_then(|owner|owner.prototypes.get(&identity)){
        if let Some(error)=&prototype.error{return Err(error.clone());}return Ok(());
    }
    if let Some(model)=&definition.model{super::world_models::prewarm(world,model.clone())?;}
    let fx_key:Arc<str>=Arc::from(format!("entity-source:{identity:x}"));
    world.get_resource_mut::<super::usable_effects::EffectRequests>().ok_or("runtime entity FX resource owner is absent")?.reserve(fx_key.clone(),effect_paths(&definition));
    let mut owner=world.get_resource_mut::<RuntimeEntities>().ok_or("runtime entity preparation owner is absent")?;
    if owner.cancelled.load(Ordering::Acquire){return Err("runtime entity acquisition is closed".into());}
    let physical=definition.components.iter().any(|component|matches!(component.kind,Kind::Rigid(_)|Kind::Throwable(_))||matches!(&component.kind,Kind::Pickable(definition) if !definition.item.is_empty()));
    owner.prototypes.insert(identity,Prototype{definition:Arc::downgrade(&definition),resources:(!physical).then(||Arc::new(Vec::new())),error:None,fx_key,wanted:true});
    if physical{owner.queue.push_back(identity);}drop(owner);
    for chunk in &definition.chunks{request(world,chunk.definition.clone())?;}Ok(())
}
pub(super) fn ready(world:&World,definition:&Definition)->Result<bool,String>{
    let Some(prototype)=world.get_resource::<RuntimeEntities>().and_then(|owner|owner.prototypes.get(&key(definition)))else{return Ok(false)};
    if let Some(error)=&prototype.error{return Err(error.clone());}
    if prototype.resources.is_none()||!super::usable_effects::reservation_ready(world,&prototype.fx_key)?{return Ok(false);}
    if let Some(model)=&definition.model{if !super::world_models::prototype_ready(world,model)?{return Ok(false);}}
    for chunk in &definition.chunks{if !ready(world,&chunk.definition)?{return Ok(false);}}Ok(true)
}
pub(super) fn release_prepared(world:&mut World,definition:&Definition)->Result<(),String>{
    if let Some(mut owner)=world.get_resource_mut::<RuntimeEntities>(){if let Some(prototype)=owner.prototypes.get_mut(&key(definition)){prototype.wanted=false;}}
    for chunk in &definition.chunks{release_prepared(world,&chunk.definition)?;}Ok(())
}
pub(super) fn advance(world:&mut World)->Result<(),String>{
    let Some(mut owner)=world.remove_resource::<RuntimeEntities>()else{return Ok(())};
    let result=(||{
        if let Some(pending)=owner.pending.as_mut(){
            let result=block_on(poll_once(pending.task.get_mut().unwrap_or_else(|error|error.into_inner())));
            if let Some(result)=result{
                let identity=pending.key;owner.pending=None;
                let result=result.and_then(|prepared|{let resources=world.resource_mut::<super::physics_scene::PhysicsScene>().publish_runtime_resources(prepared.bodies)?;owner.items.insert(identity,Arc::new(prepared.items));Ok(resources)});
                if let Some(prototype)=owner.prototypes.get_mut(&identity){match result{Ok(resources)=>prototype.resources=Some(Arc::new(resources)),Err(error)=>prototype.error=Some(error)}}
            }
        }
        if owner.pending.is_none()&&!owner.cancelled.load(Ordering::Acquire){
            while let Some(identity)=owner.queue.pop_front(){
                let Some(definition)=owner.prototypes.get(&identity).and_then(|prototype|prototype.definition.upgrade())else{continue};
                let materials=definition.model.as_ref().map(|model|model.material_requests().into_iter().map(|name|(name,super::material_loading::MaterialGeometry::Model)).collect::<Vec<_>>()).unwrap_or_default();
                let snapshot=super::runtime_materials::MaterialSnapshot::capture(world.resource::<super::runtime_materials::RuntimeMaterialOwner>(),world.resource::<super::SceneRenderQueue>(),&materials,
                    world.resource::<Assets<super::ProbeMaterial>>(),world.resource::<Assets<super::DecalMaterial>>())?;
                let source=owner.source.clone();let catalog=owner.catalog.clone();let cancelled=owner.cancelled.clone();
                let task=IoTaskPool::get().spawn(async move{
                    let mut source=source.lock().map_err(|_|"runtime physics source owner is poisoned")?;
                    let mut materials=snapshot.cache;let mut drafts=Vec::new();let mut items=HashMap::new();
                    let inventory=crate::assets::inventory_item::PickableSourceLease::for_catalog(&catalog)?;
                    for(index,component)in definition.components.iter().enumerate(){
                        if cancelled.load(Ordering::Acquire){return Err("runtime physics preparation cancelled".into());}
                        if let Kind::Pickable(definition)=&component.kind{if !definition.item.is_empty(){items.insert(index,inventory.0.lock().map_err(|_|"runtime inventory source owner is poisoned")?.acquire(&catalog,&definition.item)?);}}
                        let settings=match &component.kind{Kind::Rigid(definition)=>&definition.settings,Kind::Throwable(definition)=>&definition.rigid.settings,_=>continue};
                        let model=definition.model.as_ref().ok_or("runtime rigid component has no actual RenderModel resource")?;
                        let render=match &model.geometry{Geometry::Rigid(model)=>Some(&***model),_=>None};
                        let SourceWorker{shapes,texts}=&mut *source;
                        let draft=super::rigid_body::prepare_draft(&catalog,shapes,texts,settings,&model.placement,render,Some(&mut materials))?;
                        drafts.push(BodyDraft{component:index,draft:Arc::new(draft)});
                    }Ok(PreparedDependencies{bodies:drafts,items})
                });owner.pending=Some(Pending{key:identity,task:Mutex::new(task)});break;
            }
        }
        let dead=owner.prototypes.iter().filter_map(|(&identity,prototype)|(!prototype.wanted&&prototype.definition.strong_count()<=1&&!owner.actors.values().any(|actor|key(&actor.definition)==identity)&&owner.pending.as_ref().is_none_or(|pending|pending.key!=identity)).then_some(identity)).collect::<Vec<_>>();
        for identity in dead{if let Some(prototype)=owner.prototypes.remove(&identity){owner.items.remove(&identity);world.resource_mut::<super::usable_effects::EffectRequests>().release_reservation(prototype.fx_key);}}
        Ok(())
    })();world.insert_resource(owner);result?;super::world_models::advance(world)?;super::world_models::prune(world);Ok(())
}
pub(super) fn spawn(world:&mut World,definition:Arc<Definition>,options:SpawnOptions)->Result<EntityId,String>{spawn_inner(world,definition,options,None)}
pub(super) fn spawn_projectile(world:&mut World,definition:Arc<Definition>,options:SpawnOptions,request:crate::gameplay::projectiles::SpawnRequest)->Result<EntityId,String>{spawn_inner(world,definition,options,Some(request))}
fn spawn_inner(world:&mut World,definition:Arc<Definition>,options:SpawnOptions,projectile:Option<crate::gameplay::projectiles::SpawnRequest>)->Result<EntityId,String>{
    if !ready(world,&definition)?{return Err("runtime entity source dependencies are not ready".into());}
    let resources=world.resource::<RuntimeEntities>().prototypes[&key(&definition)].resources.clone().ok_or("runtime body resources are absent")?;
    let mut scene=world.remove_resource::<super::physics_scene::PhysicsScene>().ok_or("runtime physics scene is absent")?;
    let mut live=world.remove_resource::<crate::gameplay::entity_lifetime::LiveEntities>().ok_or("runtime managed entity owner is absent")?;
    let mut events=world.remove_resource::<crate::gameplay::entity_events::EntityEvents>().ok_or("runtime entity event owner is absent")?;
    let result=scene.spawn_runtime_actor(world,&mut live,&mut events,definition.clone(),&resources,options,projectile);
    world.insert_resource(events);world.insert_resource(live);world.insert_resource(scene);
    let id=result?;world.resource_mut::<RuntimeEntities>().actors.insert(id,Actor{definition:definition.clone(),removing:false});
    if let Some(model)=&definition.model{super::world_models::request(world,id,model.clone(),options.frame)?;super::world_models::set_scale(world,id,[options.scale;3])?;super::world_models::update(world,id,options.frame,!options.hidden_disabled)?;}
    if let Some(mut clock)=world.get_resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>(){clock.register_entity_time(id,0)?;}Ok(id)
}
pub(super) fn retire(world:&mut World,id:EntityId)->Result<bool,String>{
    let Some(mut owner)=world.remove_resource::<RuntimeEntities>()else{return Ok(true)};
    if !owner.actors.contains_key(&id){world.insert_resource(owner);return Ok(true);}
    if owner.active_retirement.is_some_and(|active|active!=id){world.insert_resource(owner);return Ok(false);}
    owner.active_retirement=Some(id);
    let result=(||{
        let actor=owner.actors.get_mut(&id).unwrap();
        if !actor.removing{super::world_models::remove(world,id)?;super::entity_graphs::retire(world,id)?;actor.removing=true;}
        if !super::world_models::retired(world,id){return Ok(false);}
        let mut scene=world.remove_resource::<super::physics_scene::PhysicsScene>().ok_or("runtime physics scene is absent")?;
        let mut live=world.remove_resource::<crate::gameplay::entity_lifetime::LiveEntities>().ok_or("runtime managed owner is absent")?;
        let mut events=world.remove_resource::<crate::gameplay::entity_events::EntityEvents>().ok_or("runtime event owner is absent")?;
        let result=scene.retire_runtime_actor(world,&mut live,&mut events,id);
        world.insert_resource(events);world.insert_resource(live);world.insert_resource(scene);
        if !result?{return Ok(false);}
        if let Some(mut clock)=world.get_resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>(){clock.unregister_entity_time(id);}
        owner.actors.remove(&id);owner.active_retirement=None;Ok(true)
    })();world.insert_resource(owner);result
}
pub(super) fn begin_retirement(world:&mut World)->Result<bool,String>{
    let Some(mut owner)=world.get_resource_mut::<RuntimeEntities>()else{return Ok(true)};owner.cancelled.store(true,Ordering::Release);owner.queue.clear();
    let ids=owner.actors.keys().copied().collect::<Vec<_>>();drop(owner);for id in ids{if !retire(world,id)?{return Ok(false);}}
    advance(world)?;let mut owner=world.resource_mut::<RuntimeEntities>();if owner.pending.is_some(){return Ok(false);}let prototypes=std::mem::take(&mut owner.prototypes);drop(owner);
    for(_,prototype)in prototypes{world.resource_mut::<super::usable_effects::EffectRequests>().release_reservation(prototype.fx_key);}Ok(true)
}
