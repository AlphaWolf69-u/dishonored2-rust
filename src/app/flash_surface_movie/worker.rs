//! 4b0b50/4b0d00 request the existing Iggy owner; 39efb0 invokes the map movie.
use super::{Command, Event, Key, Operation};
use crate::{flash::{Player, render, bitmap_resources::{MovieBitmaps,PackLifetime}},
    rendering::bevy_backend::flash_ui::FlashFrame};
use std::{collections::HashMap,sync::{Arc,mpsc::Sender}};

#[derive(Default)]
pub(in crate::app) struct Worker { movies: HashMap<Key, Movie> }
struct Movie {
    player: Player,
    images: Arc<MovieBitmaps>,
    cache: render::Cache,
    generation: u64,
    serial: u64,
    pending: f64,
    elapsed: f64,
    interval: f64,
    events: Sender<Event>,
    _residency: Option<super::super::pause_menu::MovieResidency>,
}
impl Worker {
    pub(in crate::app) fn command(&mut self, command: Command, generation: &mut u64) {
        let Command {key,operation,errors}=command;
        let result=(||->Result<(),String>{match operation {
            Operation::Prepare {definition,requires_configuration,events}=>{
                if self.movies.contains_key(&key) {return Err("world movie already admitted for this component generation".into())}
                *generation=generation.wrapping_add(1);
                let movie=Movie::load(&definition.iggy_info,*generation,events.clone());
                match movie {
                    Ok(mut movie)=>{
                        //199's synchronous map callback configures the movie
                        //before its first ordinary draw. Do not publish an
                        //unconfigured map merely to discover its real size.
                        let stage=movie.player.state.borrow().stage_size;
                        let frame=(!requires_configuration).then(||movie.frame());
                        let _=events.send(Event::Ready(key,stage,frame));
                        self.movies.insert(key,movie);
                    }
                    Err(error)=>{let _=events.send(Event::Failed(key,error));}
                }
            }
            Operation::ShowMap(definition)=>{
                let movie=self.movies.get_mut(&key).ok_or("map show targeted an unloaded world movie")?;
                let strings=movie.player.state.borrow().strings.clone().ok_or("world movie strings absent")?;
                let legends=movie.player.array(definition.legends.iter().map(|name|strings.lookup(name).into()).collect());
                movie.player.call_root_method("_C_Show",&[strings.lookup(&definition.title).into(),legends,
                    definition.has_player_position.into(),definition.theme.into()])
                    .map_err(|error|movie.player.describe(&error))?;
                movie.check_uncaught()?;
                let frame=movie.frame();
                let _=movie.events.send(Event::Configured(key,frame));
            }
            Operation::Tick(dt)=>{
                let movie=self.movies.get_mut(&key).ok_or("tick targeted an unloaded world movie")?;
                movie.pending+=dt;movie.elapsed+=dt;
                let frame=if movie.pending>=movie.interval {
                    let elapsed=std::mem::take(&mut movie.elapsed);
                    movie.pending=(movie.pending-movie.interval).max(0.);
                    movie.player.tick(elapsed).map_err(|error|movie.player.describe(&error))?;
                    Some(movie.frame())
                } else {None};
                for (name,args) in movie.player.take_notifications() {
                    if name=="PostSoundEvent" {
                        if let Some(value)=args.first(){let id=crate::flash::avm2::value::to_uint32(value.primitive_to_number());
                            let _=movie.events.send(Event::Sound(key,id));}
                    } else {super::super::pause_menu::report_unknown_callback("FlashSurface",&name);}
                }
                movie.check_uncaught()?;
                let _=movie.events.send(Event::Frame(key,frame));
            }
            Operation::Stop {events}=>{
                //0310c0 releases the live player; an accepted static frame and
                //render target independently retain their immutable image leases.
                self.movies.remove(&key);
                let _=events.send(Event::Stopped(key));
            }
        }Ok(())})();
        if let Err(error)=result {self.movies.remove(&key);let _=errors.send(Event::Failed(key,error));}
    }
}
impl Movie {
    fn load(reference:&str,generation:u64,events:Sender<Event>)->Result<Self,String>{
        let catalog=super::super::pause_menu::installed_catalog()?;
        let language=crate::assets::lang::read_text_language(catalog.base())?;
        let (path,descriptor)=super::super::pause_menu::iggy_movie_info(&catalog,reference)?;
        if crate::gameplay::options::decl::boolean(&descriptor,"m_isFullScreenUI",true){
            return Err(format!("world FlashSurface requires a non-fullscreen Iggy descriptor: {reference}"))
        }
        let (player,resources)=Player::load_installed(&catalog,language,&path)?;
        //World playback/static capture is a real resource scope. Coalesce
        //with any already-live/session library, without promoting this
        //surface's otherwise unowned root bitmap pack to session residency.
        let images=super::super::pause_menu::library_images(&catalog,&player,&resources,PackLifetime::Movie)?;
        let residency=super::super::pause_menu::MovieResidency::new("world-surface",&catalog,&resources,&images);
        let state=player.state.borrow();let stage=state.stage_size;let interval=1./state.frame_rate;drop(state);
        if !stage.0.is_finite()||!stage.1.is_finite()||stage.0<=0.||stage.1<=0.||!interval.is_finite()||interval<=0.{return Err("invalid world movie stage or interval".into())}
        let mut cache=render::Cache::default();
        cache.set_font_caching(crate::gameplay::pause_state::font_caching_fields(&descriptor,&path,stage,(stage.0.ceil() as u32,stage.1.ceil() as u32)));
        let mut movie=Self{player,images:Arc::new(images),cache,generation,serial:0,pending:0.,elapsed:0.,interval,events,_residency:residency};
        movie.check_uncaught()?;Ok(movie)
    }
    fn frame(&mut self)->FlashFrame{
        self.serial=self.serial.wrapping_add(1);
        FlashFrame {list:Arc::new(render::draw_list_cached(&self.player.vm,&mut self.cache)),images:self.images.clone(),
            game_images:Default::default(),generation:self.generation,serial:self.serial}
    }
    fn check_uncaught(&mut self)->Result<(),String>{
        let uncaught=std::mem::take(&mut self.player.state.borrow_mut().uncaught);
        if uncaught.is_empty(){Ok(())}else{Err(format!("world movie: {}",uncaught.join("; ")))}
    }
}
