//!8aa288..8aa605 actual default/section AIWorld admission. The canonical
//!runtime factory owns the actors/components; this queue owns their source
//!demand, acquisition join and the game-local list's ordered creation.
use bevy::{prelude::*,tasks::{IoTaskPool,Task,block_on,futures_lite::future::poll_once}};
use std::sync::{Arc,Mutex,atomic::{AtomicBool,AtomicU64,Ordering}};
use crate::{archive::Catalog,assets::{entities::{self,Document,PlacedModel,Value},level::LevelResources,runtime_entity::Definition},
    gameplay::{entity_lifetime::{EntityId,LiveEntities},component_state::types::ComponentTypes}};
use entities::scene_motion::{atom,child};

struct Section {name:Arc<str>,model:Option<PlacedModel>,local_bounds:Option<[[f32;3];2]>}
///CPU preparation retains only real source references and ordered section
///metadata, never the generic entities document or a duplicate model cache.
#[derive(Resource)]
pub(in crate::app) struct CreationPlan{
    source:Option<String>,sections:Vec<Section>,movers:usize,navigation:usize,
    navmesh:Option<Arc<crate::assets::physworld::navmesh::Metadata>>,
}
fn reference(value:Option<&Value>,context:&str)->Result<Option<String>,String>{
    match value{
        None=>Ok(None),Some(Value::Atom(value)) if value.is_empty()||value.eq_ignore_ascii_case("NULL")=>Ok(None),
        Some(Value::Atom(value))=>Ok(Some(value.clone())),_=>Err(format!("{context}: expected nullable source declaration reference")),
    }
}
fn inherited_fields(catalog:&Catalog,entity:&entities::Definition)->Result<Vec<(String,Value)>,String>{
    let mut result=Vec::new();
    if let Some(source)=reference(entities::field(&entity.fields,"inherit"),&entity.id)?{
        for layer in entities::player::entity_chain(&source,&mut|path|crate::assets::declaration_texts::read_common_or_catalog(catalog,path))?{
            entities::overlay_owned(&mut result,layer);
        }
    }
    //Only callers requiring inherited metadata take this transient overlay.
    //No source text or generic subtree survives CPU preparation.
    entities::overlay_owned(&mut result,entity.fields.clone());Ok(result)
}
fn is_class(types:&ComponentTypes,name:&str,base:&[u8])->Result<bool,String>{
    let classes=types.classes();
    let mut class=classes.hierarchy().registration_order().iter().copied().find(|&index|
        classes.metadata(index).is_some_and(|row|classes.name_text(row.name)==Some(name.as_bytes())))
        .ok_or_else(||format!("AIWorld source has an unregistered entity class {name}"))?;
    loop{
        let row=classes.metadata(class).ok_or("AIWorld class metadata absent")?;
        if classes.name_text(row.name)==Some(base){return Ok(true);}
        let Some(parent)=classes.hierarchy().node(class).and_then(|node|node.base)else{return Ok(false);};class=parent;
    }
}
impl CreationPlan{
    pub(in crate::app) fn prepare(catalog:&Catalog,document:&Document,level:&LevelResources)->Result<Self,String>{
        let types=ComponentTypes::acquire(catalog)?;
        let mut worlds=document.definitions.iter().filter(|entity|entity.kind=="entity"&&atom(&entity.fields,"class")==Some("idWorldspawn"));
        let world=worlds.next().ok_or("AIWorld creation has no actual worldspawn declaration")?;
        if worlds.next().is_some(){return Err("AIWorld creation has multiple worldspawn owners".into());}
        let direct=child(&world.fields,"edit");
        let source=if let Some(value)=entities::field(direct,"m_aiWorldEntityDef"){
            reference(Some(value),"world.m_aiWorldEntityDef")?
        }else{
            let fields=inherited_fields(catalog,world)?;
            reference(entities::field(child(&fields,"edit"),"m_aiWorldEntityDef"),"world.m_aiWorldEntityDef")?
        };
        //A null native reference disables this creation branch. It does not
        //mean "load aiworld_default" or construct a numeric-only recipient.
        if source.is_none(){return Ok(Self{source,sections:Vec::new(),movers:0,navigation:0,navmesh:None});}
        let mut sections=Vec::new();let mut movers=0;
        for entity in document.definitions.iter().filter(|entity|entity.kind=="entity"){
            let inherited;
            let class=if let Some(class)=atom(&entity.fields,"class"){class}else{
                inherited=inherited_fields(catalog,entity)?;
                atom(&inherited,"class").ok_or_else(||format!("{}: actual entity class absent",entity.id))?
            };
            if is_class(&types,class,b"arkWorldSectionVolume")?{
                let model=entities::model_descriptor(entity,|path|crate::assets::declaration_texts::read_common_or_catalog(catalog,path))?;
                let local_bounds = model.as_ref().map(|model| {
                    if model.resource_path.ends_with(".bmodel") {
                        crate::bmodel::acquire(catalog, &model.resource_path, crate::bmodel::CommonModels::shared(catalog))?
                            .map(|model| model.authored_bounds).ok_or_else(||format!("{}: section render model missing: {}",entity.id,model.resource_path))
                    } else if model.resource_path.ends_with(".bmd6model") {
                        crate::bmd6::ModelSources::new(catalog).acquire(catalog, &model.resource_path).map(|model|model.authored_bounds)
                    } else { Err(format!("{}: section model bounds have no actual format owner: {}",entity.id,model.resource_path)) }
                }).transpose()?;
                sections.push(Section{name:Arc::from(entity.id.as_str()),model,local_bounds});
            }else if is_class(&types,class,b"arkMover")?{movers+=1;}
        }
        let navmesh=crate::assets::physworld::navmesh::Metadata::acquire_optional(catalog,
            &crate::assets::physworld::navmesh::companion(level)?)?;
        let mut admission=crate::assets::physworld::navmesh::Admission::default();
        if let Some(metadata)=&navmesh{admission.admit(metadata)?;}
        let navigation=admission.count(0);
        Ok(Self{source,sections,movers,navigation,navmesh})
    }
}
struct Acquisition{cancel:Arc<AtomicBool>,task:Mutex<Task<Result<Arc<Definition>,String>>>}
#[derive(Resource)]
struct Creation{
    plan:CreationPlan,scope:Arc<str>,pending:Option<Acquisition>,source:Option<Arc<Definition>>,
    requested:bool,ids:Vec<EntityId>,next:usize,ready:bool,closing:bool,removals_queued:usize,error:Option<String>,
}
static NEXT_SCOPE:AtomicU64=AtomicU64::new(1);
fn initialize(world:&mut World)->Result<(),String>{
    if world.contains_resource::<Creation>(){return Ok(());}
    let Some(plan)=world.remove_resource::<CreationPlan>()else{return Ok(());};
    let scope=Arc::from(format!("ai-world-source:{}",NEXT_SCOPE.fetch_add(1,Ordering::Relaxed)));
    let mut creation=Creation{plan,scope,pending:None,source:None,requested:false,ids:Vec::new(),next:0,ready:false,closing:false,removals_queued:0,error:None};
    if let Some(path)=creation.plan.source.clone(){
        let Some(catalog)=world.get_resource::<super::super::SceneData>().and_then(|scene|scene.catalog.clone()) else {
            creation.error=Some("AIWorld creation catalog absent".into());world.insert_resource(creation);return Ok(());
        };
        let cancel=Arc::new(AtomicBool::new(false));let cancellation=cancel.clone();
        let task=IoTaskPool::get().spawn(async move{
            let cancelled=||cancellation.load(Ordering::Acquire);
            Definition::acquire(&catalog,&path,Some(&cancelled))
        });
        creation.pending=Some(Acquisition{cancel,task:Mutex::new(task)});
    }else{creation.ready=true;}
    world.insert_resource(creation);Ok(())
}
fn join(creation:&mut Creation)->Result<bool,String>{
    if let Some(pending)=&mut creation.pending{
        if creation.closing{pending.cancel.store(true,Ordering::Release);}
        let Some(result)=block_on(poll_once(pending.task.get_mut().unwrap_or_else(|error|error.into_inner())))else{return Ok(false);};
        creation.pending=None;
        if !creation.closing{creation.source=Some(result?);}
    }Ok(true)
}
fn section_bounds(world:&World,section:&Section,id:EntityId)->Result<Option<[[f32;3];2]>,String>{
    let live=world.get_resource::<LiveEntities>().ok_or("AIWorld section has no managed entity owner")?;
    if section.model.is_some(){
        if world.contains_resource::<super::super::RenderSourcePlacements>()&&world.contains_resource::<super::super::shadow_updates::ModelShadowSources>() {
            if let Some(bounds)=super::super::world_models::entity_bounds(world,live,id)? {return Ok(Some(bounds));}
        }
        //Hidden source models have no colour draw projection. Their BML/MD6
        //owner supplied these authored bounds; never substitute collision AABBs.
        let frame=world.get_resource::<super::super::physics_scene::PhysicsScene>().ok_or("AIWorld section scene absent")?
            .published_entity_frame(live,id)?;
        return Ok(Some(crate::render_model::transformed_bounds(section.local_bounds
            .ok_or("authored section model bounds absent")?,frame.pose.position,frame.pose.basis,
            frame.scale)));
    }
    let frame=world.get_resource::<super::super::physics_scene::PhysicsScene>().ok_or("AIWorld section scene absent")?
        .published_entity_frame(live,id)?;
    Ok(Some([frame.pose.position;2]))
}
fn advance(world:&mut World,creation:&mut Creation)->Result<(),String>{
    if !join(creation)?||creation.closing||creation.ready{return Ok(());}
    let source=creation.source.clone().ok_or("AIWorld source acquisition completed without a declaration")?;
    if !creation.requested{
        super::super::runtime_entities::request_for(world,creation.scope.clone(),source.clone())?;creation.requested=true;
    }
    if !super::super::runtime_entities::ready(world,&source)?{return Ok(());}
    if !world.contains_resource::<crate::gameplay::ai_world::startup_managers::NavigationInstances>() {
        if let Some(metadata)=&creation.plan.navmesh {
            world.insert_resource(crate::gameplay::ai_world::startup_managers::NavigationInstances::new(metadata.clone()));
        }
    }
    if creation.ids.is_empty(){
        let id=super::super::runtime_entities::spawn_ai_world(world,source.clone(),Arc::from("aiWorld default"),
            super::super::runtime_entities::AIWorldBirth{area:0,section:None})?;
        creation.ids.push(id);
        let expected=creation.plan.sections.len()+creation.plan.movers+1;
        if creation.plan.navigation!=expected{
            //8aa493 is a nonfatal count guard; defaultworld is still real.
            warn!("AIWorld section admission: native navigation count {} differs from {} sections + {} movers + 1; retaining default world",creation.plan.navigation,creation.plan.sections.len(),creation.plan.movers);
            creation.ready=true;return Ok(());
        }
    }
    while creation.next<creation.plan.sections.len(){
        let index=creation.next;let section=&creation.plan.sections[index];
        let id=world.resource::<LiveEntities>().find(&section.name)
            .ok_or_else(||format!("{}: actual WorldSectionVolume managed actor was not admitted",section.name))?;
        let Some(bounds)=section_bounds(world,section,id)?else{return Ok(());};
        //8aa499 zeroes EBX before formatting the source section ordinal.
        let born=super::super::runtime_entities::spawn_ai_world(world,source.clone(),Arc::from(format!("aiWorld world section {index}")),
            super::super::runtime_entities::AIWorldBirth{area:index+1,section:Some((id,bounds))})?;
        creation.ids.push(born);creation.next+=1;
    }
    creation.ready=true;Ok(())
}
pub(in crate::app) fn update(world:&mut World){
    if let Err(error)=initialize(world){error!("AIWorld creation setup failed: {error}");return;}
    let Some(mut creation)=world.remove_resource::<Creation>()else{return;};
    if creation.error.is_none(){if let Err(error)=advance(world,&mut creation){error!("AIWorld source creation failed: {error}");creation.error=Some(error);}}
    world.insert_resource(creation);
}
pub(in crate::app) fn startup_pending(world:&World)->bool{
    world.contains_resource::<CreationPlan>()||world.get_resource::<Creation>().is_some_and(|creation|!creation.ready&&!creation.closing)
}
pub(in crate::app) fn startup_error(world:&World)->Option<&str>{world.get_resource::<Creation>().and_then(|creation|creation.error.as_deref())}
///Early shutdown closes source admission and joins the actual archive worker.
///Actors remain alive for their real grenade/danger/script consumers.
pub(in crate::app) fn close_admission(world:&mut World)->Result<bool,String>{
    world.remove_resource::<CreationPlan>();
    let Some(mut creation)=world.remove_resource::<Creation>()else{return Ok(true);};
    creation.closing=true;let result=join(&mut creation);world.insert_resource(creation);result
}
///Call after dependent world actors stop. Ordinary removal owns Stop/Deinit
///and finalID return; this owner waits for those acknowledgements before
///dropping source leases or the actual game-local AIWorld metadata list.
pub(in crate::app) fn shutdown(world:&mut World)->Result<bool,String>{
    if !close_admission(world)?{return Ok(false);}
    let Some(mut creation)=world.remove_resource::<Creation>()else{return Ok(true);};
    let result=(||{
        while creation.removals_queued<creation.ids.len(){
            let id=creation.ids[creation.ids.len()-1-creation.removals_queued];
            let mut live=world.resource_mut::<LiveEntities>();
            if live.get(id).is_some(){live.request_removal(id)?;}
            creation.removals_queued+=1;
        }
        if creation.ids.iter().any(|&id|world.resource::<LiveEntities>().get(id).is_some()){return Ok(false);}
        if creation.requested{
            if let Some(source)=&creation.source{super::super::runtime_entities::release_for(world,&creation.scope,source)?;}
            creation.requested=false;
        }
        if world.get_resource::<crate::gameplay::ai_world::startup_managers::NavigationInstances>().is_some_and(|instances|!instances.is_clear()) {
            return Err("AIWorld navigation listeners survived their actual component deinitialization".into());
        }
        world.remove_resource::<crate::gameplay::ai_world::startup_managers::NavigationInstances>();
        world.remove_resource::<crate::gameplay::ai_world::Worlds>();Ok(true)
    })();
    if !matches!(result,Ok(true)){world.insert_resource(creation);}result
}
