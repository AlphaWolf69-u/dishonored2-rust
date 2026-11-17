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
enum Stage { Begin, Crossbow, Bolt, Gun, GunShot, Empty, Sheath, Draw, Focus, FocusFade, Heart, HeartUse, WheelOpen, WheelClose, CarryApproach, ContactGun, ContactSword, ContactAim, ContactFocus, ContactDamage, PickObject, ThrowObject, ObjectMotion, Done }
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
    heart_only: bool,
    carry_only: bool,
    contact_only: bool,
    melee_only: bool,
    poke_only: bool,
    contact_serial: u64,
    contact_windows: u64,
    contact_damage_seen: bool,
    contact_health: Option<f32>,
    approach_move: Option<Input>,
    fixture_gate: Option<(bool,bool,bool,bool)>,
    fixture_reports: u32,
}
impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        let heart_only=match std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(){
            Ok("heart")=>true,Ok("full")|Ok("carry")|Ok("contact")|Ok("melee")|Ok("poke")|Err(_)=>false,
            Ok(other)=>return Err(format!("unknown action smoke scenario {other}; expected full, heart, carry, contact, melee or poke")),
        };
        if super::pause_menu::debug_character() != crate::gameplay::pause_state::Character::Corvo {
            return Err("action smoke requires actual Corvo loadout (DH2_DEBUG_CHARACTER=corvo)".into());
        }
        if super::player::debug_benchmark_walk_enabled() {
            return Err("action smoke requires benchmark walking disabled; it must not steer or teleport while firing".into());
        }
        let melee_only=std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("melee");
        let poke_only=std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("poke");
        let contact_only=melee_only || poke_only || std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("contact");
        let carry_only=contact_only || std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("carry");
        if !heart_only && !carry_only {
            for (key, expected) in [(KeyCode::Digit1,25),(KeyCode::Digit3,27)] {
                if binding(Input::Key(key)) != Some(expected) { return Err(format!("installed shortcut input contract differs: {key:?} expected action{expected}")); }
            }
        }
        world.insert_resource(Receipts::default());
        eprintln!("ACTION_SMOKE_BEGIN provenance=installed_Corvo_loadout+03ab70_input+4de920_shortcuts+63a2b0_C8+802f90_Undraw setup=actual_level_spawn no_teleport=true no_fake_ACK=true");
        Ok(Self { stage:Stage::Begin,since:Instant::now(),release:None,held:None,before_ammo:0,initial_gun_ammo:None,
            seen_projectiles:HashSet::new(),shot_count:0,empty_scene:None,heart_before:0,heart_action:0,projectile_seen:false,object:None,
            focus_samples:0,focus_nonzero:false,focus_release_attack:false,expected_bolt:None,gun_trigger_pending:false,heart_only,carry_only,contact_only,melee_only,poke_only,
            contact_serial:0,contact_windows:0,contact_damage_seen:false,contact_health:None,approach_move:None,fixture_gate:None,fixture_reports:0 })
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
    fn select_heart(&mut self, world:&mut World)->Result<(),String>{
        let p=player(world)?;
        let inventory=world.get_resource::<DiscoveredDocuments>().ok_or("actual inventory absent")?;
        self.heart_action=(0..10).find_map(|index|{
            let (item,_)=inventory.wheel_shortcut_pair(index);let item=item?;
            p.equipment_target_metadata(item).filter(|(equipment,_,_)|equipment.archetype==7).map(|_|25+index as i32)
        }).ok_or("actual Corvo shortcut set has no owned Heart")?;
        self.tap(world,self.heart_action)?;self.stage(Stage::Heart,world)
    }
    fn ranged_ready(world: &World, family: RangedFamily) -> Result<bool, String> {
        let p=player(world)?;
        let Some(weapon)=&p.weapon else{return Ok(false)};
        if weapon.settings.family != family || !weapon.is_active() || !weapon.can_fire() {return Ok(false)}
        let Some((_,equipped,transitioning))=p.equipment_action_status(0)else{return Ok(false)};
        Ok(equipped && !transitioning && weapon.scene_handle()==0
            && !p.machine.is_active(StateId::FIRE_WEAPON) && !p.machine.is_active(StateId::CANCEL_FIRE_WEAPON))
    }
    fn melee_fixture_reached(world:&World,point:Vec3)->Result<bool,String>{
        let p=player(world)?;
        let (identity,_,_)=p.equipment_action_status(1).ok_or("sword fixture has no retained right-hand identity")?;
        let definition=world.resource::<super::player_actions::PlayerActions>().equipment_metadata(identity)
            .and_then(|metadata|metadata.melee.as_deref()).ok_or("sword fixture lacks actual retained Melee declaration")?;
        // Debug approach goal uses the installed blade/focus dimensions, not a
        // made-up interaction range. This does not admit a hit: the later real
        // sweep/DamageResult and Health change remain mandatory.
        // Place the debug actor well inside that authored bound; this only
        // chooses ordinary walking inputs, never the attack's admission range.
        let bound=0.5*definition.world_focus.extents.x.min(definition.dimensions.x-definition.shape_step_back);
        if bound<=0.{return Err("sword source has no positive fixture approach bound".into())}
        let (eye,axis)=p.published_camera_frame();
        Ok((point-p.physics.position()).truncate().length()<=bound
            && definition.world_focus.contains(p.physics.position(),eye,axis,definition.focus_step_back,point))
    }
    fn contact_baseline(&mut self,world:&World)->Result<(),String>{
        let id=self.object.ok_or("contact target absent")?.0;
        let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        let scene=world.resource::<super::physics_scene::PhysicsScene>();
        self.contact_health=Some(scene.entity_health(live,id)?.ok_or("contact fixture has no source Health component")?.0);
        if self.contact_health.is_some_and(|health|health<=0.){return Err("contact fixture is already dead before attack input".into())}
        if scene.breakable_component(live,id)?.is_none(){return Err("contact fixture lacks actual Started Breakable component".into())}
        self.contact_serial=super::melee::action_receipt(world,id).map_or(0,|receipt|receipt.0);
        self.contact_windows=super::melee::active_windows(world);
        eprintln!("ACTION_SMOKE_CONTACT_BEFORE entity={id:?} source_health={:?} source_expected_health=2 melee_receipt_serial={} active_windows={}",self.contact_health,self.contact_serial,self.contact_windows);
        Ok(())
    }
    fn fixture_report(&mut self,world:&mut World,reason:&str,include_owner:bool)->Result<(),String>{
        let Some((id,_))=self.object else{return Ok(())};
        let summary={
            let state=world.resource::<PlayerState>();let p=state.player.as_ref().ok_or("fixture Player absent")?;
            let (eye,axis)=p.published_camera_frame();
            let flags=[2,3,4,5,6,19,20,22,45].map(|action|(action,p.controller.down(action),p.controller.pressed(action),p.controller.released(action)));
            format!("free_fly={} physics_position={:?} physics_axis={:?} input_velocity={:?} feet={:?} published_eye={eye:?} published_axis={axis:?} body_camera_joint={} body_camera_local={:?} states={:?} possession={:?} controls_move={:?} controls_look={:?} action_down_pressed_released={flags:?} input_held={:?} paused={:?} wheel_blocks={:?}",
                state.debug_free_fly,p.physics.position(),p.physics.axis(),p.physics.input_velocity(),p.feet(),p.pose.camera,p.pose.positions.get(p.pose.camera),p.active_player_states(),p.possession_state(),p.controller.input_move,p.controller.input_look,
                world.get_resource::<super::player::PlayerInput>().map(|input|&input.held),world.get_resource::<crate::gameplay::game_clock_runtime::GamePaused>().map(|paused|paused.0),
                world.get_resource::<super::power_wheel::State>().map(|wheel|wheel.blocks_input()))
        };
        let physical={
            let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
            let scene=world.resource::<super::physics_scene::PhysicsScene>();
            format!("live={:?} stage={:?} parent={:?} physical_frame={:?} model_bounds={:?} prompt={:?}",
                live.get(id),live.stage(id),live.parent(id),scene.entity_frame(live,id),super::world_models::entity_bounds(world,live,id),
                world.get_resource::<super::interaction_focus::FocusedInteraction>().and_then(|focus|focus.current.as_ref()))
        };
        let cursors:Vec<_>=world.query::<(Entity,&bevy::window::CursorOptions)>().iter(world).map(|(entity,cursor)|(entity,cursor.visible,cursor.grab_mode)).collect();
        let transform=*world.query_filtered::<&Transform,With<Camera3d>>().single(world).map_err(|error|error.to_string())?;
        let eye=Vec3::from(super::view_to_game(transform.translation));let forward=Vec3::from(super::view_to_game(transform.forward().as_vec3()));
        eprintln!("ACTION_SMOKE_FIXTURE reason={reason} stage={:?} intended={id:?} app_frame={:?} camera_eye={eye:?} camera_forward={forward:?} camera_rotation={:?} cursor={cursors:?} queued_release={:?} held_input={:?} movement_input={:?} {summary} {physical}",
            self.stage,world.get_resource::<bevy::diagnostic::FrameCount>().map(|frame|frame.0),transform.rotation,self.release,self.held,self.approach_move);
        let scene=world.resource::<super::physics_scene::PhysicsScene>();let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        let owner=scene.point_queries()?;let queries=owner.read().map_err(|_|"fixture query owner poisoned")?;
        let mut ray=crate::gameplay::world_ray_query::WorldRayQuery::default();let mut hits=Vec::new();
        queries.ray_hits(&mut ray,&crate::gameplay::world_ray_query::WorldRayRequest{
            origin:eye.to_array(),direction:(forward*100.).to_array(),maximum_fraction:1.,
            hit:crate::gameplay::collision_hit::ShapeHitInfo{body_id:None,material_id:u16::MAX,shape_key:u32::MAX,filter_info:0,user_data:0},
            filter:Some(crate::gameplay::collision_filter::CollisionQueryFilter{layers:0x1ffff,required_flags:0x800}),
        },0,&mut hits)?;
        hits.sort_by(|a,b|a.fraction.total_cmp(&b.fraction));
        for (index,hit) in hits.iter().take(8).enumerate(){
            let target=scene.collision_entity(live,&hit.target);
            let material=queries.storage.materials.material_for_id(hit.target.material_id).map(|material|material.name.as_str());
            eprintln!("ACTION_SMOKE_FIXTURE_RAY reason={reason} index={index} closest={} intended={id:?} entity={target:?} material={material:?} contact={hit:?}",index==0);
        }
        if hits.is_empty(){eprintln!("ACTION_SMOKE_FIXTURE_RAY reason={reason} intended={id:?} no_hits=true layers=0x1ffff required_flags=0x800")}
        drop(queries);
        if include_owner{
            eprintln!("ACTION_SMOKE_FIXTURE_OWNER reason={reason} {}",super::interactions::describe_carry_fixture(world,id)?);
        }
        Ok(())
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
        if self.stage==Stage::FocusFade || self.poke_only&&self.stage==Stage::ContactDamage{self.focus_release_attack |= p.machine.is_active(StateId::SLASH);}
        Ok((fx.focus_requested(),count,count>0&&full))
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool,String> {
        // Sample every actual app frame across hold/release, including the
        // input-release frame, so endpoint-only evidence cannot hide a snap.
        let focus=if matches!(self.stage,Stage::Focus|Stage::FocusFade|Stage::ContactFocus)||self.poke_only&&self.stage==Stage::ContactDamage{Some(self.focus_curve(world)?)}else{None};
        if let Some(input)=self.release.take(){send(world,input,false)?;return Ok(false)}
        if let Some(error)=super::weapon_firing::startup_error(world){return Err(format!("actual firing owner failed during {:?}: {error}",self.stage))}
        if let Some(error)=super::heart::startup_error(world){return Err(format!("actual Heart owner failed during {:?}: {error}",self.stage))}
        if self.since.elapsed()>Duration::from_secs(90) {
            let mut snapshot=String::new();player(world)?.write_combat_diagnostics(&mut snapshot);
            super::weapon_firing::write_diagnostics(world,&mut snapshot);
            if self.melee_only||self.poke_only {
                snapshot.push_str(&super::melee::debug_startup_pending(world));
            }
            if matches!(self.stage,Stage::CarryApproach|Stage::ContactAim) {
                eprintln!("ACTION_SMOKE_FIXTURE_INCOMPLETE stage={:?} no_range_or_hit_override=true intended_actor={:?}",self.stage,self.object.map(|(id,_)|id));
                if let Err(error)=self.fixture_report(world,"timeout",true){eprintln!("ACTION_SMOKE_FIXTURE_REPORT_ERROR {error}");}
            }
            return Err(format!("action smoke timeout stage={:?} shots={} projectile_seen={} snapshot={snapshot}",self.stage,self.shot_count,self.projectile_seen));
        }
        match self.stage {
            Stage::Begin if self.carry_only => {
                // Debug fixture is the normal authored player spawn. The bottle
                // is only looked up, never moved or marked reached/admitted.
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let id=live.find("env_mulberry_mead_01_18").ok_or("carry fixture bottle is not live on this level")?;
                let frame=world.resource::<super::physics_scene::PhysicsScene>().entity_frame(live,id)?
                    .ok_or("carry fixture bottle lacks actual physical frame")?;
                self.object=Some((id,frame.position));
                eprintln!("ACTION_SMOKE_SCENARIO name=carry source=installed_env_mulberry_mead_01_18 entity={id:?} physical_position={:?} player_feet={:?} setup=normal_authored_spawn no_teleport=true",frame.position,player(world)?.feet());
                // Ordinary left click gives the virtual Window its normal mouse
                // capture through camera_controls; there is no native window.
                if binding(Input::Mouse(MouseButton::Left))!=Some(19){return Err("carry fixture setup requires normal LMB/MELEE mouse capture binding".into())}
                send(world,Input::Mouse(MouseButton::Left),true)?;self.release=Some(Input::Mouse(MouseButton::Left));
                self.stage(Stage::CarryApproach,world)?;
                self.fixture_report(world,"entry",true)?;
            }
            Stage::Begin => {
                if self.heart_only{
                    eprintln!("ACTION_SMOKE_SCENARIO name=heart coverage=Heart_real_use+wheel+ordinary_exit ranged_not_claimed=true");
                    self.select_heart(world)?;return Ok(false)
                }
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
                self.select_heart(world)?;
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
                if self.heart_only{self.stage(Stage::Done,world)?;return Ok(false)}
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
            Stage::CarryApproach => {
                let id=self.object.ok_or("carry fixture generation absent")?.0;
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let name=live.get(id).ok_or("carry fixture bottle retired during approach")?.0;
                let focused=world.get_resource::<super::interaction_focus::FocusedInteraction>()
                    .and_then(|focus|focus.current.as_ref()).is_some_and(|prompt|!prompt.locked&&prompt.entity.eq_ignore_ascii_case(name));
                let physical=world.resource::<super::physics_scene::PhysicsScene>().entity_frame(live,id)?.ok_or("carry fixture actual frame disappeared")?.position;
                let bounds=super::world_models::entity_bounds(world,live,id)?;
                let center=bounds.map_or(Vec3::from(physical),|bounds|(Vec3::from(bounds[0])+Vec3::from(bounds[1]))*0.5);
                let close=if self.melee_only||self.poke_only{Self::melee_fixture_reached(world,center)?}else{true};
                if focused&&close {
                    if let Some(input)=self.approach_move.take(){send(world,input,false)?}
                    eprintln!("ACTION_SMOKE_PASS check=fixture_real_focus entity={id:?}");
                    if self.melee_only||self.poke_only{self.stage(Stage::ContactSword,world)?;}
                    else if self.contact_only{self.select(world,KeyCode::Digit3)?;self.stage(Stage::ContactGun,world)?;}
                    else{self.tap(world,22)?;self.stage(Stage::PickObject,world)?;}
                }else{
                    let transform=*world.query_filtered::<&Transform,With<Camera3d>>().single(world).map_err(|error|error.to_string())?;
                    let eye=Vec3::from(super::view_to_game(transform.translation));
                    let forward=Vec3::from(super::view_to_game(transform.forward().as_vec3()));
                    let to=center-eye;
                    let yaw=(to.y.atan2(to.x)-forward.y.atan2(forward.x));
                    let yaw=yaw.sin().atan2(yaw.cos());
                    let pitch=-to.z.atan2(to.truncate().length())+forward.z.atan2(forward.truncate().length());
                    let gate=(focused,close,yaw.abs()<0.05,pitch.abs()<0.05);
                    if self.fixture_gate!=Some(gate){
                        self.fixture_gate=Some(gate);
                        if self.fixture_reports<32{
                            self.fixture_reports+=1;
                            eprintln!("ACTION_SMOKE_FIXTURE_GATE focused={} source_close={} yaw_error={} pitch_error={} center={center:?} eye={eye:?} source_gate={gate:?}",focused,close,yaw,pitch);
                            self.fixture_report(world,"gate-change",false)?;
                        }
                    }
                    // Host test steering produces ordinary relative mouse
                    // motion. These bounds are controller increments, never a
                    // gameplay interaction-distance/range admission override.
                    let delta=Vec2::new((-yaw*100.).clamp(-8.,8.),(pitch*100.).clamp(-8.,8.));
                    world.write_message(bevy::input::mouse::MouseMotion{delta});
                    if yaw.abs()<0.05 && pitch.abs()<0.05 {
                        if self.approach_move.is_none(){let input=input_for(2)?;send(world,input,true)?;self.approach_move=Some(input);}
                    }else if let Some(input)=self.approach_move.take(){send(world,input,false)?}
                    // Only the real focus above can admit pickup. Blocked
                    // movement/occlusion ends at the existing stage timeout.
                }
            }
            Stage::ContactGun if Self::ranged_ready(world,RangedFamily::Gun)? => {
                self.contact_baseline(world)?;
                self.stage(Stage::ContactAim,world)?;
            }
            Stage::ContactSword if player(world)?.equipment_action_status(1).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)
                && !player(world)?.machine.is_active(StateId::SLASH) => {
                self.contact_baseline(world)?;self.stage(Stage::ContactAim,world)?;
            }
            Stage::ContactAim => {
                let id=self.object.unwrap().0;
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let scene=world.resource::<super::physics_scene::PhysicsScene>();
                let bounds=super::world_models::entity_bounds(world,live,id)?.ok_or("contact target has no actual model bounds")?;
                let center=(Vec3::from(bounds[0])+Vec3::from(bounds[1]))*0.5;
                let transform=*world.query_filtered::<&Transform,With<Camera3d>>().single(world).map_err(|error|error.to_string())?;
                let eye=Vec3::from(super::view_to_game(transform.translation));
                let forward=Vec3::from(super::view_to_game(transform.forward().as_vec3()));
                // A fresh canonical physics ray along the actual camera ray is
                // mandatory. Target bounds guide only relative mouse inputs.
                let scene=world.resource::<super::physics_scene::PhysicsScene>();
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let owner=scene.point_queries()?;let queries=owner.read().map_err(|_|"contact query owner poisoned")?;
                let mut ray=crate::gameplay::world_ray_query::WorldRayQuery::default();let mut hits=Vec::new();
                queries.ray_hits(&mut ray,&crate::gameplay::world_ray_query::WorldRayRequest{
                    origin:eye.to_array(),direction:(forward*100.).to_array(),maximum_fraction:1.,
                    hit:crate::gameplay::collision_hit::ShapeHitInfo{body_id:None,material_id:u16::MAX,shape_key:u32::MAX,filter_info:0,user_data:0},
                    filter:Some(crate::gameplay::collision_filter::CollisionQueryFilter{layers:0x1ffff,required_flags:0x800}),
                },0,&mut hits)?;
                let hit=hits.iter().min_by(|a,b|a.fraction.total_cmp(&b.fraction));
                let target=hit.map(|hit|scene.collision_entity(live,&hit.target)).transpose()?.flatten();
                eprintln!("ACTION_SMOKE_CONTACT_RAY intended={id:?} actual={target:?} actual_origin={eye:?} actual_forward={forward:?} model_center={center:?}");
                drop(queries);
                if target==Some(id){
                    if self.poke_only{
                        let input=input_for(19)?;send(world,input,true)?;self.held=Some(input);self.stage(Stage::ContactFocus,world)?;
                    }else if self.melee_only{
                        self.tap(world,19)?;self.stage(Stage::ContactDamage,world)?;
                    }else{
                        self.before_ammo=player(world)?.weapon.as_ref().ok_or("actual gun absent")?.total_ammo;
                        self.tap(world,20)?;self.stage(Stage::ContactDamage,world)?;
                    }
                }else{
                    let to=center-eye;let yaw=to.y.atan2(to.x)-forward.y.atan2(forward.x);let yaw=yaw.sin().atan2(yaw.cos());
                    let pitch=-to.z.atan2(to.truncate().length())+forward.z.atan2(forward.truncate().length());
                    world.write_message(bevy::input::mouse::MouseMotion{delta:Vec2::new((-yaw*100.).clamp(-8.,8.),(pitch*100.).clamp(-8.,8.))});
                }
            }
            Stage::ContactFocus if focus.is_some_and(|(requested,_,full)|requested&&full) => {
                eprintln!("ACTION_SMOKE_PASS check=fixture_poke_actual_full_focus");
                if let Some(input)=self.held.take(){send(world,input,false)?}
                self.stage(Stage::ContactDamage,world)?;
            }
            Stage::ContactDamage => {
                let id=self.object.unwrap().0;let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let scene=world.resource::<super::physics_scene::PhysicsScene>();
                let receipt=super::melee::action_receipt(world,id);
                let melee_callback=receipt.is_some_and(|(serial,accepted,damage)|serial>self.contact_serial&&accepted&&damage>0.)
                    && super::melee::active_windows(world)>self.contact_windows;
                let consumed=if self.melee_only||self.poke_only{melee_callback}else{player(world)?.weapon.as_ref().is_some_and(|weapon|weapon.total_ammo<self.before_ammo)};
                let health=if live.get(id).is_some(){scene.entity_health(live,id)?}else{None};
                let retired=live.get(id).is_none();
                self.contact_damage_seen |= consumed && (retired || health.is_some_and(|(current,_)|current<self.contact_health.unwrap()));
                let fade_done=!self.poke_only||focus.is_some_and(|(requested,count,_)|!requested&&count==0);
                if self.contact_damage_seen&&fade_done {
                    let mut details=String::new();scene.write_breakable_diagnostics(player(world)?.feet().to_array(),&mut details);
                    let check=if self.poke_only{"actual_poke_bottle_damage"}else if self.melee_only{"actual_melee_bottle_damage"}else{"actual_gun_bottle_damage"};
                    eprintln!("ACTION_SMOKE_PASS check={check} entity={id:?} before={:?} after={health:?} retired={retired} melee_receipt={receipt:?} active_windows={} breakable_state={details}",self.contact_health,super::melee::active_windows(world));
                    eprintln!("ACTION_SMOKE_NOT_COVERED check=contact_particles_and_sound reason=health_breakable_state_is_not_output_receipt");
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
                    if frame.position!=before && live.parent(id)?.is_none(){
                        eprintln!("ACTION_SMOKE_PASS check=actual_throw_motion unparented=true entity={id:?} before={before:?} after={:?}",frame.position);
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
