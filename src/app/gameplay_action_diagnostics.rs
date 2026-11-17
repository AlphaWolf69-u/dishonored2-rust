//! Opt-in, windowless production-input smoke. Expectations come from installed
//! Corvo shortcut/loadout data and source C8 firing / draw / Undraw contracts.
//! No fake assets, actor births, animation keys, clock ticks or upload ACKs.
use super::{interaction_focus::DiscoveredDocuments, player::PlayerState};
use crate::{gameplay::{entity_lifetime::EntityId, player::{Player, states::StateId, weapons::RangedFamily}}, platform::{bindings, console}};
use bevy::{input::{ButtonState, keyboard::{KeyboardInput, Key, NativeKey}, mouse::MouseButtonInput}, prelude::*};
use std::{collections::HashSet, time::{Duration, Instant}};

#[derive(Resource, Default)]
struct Receipts { heart_uses: u64, heart: Option<EntityId> }
/// Called only after the real Heart owner accepts its pending use request.
pub(super) fn heart_used(world: &mut World, entity: EntityId, selected:bool) {
    if let Some(mut receipts) = world.get_resource_mut::<Receipts>() {
        receipts.heart_uses += 1;
        receipts.heart = Some(entity);
        eprintln!("ACTION_SMOKE_HEART_USE entity={entity:?} processed=true bark_selection_accepted={selected}");
    }
}

#[derive(Clone, Copy, Debug)]
enum Input { Key(KeyCode), Mouse(MouseButton) }
fn binding(input: Input) -> Option<i32> {
    // Reuse the production sampler's DirectInput map, including MOUSE4/5.
    let key = match input {
        Input::Key(key) => super::engine_keys::engine_key(key)?,
        Input::Mouse(button) => super::player::mouse_button_key(button)?,
    };
    console::shared().read().bindings.sets[bindings::IN_GAME].get(key as usize).map(|binding|binding.action)
}
fn input_for(action: i32) -> Result<Input, String> {
    use KeyCode::*;
    let keys = [Digit1,Digit2,Digit3,Digit4,Digit5,Digit6,Digit7,Digit8,Digit9,Digit0,
        KeyA,KeyB,KeyC,KeyD,KeyE,KeyF,KeyG,KeyH,KeyI,KeyJ,KeyK,KeyL,KeyM,KeyN,KeyO,KeyP,KeyQ,KeyR,KeyS,KeyT,KeyU,KeyV,KeyW,KeyX,KeyY,KeyZ,
        Tab,CapsLock,Space,Enter,ShiftLeft,ShiftRight,ControlLeft,ControlRight,AltLeft,AltRight,Backquote,Backspace,
        ArrowUp,ArrowDown,ArrowLeft,ArrowRight,Home,End,PageUp,PageDown,Insert,Delete,
        F1,F2,F3,F4,F5,F6,F7,F8,F9,F10,F11,F12];
    [MouseButton::Left,MouseButton::Right,MouseButton::Middle,MouseButton::Back,MouseButton::Forward].into_iter().map(Input::Mouse)
        .chain((0..7).map(|n|Input::Mouse(MouseButton::Other(n))))
        .chain(keys.into_iter().map(Input::Key)).find(|input|binding(*input)==Some(action))
        .ok_or_else(||format!("action {action} has no supported physical binding in the actual IN_GAME set"))
}
fn send(world: &mut World, input: Input, down: bool) -> Result<(), String> {
    let window = world.query_filtered::<Entity, With<Window>>().iter(world).next().ok_or("offscreen Window component absent")?;
    let state = if down { ButtonState::Pressed } else { ButtonState::Released };
    match input {
        Input::Key(key_code) => { world.write_message(KeyboardInput { key_code, logical_key:Key::Unidentified(NativeKey::Unidentified), state, text:None, repeat:false, window }); }
        Input::Mouse(button) => { world.write_message(MouseButtonInput { button, state, window }); }
    }
    eprintln!("ACTION_SMOKE_INPUT physical={input:?} action={:?} down={down}",binding(input));
    Ok(())
}
fn player(world: &World) -> Result<&Player, String> {
    world.get_resource::<PlayerState>().and_then(|state|state.player.as_ref()).ok_or_else(||"actual Player absent".into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage { Begin, Crossbow, Bolt, Gun, GunShot, Empty, Sheath, Draw, Focus, FocusFade, Heart, HeartUse, WheelOpen, WheelClose, PickObject, ThrowObject, ObjectMotion, Done }
pub(super) struct Observation {
    stage: Stage,
    since: Instant,
    release: Option<Input>,
    held: Option<Input>,
    before_ammo: u32,
    initial_gun_ammo: Option<u32>,
    seen_projectiles: HashSet<EntityId>,
    shot_count: u32,
    empty_scene: Option<u32>,
    heart_before: u64,
    heart_action: i32,
    projectile_seen: bool,
    object: Option<(EntityId,[f32;3])>,
    focus_samples: u32,
    focus_nonzero: bool,
    focus_release_attack: bool,
    expected_bolt: Option<(EntityId,String)>,
    gun_trigger_pending: bool,
}
impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        if super::pause_menu::debug_character() != crate::gameplay::pause_state::Character::Corvo {
            return Err("action smoke requires actual Corvo loadout (DH2_DEBUG_CHARACTER=corvo)".into());
        }
        if super::player::debug_benchmark_walk_enabled() {
            return Err("action smoke requires benchmark walking disabled; it must not steer or teleport while firing".into());
        }
        for (key, expected) in [(KeyCode::Digit1,25),(KeyCode::Digit3,27)] {
            if binding(Input::Key(key)) != Some(expected) { return Err(format!("installed shortcut input contract differs: {key:?} expected action{expected}")); }
        }
        world.insert_resource(Receipts::default());
        eprintln!("ACTION_SMOKE_BEGIN provenance=installed_Corvo_loadout+03ab70_input+4de920_shortcuts+63a2b0_C8+802f90_Undraw setup=actual_level_spawn no_teleport=true no_fake_ACK=true");
        Ok(Self { stage:Stage::Begin,since:Instant::now(),release:None,held:None,before_ammo:0,initial_gun_ammo:None,
            seen_projectiles:HashSet::new(),shot_count:0,empty_scene:None,heart_before:0,heart_action:0,projectile_seen:false,object:None,
            focus_samples:0,focus_nonzero:false,focus_release_attack:false,expected_bolt:None,gun_trigger_pending:false })
    }
    fn stage(&mut self, next: Stage, world: &World) -> Result<(), String> {
        eprintln!("ACTION_SMOKE_STAGE completed={:?} next={next:?} shots={} ammo_before={} elapsed={:.3}",self.stage,self.shot_count,self.before_ammo,self.since.elapsed().as_secs_f64());
        let mut snapshot=String::new(); player(world)?.write_combat_diagnostics(&mut snapshot);
        eprintln!("ACTION_SMOKE_SNAPSHOT stage={next:?}{snapshot}");
        self.stage=next;self.since=Instant::now();Ok(())
    }
    fn tap(&mut self, world: &mut World, action: i32) -> Result<(), String> {
        let input=input_for(action)?;send(world,input,true)?;self.release=Some(input);Ok(())
    }
    fn select(&mut self, world: &mut World, key: KeyCode) -> Result<(), String> {
        let input=Input::Key(key);send(world,input,true)?;self.release=Some(input);Ok(())
    }
    fn ranged_ready(world: &World, family: RangedFamily) -> Result<bool, String> {
        let p=player(world)?;
        let Some(weapon)=&p.weapon else{return Ok(false)};
        if weapon.settings.family != family || !weapon.is_active() || !weapon.can_fire() {return Ok(false)}
        let Some((_,equipped,transitioning))=p.equipment_action_status(0)else{return Ok(false)};
        Ok(equipped && !transitioning && weapon.scene_handle()==0
            && !p.machine.is_active(StateId::FIRE_WEAPON) && !p.machine.is_active(StateId::CANCEL_FIRE_WEAPON))
    }
    fn projectiles(&mut self, world: &World, collect_existing: bool) -> Result<bool,String> {
        let scene=world.get_resource::<super::physics_scene::PhysicsScene>().ok_or("actual PhysicsScene absent")?;
        let mut batch=Vec::new();let mut stops=Vec::new();scene.projectile_batch(&mut batch,&mut stops)?;
        let mut moved=false;
        for (handle,id) in batch {
            if collect_existing {self.seen_projectiles.insert(id);continue}
            if self.seen_projectiles.contains(&id) {continue}
            let projectile=scene.projectile_runtime(handle)?;
            if !self.expected_bolt.as_ref().is_some_and(|(attacker,source)|projectile.attacker==Some(*attacker)&&projectile.declaration.entity.as_str()==source.as_str()){continue}
            // A real newly Started component with a changed source-origin is
            // required. A loaded-bolt model/Prepared definition cannot pass.
            if projectile.position != projectile.origin {
                eprintln!("ACTION_SMOKE_PROJECTILE id={id:?} source={} origin={:?} position={:?} velocity={:?} moving={}",
                    projectile.declaration.entity,projectile.origin,projectile.position,projectile.velocity,projectile.movement_enabled);
                moved=true;
            }
        }
        Ok(moved)
    }
    fn focus_curve(&mut self,world:&World)->Result<(bool,usize,bool),String>{
        // Bounded diagnostic output only; production retains no frame history.
        if self.focus_samples>=4096{return Err("focus action exceeded bounded 4096-frame diagnostic curve".into())}
        self.focus_samples+=1;
        let clock=world.resource::<crate::gameplay::game_clock_runtime::ClockRuntime>().player_time().map_err(str::to_owned)?;
        let p=player(world)?;
        let fx=p.combat_fx.as_ref().ok_or("actual focused-attack FX owner absent")?;
        let mut count=0;let mut full=true;let mut nonzero=false;
        fx.visit_diagnostics(|row|{
            count+=1;full &= row.producer.blend>=1.;nonzero |= row.producer.blend>0.;
            let timing=&row.producer.timing;
            eprintln!("ACTION_SMOKE_FOCUS frame={} clock={} stage={:?} requested={} definition={} identity={:#x} time={} blend={} stopping={} active={} authored_fade_in={:?} authored_fade_out={:?} parent_fade_in={:?} parent_fade_out={:?} delay_end={} fade_in_end={} duration_end={} fading_out={} initial_main={} last_main={} targets={:?}",
                self.focus_samples,clock,self.stage,fx.focus_requested(),row.definition,row.definition_identity,row.time,row.producer.blend,row.stopping,row.producer.active,
                row.authored.timing.fade_in,row.authored.timing.fade_out,row.parent_timing.fade_in,row.parent_timing.fade_out,
                timing.delay_end,timing.fade_in_end,timing.duration_end,timing.fading_out,row.producer.initial_main,row.producer.last_main,row.targets);
        });
        if nonzero&&!self.focus_nonzero{
            eprintln!("ACTION_SMOKE_FOCUS_FIRST_NONZERO frame={} clock={} sources={:?}",self.focus_samples,clock,fx.definition_sources());
            self.focus_nonzero=true;
        }
        let camera=world.get_resource::<crate::rendering::bevy_backend::display_scene::DisplaySceneFrame>().and_then(|frame|frame.0.as_ref()).and_then(|frame|frame.camera_effects.as_ref())
            .ok_or("actual resolved camera-effects frame absent during focus")?;
        let combine=camera.settings.combine(camera.far_clip);
        eprintln!("ACTION_SMOKE_FOCUS_CAMERA frame={} clock={} requested={} producer_count={} near_sharp={} far_sharp={} autofocus={} near_dof={} opaque_enabled={} words={:?} player_states={:?}",
            self.focus_samples,clock,fx.focus_requested(),count,camera.settings.near_sharp_distance(),camera.settings.far_sharp_distance(),camera.settings.autofocus_enabled(),combine.options[5],camera.opaque_enabled,camera.settings.words(),p.active_player_states());
        if self.stage==Stage::FocusFade{self.focus_release_attack |= p.machine.is_active(StateId::SLASH);}
        Ok((fx.focus_requested(),count,count>0&&full))
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool,String> {
        // Sample every actual app frame across hold/release, including the
        // input-release frame, so endpoint-only evidence cannot hide a snap.
        let focus=if matches!(self.stage,Stage::Focus|Stage::FocusFade){Some(self.focus_curve(world)?)}else{None};
        if let Some(input)=self.release.take(){send(world,input,false)?;return Ok(false)}
        if let Some(error)=super::weapon_firing::startup_error(world){return Err(format!("actual firing owner failed during {:?}: {error}",self.stage))}
        if let Some(error)=super::heart::startup_error(world){return Err(format!("actual Heart owner failed during {:?}: {error}",self.stage))}
        if self.since.elapsed()>Duration::from_secs(90) {
            let mut snapshot=String::new();player(world)?.write_combat_diagnostics(&mut snapshot);
            super::weapon_firing::write_diagnostics(world,&mut snapshot);
            return Err(format!("action smoke timeout stage={:?} shots={} projectile_seen={} snapshot={snapshot}",self.stage,self.shot_count,self.projectile_seen));
        }
        match self.stage {
            Stage::Begin => {
                self.projectiles(world,true)?;
                self.select(world,KeyCode::Digit1)?;self.stage(Stage::Crossbow,world)?;
            }
            Stage::Crossbow if Self::ranged_ready(world,RangedFamily::Crossbow)? => {
                let weapon=player(world)?.weapon.as_ref().unwrap();
                self.before_ammo=weapon.total_ammo;
                let source=weapon.fire_source.as_ref().ok_or("ready crossbow lost actual firing source")?;
                self.expected_bolt=Some((source.attacker,source.prepared.declaration.entity.clone()));
                self.tap(world,20)?;self.stage(Stage::Bolt,world)?;
            }
            Stage::Bolt => {
                let moved=self.projectiles(world,false)?;
                self.projectile_seen |= moved;
                let consumed=player(world)?.weapon.as_ref().is_some_and(|weapon|weapon.total_ammo<self.before_ammo);
                let settled=player(world)?.equipment_action_status(0).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)
                    && player(world)?.weapon.as_ref().is_some_and(|weapon|weapon.scene_handle()==0)
                    && !player(world)?.machine.is_active(StateId::FIRE_WEAPON);
                if consumed && self.projectile_seen && settled {
                    eprintln!("ACTION_SMOKE_PASS check=crossbow_real_C8_spawn_motion");
                    self.select(world,KeyCode::Digit3)?;self.stage(Stage::Gun,world)?;
                }
            }
            Stage::Gun if Self::ranged_ready(world,RangedFamily::Gun)? => {
                let total=player(world)?.weapon.as_ref().unwrap().total_ammo;
                self.initial_gun_ammo=Some(total);self.before_ammo=total;
                self.tap(world,20)?;self.gun_trigger_pending=true;self.stage(Stage::GunShot,world)?;
            }
            Stage::GunShot => {
                let p=player(world)?;let weapon=p.weapon.as_ref().ok_or("pistol disappeared before exhaustion")?;
                if weapon.total_ammo<self.before_ammo {
                    self.shot_count+=1;self.before_ammo=weapon.total_ammo;
                    self.gun_trigger_pending=false;
                    eprintln!("ACTION_SMOKE_GUN_C8 shots={} remaining={} magazine={}",self.shot_count,self.before_ammo,weapon.magazine);
                }
                if weapon.total_ammo==0 && weapon.magazine==0 && weapon.scene_handle()==0 && !p.machine.is_active(StateId::FIRE_WEAPON) {
                    eprintln!("ACTION_SMOKE_PASS check=gun_actual_loadout_exhaust initial={:?} shots={}",self.initial_gun_ammo,self.shot_count);
                    self.tap(world,20)?;self.stage(Stage::Empty,world)?;
                } else if !self.gun_trigger_pending && Self::ranged_ready(world,RangedFamily::Gun)? {
                    self.tap(world,20)?;self.gun_trigger_pending=true;
                }
            }
            Stage::Empty => {
                let p=player(world)?;let weapon=p.weapon.as_ref().ok_or("empty pistol disappeared")?;
                if weapon.total_ammo!=0 || weapon.magazine!=0{return Err("empty fire manufactured ammunition".into())}
                if weapon.scene_handle()!=0{self.empty_scene=Some(weapon.scene_handle())}
                if self.empty_scene.is_some_and(|handle|p.animation_scene_finished(handle)) && !p.machine.is_active(StateId::FIRE_WEAPON) {
                    eprintln!("ACTION_SMOKE_PASS check=empty_fire_actual_scene handle={:?}",self.empty_scene);
                    let input=input_for(22)?;send(world,input,true)?;self.held=Some(input);self.stage(Stage::Sheath,world)?;
                }
            }
            Stage::Sheath => {
                let p=player(world)?;
                if (0..2).all(|side|p.equipment_action_status(side).is_some_and(|(_,equipped,transitioning)|!equipped&&!transitioning)) {
                    if let Some(input)=self.held.take(){send(world,input,false)?}
                    eprintln!("ACTION_SMOKE_PASS check=both_hands_sheathed");
                    // Physical attack draws through the normal controller;
                    // no direct equipment request or selection callback.
                    self.tap(world,19)?;self.stage(Stage::Draw,world)?;
                }
            }
            Stage::Draw => {
                let p=player(world)?;
                if p.equipment_action_status(1).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning) && !p.machine.is_active(StateId::SLASH) {
                    eprintln!("ACTION_SMOKE_PASS check=attack_unsheath_right");
                    let input=input_for(19)?;send(world,input,true)?;self.held=Some(input);self.stage(Stage::Focus,world)?;
                }
            }
            Stage::Focus if focus.is_some_and(|(requested,_,full)|requested&&full) => {
                eprintln!("ACTION_SMOKE_PASS check=focused_hold_actual_full_producer_blend");
                if let Some(input)=self.held.take(){send(world,input,false)?}
                self.stage(Stage::FocusFade,world)?;
            }
            Stage::FocusFade if focus.is_some_and(|(requested,count,_)|!requested&&count==0) => {
                if !self.focus_nonzero{return Err("focus had no observed nonzero producer sample".into())}
                eprintln!("ACTION_SMOKE_PASS check=focused_release_actual_producer_retirement samples={} release_slash_state_seen={}",self.focus_samples,self.focus_release_attack);
                eprintln!("ACTION_SMOKE_NOT_COVERED check=sword_contact_damage reason=no_authoritative_near_contact_fixture; slash_state_is_not_damage_receipt");
                let p=player(world)?;
                let inventory=world.get_resource::<DiscoveredDocuments>().ok_or("actual inventory absent")?;
                self.heart_action=(0..10).find_map(|index|{
                    let (item,_)=inventory.wheel_shortcut_pair(index);let item=item?;
                    p.equipment_target_metadata(item).filter(|(equipment,_,_)|equipment.archetype==7).map(|_|25+index as i32)
                }).ok_or("actual Corvo shortcut set has no owned Heart")?;
                self.tap(world,self.heart_action)?;self.stage(Stage::Heart,world)?;
            }
            Stage::Heart => {
                let p=player(world)?;
                if p.heart_entity().is_some() && p.equipment_action_status(0).is_some_and(|(identity,equipped,transitioning)|
                    equipped&&!transitioning&&p.equipment_target_metadata(identity).is_some_and(|(equipment,_,_)|equipment.archetype==7)) {
                    self.heart_before=world.resource::<Receipts>().heart_uses;
                    self.tap(world,20)?;self.stage(Stage::HeartUse,world)?;
                }
            }
            Stage::HeartUse if world.resource::<Receipts>().heart_uses>self.heart_before => {
                eprintln!("ACTION_SMOKE_PASS check=Heart_actual_bark_request_ack entity={:?} playback_not_assumed=true",world.resource::<Receipts>().heart);
                let input=input_for(45)?;send(world,input,true)?;self.held=Some(input);self.stage(Stage::WheelOpen,world)?;
            }
            Stage::WheelOpen if world.get_resource::<super::power_wheel::State>().is_some_and(|wheel|wheel.movie_frame_ready()) => {
                eprintln!("ACTION_SMOKE_PASS check=wheel_real_open");
                if let Some(input)=self.held.take(){send(world,input,false)?}
                self.stage(Stage::WheelClose,world)?;
            }
            Stage::WheelClose if world.get_resource::<super::power_wheel::State>().is_some_and(|wheel|!wheel.blocks_input()) => {
                eprintln!("ACTION_SMOKE_PASS check=wheel_real_close");
                let focused=world.get_resource::<super::interaction_focus::FocusedInteraction>().and_then(|focus|focus.current.as_ref())
                    .filter(|prompt|!prompt.locked).and_then(|prompt|{
                        let interactions=world.get_resource::<super::interactions::Interactions>()?;
                        let index=*interactions.carrying_by_entity.get(&prompt.entity)?;
                        interactions.carrying.get(index)?.definition.hold.as_ref()?;
                        world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()?.find(&prompt.entity)
                    });
                if let Some(id)=focused {
                    eprintln!("ACTION_SMOKE_OBJECT setup=already_physically_reachable_focused_target entity={id:?} no_teleport=true");
                    self.object=Some((id,[0.;3]));self.tap(world,22)?;self.stage(Stage::PickObject,world)?;
                }else{
                    eprintln!("ACTION_SMOKE_NOT_COVERED check=bottle_pickup_throw reason=no_current_reachable_carry_focus; requires_separate_reachable_saved_position");
                    self.stage(Stage::Done,world)?;
                }
            }
            Stage::PickObject if player(world)?.carrying.has_held() => {
                let held=player(world)?.carrying.held.as_ref().unwrap().entity;
                if self.object.map(|(id,_)|id)!=Some(held){return Err("carry input picked a different entity generation".into())}
                eprintln!("ACTION_SMOKE_PASS check=actual_pickup_hold entity={held:?}");
                self.stage(Stage::ThrowObject,world)?;
            }
            Stage::ThrowObject if player(world)?.possession_state()==Some(14) => {
                let id=self.object.unwrap().0;
                let frame=world.resource::<super::physics_scene::PhysicsScene>().entity_frame(world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),id)?
                    .ok_or("held object's actual frame absent")?;
                self.object=Some((id,frame.position));self.tap(world,19)?;self.stage(Stage::ObjectMotion,world)?;
            }
            Stage::ObjectMotion if !player(world)?.carrying.has_work() => {
                let (id,before)=self.object.unwrap();
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                if live.get(id).is_none(){return Err("thrown object retired before a physical motion observation".into())}
                if let Some(frame)=world.resource::<super::physics_scene::PhysicsScene>().entity_frame(live,id)? {
                    if frame.position!=before{
                        eprintln!("ACTION_SMOKE_PASS check=actual_throw_motion entity={id:?} before={before:?} after={:?}",frame.position);
                        self.stage(Stage::Done,world)?;
                    }
                }
            }
            Stage::Done => {eprintln!("ACTION_SMOKE_PASS check=sequence_complete exit=ordinary_ExitDrain");return Ok(true)}
            _ => {}
        }
        Ok(false)
    }
    pub(super) fn completed(&self)->bool{self.stage==Stage::Done}
}
