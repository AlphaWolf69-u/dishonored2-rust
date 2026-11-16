//! WeaponThrowable632ba0/632bf0 and ThrowWeapon84a480/84aa20.
//! The grenade component owns its fuse and effects. This owner keeps only
//! the selected weapon's input/animation/cooking state and source references.
use super::{Ammunition, Equipment, boolean, read_ammunition, text};
use crate::{
    entities::{Value, scene_motion::{child, items}},
    gameplay::{damage::number, entity_lifetime::EntityId},
};
use bevy_math::{Mat3, Vec3};
use std::{sync::Arc, collections::VecDeque};

type Fields = [(String, Value)];

#[derive(Clone, Debug)]
pub struct CookingStage {
    pub delay: f32,
    pub cooking_sound: Option<String>,
    pub cooked_sound: Option<String>,
}

/// Immutable632bf0 defaults and141702f40 reflected declaration fields.
#[derive(Clone, Debug)]
pub struct Definition {
    pub ammunition: Vec<Ammunition>,
    pub aim: super::AimSettings,
    pub fire_playrate_attribute: Option<String>,
    pub reload_playrate_attribute: Option<String>,
    pub cooking_stages: Box<[CookingStage]>,
    pub throw_angle: f32,
    pub throw_speed: f32,
    pub throw_angular_velocity: Vec3,
    pub angular_speed_in_camera_space: bool,
    pub low_throw_required_pitch: f32,
    pub low_throw_maximum_height: f32,
    pub low_throw_angle: f32,
    pub low_throw_speed: f32,
    pub safe_cooking_extra_time_for_ui: f32,
}
impl Definition {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        let stages = items(child(fields, "m_cookingStages"));
        //632bf0 starts the source list with two inline elements; declaration
        //list loading owns its real authored count, not a two-stage limit.
        let mut cooking_stages = Vec::with_capacity(stages.len());
        for stage in stages {
            cooking_stages.push(CookingStage {
                delay: number(stage, "m_delay", 0.)?,
                cooking_sound: text(stage, "m_cookingSoundEvent"),
                cooked_sound: text(stage, "m_cookedSoundEvent"),
            });
        }
        let angular = child(fields, "m_throwAngularSpeed");
        Ok(Self {
            ammunition: read_ammunition(fields)?, aim: super::AimSettings::read(child(fields,"m_aimSettings"))?, cooking_stages: cooking_stages.into_boxed_slice(),
            fire_playrate_attribute: text(fields,"m_firePlayrateAttribute"),
            reload_playrate_attribute: text(fields,"m_reloadPlayrateAttribute"),
            throw_angle: number(fields, "m_throwAngle", 5.)?,
            throw_speed: number(fields, "m_throwSpeed", 25.)?,
            throw_angular_velocity: angular_velocity([
                number(angular, "pitch", 0.)?, number(angular, "yaw", 0.)?, number(angular, "roll", 0.)?,
            ]),
            angular_speed_in_camera_space: boolean(fields, "m_throwAngularSpeedInCameraSpace", false)?,
            low_throw_required_pitch: number(fields, "m_lowThrowRequiredCameraPitch", 10.)?,
            low_throw_maximum_height: number(fields, "m_lowThrowMaxHeightBelowPlayerAtAimedPos", 1.)?,
            low_throw_angle: number(fields, "m_lowThrowAngle", 0.)?,
            low_throw_speed: number(fields, "m_lowThrowSpeed", 20.)?,
            safe_cooking_extra_time_for_ui: number(fields, "m_safeCookingExtraTimeForUI", 0.)?,
        })
    }
    ///632dc0 requires the real optional low-fire pointer, camera pitch and
    ///current ranged sensor target height; a missing world query is not a hit.
    pub fn low_throw(&self, equipment: &Equipment, camera_pitch: f32, player_z: f32, aim_z: f32) -> bool {
        equipment.scenes.contains_key("m_lowThrowFireMidnight")
            && self.low_throw_required_pitch <= camera_pitch
            && player_z - aim_z < self.low_throw_maximum_height
    }
    ///632f70. Aim/origin come from the same canonical638ae0 sight query as
    ///other ranged equipment. The caller supplies the world RNG's real sample.
    pub fn launch(&self, origin: Vec3, aim: Vec3, weapon_axis: Mat3, camera_axis: Mat3,
        low: bool, failure_chance: f32, failure_sample_percent: f32) -> (Vec3, Vec3) {
        let direction = aim - origin;
        let direction = direction / direction.length_squared().max(f32::MIN_POSITIVE).sqrt();
        let basis = crate::gameplay::reference_frame::direction_basis(direction);
        let (angle, speed) = if low { (self.low_throw_angle, self.low_throw_speed) }
            else { (self.throw_angle, self.throw_speed) };
        let radians = angle * f32::from_bits(0x3c8e_fa35);
        let (sine, cosine) = radians.sin_cos();
        let mut linear = (basis.x_axis * cosine + basis.z_axis * sine) * speed;
        if failure_sample_percent < failure_chance { linear *= f32::from_bits(0x3dcc_cccd); }
        let axis = if self.angular_speed_in_camera_space { camera_axis } else { weapon_axis };
        (linear, axis * self.throw_angular_velocity)
    }
}

///10d3d0 ->10d780 ->111550: authored idAngles become the angular
///velocity vector of that rotation, rather than three independent Euler rates.
fn angular_velocity([pitch, yaw, roll]: [f32; 3]) -> Vec3 {
    let radians = f32::from_bits(0x3c8e_fa35);
    if pitch == 0. && yaw == 0. { return Vec3::X * (roll * radians); }
    if pitch == 0. && roll == 0. { return Vec3::Z * (yaw * radians); }
    if yaw == 0. && roll == 0. { return Vec3::Y * (pitch * radians); }
    let (sp, cp) = (pitch * radians * 0.5).sin_cos();
    let (sy, cy) = (yaw * radians * 0.5).sin_cos();
    let (sr, cr) = (roll * radians * 0.5).sin_cos();
    let vector = Vec3::new(cr * sp * sy - sr * cp * cy,
        -(cr * sp * cy) - sr * cp * sy, sr * sp * cy - cr * cp * sy);
    let scalar = sr * sp * sy + cr * cp * cy;
    if scalar >= 1. { return Vec3::ZERO; }
    let axis = vector / vector.length_squared().max(f32::MIN_POSITIVE).sqrt();
    -axis * (scalar.max(-1.).acos() * 2.)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum Stage { #[default] Inactive = 0, Ready = 1, Cooking = 2, Throw = 3, Reload = 4, Empty = 5 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene { CookingIn, CookingIdle, Fire, Reload, Empty }
#[derive(Clone, Copy, Debug, Default)]
pub struct GrenadeState { pub armed: bool, pub exploded: bool }

///The adapter addresses the actual checked weapon generation; in particular
///Deactivation must not clean up a subsequently selected different weapon.
pub trait StateHost {
    fn current_weapon(&self) -> Option<EntityId>;
    fn weapon_alive(&self, weapon: EntityId) -> bool;
    fn grenade_state(&self, weapon: EntityId) -> Result<Option<GrenadeState>, String>;
    fn button_down(&self) -> bool;
    fn scene_finished(&self) -> bool;
    fn begin_scene(&mut self, weapon: EntityId, scene: Scene) -> Result<(), String>;
    fn stop_scene(&mut self);
    fn has_remaining_ammunition(&self, weapon: EntityId) -> bool;
    fn update_cooking(&mut self, weapon: EntityId) -> Result<(), String>;
    fn cancel_cooking(&mut self, weapon: EntityId) -> Result<(), String>;
    fn disarm(&mut self, weapon: EntityId) -> Result<(), String>;
    fn exploded_cleanup(&mut self, weapon: EntityId) -> Result<(), String>;
    fn finish_weapon_action(&mut self, weapon: EntityId) -> Result<(), String>;
    fn quit(&mut self);
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThrowState {
    pub stage: Stage,
    pub button_down: bool,
    weapon: Option<EntityId>,
}
impl ThrowState {
    ///84a480: left slot/archetype4, checked weapon reference, stage1.
    pub fn activate(&mut self, host: &mut impl StateHost) -> Result<(), String> {
        let Some(weapon) = host.current_weapon() else { host.quit(); return Ok(()); };
        self.weapon = Some(weapon);
        self.button_down = true;
        self.stage = Stage::Ready;
        host.begin_scene(weapon, Scene::CookingIn)
    }
    ///84aa20: phase changes are actual scene completion/release callbacks,
    ///never invented cook timers. The grenade owner independently owns fuse.
    pub fn update(&mut self, host: &mut impl StateHost) -> Result<(), String> {
        let Some(weapon) = host.current_weapon() else { host.quit(); return Ok(()); };
        if host.grenade_state(weapon)?.is_some_and(|state| state.exploded) { host.quit(); return Ok(()); }
        if self.button_down && !host.button_down() {
            self.button_down = false;
            if self.stage == Stage::Cooking {
                self.stage = Stage::Throw;
                host.begin_scene(weapon, Scene::Fire)?;
            }
        }
        if host.scene_finished() {
            match self.stage {
                Stage::Ready => {
                    if self.button_down {
                        self.stage = Stage::Cooking;
                        host.begin_scene(weapon, Scene::CookingIdle)?;
                    } else {
                        self.stage = Stage::Throw;
                        host.begin_scene(weapon, Scene::Fire)?;
                    }
                }
                Stage::Cooking => {},
                Stage::Throw => {
                    if host.grenade_state(weapon)?.is_some_and(|state| state.armed) { host.quit(); return Ok(()); }
                    if host.has_remaining_ammunition(weapon) {
                        self.stage = Stage::Reload;
                        host.begin_scene(weapon, Scene::Reload)?;
                    } else {
                        self.stage = Stage::Empty;
                        host.begin_scene(weapon, Scene::Empty)?;
                    }
                }
                _ => { self.stage = Stage::Inactive; host.quit(); }
            }
        }
        if self.stage == Stage::Cooking { host.update_cooking(weapon)?; }
        Ok(())
    }
    ///84a6f0: interrupt through the stored checked weapon, not a replacement.
    pub fn deactivate(&mut self, host: &mut impl StateHost) -> Result<(), String> {
        host.stop_scene();
        if let Some(weapon) = self.weapon.filter(|&weapon| host.weapon_alive(weapon)) {
            let state = host.grenade_state(weapon)?;
            if state.is_some_and(|state| state.exploded) { host.exploded_cleanup(weapon)?; }
            else if self.stage != Stage::Inactive {
                match self.stage {
                    Stage::Ready | Stage::Cooking => {
                        host.cancel_cooking(weapon)?;
                        host.finish_weapon_action(weapon)?;
                    }
                    Stage::Throw => {
                        if state.is_some_and(|state| state.armed) { host.disarm(weapon)?; }
                        host.finish_weapon_action(weapon)?;
                    }
                    Stage::Reload => host.finish_weapon_action(weapon)?,
                    _ => {},
                }
            }
        }
        self.weapon = None;
        self.button_down = false;
        self.stage = Stage::Inactive;
        Ok(())
    }
}

///632ba0's owned immutable declaration and live cooking counters. Audio
///events keep declaration references; the audio owner performs actual leasing.
pub struct WeaponRuntime {
    pub entity: EntityId,
    pub definition: Arc<Definition>,
    pub cooking_duration: f32,
    pub cooking_stage: i32,
    pub low_throw: bool,
    pub grenade_state: Option<GrenadeState>,
    pub ready: bool,
    pub ammunition_index: usize,
    pub magazine: i32,
    pub total_ammo: u32,
    pub target_flags: u32,
    pub active_scene: u32,
    pub equipment_target: usize,
    pub source: Option<Source>,
    pub entity_delta: f32,
    effects: VecDeque<Effect>,
}
///632f70 snapshots the source component and accepted aim before an animation
///callback can change selection. Definitions and source IDs remain retained
///through the actual spawn/transfer acknowledgement.
#[derive(Clone)]
pub struct Source {
    pub definition: Option<Arc<crate::assets::runtime_entity::Definition>>,
    pub attacker: EntityId,
    pub origin: Vec3,
    pub eye: Vec3,
    pub aim: Vec3,
    pub weapon_axis: Mat3,
    pub weapon_position: Vec3,
    pub camera_axis: Mat3,
    pub failure_chance: f32,
}
#[derive(Clone)]
pub enum Effect {
    Arm { in_hand: bool, bypass_hit_floor: bool },
    Disarm,
    Reset,
    ConsumeAmmo(String),
    FiredNotice { attacker: EntityId, position: Vec3 },
    Sound(String),
    Fire { source: Source, linear: Vec3, angular: Vec3, target_flags: u32 },
}
impl WeaponRuntime {
    pub fn new(entity: EntityId, definition: Arc<Definition>) -> Self {
        Self { entity, definition, cooking_duration: 0., cooking_stage: -1, low_throw: false,
            grenade_state: None, ready: false, ammunition_index: 0, magazine: 0, total_ammo: 0,
            target_flags: 0, active_scene: 0, equipment_target: 3, source: None, entity_delta:0., effects: VecDeque::new() }
    }
    pub fn ammunition(&self) -> Option<&Ammunition> { self.definition.ammunition.get(self.ammunition_index) }
    pub fn can_fire(&self) -> bool { self.ready && self.source.is_some() && self.magazine > 0 }
    pub fn push_effect(&mut self, effect: Effect) { self.effects.push_back(effect); }
    pub fn drain_effects(&mut self) -> impl Iterator<Item=Effect> + '_ { self.effects.drain(..) }
    pub fn reload(&mut self, inventory: &(impl super::AmmunitionInventory + ?Sized)) {
        self.total_ammo = self.ammunition().map_or(0, |ammo| inventory.quantity(&ammo.item));
        self.magazine = i32::from(self.total_ammo != 0);
    }
    ///632f70 consumes before actor construction; the journal retains the
    ///already accepted source and velocities across asynchronous callbacks.
    pub fn fire(&mut self, inventory: &mut (impl super::AmmunitionInventory + ?Sized), sample: f32) {
        if self.grenade_state.is_some_and(|state| state.exploded) || !self.can_fire() { return; }
        let source = self.source.as_ref().expect("can_fire validates source").clone();
        let item = self.ammunition().expect("can_fire validates ammunition").item.clone();
        inventory.consume(&item, 1);
        self.total_ammo = inventory.quantity(&item);
        self.magazine = (self.magazine - 1).max(0);
        let (linear, angular) = self.definition.launch(source.origin, source.aim, source.weapon_axis,
            source.camera_axis, self.low_throw, source.failure_chance, sample);
        self.effects.push_back(Effect::Fire { source, linear, angular, target_flags: self.target_flags });
        if let Some(sound) = self.definition.cooking_stages.get(self.cooking_stage.max(0) as usize)
            .filter(|_| self.cooking_stage >= 0).and_then(|stage| stage.cooked_sound.as_ref()) {
            self.effects.push_back(Effect::Sound(sound.clone()));
        }
    }
    ///633790 resets both counters when selecting CookingIn.
    pub fn begin_cooking_in(&mut self) { self.cooking_duration = 0.; self.cooking_stage = -1; }
    ///6339a0 advances at most one authored sound stage each state update.
    pub fn update_cooking(&mut self, dt: f32) -> Option<&str> {
        self.cooking_duration += dt;
        let (index, stage) = self.definition.cooking_stages.iter().enumerate()
            .find(|(index, stage)| self.cooking_duration >= stage.delay && (*index as i32) > self.cooking_stage)?;
        self.cooking_stage = index as i32;
        stage.cooking_sound.as_deref()
    }
    ///632d40 resets elapsed time, while retaining its last stage index.
    pub fn cancel_cooking(&mut self) -> Option<&str> {
        self.cooking_duration = 0.;
        self.definition.cooking_stages.get(usize::try_from(self.cooking_stage).ok()?)?.cooked_sound.as_deref()
    }
    pub fn scene<'a>(&self, equipment: &'a Equipment, scene: Scene, sliding: bool) -> Option<&'a str> {
        let (normal, slide, low) = match scene {
            Scene::CookingIn => ("m_cookingInMidnight", Some("m_cookingInMidnightSlide"), None),
            Scene::CookingIdle => ("m_cookingIdleMidnight", Some("m_cookingIdleMidnightSlide"), None),
            Scene::Fire => ("m_fireMidnight", Some("m_fireMidnightSlide"), Some("m_lowThrowFireMidnight")),
            Scene::Reload => ("m_reloadMidnight", Some("m_reloadMidnightSlide"), Some("m_lowThrowReloadMidnight")),
            Scene::Empty => ("m_outOfAmmoMidnight", Some("m_outOfAmmoMidnightSlide"), Some("m_lowThrowOutOfAmmoMidnight")),
        };
        if self.low_throw {
            if let Some(scene) = low.and_then(|key| equipment.scenes.get(key)) { return Some(scene); }
        }
        if sliding {
            if let Some(scene) = slide.and_then(|key| equipment.scenes.get(key)) { return Some(scene); }
        }
        equipment.scenes.get(normal).map(String::as_str)
    }
}

impl super::super::Player {
    pub fn equipment_ammunition(&self,identity:&str)->Option<(&[super::Ammunition],Option<&str>)>{
        if let Some(runtime)=self.registered_ranged(identity){
            return Some((&runtime.settings.ammunition,Some(runtime.ammunition().item.as_str())));
        }
        self.registered_throwable(identity).map(|runtime|(runtime.definition.ammunition.as_slice(),runtime.ammunition().map(|ammo|ammo.item.as_str())))
    }
    pub fn throwable_fire_source(&self,id:EntityId)->Option<(Source,f32)>{
        self.throwable.iter().chain(self.holstered_throwables.values()).find(|runtime|runtime.entity==id)
            .and_then(|runtime|runtime.source.clone().map(|source|(source,runtime.definition.aim.max_deviation_angle)))
    }
    pub fn set_throwable_origin(&mut self,id:EntityId,origin:Vec3){
        if let Some(source)=self.throwable.iter_mut().chain(self.holstered_throwables.values_mut())
            .find(|runtime|runtime.entity==id).and_then(|runtime|runtime.source.as_mut()){source.origin=origin;}
    }
    pub fn register_throwable(&mut self,identity:&str,entity:EntityId,definition:Arc<Definition>,inventory:&dyn super::AmmunitionInventory)->Result<(),String>{
        if self.registered_throwable(identity).is_some(){return Ok(());}
        let mut runtime=WeaponRuntime::new(entity,definition);
        runtime.reload(inventory);
        self.holstered_throwables.insert(identity.to_owned(),runtime);
        Ok(())
    }
    pub fn registered_throwable(&self,identity:&str)->Option<&WeaponRuntime>{
        if self.throwable_identity.as_deref()==Some(identity){self.throwable.as_ref()}else{self.holstered_throwables.get(identity)}
    }
    pub fn equip_throwable(&mut self,identity:&str)->Result<(),String>{
        if self.throwable_identity.as_deref()==Some(identity){return Ok(());}
        self.unequip_throwable();
        self.throwable=Some(self.holstered_throwables.remove(identity).ok_or("owned WeaponThrowable was not initialized")?);
        self.throwable_identity=Some(identity.to_owned());Ok(())
    }
    pub fn select_throwable_ammunition(&mut self,identity:&str,item:&str,inventory:&dyn super::AmmunitionInventory)->Result<(),String>{
        let runtime=if self.throwable_identity.as_deref()==Some(identity){self.throwable.as_mut()}else{self.holstered_throwables.get_mut(identity)}
            .ok_or("owned WeaponThrowable is absent during ammunition selection")?;
        let Some(index)=runtime.definition.ammunition.iter().position(|ammo|ammo.item==item)else{return Err(format!("WeaponThrowable {identity} has no ammunition {item}"));};
        runtime.magazine=0;
        if runtime.ammunition_index!=index{runtime.ammunition_index=index;runtime.ready=false;runtime.source=None;}
        runtime.reload(inventory);Ok(())
    }
    pub fn unequip_throwable(&mut self){
        if let Some(mut runtime)=self.throwable.take(){
            runtime.active_scene=0;
            if runtime.grenade_state.is_some_and(|state|state.armed){runtime.push_effect(Effect::Disarm);}
            if let Some(identity)=self.throwable_identity.take(){self.holstered_throwables.insert(identity,runtime);}
        }
    }
    pub fn retire_throwable(&mut self,identity:&str){
        if self.throwable_identity.as_deref()==Some(identity){self.unequip_throwable();}
        if let Some(mut runtime)=self.holstered_throwables.remove(identity){
            let id=runtime.entity;self.throwable_effects.extend(runtime.drain_effects().map(|effect|(id,effect)));
        }
    }
    pub fn drain_throwable_effects(&mut self)->impl Iterator<Item=(EntityId,Effect)>+'_ {
        if let Some(runtime)=&mut self.throwable{
            let id=runtime.entity;self.throwable_effects.extend(runtime.drain_effects().map(|effect|(id,effect)));
        }
        for runtime in self.holstered_throwables.values_mut(){
            let id=runtime.entity;self.throwable_effects.extend(runtime.drain_effects().map(|effect|(id,effect)));
        }
        self.throwable_effects.drain(..)
    }
    pub fn handle_throwable_animation_event(&mut self,event:&super::super::AnimationEvent,equipment:Option<EntityId>,inventory:&mut dyn super::AmmunitionInventory)->Result<(),String>{
        let Some(id)=equipment else{return Ok(());};
        let runtime=self.throwable.iter_mut().chain(self.holstered_throwables.values_mut()).find(|runtime|runtime.entity==id);
        let Some(runtime)=runtime else{return Ok(());};
        if runtime.active_scene==0 || runtime.active_scene!=event.handle || !(event.target==0 || event.target==runtime.equipment_target){return Ok(());}
        match event.name.as_str(){
            "ARK_MDNT_MSG_WEAPON_FIRE"=>runtime.fire(inventory,crate::random::lcg_unit(&mut self.random)*100.),
            "ARK_MDNT_MSG_WEAPON_RELOAD"=>runtime.reload(inventory),
            _=>{},
        }Ok(())
    }
}
