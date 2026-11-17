//! World Iggy players share PauseMenu's worker, file scope and Flash renderer.
//! 4b0b50/4b0bc0 publish real diffuseMap/Color overrides, never a screen layer.
use super::flash_surfaces::Key;
use crate::{assets::flash_surface::{Definition,MapDefinition},rendering::bevy_backend::flash_ui::{
    FlashFrame,FlashWorldAcknowledgement,FlashWorldFrame,FlashWorldFrames,FlashWorldKey}};
use bevy::prelude::*;
use std::{collections::HashMap,sync::{Arc,Mutex,mpsc::{self,Sender,Receiver}}};
mod worker;
mod material;
pub(super) use worker::Worker;

pub(super) struct Command {pub key:Key,pub operation:Operation,pub errors:Sender<Event>}
pub(super) enum Operation {
    Prepare {definition:Arc<Definition>,requires_configuration:bool,events:Sender<Event>},
    ShowMap(Arc<MapDefinition>),Tick(f64),Stop {events:Sender<Event>},
}
pub(super) enum Event {Ready(Key,(f64,f64),Option<FlashFrame>),Frame(Key,Option<FlashFrame>),Configured(Key,FlashFrame),Stopped(Key),Sound(Key,u32),Failed(Key,String)}
#[derive(Clone,Copy,Default)]
pub(super) struct Status {pub loaded:bool,pub playing:bool,pub captured:bool,pub configured:bool,pub retired:bool}
struct Entry {
    status:Status,
    error:Option<String>,
    target:Option<Handle<Image>>,
    ack:Option<Arc<FlashWorldAcknowledgement>>,
    serial:Option<u64>,
    published:bool,
    map_configuring:bool,
    pending_delta:f64,
    bindings:material::Bindings,
    in_flight:bool,
    stopping:bool,
    capture_stop:bool,
    stopped:bool,
    removing:bool,
    withdrawn:bool,
}
impl Default for Entry {fn default()->Self{Self{status:Default::default(),error:None,target:None,ack:None,
    serial:None,published:false,map_configuring:false,pending_delta:0.,bindings:Default::default(),in_flight:false,stopping:false,capture_stop:false,stopped:false,removing:false,withdrawn:false}}}
#[derive(Resource)]
struct Movies {entries:HashMap<Key,Entry>,models:HashMap<crate::gameplay::entity_lifetime::EntityId,material::Bindings>,events:Mutex<Receiver<Event>>,sender:Sender<Event>}
impl Default for Movies {fn default()->Self{let(sender,events)=mpsc::channel();Self{entries:HashMap::new(),models:HashMap::new(),events:Mutex::new(events),sender}}}
/// Called during actual level preparation, before LevelLifetime captures its
/// resource roster. Requests must not introduce an untracked level owner.
pub(super) fn initialize(world:&mut World){world.init_resource::<Movies>();}
/// Called only after the real Scene Stop/remove acknowledgements. The session
/// Iggy worker outlives a map, but no movie key may outlive its component.
pub(super) fn shutdown(world:&mut World)->Result<bool,String>{
    poll(world)?;
    if !world.contains_resource::<Movies>(){return Ok(true)}
    if world.get_resource::<Movies>().is_some_and(|movies|!movies.entries.is_empty()){
        return Err("Scene retired before its world movie generations were forgotten".into())
    }
    with_movies(world,|world,movies|{
        while let Some(id)=movies.models.keys().next().copied(){
            movies.models.get_mut(&id).unwrap().restore(world)?;movies.models.remove(&id);
        }Ok(())
    })?;
    world.remove_resource::<Movies>();Ok(true)
}
///Actual render-model destruction releases its parameter override scope.
///Ordinary component Stop instead keeps the source `_default`/white values.
pub(super) fn retire_model(world:&mut World,id:crate::gameplay::entity_lifetime::EntityId)->Result<(),String>{
    if !world.contains_resource::<Movies>(){return Ok(())}
    with_movies(world,|world,movies|{
        if movies.entries.keys().any(|key|key.entity==id){return Err("render model retired before its FlashSurface Stop acknowledgement".into())}
        if let Some(bindings)=movies.models.get_mut(&id){bindings.restore(world)?;}
        movies.models.remove(&id);Ok(())
    })
}
///Only a real accepted movie needs the pre-model native component Stop fence.
///Ordinary actors avoid borrowing/copying Scene for an unrelated UI owner.
pub(super) fn model_retirement_pending(world:&World,id:crate::gameplay::entity_lifetime::EntityId)->bool{
    world.get_resource::<Movies>().is_some_and(|movies|movies.entries.keys().any(|key|key.entity==id))
}
fn render_key(key:Key)->FlashWorldKey{FlashWorldKey{entity:key.entity,handle:key.handle.packed(),generation:key.generation}}
fn send(world:&World,key:Key,operation:Operation,errors:&Sender<Event>)->Result<(),String>{
    world.get_non_send_resource::<super::pause_menu::PauseMenu>().ok_or("world movie Iggy worker absent")?
        .surface_command(Command{key,operation,errors:errors.clone()})
}
fn with_movies<T>(world:&mut World,run:impl FnOnce(&mut World,&mut Movies)->Result<T,String>)->Result<T,String>{
    let mut movies=world.remove_resource::<Movies>().ok_or("world movie level owner not initialized")?;
    let result=run(world,&mut movies);world.insert_resource(movies);result
}
pub(super) fn request(world:&mut World,key:Key,definition:Arc<Definition>,requires_configuration:bool)->Result<(),String>{
    if definition.iggy_info.is_empty(){return Err("world FlashSurface has no IggyInfo".into())}
    with_movies(world,|world,movies|{
        if movies.entries.contains_key(&key){return Err("world movie key reused before retirement acknowledgement".into())}
        send(world,key,Operation::Prepare{definition,requires_configuration,events:movies.sender.clone()},&movies.sender)?;
        let bindings=movies.models.remove(&key.entity).unwrap_or_default();
        movies.entries.insert(key,Entry{bindings,in_flight:true,..Default::default()});Ok(())
    })
}
pub(super) fn status(world:&mut World,key:Key)->Result<Option<Status>,String>{
    let Some(movies)=world.get_resource::<Movies>() else{return Ok(None)};
    let Some(entry)=movies.entries.get(&key) else{return Ok(None)};
    if !entry.status.retired{if let Some(error)=&entry.error{return Err(error.clone())}}Ok(Some(entry.status))
}
pub(super) fn show_map(world:&mut World,key:Key,map:Arc<MapDefinition>)->Result<(),String>{
    let entry=world.get_resource::<Movies>().and_then(|movies|movies.entries.get(&key)).ok_or("world map movie not requested")?;
    if !entry.status.loaded||entry.stopping{return Err("world map show targeted inactive playback".into())}
    send(world,key,Operation::ShowMap(map),&world.resource::<Movies>().sender)?;
    let entry=world.resource_mut::<Movies>().into_inner().entries.get_mut(&key).ok_or("world map generation lost")?;
    entry.status.configured=false;entry.status.captured=false;entry.map_configuring=false;entry.in_flight=true;Ok(())
}
pub(super) fn stop(world:&mut World,key:Key)->Result<(),String>{
    with_movies(world,|world,movies|{
        let entry=movies.entries.get_mut(&key).ok_or("world movie stop targeted unknown generation")?;
        if !entry.stopping{
            if entry.status.captured&&!entry.in_flight{send(world,key,Operation::Stop{events:movies.sender.clone()},&movies.sender)?;entry.stopping=true;}
            else{entry.capture_stop=true;}
        }Ok(())
    })
}
pub(super) fn remove(world:&mut World,key:Key)->Result<(),String>{
    with_movies(world,|world,movies|{
        let entry=movies.entries.get_mut(&key).ok_or("world movie removal lost generation")?;
        if !entry.stopping{send(world,key,Operation::Stop{events:movies.sender.clone()},&movies.sender)?;entry.stopping=true;}
        entry.capture_stop=false;entry.removing=true;Ok(())
    })
}
pub(super) fn forget(world:&mut World,key:Key)->Result<(),String>{
    let Some(mut movies)=world.get_resource_mut::<Movies>() else{return Err("world movie retirement owner absent".into())};
    if !movies.entries.get(&key).is_some_and(|entry|entry.status.retired){return Err("world movie generation forgotten before target retirement".into())}
    if movies.models.contains_key(&key.entity){return Err("model already owns a stopped FlashSurface parameter scope".into())}
    let entry=movies.entries.remove(&key).unwrap();movies.models.insert(key.entity,entry.bindings);Ok(())
}
pub(super) fn tick(world:&mut World,dt:f32)->Result<(),String>{
    if !dt.is_finite()||dt<0.{return Err("invalid world movie clock delta".into())}
    if dt==0.||!world.contains_resource::<Movies>(){return Ok(())}
    with_movies(world,|world,movies|{for (&key,entry) in &mut movies.entries{
        if !entry.stopping {entry.pending_delta+=f64::from(dt);}
        if entry.status.loaded&&!entry.stopping&&!entry.capture_stop&&!entry.in_flight
            &&entry.ack.as_ref().zip(entry.serial).is_some_and(|(ack,serial)|ack.rendered(serial)){
            send(world,key,Operation::Tick(entry.pending_delta),&movies.sender)?;entry.pending_delta=0.;entry.in_flight=true;
        }
    }Ok(())})
}
/// Both ordinary frames and outgoing-level Stop call this same bounded owner.
pub(super) fn poll(world:&mut World)->Result<(),String>{
    if !world.contains_resource::<Movies>(){return Ok(())}
    with_movies(world,|world,movies|{
        for entry in movies.entries.values_mut(){observe_render(entry);}
        loop {
            let event=movies.events.lock().map_err(|_|"world movie mailbox poisoned")?.try_recv();
            let event=match event{Ok(event)=>event,Err(mpsc::TryRecvError::Empty)=>break,
                Err(mpsc::TryRecvError::Disconnected)=>return Err("world movie mailbox disconnected".into())};
            match event {
                Event::Ready(key,(w,h),frame)=>{
                    let entry=movies.entries.get_mut(&key).ok_or("world movie ready has stale generation")?;
                    if !w.is_finite()||!h.is_finite()||w<=0.||h<=0.{return Err("world movie has invalid target dimensions".into())}
                    //02c010/2f4060 create typeless storage with an sRGB
                    //default sample view.31a600 overrides only GDraw's RTV
                    //to UNORM; both views share this one physical image.
                    let mut image=Image::new_target_texture(w.ceil() as u32,h.ceil() as u32,
                        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,None);
                    image.texture_descriptor.view_formats=&[bevy::render::render_resource::TextureFormat::Rgba8Unorm];
                    let target=world.resource_mut::<Assets<Image>>().add(image);
                    entry.target=Some(target);entry.status.loaded=true;entry.status.playing=true;entry.in_flight=false;
                    if let Some(frame)=frame {publish(world,key,entry,frame)?;}
                }
                Event::Frame(key,frame)=>{
                    let entry=movies.entries.get_mut(&key).ok_or("world movie frame has stale generation")?;
                    entry.in_flight=false;
                    if let Some(frame)=frame {publish(world,key,entry,frame)?;}
                }
                Event::Configured(key,frame)=>{
                    let entry=movies.entries.get_mut(&key).ok_or("world map configuration has stale generation")?;
                    entry.in_flight=false;entry.map_configuring=true;
                    publish(world,key,entry,frame)?;
                }
                Event::Stopped(key)=>{let entry=movies.entries.get_mut(&key).ok_or("world movie stop has stale generation")?;
                    entry.stopped=true;entry.in_flight=false;}
                Event::Failed(key,error)=>{let entry=movies.entries.get_mut(&key).ok_or("world movie failure has stale generation")?;
                    entry.error=Some(error);entry.in_flight=false;}
                Event::Sound(key,id)=>{
                    if !movies.entries.contains_key(&key){return Err("world movie sound has stale generation".into())}
                    //02ca20 forwards the movie's numeric SoundEvent via the
                    //same canonical UI sound route as other installed movies.
                    world.write_message(super::audio::SoundEvent(crate::gameplay::sound::Request::Post{
                        event:id,emitter:crate::gameplay::sound::Emitter::Menu}));
                }
            }
        }
        for (&key,entry) in &mut movies.entries {
            if !entry.removing{if let Some(error)=&entry.error{return Err(error.clone())}}
            if !entry.removing {
                if entry.published {
                    if let Some(target)=&entry.target {entry.bindings.bind(world,key.entity,target,true)?;}
                } else {
                    let default=default_texture(world)?;entry.bindings.bind(world,key.entity,&default,false)?;
                }
                observe_render(entry);
                entry.status.captured&=entry.bindings.bound();
                entry.status.playing=entry.status.loaded&&!entry.stopped;
                if entry.capture_stop&&entry.status.captured&&!entry.in_flight{
                    send(world,key,Operation::Stop{events:movies.sender.clone()},&movies.sender)?;entry.stopping=true;entry.capture_stop=false;
                }
                if entry.stopped {
                    //Static capture keeps only the actual target on its model.
                    //Release the movie draw list/bitmap/glyph leases and mutable
                    //GDraw buffers through the same real retirement handshake.
                    withdraw(world,key,entry);
                    if entry.withdrawn&&entry.ack.as_ref().is_some_and(|ack|ack.retired()){
                        entry.ack=None;entry.serial=None;
                    }
                }
            } else if entry.stopped {
                let default=default_texture(world)?;entry.bindings.set_target(world,&default,false)?;
                //A pre-first-render Stop still traverses the actual render-owner
                //admission/withdrawal boundary. Never invent an absent-GPU ACK.
                withdraw(world,key,entry);
                if entry.withdrawn&&entry.ack.as_ref().is_none_or(|ack|ack.retired()){
                    entry.target=None;entry.ack=None;entry.serial=None;entry.status=Status{retired:true,..Default::default()};
                }
            }
        }
        if !movies.entries.is_empty()&&world.get_non_send_resource::<super::pause_menu::PauseMenu>().is_none_or(|menu|menu.ui_worker_failed()){
            return Err("Iggy worker exited with world movie generations still admitted".into())
        }
        Ok(())
    })
}
fn publish(world:&mut World,key:Key,entry:&mut Entry,frame:FlashFrame)->Result<(),String>{
    entry.published=true;
    entry.status.captured=false;
    entry.serial=Some(frame.serial);
    let target=entry.target.clone().ok_or("world movie frame target absent")?;
    let ack=entry.ack.get_or_insert_with(Arc::default).clone();
    world.get_resource_mut::<FlashWorldFrames>().ok_or("world Flash renderer not installed")?.0.insert(render_key(key),FlashWorldFrame{frame,target,ack});
    Ok(())
}
fn default_texture(world:&World)->Result<Handle<Image>,String>{
    world.get_resource::<super::resident_images::ResidentImages>().and_then(|images|images.default_texture())
        .ok_or_else(||"world movie reset requires the prepared ImageManager _default view".into())
}
fn observe_render(entry:&mut Entry){
    if entry.ack.as_ref().zip(entry.serial).is_some_and(|(ack,serial)|ack.rendered(serial)){
        entry.status.captured=true;
        if entry.map_configuring {entry.status.configured=true;entry.map_configuring=false;}
    }
}
fn withdraw(world:&mut World,key:Key,entry:&mut Entry){
    if entry.ack.as_ref().is_none_or(|ack|ack.admitted())&&!entry.withdrawn {
        if let Some(mut frames)=world.get_resource_mut::<FlashWorldFrames>(){frames.0.remove(&render_key(key));}
        entry.withdrawn=true;
    }
}
