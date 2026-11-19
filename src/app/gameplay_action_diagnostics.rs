//! Opt-in, windowless production-input smoke. Expectations come from installed
//! Corvo shortcut/loadout data and source C8 firing / draw / Undraw contracts.
//! No fake assets, actor births, animation keys, clock ticks or upload ACKs.
use super::{interaction_focus::DiscoveredDocuments, player::PlayerState};
use crate::{gameplay::{entity_lifetime::EntityId, player::{Player, states::StateId, weapons::RangedFamily}}, platform::{bindings, console}};
use bevy::{input::{ButtonState, keyboard::{KeyboardInput, Key, NativeKey}, mouse::MouseButtonInput}, prelude::*};
use std::{collections::HashSet, time::{Duration, Instant}};
mod blink;
mod blink_water;
mod bolt;
pub(super) mod grenade;
mod contact_fx;
mod constraints;
mod contact_fixture;
mod wall_thrust;

#[derive(Resource, Default)]
struct Receipts { heart_uses: u64, heart: Option<EntityId>, heart_selected:Option<bool>,
    heart_events:HashSet<(u32,crate::gameplay::sound::Emitter)>, heart_audio_start:usize,
    heart_source:Option<(u32,u32,crate::gameplay::sound::Emitter,u32)> }
/// Called only after the real Heart owner accepts its pending use request.
pub(super) fn heart_used(world: &mut World, entity: EntityId, selected:bool) {
    if let Some(mut receipts) = world.get_resource_mut::<Receipts>() {
        receipts.heart_uses += 1;
        receipts.heart = Some(entity);
        receipts.heart_selected=Some(selected);
        eprintln!("ACTION_SMOKE_HEART_USE entity={entity:?} processed=true bark_selection_accepted={selected}");
    }
}

///Opt-in exact contact witness, no second physics query or callback.
pub(super) fn projectile_birth(world:&mut World,id:EntityId,attacker:Option<EntityId>,equipment:Option<EntityId>,definition:usize){bolt::record_birth(world,id,attacker,equipment,definition);}
pub(super) fn projectile_contact(world:&mut World,step:&super::physics_scene::projectiles::ProjectileStep){bolt::record_contact(world,step);}
pub(super) fn projectile_attachment(world:&mut World,id:EntityId){bolt::record_attachment(world,id);}

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
enum Stage { Begin, Crossbow, Bolt, Gun, GunShot, Empty, Sheath, Draw, Focus, FocusFade, Heart, HeartUse, WheelOpen, WheelMove, WheelClose, CarryApproach, ContactGun, ContactSword, ContactAim, ContactLookAway, ContactFocus, ContactPokeAim, ContactDamage, WorldPower, WorldSword, WorldCharge, WorldAim, WorldContact, WorldTail, PickObject, DropObject, DroppedObject, ThrowObject, ObjectMotion, Done }
struct WorldEffectObservation {
    ticket:super::usable_effects::EffectTicket,
    identity:Option<(std::sync::Arc<str>,std::sync::Arc<str>)>,
    publication:Option<(u32,u64)>,
    started:Option<u32>,
    started_app_frame:Option<u32>,
    particles:bool,
    retired:bool,
    emitters:HashSet<String>,
    expected_events:HashSet<u32>,
    admissions:HashSet<u32>,
    processed:HashSet<u32>,
    last:Option<crate::rendering::bevy_backend::world_particle_state::OwnerSnapshot>,
}
pub(super) struct Observation {
    stage: Stage,
    since: Instant,
    release: Option<Input>,
    held: Option<Input>,
    before_ammo: u32,
    initial_gun_ammo: Option<u32>,
    seen_projectiles: HashSet<EntityId>,
    shot_count: u32,
    shot_frame_time: Option<Instant>,
    longest_shot_frame: Duration,
    shot_frames: u32,
    empty_scene: Option<u32>,
    heart_before: u64,
    heart_action: i32,
    projectile_seen: bool,
    object: Option<(EntityId,[f32;3])>,
    object_motion_seen: bool,
    object_drop_seen: bool,
    object_throw_frame: Option<i32>,
    object_motion_frame: Option<i32>,
    object_contact: Option<crate::gameplay::physics_world::contacts::BodyContactObservation>,
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
    contact_health_reported: bool,
    contact_effects: Option<contact_fx::ContactEffects>,
    named_contact:Option<contact_fixture::Named>,
    approach_move: Option<Input>,
    fixture_gate: Option<(bool,bool,bool,bool)>,
    fixture_reports: u32,
    projectile_draw_reports: HashSet<(Entity,u8)>,
    stuck_projectile_reports: HashSet<EntityId>,
    audio_admissions: usize,
    expected_audio: HashSet<u32>,
    expected_fx: HashSet<String>,
    observed_fx: HashSet<String>,
    bolt_c8_observed: bool,
    wheel_cursor: Option<(f64, f64)>,
    world_only: bool,
    world_poke: bool,
    world_power: bool,
    world_recovery_baseline:u64,
    world_chain_inputs:u8,
    world_chain_activations:u8,
    world_replacement_pending:Option<(u64,u32)>,
    world_recovery_pending:Option<(u64,u32)>,
    world_left_response_seen:bool,
    world_prior_effects:Vec<(u8,WorldEffectObservation)>,
    world_crouch: Option<Input>,
    world_target: Option<crate::gameplay::collision_hit::ShapeHitInfo>,
    world_contacts:u8,
    world_effect:Option<WorldEffectObservation>,
    blink: Option<blink::Observation>,
    blink_water: Option<blink_water::Observation>,
    bolt:Option<bolt::Observation>,
    grenade: Option<grenade::Observation>,
    wall: Option<wall_thrust::Observation>,
    constraints:Option<constraints::Observation>,
}
impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        let heart_only=match std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(){
            Ok("heart")=>true,Ok("full")|Ok("carry")|Ok("contact")|Ok("melee")|Ok("poke")|Ok("world")|Ok("world-poke")|Ok("world-power")|Ok("world-power-poke")|Ok("world-wall-poke")|Ok("blink")|Ok("blink-water")|Ok("bolt")|Ok("bolt-bottle")|Ok("grenade")|Ok("constraints")|Err(_)=>false,
            Ok(other)=>return Err(format!("unknown action smoke scenario {other}; expected full, heart, carry, contact, melee, poke, world, world-poke, world-power, world-power-poke, blink, blink-water, bolt, bolt-bottle, grenade or constraints")),
        };
        if super::pause_menu::debug_character() != crate::gameplay::pause_state::Character::Corvo {
            return Err("action smoke requires actual Corvo loadout (DH2_DEBUG_CHARACTER=corvo)".into());
        }
        if super::player::debug_benchmark_walk_enabled() {
            return Err("action smoke requires benchmark walking disabled; it must not steer or teleport while firing".into());
        }
        let melee_only=std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("melee");
        let poke_only=std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("poke");
        let world_poke=matches!(std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(),Ok("world-poke"|"world-power-poke"));
        let world_power=matches!(std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(),Ok("world-power"|"world-power-poke"));
        let world_only=world_power||world_poke||std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("world");
        let contact_only=melee_only || poke_only || std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("contact");
        let carry_only=contact_only || std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("carry");
        let grenade_only=std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("grenade");
        let constraint_only=std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("constraints");
        if !heart_only && !carry_only && !world_only && !grenade_only && !constraint_only {
            for (key, expected) in [(KeyCode::Digit1,25),(KeyCode::Digit3,27)] {
                if binding(Input::Key(key)) != Some(expected) { return Err(format!("installed shortcut input contract differs: {key:?} expected action{expected}")); }
            }
        }
        let named_contact=contact_fixture::Named::begin(world,melee_only||poke_only)?;
        world.insert_resource(Receipts::default());
        let blink=if std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("blink"){Some(blink::Observation::begin(world)?)}else{None};
        let blink_water=if std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("blink-water"){Some(blink_water::Observation::begin(world)?)}else{None};
        let bolt=match std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(){Ok("bolt")=>Some(bolt::Observation::begin(world)?),Ok("bolt-bottle")=>Some(bolt::Observation::begin_bottle(world)?),_=>None};
        let constraints=if constraint_only{Some(constraints::Observation::begin(world)?)}else{None};
        let grenade=if grenade_only{Some(grenade::Observation::begin(world)?)}else{None};
        let wall=if std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref()==Ok("world-wall-poke"){Some(wall_thrust::Observation::begin(world)?)}else{None};
        let audio_admissions=super::audio::observation::snapshot(world).admissions.len();
        eprintln!("ACTION_SMOKE_BEGIN provenance=installed_Corvo_loadout+03ab70_input+4de920_shortcuts+63a2b0_C8+802f90_Undraw setup={} no_fake_ACK=true",if wall.is_some()||blink_water.is_some(){"captured_F8_feet_restore_once"}else{"actual_level_spawn no_teleport=true"});
        Ok(Self { bolt,stage:Stage::Begin,since:Instant::now(),release:None,held:None,before_ammo:0,initial_gun_ammo:None,
            seen_projectiles:HashSet::new(),shot_count:0,empty_scene:None,heart_before:0,heart_action:0,projectile_seen:false,object:None,object_motion_seen:false,object_drop_seen:false,object_throw_frame:None,object_motion_frame:None,object_contact:None,
            shot_frame_time:None,longest_shot_frame:Duration::ZERO,shot_frames:0,
            focus_samples:0,focus_nonzero:false,focus_release_attack:false,expected_bolt:None,gun_trigger_pending:false,heart_only,carry_only,contact_only,melee_only,poke_only,
            contact_serial:0,contact_windows:0,contact_damage_seen:false,contact_health:None,contact_health_reported:false,contact_effects:None,named_contact,approach_move:None,fixture_gate:None,fixture_reports:0,projectile_draw_reports:HashSet::new(),stuck_projectile_reports:HashSet::new(),
            audio_admissions,expected_audio:HashSet::new(),expected_fx:HashSet::new(),observed_fx:HashSet::new(),bolt_c8_observed:false,wheel_cursor:None,
            world_only,world_poke,world_power,world_recovery_baseline:0,world_chain_inputs:0,world_chain_activations:0,world_replacement_pending:None,world_recovery_pending:None,world_left_response_seen:false,world_prior_effects:Vec::new(),world_crouch:None,world_target:None,world_contacts:0,world_effect:None,blink,blink_water,grenade,wall,constraints })
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
    fn weapon_contract(&mut self,world:&World)->Result<(),String>{
        let p=player(world)?;let Some(weapon)=p.weapon.as_ref()else{return Ok(())};
        let identity=p.ranged_identity().ok_or("selected ranged identity absent")?;
        let source=weapon.fire_source.as_ref().ok_or("selected ranged source absent")?;
        let fired=super::runtime_entities::fired_sound(world,&source.prepared.entity);
        if let Some(event)=&fired{self.expected_audio.insert(event.id);}
        self.expected_fx.extend(source.muzzle_world_fx.iter().map(|path|path.to_string()));
        if weapon.settings.family==RangedFamily::Gun{self.expected_fx.extend(weapon.settings.muzzle_attached_fx.iter().cloned());}
        eprintln!("ACTION_SMOKE_WEAPON_CONTRACT identity={identity} family={:?} equipment={:?} attacker={:?} ammunition={} projectile={} actual_projectile_fired_sound_path={:?} actual_prepared_fired_event={:?} muzzle_world_fx={:?} muzzle_attached_fx={:?} muzzle_tag={:?}",
            weapon.settings.family,source.equipment,source.attacker,weapon.ammunition().item,source.prepared.declaration.entity,source.prepared.declaration.effects.fired_sound,fired.as_ref().map(|event|event.id),source.muzzle_world_fx,weapon.settings.muzzle_attached_fx,weapon.settings.muzzle_tag);
        Ok(())
    }
    fn weapon_evidence(&mut self,world:&mut World,reason:&str)->Result<(),String>{
        // On-demand stage/actual-shot observations only. Existing owners keep
        // the definitions, instances, voice receipts and output clock.
        if let Some(p)=world.get_resource::<PlayerState>().and_then(|state|state.player.as_ref()){
            if let Some(weapon)=p.weapon.as_ref().filter(|weapon|weapon.settings.family==RangedFamily::Crossbow){
                if let Some(source)=&weapon.fire_source{
                    let child=super::equipment_attachment_admission::ammo_child(world,source.equipment);
                    eprintln!("ACTION_SMOKE_LOADED_BOLT reason={reason} weapon={:?} magazine={} total={} actual_child={child:?} reload_requested={} scene={}",source.equipment,weapon.magazine,weapon.total_ammo,weapon.reload_requested,weapon.scene_handle());
                    if let (Some((child,_)),Some(arms))=(child,world.get_resource::<super::player_arms::PlayerArms>()){
                        eprintln!("ACTION_SMOKE_LOADED_BOLT_MODEL reason={reason} child={child:?} actual_model_frame={:?} model_visible={}",arms.attachments.frame(child),arms.attachments.visible(child));
                        if let Some(draws)=arms.attachments.projection_draws(child){for (entity,_,_,_,_) in draws.take(16){
                            eprintln!("ACTION_SMOKE_LOADED_BOLT_DRAW reason={reason} child={child:?} drawable={entity:?} visibility={:?} actual_global={:?}",world.get::<Visibility>(entity),world.get::<GlobalTransform>(entity));
                        }}
                    }
                }
            }
        }
        if let Some(sources)=world.get_resource::<super::usable_effects::UsableEffects>(){
            for (key,source) in &sources.ready{
                for path in &self.expected_fx{
                    if let Some(Ok(definition))=source.effects.get(path){
                        for event in definition.groups.iter().flat_map(|group|&group.sounds).flat_map(|sound|[&sound.start,&sound.stop]).flatten(){
                            if self.expected_audio.insert(event.declaration.id){
                                eprintln!("ACTION_SMOKE_WEAPON_SOUND_SOURCE fx={path} sound={} event={} source=actual_prepared_FX_definition",event.name,event.declaration.id);
                            }
                        }
                    }
                }
                let super::usable_effects::EffectOwner::Entity(name)=key else{continue};
                // These are the canonical firing owner's generated keys,
                // not guessed installed effect names or a second FX registry.
                if !(name.starts_with("weapon_muzzle:")||name.starts_with("weapon_barrel:")){continue}
                if self.observed_fx.len()>=128||!self.observed_fx.insert(name.to_string()){continue}
                let actual=world.get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
                    .map(|simulation|simulation.observe_fx_owner(&format!("entity:{name}")));
                let definitions=source.effects.iter().map(|(path,definition)|(path,definition.as_ref().map(|definition|definition.groups.len()).map_err(String::as_str))).collect::<Vec<_>>();
                eprintln!("ACTION_SMOKE_WEAPON_FX reason={reason} owner={name} definitions={definitions:?} actual_simulation={actual:?} GPU_or_visible_output_not_claimed=true");
            }
        }
        let audio=super::audio::observation::snapshot(world);
        for admission in audio.admissions.iter().skip(self.audio_admissions).take(256){
            eprintln!("ACTION_SMOKE_AUDIO_ADMISSION reason={reason} expected_weapon_declaration={} event={} playing_id={} object={} message_id={:?} cookies={} error={:?} attribution=event_and_emitter_and_playing_id_required not_audibility_proof=true",
                self.expected_audio.contains(&admission.event),admission.event,admission.playing_id,admission.object,admission.message_id,admission.cookies_after,admission.error);
        }
        if audio.admissions.len().saturating_sub(self.audio_admissions)>256{eprintln!("ACTION_SMOKE_AUDIO_TRUNCATED reason={reason} additional={}",audio.admissions.len()-self.audio_admissions-256);}
        self.audio_admissions=audio.admissions.len();
        for callback in audio.callbacks.iter().take(256){eprintln!("ACTION_SMOKE_AUDIO_CALLBACK reason={reason} receipt={callback:?} not_audibility_proof=true");}
        eprintln!("ACTION_SMOKE_AUDIO_OUTPUT reason={reason} actual_output={:?} consumed_samples={} aggregate_nonzero={} pending={:?} source_jobs={:?} expected_weapon_events={:?} individual_event_contribution_not_inferred=true",audio.output,audio.samples,audio.nonzero_samples,audio.pending_ids,audio.source_jobs,self.expected_audio);
        Ok(())
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
    fn fixture_outside_focus(world:&World,id:EntityId)->Result<bool,String>{
        let p=player(world)?;let (identity,_,_)=p.equipment_action_status(1).ok_or("charge setup sword identity absent")?;
        let definition=world.resource::<super::player_actions::PlayerActions>().equipment_metadata(identity)
            .and_then(|metadata|metadata.melee.as_deref()).ok_or("charge setup source Melee declaration absent")?;
        let bounds=super::world_models::entity_bounds(world,world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),id)?
            .ok_or("charge setup actual bottle bounds absent")?;
        let (eye,axis)=p.published_camera_frame();
        let origin=p.physics.position();
        let world_base=origin+(eye-origin)*definition.world_focus.height_scale-axis.x_axis*definition.focus_step_back;
        let npc_base=origin+(eye-origin)*definition.npc_focus.height_scale-axis.x_axis*definition.focus_step_back;
        Ok(axis.x_axis.z>=0.&&(0..8).all(|corner|{
            let point=Vec3::new(bounds[corner&1][0],bounds[(corner>>1)&1][1],bounds[(corner>>2)&1][2]);
            //635a00's box starts at its actual base. A common separating
            //front plane for all eight corners excludes the whole AABB;
            //merely testing eight unrelated outside points would not.
            axis.x_axis.dot(point-world_base)<0.&&axis.x_axis.dot(point-npc_base)<0.
                && !definition.world_focus.contains(origin,eye,axis,definition.focus_step_back,point)
                && !definition.npc_focus.contains(origin,eye,axis,definition.focus_step_back,point)
        }))
    }
    fn contact_baseline(&mut self,world:&World)->Result<(),String>{
        let id=self.object.ok_or("contact target absent")?.0;
        let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        let scene=world.resource::<super::physics_scene::PhysicsScene>();
        self.contact_health=Some(scene.entity_health(live,id)?.ok_or("contact fixture has no source Health component")?.0);
        if self.contact_health.is_some_and(|health|health<=0.){return Err("contact fixture is already dead before attack input".into())}
        if scene.breakable_component(live,id)?.is_none(){return Err("contact fixture lacks actual Started Breakable component".into())}
        self.contact_effects=Some(contact_fx::ContactEffects::baseline(world,id)?);
        self.contact_serial=super::melee::action_receipt(world,id).map_or(0,|receipt|receipt.0);
        self.contact_windows=super::melee::active_windows(world);
        eprintln!("ACTION_SMOKE_CONTACT_BEFORE entity={id:?} source_health={:?} source_expected_health=actual_authored_value melee_receipt_serial={} active_windows={}",self.contact_health,self.contact_serial,self.contact_windows);
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
                //627110 leaves a flying bolt unbound. Owner/attacker identity
                //is not an attachment parent. Observe the actual renderer-side
                //publication after the normal end-of-frame model update.
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let parent=live.parent(id)?;
                let owner=world.get_resource::<super::world_models::WorldModels>().ok_or("projectile world-model owner absent")?;
                let Some((pose,scale,rigid))=owner.publication_pose(id)else{continue};
                if !rigid{return Err("installed regular bolt fixture did not publish its rigid LWO geometry".into())}
                let affine=crate::render_model::affine_rows(pose.position,pose.basis,scale);
                let expected=super::view_geometry::game_affine_to_view(&std::array::from_fn(|index|affine[index/4][index%4]));
                let expected=GlobalTransform::from(expected).to_matrix().to_cols_array();
                let mut count=0;let mut matching=true;let mut draws_visible=true;let mut max_error=0.0f32;
                if let Some(draws)=owner.publication_draws(id){for entity in draws{
                    count+=1;
                    let global=world.get::<GlobalTransform>(entity);
                    let local=world.get::<Transform>(entity);
                    let visibility=world.get::<Visibility>(entity);
                    let flags=u8::from(global.is_some())|u8::from(local.is_some())<<1|u8::from(visibility.is_some())<<2
                        |u8::from(visibility.is_some_and(|value|*value==Visibility::Hidden))<<3|u8::from(world.get_entity(entity).is_ok())<<4;
                    if self.projectile_draw_reports.len()<32&&self.projectile_draw_reports.insert((entity,flags)){
                        let components=world.get_entity(entity).ok().map(|entry|entry.archetype().components().iter()
                            .filter_map(|id|world.components().get_info(*id)).map(|info|info.name().to_string()).collect::<Vec<_>>());
                        eprintln!("ACTION_SMOKE_PROJECTILE_DRAW id={id:?} drawable={entity:?} exists={} component_flags={flags:#x} mesh={:?} local={local:?} global={global:?} expected_matrix={expected:?} visibility={visibility:?} inherited_visibility={:?} view_visibility={:?} bevy_parent={:?} components={components:?}",
                            world.get_entity(entity).is_ok(),world.get::<Mesh3d>(entity),world.get::<InheritedVisibility>(entity),world.get::<ViewVisibility>(entity),world.get::<ChildOf>(entity));
                    }
                    let Some(actual)=world.get::<GlobalTransform>(entity)else{matching=false;continue};
                    let actual=actual.to_matrix().to_cols_array();
                    // The reported failure is a drawable stuck at the muzzle.
                    // Require its exact published translation; rotation error
                    // is diagnostic data, without an invented pass threshold.
                    matching &= actual[12..15]==expected[12..15];
                    for (actual,expected) in actual.into_iter().zip(expected){
                        let error=(actual-expected).abs();max_error=max_error.max(error);
                        matching &= actual.is_finite();
                    }
                    draws_visible &= world.get::<Visibility>(entity).is_some_and(|visibility|*visibility!=Visibility::Hidden);
                }}
                let in_flight=projectile.movement_enabled&&!projectile.impact_stopped&&!projectile.has_hit_and_stopped&&!projectile.picked_up;
                let published=in_flight&&parent.is_none()&&pose.position==projectile.position.to_array()
                    &&count>0&&matching&&owner.interaction_visible(id)&&owner.interaction_bounds(id).is_some();
                eprintln!("ACTION_SMOKE_PROJECTILE_PUBLICATION id={id:?} in_flight={in_flight} parent={parent:?} runtime_position={:?} published_position={:?} actual_draws={count} translations_match={matching} max_matrix_error={max_error} draws_visible={draws_visible} source_visibility={:?} visible={} bounds={:?} complete={published} witness=actual_CPU_draw_GlobalTransforms_not_GPU_draw",projectile.position,pose.position,owner.publication_visibility(id),owner.interaction_visible(id),owner.interaction_bounds(id));
                let query_state=world.get_resource::<super::SceneRenderQueue>()
                    .and_then(|queue|queue.shadow_ordinary_query.as_ref()).and_then(|query|query.state.as_ref());
                let pixels=world.get_resource::<super::SceneRenderQueue>().map_or(0,|queue|queue.texture_viewport_pixels);
                if let Some(explanation)=owner.query_diagnostic(id,query_state,pixels)? {
                    eprintln!("ACTION_SMOKE_PROJECTILE_QUERY id={id:?} {explanation}");
                }
                if (projectile.impact_stopped||projectile.has_hit_and_stopped)
                    &&scene.projectile_throwable_started(handle)?
                    &&self.stuck_projectile_reports.len()<32&&self.stuck_projectile_reports.insert(id){
                    let (eye,axis)=player(world)?.published_camera_frame();
                    let sensor=world.get_resource::<super::interactions::Interactions>()
                        .ok_or("actual bolt interaction sensor owner absent")?.sensor;
                    eprintln!("ACTION_SMOKE_STUCK_PROJECTILE id={id:?} source=actual_fired_bolt_after_Throwable_Start current_eye={eye:?} current_forward={:?} recovered_or_forced=false\n{}",axis.x_axis,
                        scene.describe_interaction_fixture(live,id,eye,axis.x_axis,sensor)?);
                    if let Some(state)=query_state{
                        //On-demand area normalization comparison only. Reuse
                        //the same actual rows, grid, bounds and flags; this
                        //does not claim a new camera or visible GPU output.
                        if let Some(explanation)=owner.query_diagnostic(id,Some(state),1366*768)?{
                            eprintln!("ACTION_SMOKE_STUCK_PROJECTILE_QUERY_1366X768 id={id:?} copied_descriptor=true rows_and_depth_grid_unchanged=true actual_tome_tile={:?} diagnostic_only_not_actual_user_output=true {explanation}",state.viewport);
                        }
                    }
                }
                if !published{continue}
                if !draws_visible{eprintln!("ACTION_SMOKE_NOT_COVERED check=bolt_visible_render reason=actual_camera_culled_draw source_visibility={:?} CPU_pose_publication_verified=true GPU_or_visible_output_not_claimed=true",owner.publication_visibility(id));}
                eprintln!("ACTION_SMOKE_PROJECTILE id={id:?} source={} origin={:?} position={:?} velocity={:?} moving={}",
                    projectile.declaration.entity,projectile.origin,projectile.position,projectile.velocity,projectile.movement_enabled);
                moved=true;
            }
        }
        Ok(moved)
    }
    /// Fixture selection uses the SAME 630890 camera ray, actual sword length
    /// and 62f900 WORLD focus bounds. It does not manufacture a melee hit.
    fn world_fixture(world:&World)->Result<Option<crate::gameplay::collision_hit::ShapeHitInfo>,String>{
        let p=player(world)?;
        let (identity,_,_)=p.equipment_action_status(1).ok_or("WORLD fixture sword absent")?;
        let definition=world.resource::<super::player_actions::PlayerActions>().equipment_metadata(identity)
            .and_then(|metadata|metadata.melee.as_ref()).ok_or("WORLD fixture actual sword declaration absent")?;
        let (eye,axis)=p.published_camera_frame();
        let equipment=super::equipment_identity::id(world,identity);
        let scene=world.resource::<super::physics_scene::PhysicsScene>();
        let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        let mut queries=super::physics_scene::melee::MeleeQueries::default();
        let contacts=scene.melee_ray_contacts(live,&mut queries,
            &super::physics_scene::melee::Ray{origin:eye.to_array(),direction:(axis.x_axis*definition.dimensions.x).to_array(),maximum_fraction:1.,layers:0x443e})?;
        for contact in contacts {
            // A real static WORLD witness is required. Props, even props that
            // additionally use WORLD focus, cannot pass this separate case.
            if contact.entity.is_some()&&(contact.entity==p.managed_entity()||contact.entity==equipment){continue}
            //636b40's closest-ray WORLD bank admits null/idWorldspawn, not
            //arbitrary non-damageable props. Do not look through occluders.
            if !scene.melee_contact_world(live,contact,3)?{return Ok(None)}
            let hit=match contact.hit{
                super::physics_scene::melee::Hit::Ray(hit)=>hit,
                super::physics_scene::melee::Hit::Shape(_)=>return Err("WORLD fixture ray produced a non-ray witness".into()),
            };
            //The first floor ray barely entering WORLD focus can be at the
            //far box boundary while the real sweep selects a farther packet.
            //Reuse the existing authored-dimension debug approach predicate;
            //this only continues physical mouse aiming, never admits a hit.
            let point=Vec3::from_slice(&hit.position);
            let inside=Self::melee_fixture_reached(world,point)?;
            if inside{
                let approach_bound=0.5*definition.world_focus.extents.x.min(definition.dimensions.x-definition.shape_step_back);
                eprintln!("ACTION_SMOKE_WORLD_FIXTURE_RAY app_frame={:?} actual_hit={hit:?} actual_entity={:?} player_origin={:?} camera_eye={eye:?} camera_axis={axis:?} authored_blade_dimensions={:?} authored_shape_step_back={} authored_focus_step_back={} authored_WORLD_focus={:?} authored_NPC_focus={:?} actual_horizontal_distance={} authored_approach_bound={approach_bound} ray_in_focus=true swept_packet_in_focus_not_inferred=true",
                    world.get_resource::<bevy::diagnostic::FrameCount>().map(|frame|frame.0),contact.entity,p.physics.position(),definition.dimensions,definition.shape_step_back,definition.focus_step_back,definition.world_focus,definition.npc_focus,(point-p.physics.position()).truncate().length());
            }
            return Ok(inside.then_some(hit.target));
        }
        Ok(None)
    }
    fn world_evidence(&self,world:&World,reason:&str)->Result<(),String>{
        let p=player(world)?;
        let (identity,_,_)=p.equipment_action_status(1).ok_or("WORLD evidence sword absent")?;
        let definition=world.resource::<super::player_actions::PlayerActions>().equipment_metadata(identity)
            .and_then(|metadata|metadata.melee.as_ref()).ok_or("WORLD evidence declaration absent")?;
        let scene=world.resource::<super::physics_scene::PhysicsScene>();
        let owner=scene.point_queries()?;let queries=owner.read().map_err(|_|"WORLD evidence query owner poisoned")?;
        let receipt=super::melee::world_action_receipt(world);
        let material=match receipt {
            Some((_,hit,_,_,_,_))=>Some(queries.storage.materials.material_for_id(hit.material_id)
                .map_err(|error|format!("WORLD evidence reason={reason} material={} hit={hit:?}: {error}",hit.material_id))?.name.as_str()),
            None=>None,
        };
        eprintln!("ACTION_SMOKE_WORLD reason={reason} charged={} fixture={:?} receipt={receipt:?} actual_target_material={material:?} authored_source_material={:?} authored_focused_material={:?} native_charge={:?} {}",
            self.world_poke,self.world_target,definition.material,definition.focused_material,p.machine.melee_focus_charge(),super::melee::debug_startup_pending(world));
        Ok(())
    }
    fn world_followup_input(&mut self,world:&mut World)->Result<(),String>{
        if let Some((serial,handle))=self.world_recovery_pending {
            let p=player(world)?;
            if p.body.midnight.finished(handle){return Err("WORLD diagnostic steering missed the source recovery animation; early-input proof not achieved".into());}
            if p.machine.is_active(StateId::RECOIL){return Err("WORLD followup attempted before actual Recoil quit".into());}
            eprintln!("ACTION_SMOKE_WORLD_RECOIL_INPUT actual_marker_serial={serial} source_recovery_handle={handle} source_recovery_scene_still_playing=true actual_physical_attack_input=true");
            if self.world_replacement_pending.is_some(){return Err("previous WORLD followup has no actual Slash activation witness".into());}
            world.resource_mut::<PlayerState>().player.as_mut().ok_or("actual Player absent")?
                .body.midnight.debug_observe_replacement(handle)?;
            self.world_chain_inputs+=1;self.world_recovery_pending=None;
            self.world_replacement_pending=Some((serial,handle));
        }
        Ok(())
    }
    fn world_followup_activation(&mut self,world:&mut World)->Result<(),String>{
        let Some((serial,previous))=self.world_replacement_pending else{return Ok(())};
        let p=player(world)?;
        if let Some(replacement)=p.body.midnight.debug_scene_replacement(){
            let slash=p.machine.state(StateId::SLASH);
            let actual_slash=slash.is_some_and(|state|state.active&&state.midnight==replacement.next_handle)
                &&!p.body.midnight.finished(replacement.next_handle);
            if replacement.previous_handle!=previous || replacement.previous_time_ms>=replacement.previous_length_ms
                || replacement.previous_parts&replacement.next_parts&4==0 || !actual_slash {
                return Err(format!("WORLD physical followup did not prove actual early Slash activation: marker={serial} replacement={replacement:?} active_Slash_handle={:?} active_Slash={}",slash.map(|state|state.midnight),p.machine.is_active(StateId::SLASH)));
            }
            eprintln!("ACTION_SMOKE_WORLD_RECOIL_ACTIVATION actual_marker_serial={serial} replacement={replacement:?} actual_new_Slash_playing=true previous_recovery_unfinished_at_Start=true no_time_or_state_override=true");
            self.world_chain_activations+=1;self.world_replacement_pending=None;
            world.resource_mut::<PlayerState>().player.as_mut().ok_or("actual Player absent")?
                .body.midnight.debug_clear_replacement();
        }else if p.body.midnight.finished(previous){
            return Err(format!("WORLD physical followup waited past actual recovery completion without a new Slash replacement: marker={serial} previous={previous}"));
        }
        Ok(())
    }
    fn world_effect_progress(world:&mut World,observation:&mut WorldEffectObservation,contact:u8)->Result<bool,String>{
        if observation.retired{return Ok(true)}
        if observation.identity.is_none(){observation.identity=super::usable_effects::ticket_request_identity(world,&observation.ticket);}
        if let Some((raw,path))=&observation.identity{
            let owner=format!("entity:{raw}");
            if let Some(dynamic)=world.get_resource::<crate::rendering::bevy_backend::world_effects::DynamicEffects>(){
                if let Some(entry)=dynamic.0.iter().find(|entry|entry.owner==owner){
                    if observation.publication.is_none(){
                        let frame=world.resource::<bevy::diagnostic::FrameCount>().0;
                        observation.publication=Some((frame,entry.revision));
                        eprintln!("ACTION_SMOKE_WORLD_FX_PUBLICATION contact={} owner={owner} actual_path={} publication_app_frame={frame} revision={} ticket={}",contact,entry.effect,entry.revision,super::usable_effects::debug_ticket(world,&observation.ticket));
                    }
                    for (source,definition) in &entry.sources.effects{
                        let definition=definition.as_ref().map_err(|error|format!("WORLD FX source {source}: {error}"))?;
                        for sound in definition.groups.iter().flat_map(|group|&group.sounds){
                            for event in [&sound.start,&sound.stop].into_iter().flatten(){observation.expected_events.insert(event.declaration.id);}
                        }
                    }
                }
            }
            if let Some(simulation)=world.get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>(){
                let snapshot=simulation.observe_fx_owner(&owner);
                if !snapshot.handles.is_empty()&&snapshot.pending_starts==0{
                    if observation.started.is_none(){
                        observation.started=snapshot.frame;
                        observation.started_app_frame=world.get_resource::<bevy::diagnostic::FrameCount>().map(|frame|frame.0);
                        eprintln!("ACTION_SMOKE_WORLD_FX_CPU_START contact={} owner={owner} path={path} FX_game_clock_frame={:?} observed_CPU_start_app_frame={:?} publication_app_frame_and_revision={:?}",contact,snapshot.frame,observation.started_app_frame,observation.publication);
                    }
                    observation.particles|=snapshot.particles>0;
                }
                for object in simulation.observe_fx_owner_audio(&owner)?{observation.emitters.insert(format!("{object:?}"));}
                if observation.last.as_ref()!=Some(&snapshot){
                    eprintln!("ACTION_SMOKE_WORLD_FX_CHAINS contact={} owner={owner} path={path} actual={snapshot:?} actual_sound_objects={:?}",contact,observation.emitters);
                    observation.last=Some(snapshot);
                }
            }
        }
        // The observer receives copies of actual callbacks. It never changes
        // the native mask or routes additional messages into mutable owners.
        let audio=super::audio::observation::snapshot(world);
        for admission in &audio.admissions{
            if observation.emitters.contains(&admission.object)&&observation.expected_events.contains(&admission.event)
                &&observation.admissions.insert(admission.playing_id){
                eprintln!("ACTION_SMOKE_WORLD_SOUND_ADMITTED contact={} actual_event={} playing_id={} actual_object={} error={:?} publication={:?} CPU_start={:?}",contact,admission.event,admission.playing_id,admission.object,admission.error,observation.publication,observation.started);
                if let Some(error)=&admission.error{return Err(format!("WORLD sound admission {} on {}: {error}",admission.event,admission.object))}
            }
        }
        for callback in &audio.callbacks{
            if observation.emitters.contains(&callback.object){
                eprintln!("ACTION_SMOKE_WORLD_SOUND_CALLBACK contact={} playing_id={} actual_declaration_reference={} object={} kind={} output_blocks={} not_audibility_proof=true",contact,callback.playing_id,callback.declaration,callback.object,callback.kind,audio.output.blocks);
                if callback.kind=="SourceReady"{observation.processed.insert(callback.playing_id);}
            }
        }
        let complete=super::usable_effects::effect_completed(&observation.ticket);
        let retired=super::usable_effects::entity_retired(world,&observation.ticket);
        if !complete||!retired{return Ok(false)}
        let started=observation.started.is_some();
        if !started{
            return Err(format!("WORLD FX retired without actual start: contact={} publication={:?} CPU_start={:?} particles={} authored_sound_events={:?} objects={:?} admissions={:?} processed={:?} ticket={}",contact,observation.publication,observation.started,observation.particles,observation.expected_events,observation.emitters,observation.admissions,observation.processed,super::usable_effects::debug_ticket(world,&observation.ticket)))
        }
        eprintln!("ACTION_SMOKE_WORLD_FX_RETIRED contact={} publication_app_frame_and_revision={:?} CPU_start_FX_game_clock_frame={:?} observed_CPU_start_app_frame={:?} real_particle_chains_seen={} source_processed_playing_ids={:?} actual_sound_admissions={:?} visible_or_audible_output_not_claimed=true",contact,observation.publication,observation.started,observation.started_app_frame,observation.particles,observation.processed,observation.admissions);
        observation.retired=true;
        Ok(true)
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
    fn observe_object_motion(&mut self,world:&World)->Result<(),String>{
        let Some((id,before))=self.object else{return Ok(())};
        if player(world)?.carrying.has_work()||!player(world)?.carrying.observed_throw_ack(id){return Ok(())}
        let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        if live.get(id).is_none(){return Ok(())}
        let scene=world.resource::<super::physics_scene::PhysicsScene>();
        let Some(frame)=scene.entity_frame(live,id)?else{return Ok(())};
        if frame.position==before||live.parent(id)?.is_some(){return Ok(())}
        let Some(motion)=scene.observe_rigid_completed_motion(live,id)?else{return Ok(())};
        //The actual149 request and solve may share an accepted clock marker.
        if self.object_throw_frame.is_none_or(|release|motion.0<release){return Ok(())}
        if !self.object_motion_seen{
            eprintln!("ACTION_CARRY_POSE stage=after_throw {}",super::physics_scene::carrying_host::describe_pose(world,id)?);
            eprintln!("ACTION_SMOKE_PASS check=actual_throw_owner_ack_and_completed_motion entity={id:?} actual_body_motion={motion:?} held_baseline={before:?} actor_position={:?} scope=throw_branch_ack_plus_postrelease_body_motion_not_direction_or_ballistic_parity",frame.position);
            eprintln!("ACTION_SMOKE_PASS check=actual_throw_motion unparented=true entity={id:?} before={before:?} after={:?} actual_contact_still_required=true",frame.position);
            self.object_motion_frame=Some(motion.0);self.object_motion_seen=true;
        }
        if self.object_contact.is_none(){
            if let Some(contact)=scene.observe_rigid_contact(live,id)?{
                if contact.bodies[0]!=motion.1{return Err("throw contact body generation differs from actual motion".into())}
                if self.object_motion_frame.is_some_and(|motion|contact.completed_frame>=motion){
                    eprintln!("ACTION_SMOKE_PASS check=actual_throw_contact unparented=true entity={id:?} after={:?} actual_solver_contact={contact:?} scope=actual_postrelease_impulse_not_persistent_floor_support",frame.position);
                    if let Some(support)=scene.observe_rigid_world_support(live,id)?{eprintln!("ACTION_SMOKE_PASS check=actual_throw_world_support entity={id:?} actual_solver_support={support:?}");}
                    eprintln!("ACTION_CARRY_POSE stage=contact {}",super::physics_scene::carrying_host::describe_pose(world,id)?);
                    self.object_contact=Some(contact);
                }
            }
        }
        Ok(())
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool,String> {
        if let Some(constraints)=&mut self.constraints{
            let complete=constraints.poll(world)?;
            if complete{self.stage=Stage::Done;}
            return Ok(complete)
        }

        if let Some(named)=&mut self.named_contact{named.frame(self.stage);}

        if let Some(wall)=&mut self.wall {return wall.poll(world)}
        if let Some(water)=&mut self.blink_water{
            let complete=water.poll(world)?;
            if complete{self.stage=Stage::Done;}
            return Ok(complete)
        }
        if let Some(bolt)=&mut self.bolt{
            let complete=bolt.poll(world)?;
            if complete{self.stage=Stage::Done;}
            return Ok(complete)
        }
        if let Some(grenade)=&mut self.grenade{
            let complete=grenade.poll(world)?;
            if complete{self.stage=Stage::Done;}
            return Ok(complete)
        }
        if let Some(blink)=&mut self.blink{
            let complete=blink.poll(world)?;
            if complete{self.stage=Stage::Done;}
            return Ok(complete)
        }
        self.world_followup_activation(world)?;
        for (contact,observation) in &mut self.world_prior_effects {
            Self::world_effect_progress(world,observation,*contact)?;
        }
        if self.world_power {
            let p=player(world)?;
            if let Some(handle)=p.machine.melee_left_response().filter(|handle|!p.body.midnight.finished(*handle)) {
                if !self.world_left_response_seen {
                    eprintln!("ACTION_SMOKE_WORLD_POWER_LEFT_RESPONSE actual_handle={handle} actual_scene_playing=true selected_power={:?} not_pixel_proof=true",p.selected_power().map(|power|power.settings.id));
                    self.world_left_response_seen=true;
                }
            }
        }
        // Sample every actual app frame across hold/release, including the
        // input-release frame, so endpoint-only evidence cannot hide a snap.
        let focus=if matches!(self.stage,Stage::Focus|Stage::FocusFade|Stage::ContactFocus|Stage::ContactPokeAim)||self.poke_only&&self.stage==Stage::ContactDamage{Some(self.focus_curve(world)?)}else{None};
        //Observe the first real release solve before the physical mouse-up
        //early return; otherwise an immediate impact disappears unobserved.
        if self.stage==Stage::ObjectMotion{self.observe_object_motion(world)?;}
        if let Some(input)=self.release.take(){send(world,input,false)?;return Ok(false)}
        if let Some(error)=super::weapon_firing::startup_error(world){return Err(format!("actual firing owner failed during {:?}: {error}",self.stage))}
        if let Some(error)=super::heart::startup_error(world){return Err(format!("actual Heart owner failed during {:?}: {error}",self.stage))}
        if self.since.elapsed()>Duration::from_secs(90) {
            let mut snapshot=String::new();player(world)?.write_combat_diagnostics(&mut snapshot);
            super::weapon_firing::write_diagnostics(world,&mut snapshot);
            if matches!(self.stage,Stage::Heart|Stage::HeartUse){
                if let Some(hearts)=world.get_resource::<super::heart::Hearts>(){hearts.write_diagnostics(&mut snapshot);}
            }
            if self.melee_only||self.poke_only||self.world_only {
                snapshot.push_str(&super::melee::debug_startup_pending(world));
            }
            if matches!(self.stage,Stage::CarryApproach|Stage::ContactAim|Stage::PickObject|Stage::ObjectMotion) {
                eprintln!("ACTION_SMOKE_FIXTURE_INCOMPLETE stage={:?} no_range_or_hit_override=true intended_actor={:?}",self.stage,self.object.map(|(id,_)|id));
                if let Err(error)=self.fixture_report(world,"timeout",true){eprintln!("ACTION_SMOKE_FIXTURE_REPORT_ERROR {error}");}
                if self.stage==Stage::ObjectMotion{
                    if let Some((id,_))=self.object{eprintln!("ACTION_CARRY_TIMEOUT {}",super::physics_scene::carrying_host::describe_pose(world,id)?);}
                }
            }
            self.weapon_evidence(world,"timeout")?;
            // The host's error packet is bounded. Preserve the full retained
            // witness on stderr before passing the summary across that link.
            eprintln!("ACTION_SMOKE_TIMEOUT_DIAGNOSTICS{snapshot}");
            return Err(format!("action smoke timeout stage={:?} shots={} projectile_seen={} snapshot={snapshot}",self.stage,self.shot_count,self.projectile_seen));
        }
        match self.stage {
            Stage::Begin if self.world_only => {
                let name=match(self.world_power,self.world_poke){(true,true)=>"world-power-poke",(true,false)=>"world-power",(false,true)=>"world-poke",_=>"world"};
                eprintln!("ACTION_SMOKE_SCENARIO name={name} setup=actual_level_spawn+physical_crouch+mouse source=630890+62f900 no_teleport=true no_range_override=true");
                if self.world_power {
                    let inventory=world.resource::<DiscoveredDocuments>();
                    let powers=world.resource::<super::powers::Powers>();
                    let power=powers.definitions.0.iter().find(|power|
                        crate::assets::power_journal::power_id(&power.power_id)==Some(2)&&inventory.quantity(&power.item)>0)
                        .ok_or("WORLD power scenario requires actually owned Blink")?;
                    let action=(0..10).find(|&index|inventory.wheel_shortcut_pair(index).0==Some(power.item.as_str()))
                        .map(|index|25+index as i32).ok_or("owned Blink has no actual shortcut")?;
                    eprintln!("ACTION_SMOKE_WORLD_POWER_SELECT actual_owned_item={} shortcut_action={action}",power.item);
                    self.tap(world,action)?;self.stage(Stage::WorldPower,world)?;
                }else{self.tap(world,19)?;self.stage(Stage::WorldSword,world)?;}
            }
            Stage::WorldPower if player(world)?.selected_power().is_some_and(|power|power.settings.id==2)
                &&player(world)?.equipment_action_status(0).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)=>{
                eprintln!("ACTION_SMOKE_WORLD_POWER_EQUIPPED actual_power=2 actual_left_transition_complete=true");
                self.tap(world,19)?;self.stage(Stage::WorldSword,world)?;
            }
            Stage::WorldSword if player(world)?.equipment_action_status(1).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)
                &&!player(world)?.machine.is_active(StateId::SLASH) => {
                let input=input_for(player(world)?.controller.sneak_action)?;
                send(world,input,true)?;self.world_crouch=Some(input);
                if self.world_poke{self.stage(Stage::WorldCharge,world)?;}else{self.stage(Stage::WorldAim,world)?;}
            }
            Stage::WorldCharge => {
                // Charge while looking upward, excluding the actual floor
                // from the initial ordinary swing. Aim down only AFTER the
                // state's own 86bec0 elapsed/declared-duration predicate.
                let (_,axis)=player(world)?.published_camera_frame();
                if self.held.is_none(){
                    if axis.x_axis.z<=0.{world.write_message(bevy::input::mouse::MouseMotion{delta:Vec2::new(0.,-8.)});}
                    else{self.world_followup_input(world)?;let input=input_for(19)?;send(world,input,true)?;self.held=Some(input);}
                }else if let Some((elapsed,required))=player(world)?.machine.melee_focus_charge(){
                    if elapsed>=required{
                        eprintln!("ACTION_SMOKE_WORLD_CHARGE elapsed={elapsed} authored_focus_hold_duration={required} ready=true source=86bec0");
                        self.stage(Stage::WorldAim,world)?;
                    }
                }
            }
            Stage::WorldAim => {
                if let Some(hit)=Self::world_fixture(world)?{
                    if let Some(fixture)=self.world_target{
                        let expected=crate::gameplay::collision_user_data::CollisionUserData::decode(fixture.user_data)?.component;
                        let current=crate::gameplay::collision_user_data::CollisionUserData::decode(hit.user_data)?.component;
                        if current!=expected{
                            return Err(format!("repeated WORLD fixture changed actual collision owner: first={fixture:?} current={hit:?}"))
                        }
                        eprintln!("ACTION_SMOKE_WORLD_AIM_OWNER component={current:?} initial_ray={fixture:?} current_ray={hit:?} body_material_equality_not_required=true");
                    }else{self.world_target=Some(hit);}
                    self.contact_serial=super::melee::world_action_receipt(world).map_or(0,|receipt|receipt.0);
                    self.contact_windows=super::melee::active_windows(world);
                    if self.world_power {
                        let p=player(world)?;
                        if !p.selected_power().is_some_and(|power|power.settings.id==2)
                            || !p.equipment_action_status(0).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning) {
                            return Err("WORLD attack lost its actual equipped power".into());
                        }
                        self.world_recovery_baseline=p.machine.melee_recoil_interrupt_receipt().ok_or("WORLD power marker recorder absent")?.0;
                    }
                    self.world_evidence(world,"before-physical-attack")?;
                    if self.world_poke{
                        let (elapsed,required)=player(world)?.machine.melee_focus_charge().ok_or("WORLD poke lost held focus before release")?;
                        if elapsed<required{return Err("WORLD poke native charge regressed before release".into())}
                        let input=self.held.take().ok_or("WORLD poke physical hold absent")?;send(world,input,false)?;
                    }else{self.world_followup_input(world)?;self.tap(world,19)?;}
                    self.stage(Stage::WorldContact,world)?;
                }else{
                    // Controller fixture increments only. No collision point,
                    // callback, weapon extent, elapsed time or pose is changed.
                    world.write_message(bevy::input::mouse::MouseMotion{delta:Vec2::new(0.,8.)});
                }
            }
            Stage::WorldContact => {
                if let Some((serial,hit,focused,forward,recoil,ticket))=super::melee::world_action_receipt(world){
                    if serial>self.contact_serial&&super::melee::active_windows(world)>self.contact_windows
                        &&focused==self.world_poke&&(!self.world_poke||forward)&&ticket {
                        let fixture=self.world_target.ok_or("WORLD receipt fixture absent")?;
                        //637160's sweep/blade ray and630890/636630's final
                        //retarget are not this diagnostic's setup camera ray.
                        //Require its actual WORLD owner, not a forced body.
                        let expected=crate::gameplay::collision_user_data::CollisionUserData::decode(fixture.user_data)?.component;
                        let actual=crate::gameplay::collision_user_data::CollisionUserData::decode(hit.user_data)?.component;
                        if actual!=expected{return Err(format!("WORLD callback changed actual collision owner: aimed={fixture:?} actual={hit:?}"))}
                        let scene=world.resource::<super::physics_scene::PhysicsScene>();
                        let owner=scene.point_queries()?;let queries=owner.read().map_err(|_|"WORLD receipt query owner poisoned")?;
                        let body_id=hit.body_id.ok_or("WORLD callback has no actual body identity")?;
                        let body=queries.storage.bodies.get(body_id).ok_or("WORLD callback body generation is no longer current")?;
                        if body.user_data!=hit.user_data{return Err("WORLD callback body no longer belongs to its reported collision owner".into())}
                        queries.storage.materials.material_for_id(hit.material_id)
                            .map_err(|error|format!("WORLD callback material={} serial={serial} hit={hit:?}: {error}",hit.material_id))?;
                        drop(queries);
                        eprintln!("ACTION_SMOKE_WORLD_SELECTED_OWNER component={actual:?} aimed={fixture:?} selected={hit:?} actual_body_generation_valid=true actual_material_valid=true body_material_equality_not_required=true");
                        self.world_evidence(world,"accepted-callback")?;
                        let ticket=super::melee::world_effect_ticket(world).ok_or("WORLD callback lost actual weak FX ticket")?;
                        eprintln!("ACTION_SMOKE_WORLD_CALLBACK contact={} serial={serial} focused={focused} forward={forward} recoil_requested={recoil} actual_FX_ticket=true acceptance_is_not_activation=true",self.world_contacts+1);
                        self.world_effect=Some(WorldEffectObservation{ticket,identity:None,publication:None,started:None,started_app_frame:None,particles:false,retired:false,
                            emitters:HashSet::new(),expected_events:HashSet::new(),admissions:HashSet::new(),processed:HashSet::new(),last:None});
                        self.stage(Stage::WorldTail,world)?;
                    }
                }
            }
            Stage::WorldTail => {
                let observation=self.world_effect.as_mut().ok_or("WORLD callback FX receipt absent")?;
                let retired=Self::world_effect_progress(world,observation,self.world_contacts+1)?;
                let p=player(world)?;
                let animation_ready=p.equipment_action_status(1).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)
                    &&!p.machine.is_active(StateId::SLASH)&&!p.machine.is_active(StateId::RECOIL);
                let marker=if self.world_power{p.machine.melee_recoil_interrupt_receipt()}else{None};
                let early_recovery=marker.is_some_and(|(serial,source,current,active)|serial>self.world_recovery_baseline
                    &&source!=0&&source==current&&active&&!p.machine.is_active(StateId::RECOIL)
                    &&!p.body.midnight.finished(source));
                if self.world_power&&!self.world_poke&&self.world_contacts<2&&animation_ready&&early_recovery{
                    let receipt=marker.unwrap();
                    eprintln!("ACTION_SMOKE_WORLD_RECOIL_CHAIN actual_marker={receipt:?} native_recovery_scene_still_playing=true physical_followup_allowed=true no_time_or_state_override=true prior_tail_retired={retired}");
                    self.world_prior_effects.push((self.world_contacts+1,self.world_effect.take().unwrap()));
                    self.world_contacts+=1;self.world_recovery_pending=Some((receipt.0,receipt.1));
                    self.stage(if self.world_poke{Stage::WorldCharge}else{Stage::WorldAim},world)?;
                }else if retired&&animation_ready&&self.world_prior_effects.iter().all(|(_,effect)|effect.retired){
                    if self.world_power&&(!self.world_left_response_seen || !self.world_poke&&(self.world_contacts!=2||self.world_chain_inputs!=2||self.world_chain_activations!=2||self.world_replacement_pending.is_some())){
                        return Err("WORLD power sequence lacks two real early-marker physical inputs AND actual early Slash activations, or actual left response".into());
                    }
                    self.world_contacts+=1;self.world_effect=None;
                    eprintln!("ACTION_SMOKE_PASS check={} accepted_contact={} same_WORLD_owner_fixture={:?} actual_FX_started_and_retired=true actual_animation_ready=true power_equipped={} early_marker_followups={} actual_early_Slash_activations={} left_response_started={}",if self.world_poke{"actual_WORLD_focused_forward"}else{"actual_WORLD_ordinary"},self.world_contacts,self.world_target,self.world_power,self.world_chain_inputs,self.world_chain_activations,self.world_left_response_seen);
                    if self.world_contacts==3{
                        if let Some(input)=self.world_crouch.take(){send(world,input,false)?}
                        self.stage(Stage::Done,world)?;
                    }else if self.world_power&&!self.world_poke{return Err("WORLD recovery scene ended without the actual early interrupt receipt".into())}
                    else{self.stage(if self.world_poke{Stage::WorldCharge}else{Stage::WorldAim},world)?;}
                }
            }
            Stage::Begin if self.carry_only => {
                // Debug fixture is the normal authored player spawn. The bottle
                // is only looked up, never moved or marked reached/admitted.
                if let Some(named)=&mut self.named_contact{named.restore(world)?;}
                let name=self.named_contact.as_ref().map_or("env_mulberry_mead_01_18",|named|named.target());
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let id=live.find(name).ok_or_else(||format!("named contact/carry fixture {name} is not live on this level"))?;
                if self.named_contact.is_some(){
                    let scene=world.resource::<super::physics_scene::PhysicsScene>();
                    if scene.entity_health(live,id)?.is_none_or(|(health,_)|health<=0.)||scene.breakable_component(live,id)?.is_none(){
                        return Err("named contact fixture lacks actual live Health and Started Breakable".into())
                    }
                }
                let frame=world.resource::<super::physics_scene::PhysicsScene>().entity_frame(live,id)?
                    .ok_or("carry fixture bottle lacks actual physical frame")?;
                self.object=Some((id,frame.position));
                eprintln!("ACTION_SMOKE_SCENARIO name=carry source=actual_managed_name target={name} entity={id:?} physical_position={:?} player_feet={:?} setup=normal_authored_spawn no_teleport=true",frame.position,player(world)?.feet());
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
                self.weapon_contract(world)?;self.weapon_evidence(world,"crossbow-before-input")?;
                self.tap(world,20)?;self.stage(Stage::Bolt,world)?;
            }
            Stage::Bolt => {
                let moved=self.projectiles(world,false)?;
                self.projectile_seen |= moved;
                let consumed=player(world)?.weapon.as_ref().is_some_and(|weapon|weapon.total_ammo<self.before_ammo);
                let settled=player(world)?.equipment_action_status(0).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)
                    && player(world)?.weapon.as_ref().is_some_and(|weapon|weapon.scene_handle()==0)
                    && !player(world)?.machine.is_active(StateId::FIRE_WEAPON);
                if consumed&&!self.bolt_c8_observed{self.bolt_c8_observed=true;self.weapon_evidence(world,"crossbow-ammo-consumed-postframe")?;}
                if consumed && self.projectile_seen && settled {
                    eprintln!("ACTION_SMOKE_PASS check=crossbow_real_C8_spawn_motion");
                    self.weapon_evidence(world,"crossbow-shot-completed")?;
                    self.select(world,KeyCode::Digit3)?;self.stage(Stage::Gun,world)?;
                }
            }
            Stage::Gun if Self::ranged_ready(world,RangedFamily::Gun)? => {
                let total=player(world)?.weapon.as_ref().unwrap().total_ammo;
                self.initial_gun_ammo=Some(total);self.before_ammo=total;
                self.weapon_contract(world)?;self.weapon_evidence(world,"gun-before-input")?;
                self.tap(world,20)?;self.gun_trigger_pending=true;self.stage(Stage::GunShot,world)?;
            }
            Stage::GunShot => {
                let now = Instant::now();
                let previous = self.shot_frame_time.replace(now).unwrap_or(self.since);
                self.longest_shot_frame = self.longest_shot_frame.max(now.duration_since(previous));
                self.shot_frames += 1;
                let p=player(world)?;let weapon=p.weapon.as_ref().ok_or("pistol disappeared before exhaustion")?;
                let observed_shot=weapon.total_ammo<self.before_ammo;
                if observed_shot {
                    self.shot_count+=1;self.before_ammo=weapon.total_ammo;
                    self.gun_trigger_pending=false;
                    eprintln!("ACTION_SMOKE_GUN_C8 shots={} remaining={} magazine={}",self.shot_count,self.before_ammo,weapon.magazine);
                }
                if weapon.total_ammo==0 && weapon.magazine==0 && weapon.scene_handle()==0 && !p.machine.is_active(StateId::FIRE_WEAPON) {
                    eprintln!("ACTION_GUN_FRAME_INTERVALS shots={} frames={} max_ms={:.2}",
                        self.shot_count, self.shot_frames, self.longest_shot_frame.as_secs_f64() * 1000.);
                    eprintln!("ACTION_SMOKE_PASS check=gun_actual_loadout_exhaust initial={:?} shots={}",self.initial_gun_ammo,self.shot_count);
                    self.tap(world,20)?;self.stage(Stage::Empty,world)?;
                } else if !self.gun_trigger_pending && Self::ranged_ready(world,RangedFamily::Gun)? {
                    self.tap(world,20)?;self.gun_trigger_pending=true;
                }
                if observed_shot{self.weapon_evidence(world,"gun-C8-consumed")?;}
            }
            Stage::Empty => {
                let p=player(world)?;let weapon=p.weapon.as_ref().ok_or("empty pistol disappeared")?;
                if weapon.total_ammo!=0 || weapon.magazine!=0{return Err("empty fire manufactured ammunition".into())}
                if weapon.scene_handle()!=0{self.empty_scene=Some(weapon.scene_handle())}
                if self.empty_scene.is_some_and(|handle|p.animation_scene_finished(handle)) && !p.machine.is_active(StateId::FIRE_WEAPON) {
                    eprintln!("ACTION_SMOKE_PASS check=empty_fire_actual_scene handle={:?}",self.empty_scene);
                    self.weapon_evidence(world,"gun-empty-scene-completed")?;
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
                    let start=super::audio::observation::snapshot(world).admissions.len();
                    let mut receipts=world.resource_mut::<Receipts>();
                    receipts.heart_audio_start=start;receipts.heart_events.clear();receipts.heart_source=None;drop(receipts);
                    self.tap(world,20)?;self.stage(Stage::HeartUse,world)?;
                }
            }
            Stage::HeartUse if world.resource::<Receipts>().heart_uses>self.heart_before => {
                let receipts=world.resource::<Receipts>();
                let id=receipts.heart.ok_or("Heart use receipt has no actual actor")?;
                if receipts.heart_selected!=Some(true){
                    let mut report=String::new();
                    if let Some(hearts)=world.get_resource::<super::heart::Hearts>(){hearts.write_diagnostics(&mut report);}
                    eprintln!("ACTION_SMOKE_HEART_SELECTION_FAILED {report}");
                    return Err(format!("Heart use processed but real bark selection was {:?}; actor={id:?}; actual Heart diagnostics emitted",receipts.heart_selected));
                }
                if let Some(event)=world.get_resource::<super::heart::Hearts>().and_then(|hearts|hearts.current_bark_event(id)){
                    if world.resource_mut::<Receipts>().heart_events.insert(event){eprintln!("ACTION_SMOKE_HEART_SELECTED_EVENT entity={id:?} event={:#010x} object={:?} source=canonical_Heart_Device_voice",event.0,event.1);}
                }
                let audio=super::audio::observation::snapshot(world);
                if world.resource::<Receipts>().heart_source.is_none(){
                    let receipts=world.resource::<Receipts>();
                    let source=audio.source_samples.iter().find(|source|
                        receipts.heart_events.contains(&(source.event,source.object))
                        && audio.source_processed_ids.contains(&source.playing_id)
                        && audio.admissions.iter().skip(receipts.heart_audio_start).any(|admission|
                            admission.error.is_none()&&admission.playing_id==source.playing_id&&admission.event==source.event
                            &&admission.object==format!("{:?}",source.object)));
                    if let Some(source)=source{
                        let key=(source.event,source.playing_id,source.object,source.selected_source);
                        eprintln!("ACTION_SMOKE_HEART_PLAYBACK_RETAIN event={:#010x} playing_id={} object={:?} selected_source={:#010x} first_receipt_reason={} first_receipt_peak={} lifecycle_not_complete=true",key.0,key.1,key.2,key.3,source.reason,source.dry_peak);
                        world.resource_mut::<Receipts>().heart_source=Some(key);
                    }
                }
                let retained=world.resource::<Receipts>().heart_source;
                let finished=retained.and_then(|(event,playing,object,node)|audio.source_samples.iter().find(|source|
                    source.terminal&&source.event==event&&source.playing_id==playing&&source.object==object&&source.selected_source==node
                    &&audio.ended_ids.contains(&playing)));
                if let Some(source)=finished{
                    eprintln!("ACTION_SMOKE_HEART_PLAYBACK_COMPLETE entity={id:?} event={:#010x} playing_id={} object={:?} selected_source={:#010x} frames={} terminal_reason={} decoded_peak={} lifetime_post_voice_dry_peak={} lifetime_post_voice_aux_peak={} speaker_gain_peak={} route_bus={:?} route_target_peak={:?} master_target_peak={:?} final_route_next_block_gain={:?} final_master_next_block_gain={:?} actual_End_callback=true aggregate_output_samples={} aggregate_output_peak={} individual_final_bus_contribution_not_attributed=true physical_audibility_not_claimed=true",
                        source.event,source.playing_id,source.object,source.selected_source,source.frames,source.reason,
                        source.decoded_peak,source.dry_peak,source.aux_peak,source.gain_peak,source.route_bus,source.route_target_peak,source.master_target_peak,
                        source.route_next_block_gain,source.master_next_block_gain,audio.samples,audio.peak);
                    if source.decoded_peak==0.||(source.dry_peak==0.&&source.aux_peak==0.){
                        return Err("actual selected Heart source ended without decoded/nonzero post-voice output; complete lifecycle receipt emitted".into());
                    }
                    eprintln!("ACTION_SMOKE_PASS check=Heart_actual_bark_selection_and_nonzero_pre_bus_samples entity={id:?} selection=true source_callback=true actual_admission=true source_lifecycle_complete=true actual_End_callback=true final_bus_or_physical_audibility_not_claimed=true");
                    let input=input_for(45)?;send(world,input,true)?;self.held=Some(input);self.stage(Stage::WheelOpen,world)?;
                }
            }
            Stage::WheelOpen if world.get_resource::<super::power_wheel::State>().is_some_and(|wheel|wheel.movie_frame_ready()) => {
                eprintln!("ACTION_SMOKE_PASS check=wheel_real_open");
                {
                    let mut windows=world.query::<&mut Window>();
                    let mut window=windows.iter_mut(world).next().ok_or("wheel window absent")?;
                    let position=Vec2::new(window.width()*0.5+24.,window.height()*0.5+12.);
                    self.wheel_cursor=Some((position.x as f64*1920./window.width() as f64,position.y as f64*1080./window.height() as f64));
                    window.set_cursor_position(Some(position));
                }
                self.stage(Stage::WheelMove,world)?;
            }
            Stage::WheelMove if self.wheel_cursor.is_some_and(|position|world.get_resource::<super::power_wheel::State>().is_some_and(|wheel|wheel.cursor_frame_ready(position))) => {
                eprintln!("ACTION_SMOKE_PASS check=wheel_actual_mouse_frame");
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
                // Named non-pickable targets still require the same actual
                // melee reach goal. ContactAim subsequently requires the real
                // nearest ray hit before any attack input can be issued.
                if close&&(focused||self.named_contact.is_some()) {
                    if let Some(input)=self.approach_move.take(){send(world,input,false)?}
                    eprintln!("ACTION_SMOKE_PASS check={} entity={id:?}",if self.named_contact.is_some(){"fixture_actual_melee_approach"}else{"fixture_real_focus"});
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
                self.weapon_contract(world)?;self.weapon_evidence(world,"contact-gun-ready")?;
                self.contact_baseline(world)?;
                self.stage(Stage::ContactAim,world)?;
            }
            Stage::ContactSword if player(world)?.equipment_action_status(1).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)
                && !player(world)?.machine.is_active(StateId::SLASH) => {
                self.contact_baseline(world)?;self.stage(Stage::ContactAim,world)?;
            }
            Stage::ContactAim|Stage::ContactPokeAim => {
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
                    if self.stage==Stage::ContactPokeAim{
                        let (elapsed,required)=player(world)?.machine.melee_focus_charge()
                            .ok_or("poke fixture lost actual held Slash focus before release")?;
                        eprintln!("ACTION_SMOKE_POKE_CHARGE source=86bec0 elapsed={elapsed} authored_focus_hold_duration={required} ready={}",elapsed>=required);
                        if elapsed<required{return Ok(false)}
                        // The initial normal swing is not poke evidence. The
                        // target must survive charging, and only a NEW actual
                        // focused/forward callback after this release can pass.
                        let current=world.resource::<super::physics_scene::PhysicsScene>().entity_health(world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),id)?
                            .ok_or("poke fixture Health disappeared before release")?.0;
                        if current<self.contact_health.unwrap(){return Err("poke fixture damaged during held charge before release; not accepted as poke evidence".into())}
                        self.contact_baseline(world)?;
                        eprintln!("ACTION_SMOKE_POKE_RELEASE target={id:?} baseline_serial={} active_windows={} health={:?}",self.contact_serial,self.contact_windows,self.contact_health);
                        if let Some(input)=self.held.take(){send(world,input,false)?}
                        self.stage(Stage::ContactDamage,world)?;
                    }else if self.poke_only{
                        self.stage(Stage::ContactLookAway,world)?;
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
            Stage::ContactLookAway => {
                let id=self.object.ok_or("charge setup target absent")?.0;
                if Self::fixture_outside_focus(world,id)?{
                    eprintln!("ACTION_SMOKE_POKE_CHARGE_SETUP target={id:?} source_focus_front_planes_exclude_actual_model_bounds=true input=physical_MELEE_hold");
                    let input=input_for(19)?;send(world,input,true)?;self.held=Some(input);self.stage(Stage::ContactFocus,world)?;
                }else{
                    // First level the real view, then turn away. Continuing
                    // to pitch upward can hit the authored pitch limit before
                    // a low-height source focus box excludes the bottle.
                    let (_,axis)=player(world)?.published_camera_frame();
                    let delta=if axis.x_axis.z<0.{Vec2::new(0.,-8.)}else{Vec2::new(8.,0.)};
                    world.write_message(bevy::input::mouse::MouseMotion{delta});
                }
            }
            Stage::ContactFocus if focus.is_some_and(|(requested,_,full)|requested&&full) => {
                eprintln!("ACTION_SMOKE_PASS check=fixture_poke_actual_full_focus");
                // Keep the physical button held while aiming back. The same
                // canonical nearest-hit ray above must see the real bottle.
                self.stage(Stage::ContactPokeAim,world)?;
            }
            Stage::ContactDamage => {
                let id=self.object.unwrap().0;
                // Observe real tickets from the first damage frame, including
                // while the existing poke focus fade is still progressing.
                let effects_done=self.contact_effects.as_mut().ok_or("contact FX baseline absent")?.update(world,id)?;
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                let scene=world.resource::<super::physics_scene::PhysicsScene>();
                let receipt=super::melee::action_receipt(world,id);
                let focused_receipt=super::melee::focused_action_receipt(world,id);
                let focused_callback=!self.poke_only||focused_receipt.is_some_and(|(serial,accepted,damage,focused,forward)|serial>self.contact_serial&&accepted&&damage>0.&&focused&&forward);
                let melee_callback=focused_callback&&receipt.is_some_and(|(serial,accepted,damage)|serial>self.contact_serial&&accepted&&damage>0.)
                    && super::melee::active_windows(world)>self.contact_windows;
                let consumed=if self.melee_only||self.poke_only{melee_callback}else{player(world)?.weapon.as_ref().is_some_and(|weapon|weapon.total_ammo<self.before_ammo)};
                let health=if live.get(id).is_some(){scene.entity_health(live,id)?}else{None};
                let retired=live.get(id).is_none();
                self.contact_damage_seen |= consumed && (retired || health.is_some_and(|(current,_)|current<self.contact_health.unwrap()));
                let fade_done=!self.poke_only||focus.is_some_and(|(requested,count,_)|!requested&&count==0);
                if self.contact_damage_seen&&fade_done&&!self.contact_health_reported {
                    let mut details=String::new();scene.write_breakable_diagnostics(player(world)?.feet().to_array(),&mut details);
                    let check=if self.named_contact.is_some(){"actual_named_melee_breakable_damage"}else if self.poke_only{"actual_poke_bottle_damage"}else if self.melee_only{"actual_melee_bottle_damage"}else{"actual_gun_bottle_damage"};
                    eprintln!("ACTION_SMOKE_PASS check={check} entity={id:?} before={:?} after={health:?} retired={retired} melee_receipt={receipt:?} focused_forward_receipt={focused_receipt:?} active_windows={} breakable_state={details}",self.contact_health,super::melee::active_windows(world));
                    self.weapon_evidence(world,"contact-health-changed")?;
                    self.contact_health_reported=true;
                }
                if self.contact_health_reported&&effects_done {
                    self.contact_effects.as_ref().unwrap().report(id);
                    self.contact_effects=None;
                    if let Some(named)=&mut self.named_contact{named.report();}
                    self.stage(Stage::Done,world)?;
                }
            }
            Stage::PickObject if player(world)?.carrying.has_held() => {
                let held=player(world)?.carrying.held.as_ref().unwrap().entity;
                if self.object.map(|(id,_)|id)!=Some(held){return Err("carry input picked a different entity generation".into())}
                eprintln!("ACTION_SMOKE_PASS check=actual_pickup_hold entity={held:?}");
                eprintln!("ACTION_CARRY_POSE stage=held {}",super::physics_scene::carrying_host::describe_pose(world,held)?);
                self.stage(if self.object_drop_seen{Stage::ThrowObject}else{Stage::DropObject},world)?;
            }
            Stage::DropObject if player(world)?.possession_state()==Some(14) => {
                let id=self.object.unwrap().0;
                eprintln!("ACTION_CARRY_POSE stage=before_drop {}",super::physics_scene::carrying_host::describe_pose(world,id)?);
                self.tap(world,22)?;self.stage(Stage::DroppedObject,world)?;
            }
            Stage::DroppedObject if !player(world)?.carrying.has_work() => {
                let id=self.object.unwrap().0;
                if !player(world)?.carrying.observed_drop_ack(id){return Ok(false)}
                let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
                if live.parent(id)?.is_some(){return Ok(false)}
                eprintln!("ACTION_SMOKE_PASS check=actual_drop_owner_ack unparented=true entity={id:?}");
                eprintln!("ACTION_CARRY_POSE stage=after_drop {}",super::physics_scene::carrying_host::describe_pose(world,id)?);
                self.object_drop_seen=true;
                self.stage(Stage::CarryApproach,world)?;
            }
            Stage::ThrowObject if player(world)?.possession_state()==Some(14) => {
                // Pickup aimed down at the real bottle. Aim level/up through
                // ordinary input before throwing, so a valid immediate impact
                // cannot retire it before the physical-motion observation.
                let (_,camera)=player(world)?.published_camera_frame();
                if camera.x_axis.z<0. {
                    world.write_message(bevy::input::mouse::MouseMotion{delta:Vec2::new(0.,-8.)});
                    return Ok(false);
                }
                let id=self.object.unwrap().0;
                let frame=world.resource::<super::physics_scene::PhysicsScene>().entity_frame(world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),id)?
                    .ok_or("held object's actual frame absent")?;
                self.object_throw_frame=Some(world.resource::<crate::gameplay::game_clock_runtime::ClockRuntime>().clock().map_err(str::to_owned)?.frame_marker());
                eprintln!("ACTION_CARRY_POSE stage=before_throw {}",super::physics_scene::carrying_host::describe_pose(world,id)?);
                self.object=Some((id,frame.position));self.tap(world,19)?;self.stage(Stage::ObjectMotion,world)?;
            }
            Stage::ObjectMotion if !player(world)?.carrying.has_work() => {
                let id=self.object.unwrap().0;
                if self.object_motion_seen&&self.object_contact.is_some(){self.stage(Stage::Done,world)?;}
                else if world.resource::<crate::gameplay::entity_lifetime::LiveEntities>().get(id).is_none(){
                    return Err(format!("thrown object retired without a captured actual postrelease contact; motion_seen={}; disappearance alone is not accepted",self.object_motion_seen));
                }
            }
            Stage::Done => {self.weapon_evidence(world,"sequence-complete")?;eprintln!("ACTION_SMOKE_PASS check=sequence_complete exit=ordinary_ExitDrain");return Ok(true)}
            _ => {}
        }
        Ok(false)
    }
    pub(super) fn release_contact_effects(&mut self) { self.contact_effects = None; }
    pub(super) fn completed(&self)->bool{self.stage==Stage::Done}
}
