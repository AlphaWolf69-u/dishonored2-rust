//! The first-person player: controller input, the locomotion state machine,
//! physics with its character proxy, and the camera, updated in the game
//! frame's stage order (FUN_1408ad260):
//! input dispatch → QUERY_PHYSICS (physics queries) → DEFAULT (physics query
//! results, controller, state machine, camera) → PRE_PHYSICS (physics step).
pub mod animation;
pub mod adrenaline;
pub mod arms;
pub mod equipment;
pub mod equipment_attachment;
pub mod weapons;
pub mod power_states;
pub mod grapple;
pub mod grapple_state;
pub mod combat;
mod diagnostics;
pub mod combat_fx;
pub mod chaos;
pub mod melee;
mod power_wheel;
pub mod carrying;
pub mod consumables;
pub mod slash_rules;
pub mod equipment_states;
pub mod body;
pub mod body_parts;
pub mod camera;
pub mod character_proxy;
pub mod controller;
pub mod decls;
pub mod footsteps;
pub mod health;
pub mod mana;
pub mod midnight;
pub mod physics;
pub mod swimming;
pub mod sensors;
pub mod shape;
pub mod simplex_solver;
pub mod states;
pub mod read_note;
mod world_pickup;

use bevy_math::{Mat3, Vec3};
use std::sync::Arc;
use body::Body;
use camera::{BodyPose, PlayerCamera, View};
use character_proxy::{PlayerWorld, SupportState};
use controller::{ActionEvent, Controller, LookSettings};
use physics::{MotionInput, PlayerPhysics};
use sensors::{AutoCrouchSensor, LeanSensor, MantleSensor, VaultSensor};
use states::{Attributes, Frame, Kind, LocoStances, MotionRequests, Retargeting, Scenes, StateId, StateMachine};

/// One first-person arm for the renderer (arkCpntPlayerBody arm parts):
/// its md6 and its joints' world transforms (game Z-up, rows of [R | t]),
/// in the arm skeleton's joint order. `visible` is the part's m_visible.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArmRender {
    pub model: String,
    /// Resource identity selects geometry; an actual equipment entity owns
    /// its independent transform/visibility even when geometry is shared.
    pub managed_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub joints: Vec<[[f32; 4]; 3]>,
    pub visible: bool,
}

/// Numeric action history retained so F8 need not land on a short animation.
#[derive(Debug)]
pub struct ArmActionDiagnostic {
    pub selected_hands: Option<[bool; 2]>,
    pub scenes: Vec<midnight::ActionSceneDiagnostic>,
    pub scene_start_error: Option<String>,
    pub frames: u32,
    pub tree_drove: [bool; 3],
    pub published: [bool; 2],
    pub visible: [bool; 2],
    pub ever_in_front: [bool; 2],
    /// Min/max horizontal and vertical camera slopes while in front.
    pub camera_slopes: [[f32; 4]; 2],
    pub pose_error: [bool; 2],
    pub tree_frames: [u32; 3],
    pub visible_frames: [u32; 2],
    pub in_front_frames: [u32; 2],
}

/// A point event emitted by the installed Midnight track. Consumers associate
/// the scene handle with the action they requested; no guessed timer or alias
/// determines when mana, ammunition or a hit becomes effective.
pub type AnimationEvent = midnight::PointEvent;

/// Component/state requests pass through the player attribute owner so its
/// actual notifications reach health, mana, powers and equipped components.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeModifierRequest {
    pub owner: String,
    pub index: usize,
    pub tag: String,
    pub increase: f32,
    pub percentage: f32,
    pub duration: f32,
    pub added: bool,
}

struct ActiveModifier {
    owner: String,
    index: usize,
    tag: String,
    increase: f32,
    percentage: f32,
    expires_at: Option<f64>,
}

/// Entity notifications published to attached component owners. The original
/// incapacitation message carries damage parameters; this records the damage
/// type needed by the connected drowning consumer.
#[derive(Clone, Copy, Debug)]
pub enum EntityEvent {
    Damage { params: crate::gameplay::damage::Parameters, result: crate::gameplay::damage::DamageResult, was_dead: bool },
}

pub struct Player {
    pub physics: PlayerPhysics,
    pub controller: Controller,
    pub machine: StateMachine,
    pub combat_fx: Option<combat_fx::Runtime>,
    pub chaos: chaos::Runtime,
    pub carrying: carrying::Runtime,
    swimming_machine: StateMachine,
    swimming_active: bool,
    swimming_eye_height: f32,
    swimming_current: swimming::WaterCurrent,
    swimming_waves: swimming::WaveFilters,
    swimming_wave_pose: (f32, Vec3),
    breath: swimming::Breath,
    pub incoming_damage_factor: f32,
    pub incoming_damage_back_factor: f32,
    pub health_regen_factor: f32,
    pub health_heal_factor: f32,
    pub mana_replenishes: bool,
    pub camera: PlayerCamera,
    camera_axis: Mat3,
    body_parts: body_parts::BodyParts,
    pub stances: LocoStances,
    pub attributes: Attributes,
    pub ranged_target: weapons::shooting::RangedTargetDecl,
    pub rune_charms: std::sync::Arc<weapons::heart::sensor::Definition>,
    heart_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    ranged_target_hit: Option<Vec3>,
    ranged_target_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    heart_bark_playing:bool,
    heart_requests:std::collections::VecDeque<weapons::heart::UseRequest>,
    active_modifiers: Vec<ActiveModifier>,
    next_modifier_expiration: Option<f64>,
    attribute_modifier_events: Vec<(String, bool)>,
    attribute_requests: Vec<AttributeModifierRequest>,
    entity_events: Vec<EntityEvent>,
    managed_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub body: Body,
    /// The last body pose sampled for the view.
    pub pose: BodyPose,
    pub autocrouch: AutoCrouchSensor,
    pub lean_sensor: LeanSensor,
    pub catfall_sensor: sensors::CatfallSensor,
    pub mantle_sensor: MantleSensor,
    pub vault_sensor: VaultSensor,
    pub swimming_settings: swimming::SwimmingSettings,
    swimming_available: bool,
    pub swimming_contact: Option<swimming::SwimmingContact>,
    /// The mantle dummy's arkCpntRetargeting while it drives the player.
    pub retargeting: Option<Retargeting>,
    scenes: Scenes,
    md6: String,
    health: health::Health,
    damage_receiver: crate::gameplay::damage::Receiver,
    fall_damage: Option<std::sync::Arc<crate::assets::damage::Declaration>>,
    drowning_damage_enabled: bool,
    drowning_damage: Option<std::sync::Arc<crate::assets::damage::Declaration>>,
    mana: mana::Mana,
    powers: power_states::Runtime,
    equipment_changes: equipment_states::Runtime,
    pub weapon: Option<weapons::Runtime>,
    pub throwable: Option<weapons::throwable::WeaponRuntime>,
    throwable_identity: Option<String>,
    holstered_throwables: std::collections::HashMap<String,weapons::throwable::WeaponRuntime>,
    throwable_effects: Vec<(crate::gameplay::entity_lifetime::EntityId,weapons::throwable::Effect)>,
    weapon_identity: Option<String>,
    holstered_weapons: std::collections::HashMap<String, weapons::Runtime>,
    weapon_effects: Vec<weapons::WeaponMessage>,
    pub adrenaline: adrenaline::Adrenaline,
    /// Populated by the combat and bloodthirsty owners when they exist.
    pub adrenaline_in_combat: bool,
    pub bloodthirsty_active: bool,
    game_over_requested: bool,
    game_over_fallback: Option<f64>,
    entity_event_time: f64,
    drowning_fx_requested: Option<String>,
    /// Shared gameplay stream (gameLocal+0x600f8). 8a37c0 seeds it with
    /// zero; 8ac040 uses the same 15-bit sample as random::lcg_unit.
    random: u32,
    /// `ark_playerSmoothedLoco` / `ark_playerBobAmount` and the difficulty's
    /// lean limits, set by the host.
    pub body_settings: body::BodySettings,
    requests: MotionRequests,
    /// The arms as of the last view (PRE_RENDER).
    arm_render: Vec<ArmRender>,
    arm_pose_errors: [Option<String>; 2],
    arm_action_history: [Option<ArmActionDiagnostic>; 2],
    arm_action_active: [bool; 2],
    animation_events: Vec<AnimationEvent>,
    animation_sound_emitters: Vec<crate::gameplay::sound::Emitter>,
    /// Host receipt for the original pre-component Midnight stage. The
    /// app's actual clock marker prevents a second advance in Player::frame.
    prepared_animation_frame: Option<i32>,
    equipment_action_handles: [Option<u32>; 2],
    equipment_frames: [Option<(Vec3, Mat3)>; 2],
    equipment_entity_frames: Vec<(crate::gameplay::sound::Emitter, Vec3, Mat3, Option<crate::gameplay::sound::component::Settings>)>,
    equipment_archetypes: [Option<u32>; 2],
}

/// Everything `Player::new` reads from the installed declarations.
pub struct PlayerSetup {
    pub chaos: chaos::Definition,
    pub combat_fx: Option<std::sync::Arc<combat_fx::Definition>>,
    pub physics: physics::PhysicsDecl,
    pub camera: camera::CameraDecl,
    pub mapping: Vec<controller::StateMapping>,
    pub states: Vec<states::StateDecl>,
    pub rules: Vec<states::RuleDecl>,
    pub default_state: StateId,
    pub attributes: Attributes,
    pub ranged_target: weapons::shooting::RangedTargetDecl,
    pub rune_charms: std::sync::Arc<weapons::heart::sensor::Definition>,
    pub body: Body,
    pub autocrouch: sensors::AutoCrouchDecl,
    pub lean: sensors::LeanDecl,
    pub mantle: sensors::MantleDecl,
    pub slipping: controller::SlippingThresholds,
    /// The midnight scenes the states play, prepared for the body.
    pub scenes: Scenes,
    /// The body md6 the scenes' CONDMD6 nodes match.
    pub md6: String,
    /// Health decl m_health before attributes.
    pub health: health::Settings,
    pub damage_receiver: crate::gameplay::damage::Receiver,
    pub fall_damage: Option<std::sync::Arc<crate::assets::damage::Declaration>>,
    pub slash_rules: Option<std::sync::Arc<crate::assets::slash_rules::Definition>>,
    pub mana: mana::Settings,
    pub adrenaline: adrenaline::Settings,
    pub swimming_settings: swimming::SwimmingSettings,
    pub swimming_states: Vec<swimming::SwimmingStateDecl>,
    pub swimming_equipment: Vec<states::StateDecl>,
    pub swimming_rules: Vec<states::RuleDecl>,
    pub swimming_default_state: StateId,
    pub swimming_has_mantle: bool,
    pub swimming_read_note: Option<states::StateDecl>,
    pub swimming_death: Option<states::DrowningDeathDecl>,
    pub breath: swimming::Breath,
    pub accepts_water_damage: bool,
    pub drowning_damage_enabled: bool,
    pub drowning_damage: Option<std::sync::Arc<crate::assets::damage::Declaration>>,
}

impl Player {
    /// Start an acquired scene on its declared player cast. Equipment/action
    /// admission remains with the requesting component, as in 8542b0.
    pub fn start_animation_scene(&mut self, scene: &std::sync::Arc<midnight::PreparedScene>, rate: f32, parts: u8) -> Result<u32, String> {
        if self.health.is_dead() { return Err("dead player cannot start an equipment action".into()); }
        if parts == 0 || parts & !0b1_1111 != 0 { return Err("invalid player animation cast mask".into()); }
        let models = body::cast_md6(&self.md6, &self.body.arms, &self.body.equipment);
        let cast = body::cast(&self.body.animations, &self.body.arms, &self.body.equipment);
        self.body.midnight.start_with_parts(scene, models, rate, cast, parts, &mut self.random)
    }

    pub fn cancel_animation_scene(&mut self, handle: u32) {
        self.body.midnight.stop(handle);
        self.animation_events.retain(|event| event.handle != handle);
    }

    pub fn animation_scene_finished(&self, handle: u32) -> bool { self.body.midnight.finished(handle) }

    pub fn take_animation_events(&mut self) -> Vec<AnimationEvent> { std::mem::take(&mut self.animation_events) }
    pub fn drain_animation_events(&mut self) -> std::vec::Drain<'_, AnimationEvent> { self.animation_events.drain(..) }

    pub fn install_animation_scenes(&mut self, scenes: Scenes) { self.scenes.extend(scenes); }
    pub fn equip_ranged(&mut self, identity: &str, settings: std::sync::Arc<weapons::Ranged>, inventory: &dyn weapons::AmmunitionInventory) -> Result<(), String> {
        if self.weapon_identity.as_deref() == Some(identity) { return Ok(()); }
        self.register_ranged(identity, settings, inventory)?;
        let mut runtime = self.holstered_weapons.remove(identity).ok_or("registered ranged component is absent")?;
        runtime.refresh_attributes(&self.attributes);
        runtime.update_inventory(inventory.quantity(&runtime.ammunition().item));
        // 63a090 invokes shared virtual148=63a2b0 only for an empty
        // magazine; subclass virtual108 owns the resulting attachment update.
        if runtime.magazine == 0 { runtime.reload_magazine(); }
        self.unequip_ranged();
        runtime.activate();
        self.weapon = Some(runtime);
        self.weapon_identity = Some(identity.to_owned());
        Ok(())
    }
    /// 639f00 starts each actual owned component before selection, choosing
    /// its first ammunition and initializing the subclass attachment callback.
    pub fn register_ranged(&mut self, identity: &str, settings: std::sync::Arc<weapons::Ranged>, inventory: &dyn weapons::AmmunitionInventory) -> Result<(), String> {
        if self.registered_ranged(identity).is_some() { return Ok(()); }
        let total = inventory.quantity(&settings.ammunition.first().ok_or("ranged equipment has no ammunition")?.item);
        let capacity = settings.capacity(0, &self.attributes);
        let mut runtime = weapons::Runtime::new(settings, capacity, total, 3)?;
        runtime.identity = Some(std::sync::Arc::from(identity));
        // Actual owned-component Start639f00 always invokes OnAmmoChanged100;
        // queue the gun's44e720 notification with its real actor identity.
        if let Some(message) = runtime.capacity_recomputed().and_then(|effect| runtime.message(effect)) { self.weapon_effects.push(message); }
        self.holstered_weapons.insert(identity.to_owned(), runtime);
        Ok(())
    }
    pub fn registered_ranged(&self, identity: &str) -> Option<&weapons::Runtime> {
        if self.weapon_identity.as_deref() == Some(identity) { self.weapon.as_ref() } else { self.holstered_weapons.get(identity) }
    }
    /// The canonical actual owned Start supplies the component's parent
    /// generation. Queued callbacks retain it across deletion/readmission.
    pub fn bind_ranged_entity(&mut self, identity: &str, entity: crate::gameplay::entity_lifetime::EntityId) -> Result<(), String> {
        let runtime = if self.weapon_identity.as_deref() == Some(identity) { self.weapon.as_mut() }
            else { self.holstered_weapons.get_mut(identity) };
        let runtime = runtime.ok_or("ranged entity bound before actual component Start")?;
        if runtime.managed_entity.is_some_and(|previous| previous != entity) { return Err("live ranged component rebound to another entity generation".into()); }
        runtime.managed_entity = Some(entity);
        for message in &mut self.weapon_effects {
            if message.entity.is_none() && message.identity.as_ref() == identity { message.entity = Some(entity); }
        }
        Ok(())
    }
    pub fn install_ranged_projectiles(&mut self, identity: &str,
        leases: Vec<Option<std::sync::Arc<crate::assets::projectiles::Prepared>>>) -> Result<(), String> {
        let runtime = if self.weapon_identity.as_deref() == Some(identity) { self.weapon.as_mut() }
            else { self.holstered_weapons.get_mut(identity) };
        runtime.ok_or("projectiles prepared before actual owned ranged Start")?.install_projectiles(leases)
    }
    pub fn install_ranged_ammunition_flags(&mut self, identity: &str, flags: Vec<u32>) -> Result<(), String> {
        let runtime = if self.weapon_identity.as_deref() == Some(identity) { self.weapon.as_mut() }
            else { self.holstered_weapons.get_mut(identity) };
        runtime.ok_or("ammo upgrades prepared before actual owned ranged Start")?.install_ammunition_flags(flags)
    }
    pub fn ranged_identity(&self) -> Option<&str> { self.weapon_identity.as_deref() }
    pub fn ranged_ammunition_index(&self, identity: &str) -> Option<usize> {
        if self.weapon_identity.as_deref() == Some(identity) { self.weapon.as_ref() }
        else { self.holstered_weapons.get(identity) }.map(|weapon| weapon.ammunition_index())
    }
    /// The last published camera world frame, available without sampling a
    /// second body pose during a component callback.
    pub fn published_camera_frame(&self) -> (Vec3, Mat3) { (self.camera.eye, self.camera_axis) }
    pub fn set_ranged_target_hit(&mut self, hit: Option<Vec3>) { self.ranged_target_hit = hit; }
    pub fn ranged_target_hit(&self) -> Option<Vec3> { self.ranged_target_hit }
    pub fn set_ranged_target_entity(&mut self, entity: Option<crate::gameplay::entity_lifetime::EntityId>) { self.ranged_target_entity = entity; }
    pub fn ranged_target_entity(&self) -> Option<crate::gameplay::entity_lifetime::EntityId> { self.ranged_target_entity }
    pub fn unequip_ranged(&mut self) {
        if let Some(mut weapon) = self.weapon.take() {
            let handle = weapon.scene_handle();
            if handle != 0 { self.body.midnight.stop(handle); }
            weapon.deactivate();
            self.weapon_effects.extend(weapon.drain_effects());
            if let Some(identity) = self.weapon_identity.take() { self.holstered_weapons.insert(identity, weapon); }
        }
    }
    /// The inventory's actual destruction notification retires the component.
    pub fn retire_ranged(&mut self, identity: &str) {
        if self.weapon_identity.as_deref() == Some(identity) { self.unequip_ranged(); }
        self.holstered_weapons.remove(identity);
    }
    pub fn drain_weapon_effects(&mut self) -> std::vec::Drain<'_, weapons::WeaponMessage> {
        if let Some(weapon) = &mut self.weapon { self.weapon_effects.extend(weapon.drain_effects()); }
        self.weapon_effects.drain(..)
    }
    /// 855230 queries the active equipment action component, rather than a
    /// locomotion blend. The equipment switch owner registers its real scene.
    pub fn set_equipment_action_handle(&mut self, side: usize, handle: Option<u32>) {
        if let Some(slot) = self.equipment_action_handles.get_mut(side) { *slot = handle; }
    }
    /// Side order is the actual player cast: left(slot1), right(slot0).
    pub fn set_equipped_archetype(&mut self, side: usize, archetype: Option<u32>) {
        if let Some(value) = self.equipment_archetypes.get_mut(side) { *value = archetype; }
    }

    /// Change only prepared hands. This does not alter visibility requests
    /// belonging to swimming, mantle or a full-body scene.
    pub fn request_arm_equipment(&mut self, equipment: [&str; 2]) -> bool {
        if self.body.arms.len() != 2 || self.body.arms.iter().zip(equipment).any(|(arm, name)| !arm.has_equipment(name)) { return false; }
        for (arm, name) in self.body.arms.iter_mut().zip(equipment) { arm.request_equipment(name); }
        true
    }

    /// The receiving health component owns scaling and immunity. The player
    /// consumes its mana split and publishes the existing incapacitation path.
    pub fn receive_damage(&mut self, parameters: &crate::gameplay::damage::Parameters) -> crate::gameplay::damage::DamageResult {
        use crate::gameplay::damage::Context;
        let was_dead = self.health.is_dead();
        if was_dead {
            let result = Default::default();
            self.entity_events.push(EntityEvent::Damage { params: *parameters, result, was_dead });
            return result;
        }
        let attribute = |name: &str| self.attributes.0.get(name).copied().unwrap_or(1.0);
        let forward = self.physics.axis().x_axis;
        let difficulty_factor = if forward.dot(parameters.hit_direction) <= 0.0 { self.incoming_damage_factor } else { self.incoming_damage_back_factor };
        let context = Context { forward, difficulty_factor,
            incoming_scale: attribute("ARK_ATTRIBUTE_INCOMING_DAMAGE_SCALE"), explosion_factor: attribute("ARK_ATTRIBUTE_EXPLOSION_DAMAGE_FACTOR"),
            health_to_mana: self.attributes.get("ARK_ATTRIBUTE_MALUS_HEALTH_TO_MANA_DAMAGES"), mana: self.mana.current(), evolved: true, ..Default::default() };
        let result = self.health.receive_damage(&self.damage_receiver, parameters, context, &self.attributes);
        if result.mana_damage > 0.0 { self.mana.consume(result.mana_damage, &self.attributes, self.mana_replenishes); }
        if result.incapacitated {
            self.adrenaline.stop();
            self.controller.input_move = Vec3::ZERO;
            self.controller.input_look = Vec3::ZERO;
            self.game_over_fallback = Some(self.entity_event_time + 7.0);
            self.machine.request(StateId::DEAD);
            self.swimming_machine.request(StateId::DEAD);
        }
        self.entity_events.push(EntityEvent::Damage { params: *parameters, result, was_dead });
        result
    }

    pub fn read_note_state_allowed(&self) -> Option<bool> {
        if self.swimming_active { self.swimming_machine.read_note_state_allowed() } else { self.machine.read_note_state_allowed() }
    }
    /// The interaction owner checks item and game-logic admission before
    /// requesting the mode's parameterized READ_NOTE state.
    pub fn request_read_note(&mut self, presentation: std::sync::Arc<read_note::Presentation>) -> bool {
        if self.machine.read_note_busy() || self.swimming_machine.read_note_busy() { return false; }
        if self.swimming_active { self.swimming_machine.request_read_note(presentation) } else { self.machine.request_read_note(presentation) }
    }
    pub fn take_read_note_result(&mut self) -> Option<read_note::ActivationResult> {
        self.machine.take_read_note_result().or_else(|| self.swimming_machine.take_read_note_result())
    }
    pub fn read_note_request_pending(&self) -> bool {
        self.machine.read_note_request_pending() || self.swimming_machine.read_note_request_pending()
    }
    pub fn set_read_note_admission(&mut self, available: bool) {
        self.machine.set_read_note_admission(available);
        self.swimming_machine.set_read_note_admission(available);
    }
    pub fn read_note_presentation(&self) -> Option<&std::sync::Arc<read_note::Presentation>> {
        self.machine.read_note_presentation().or_else(|| self.swimming_machine.read_note_presentation())
    }
    pub fn set_read_note_presentation(&mut self, presentation: std::sync::Arc<read_note::Presentation>) -> bool {
        if self.swimming_active { self.swimming_machine.set_read_note_presentation(presentation) } else { self.machine.set_read_note_presentation(presentation) }
    }
    pub fn finish_read_note(&mut self) {
        self.machine.finish_read_note();
        self.swimming_machine.finish_read_note();
    }

    /// The current health owned by the player; HUD consumers do not mutate it.
    pub fn health(&self) -> f32 {
        self.health.current()
    }
    pub fn mana(&self) -> f32 { self.mana.current() }
    pub fn max_mana(&self) -> f32 { self.mana.maximum(&self.attributes) }
    pub fn mana_hud_values(&self) -> (f32, f32, f32, bool, bool, u8) {
        (self.mana(), self.max_mana(), self.mana.regen_target(&self.attributes, self.mana_replenishes),
            self.mana.has_extension(&self.attributes), self.mana.magic_suppressed(&self.attributes), self.mana.lock_flags())
    }
    pub fn set_mana_lock_flags(&mut self, flags: u8) { self.mana.set_lock_flags(flags); }
    /// Attribute modifier lifecycle notification (0xd6). The adrenaline
    /// component remains attached when stopped, so it can receive this event.
    pub fn attribute_modifier_changed(&mut self, tag: &str, added: bool) {
        if tag == "ARK_ATTRIBUTE_GUN_AMMO_CAPACITY" {
            if let Some(weapon) = &mut self.weapon { weapon.refresh_attributes(&self.attributes); }
            for weapon in self.holstered_weapons.values_mut() { weapon.refresh_attributes(&self.attributes); }
        }
        self.health.attribute_changed(tag, added, &self.attributes, self.health_regen_factor);
        if tag == "ARK_ATTRIBUTE_MANA_MAX" { self.mana.maximum_changed(added, &self.attributes); }
        if tag == "ARK_ATTRIBUTE_BLOODTHIRSTY_ENABLED" {
            if added { self.adrenaline.start(); } else { self.adrenaline.stop(); }
        }
        self.attribute_modifier_events.push((tag.to_owned(), added));
    }
    /// Drain the 0xd6 notifications for other attached component owners.
    pub fn take_attribute_modifier_events(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.attribute_modifier_events)
    }
    pub fn take_entity_events(&mut self) -> Vec<EntityEvent> { std::mem::take(&mut self.entity_events) }
    pub fn drain_entity_events(&mut self) -> std::vec::Drain<'_, EntityEvent> { self.entity_events.drain(..) }
    pub fn set_managed_entity(&mut self, entity: Option<crate::gameplay::entity_lifetime::EntityId>) { self.managed_entity = entity; }
    pub fn managed_entity(&self) -> Option<crate::gameplay::entity_lifetime::EntityId> { self.managed_entity }
    pub fn attribute_differs_from_base(&self, tag: &str) -> bool {
        self.attributes.1.get(tag).is_some_and(|value| self.attributes.get(tag) != value.base)
    }
    fn refresh_modifier_deadline(&mut self) {
        self.next_modifier_expiration = self.active_modifiers.iter().filter_map(|value| value.expires_at)
            .min_by(f64::total_cmp);
    }
    /// Apply an inventory/power modifier and deliver the matching 0xd6 event.
    pub fn apply_attribute_modifier(&mut self, owner: &str, index: usize, tag: &str, increase: f32, percentage: f32, duration: f32, added: bool, now: f64) -> bool {
        if added {
            if !self.attributes.modifier(tag, increase, percentage, true) { return false; }
            let expires_at = (duration > 0.0).then_some(now + f64::from(duration));
            self.active_modifiers.push(ActiveModifier { owner: owner.to_owned(), index, tag: tag.to_owned(), increase, percentage, expires_at });
            if let Some(deadline) = expires_at {
                self.next_modifier_expiration = Some(self.next_modifier_expiration.map_or(deadline, |next| next.min(deadline)));
            }
            self.attribute_modifier_changed(tag, true);
            true
        } else if let Some(position) = self.active_modifiers.iter().position(|value| value.owner == owner && value.index == index) {
            let value = self.active_modifiers.remove(position);
            if value.expires_at.is_some() && value.expires_at == self.next_modifier_expiration { self.refresh_modifier_deadline(); }
            self.attributes.modifier(&value.tag, value.increase, value.percentage, false);
            self.attribute_modifier_changed(&value.tag, false);
            true
        } else if duration > 0.0 && self.attributes.1.contains_key(tag) {
            // 14060c9b0 still publishes the remove event after a timed
            // record has expired, without subtracting its value twice.
            self.attribute_modifier_changed(tag, false);
            true
        } else { false }
    }

    pub fn request_carried_object(&mut self, entity: crate::gameplay::entity_lifetime::EntityId,
        definition: Arc<crate::gameplay::pickable_component::PickableDefinition>) -> bool {
        let machine = if self.swimming_active { &mut self.swimming_machine } else { &mut self.machine };
        let state=StateId(definition.pickup_interaction.player_state);
        let grab=match machine.state(state).map(|state|&state.decl.kind){Some(Kind::GrabObject(value))=>value.clone(),_=>return false};
        let hold_state=if definition.hold.as_ref().is_some_and(|props|props.both_hands){StateId(50)}else{StateId(49)};
        let hold=match machine.state(hold_state).map(|state|&state.decl.kind){Some(Kind::HoldObject(value))=>value.clone(),_=>return false};
        if !self.carrying.request(carrying::Target {entity,definition,grab,hold}){return false;}
        machine.request(state);
        true
    }
    ///83d3e0 compares the current and authored base of AttributesPlayer+118.
    ///435660 binds that slot through14397ea50;14137abc0 names STRENGTH.
    pub fn carrying_greater_throw(&self) -> bool {
        self.attributes.1.get("ARK_ATTRIBUTE_STRENGTH")
            .is_some_and(|attribute| self.attributes.get("ARK_ATTRIBUTE_STRENGTH") != attribute.base)
    }
    ///435660 binds11c to14397ea58;14137abe0 registers this exact tag.
    ///808480 uses1 only when the actual cached attribute is absent.
    pub fn carrying_throw_damage_multiplier(&self) -> f32 {
        if self.attributes.1.contains_key("ARK_ATTRIBUTE_STRENGTH_DAMAGE_MULTIPLIER") {
            self.attributes.get("ARK_ATTRIBUTE_STRENGTH_DAMAGE_MULTIPLIER")
        } else { 1.0 }
    }

    pub fn carried_object_reached(&mut self, entity: crate::gameplay::entity_lifetime::EntityId) {
        if let Some(state) = self.carrying.reached(entity) {
            let machine = if self.swimming_active { &mut self.swimming_machine } else { &mut self.machine };
            machine.request(state);
        }
    }
    pub fn carrying_grab_decl(&self) -> Option<Arc<carrying::GrabDecl>> {
        let machine = if self.swimming_active { &self.swimming_machine } else { &self.machine };
        match &machine.state(self.carrying.grab_state()?)?.decl.kind {
            states::Kind::GrabObject(value) => Some(value.clone()), _ => None,
        }
    }
    pub fn carrying_hold_decl(&self, id: StateId) -> Option<Arc<carrying::HoldDecl>> {
        let machine = if self.swimming_active { &self.swimming_machine } else { &self.machine };
        match &machine.state(id)?.decl.kind {
            states::Kind::HoldObject(value) => Some(value.clone()), _ => None,
        }
    }

    pub fn max_health(&self) -> f32 { self.health.maximum(&self.attributes) }
    pub fn is_dead(&self) -> bool { self.health.is_dead() }
    pub fn projected_health(&self) -> f32 { self.health.projected(&self.attributes, self.health_regen_factor) }
    /// The body md6 the scenes' CONDMD6 nodes match.
    pub fn md6(&self) -> &str { &self.md6 }
    pub fn health_hud(&self) -> Option<(f32, f32, f32, bool, i32)> {
        let extensions = self.health.extensions(&self.attributes);
        Some((self.health(), self.max_health(), self.projected_health(), self.health() < 20.0, extensions))
    }
    /// Advance the same game-local LCG already used by player scene picks.
    pub fn choose_random(&mut self, count: usize) -> Option<usize> {
        if count == 0 { return None; }
        if count == 1 { return Some(0); }
        self.random = self.random.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
        Some(((self.random >> 10) & 0x7fff) as usize % count)
    }
    pub fn take_game_over_request(&mut self) -> bool {
        std::mem::take(&mut self.game_over_requested)
    }
    pub fn game_over_requested(&self) -> bool { self.game_over_requested }
    /// Game-clock group 2, used by the engine's delayed entity events.
    pub fn set_entity_event_time(&mut self, time: f64) { self.entity_event_time = time; }
    pub fn drowning_fx_requested(&self) -> bool { self.drowning_fx_requested.is_some() }
    pub fn take_drowning_fx_request(&mut self) -> Option<String> { self.drowning_fx_requested.take() }

    /// 14088a8c0 compares the gameplay eye with the completed head sample.
    pub fn is_underwater(&self) -> bool {
        self.swimming_contact.is_some_and(|water| self.camera.eye.z <= water.head)
    }

    pub fn is_swimming(&self) -> bool { self.swimming_active }

    /// The actual body model selected at player setup, for F8 lookup.
    pub(crate) fn body_model(&self) -> &str { &self.md6 }
    /// The active mode's states, including when the swimming mode owns them.
    pub fn active_player_states(&self) -> &[StateId] { self.machine.active_states() }
    /// Controller-owned possession state. Possession transitions and saved
    /// state restoration are not connected yet.
    pub fn possession_state(&self) -> Option<i32> { Some(self.controller.possession_state()) }

    /// Breath is a percentage in the original component (0..100).
    pub fn breath(&self) -> f32 { self.breath.remaining }

    pub fn arm_pose_errors(&self) -> &[Option<String>; 2] {
        &self.arm_pose_errors
    }
    pub fn arm_action_history(&self) -> &[Option<ArmActionDiagnostic>; 2] { &self.arm_action_history }

    /// Spawn with the feet at `position` (idPlayerStart.spawnPosition) facing
    /// `yaw` radians.
    pub fn new(mut setup: PlayerSetup, position: Vec3, yaw: f32, base_fov: f32) -> Result<Self, String> {
        let stances = LocoStances { transitions: setup.body.stance_transition_durations(), ..Default::default() };
        let pose = setup.body.pose()?;
        // 140888f10 reads the camera joint's bind-pose Z from the body rig.
        let swimming_eye_height = pose.positions[pose.camera].z;
        let physics = PlayerPhysics::new(&setup.physics, position, yaw);
        let autocrouch = AutoCrouchSensor::new(setup.autocrouch, &physics);
        let mantle_sensor = MantleSensor::new(setup.mantle, &physics);
        let limits = setup.states.iter().find_map(|s| match &s.kind {
            Kind::Vault(v) => Some(v.limits),
            _ => None,
        });
        let limits = limits.unwrap_or_else(|| states::VaultDecl::default().limits);
        let radius = physics.posture_settings(shape::POSTURE_STANDING).radius;
        let vault_sensor = VaultSensor::new(limits, radius, physics.ground_ray_normal_average);
        let mut controller = Controller::new(setup.mapping);
        controller.slipping = setup.slipping;
        let swimming_available = !setup.swimming_states.is_empty();
        let mut swimming_decls: Vec<_> = setup.swimming_states.into_iter().map(|swimming| {
            let mut state = states::StateDecl::class_default(Kind::Swimming(swimming.clone()));
            state.navigation = swimming.navigation.clone();
            state
        }).collect();
        swimming_decls.extend(setup.swimming_equipment);
        if setup.swimming_has_mantle {
            swimming_decls.push(states::StateDecl::class_default(Kind::SwimMantle));
        }
        if let Some(dead) = setup.swimming_death {
            swimming_decls.push(states::StateDecl::class_default(Kind::Dead(dead)));
        }
        if let Some(read_note) = setup.swimming_read_note { swimming_decls.push(read_note); }
        let health = health::Health::new(setup.health, setup.accepts_water_damage, &setup.attributes);
        let mana = mana::Mana::new(setup.mana, &setup.attributes);
        let adrenaline = adrenaline::Adrenaline::new(setup.adrenaline, &setup.attributes);
        let mut machine = StateMachine::new(setup.states, &setup.rules, setup.default_state);
        if let Some(rules) = setup.slash_rules { machine.install_slash_rules(rules); }
        let camera = PlayerCamera::new(setup.camera, base_fov);
        let camera_controls = camera.controls(&pose, setup.body.follow_slope.yaw_offset);
        let camera_axis = camera.view_axis(&pose, &physics, &camera_controls);
        Ok(Self {
            physics,
            controller,
            machine,
            swimming_machine: StateMachine::new(swimming_decls, &setup.swimming_rules, setup.swimming_default_state),
            carrying: Default::default(),
            swimming_active: false,
            swimming_eye_height,
            swimming_current: swimming::WaterCurrent::default(),
            swimming_waves: swimming::WaveFilters::default(),
            swimming_wave_pose: (0.0, Vec3::Z),
            breath: setup.breath,
            combat_fx: setup.combat_fx.map(combat_fx::Runtime::new),
            chaos: chaos::Runtime::new(setup.chaos),
            incoming_damage_factor: 1.0,
            incoming_damage_back_factor: 1.0,
            health_regen_factor: 1.0,
            health_heal_factor: 1.0,
            mana_replenishes: true,
            camera,
            camera_axis,
            body_parts: Default::default(),
            stances,
            attributes: setup.attributes,
            ranged_target: setup.ranged_target,
            rune_charms: setup.rune_charms,
            heart_entity: None,
            ranged_target_hit: None,
            ranged_target_entity: None,
            heart_bark_playing:false,
            heart_requests:Default::default(),
            active_modifiers: Vec::new(),
            next_modifier_expiration: None,
            attribute_modifier_events: Vec::new(),
            attribute_requests: Vec::new(),
            entity_events: Vec::new(),
            managed_entity: None,
            body: setup.body,
            pose,
            autocrouch,
            lean_sensor: LeanSensor::new(setup.lean),
            catfall_sensor: sensors::CatfallSensor::new(Default::default()),
            mantle_sensor,
            vault_sensor,
            swimming_settings: setup.swimming_settings,
            swimming_available,
            swimming_contact: None,
            retargeting: None,
            scenes: setup.scenes,
            md6: setup.md6,
            health,
            damage_receiver: setup.damage_receiver,
            fall_damage: setup.fall_damage,
            drowning_damage_enabled: setup.drowning_damage_enabled,
            drowning_damage: setup.drowning_damage,
            mana,
            powers: Default::default(),
            equipment_changes: Default::default(),
            weapon: None,
            throwable: None,
            throwable_identity: None,
            holstered_throwables: Default::default(),
            throwable_effects: Vec::new(),
            weapon_identity: None,
            holstered_weapons: Default::default(),
            weapon_effects: Vec::new(),
            adrenaline,
            adrenaline_in_combat: false,
            bloodthirsty_active: false,
            game_over_requested: false,
            game_over_fallback: None,
            entity_event_time: 0.0,
            drowning_fx_requested: None,
            random: 0,
            arm_render: Vec::new(),
            arm_pose_errors: [None, None],
            arm_action_history: [None, None],
            arm_action_active: [false; 2],
            animation_events: Vec::new(),
            animation_sound_emitters: Vec::new(),
            prepared_animation_frame: None,
            equipment_action_handles: [None; 2],
            equipment_frames: [None; 2],
            equipment_entity_frames: Vec::new(),
            equipment_archetypes: [None; 2],
            body_settings: body::BodySettings::default(),
            requests: MotionRequests::default(),
        })
    }

    /// One simulated frame. `events` are the bound input events accumulated
    /// since the last simulated frame; `dt` is the player's clock-group delta
    /// and `frame_delta` clock group 0's.
    pub fn frame(&mut self, world: &dyn PlayerWorld, events: &[ActionEvent], settings: &LookSettings, dt: f32, frame_delta: f32, now: f64, base_fov: f32) {
        self.frame_impl(world, events, settings, dt, frame_delta, now, base_fov, None, None);
    }

    pub fn frame_with_inventory(&mut self, world: &dyn PlayerWorld, events: &[ActionEvent], settings: &LookSettings, dt: f32, frame_delta: f32, now: f64, base_fov: f32,
        inventory: &mut dyn weapons::AmmunitionInventory) {
        self.frame_impl(world, events, settings, dt, frame_delta, now, base_fov, Some(inventory), None);
    }

    pub fn frame_with_inventory_for_clock(&mut self, world: &dyn PlayerWorld, events: &[ActionEvent],
        settings: &LookSettings, dt: f32, frame_delta: f32, now: f64, base_fov: f32,
        inventory: &mut dyn weapons::AmmunitionInventory, marker: i32) {
        self.frame_impl(world, events, settings, dt, frame_delta, now, base_fov, Some(inventory), Some(marker));
    }
    /// 8ad260 advances Midnight and flushes its callbacks before the player
    /// component/state phases. Standalone callers retain the same helper.
    pub fn prepare_animation_frame(&mut self, marker: i32, dt: f32, now: f64,
        inventory: &mut dyn weapons::AmmunitionInventory) {
        if self.prepared_animation_frame == Some(marker) { return; }
        self.advance_animation_events(dt, now, &mut Some(inventory));
        self.prepared_animation_frame = Some(marker);
    }
    fn advance_animation_events(&mut self, dt: f32, now: f64,
        inventory: &mut Option<&mut dyn weapons::AmmunitionInventory>) {
        let gun_socket = self.weapon.as_ref().filter(|weapon| weapon.settings.family == weapons::RangedFamily::Gun)
            .and_then(|weapon| weapon.settings.projectile_fire_tag.as_deref()).and_then(|tag| self.equipment_tag_world(0, tag));
        if let Some(weapon) = &mut self.weapon {
            if weapon.settings.family == weapons::RangedFamily::Gun { weapon.projectile_frame = gun_socket; }
        }
        while self.next_modifier_expiration.is_some_and(|deadline| deadline <= now) {
            let next = self.active_modifiers.iter().enumerate().filter_map(|(index, value)| {
                value.expires_at.filter(|deadline| *deadline <= now).map(|deadline| (index, deadline))
            }).min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((index, _)) = next else { break };
            let value = self.active_modifiers.remove(index);
            self.refresh_modifier_deadline();
            self.attributes.modifier(&value.tag, value.increase, value.percentage, false);
            self.attribute_modifier_changed(&value.tag, false);
        }
        // The midnight manager before the component stages, then its
        // message flush (BREAK_PLAYER_STATE quits a breakable state).
        let cast = body::cast(&self.body.animations, &self.body.arms, &self.body.equipment);
        match self.body.midnight.advance(dt, cast, &mut self.random) {
            Ok(messages) => {
                for (event, handle) in messages {
                    // 854fd0 subscribes the action owner to numeric0xf6;
                    // its registered Midnight spelling is143df1790.
                    if event == "ARK_MDNT_MSG_TRIGGER_PLAYER_STATE_ACTION" {
                        self.power_action_message(handle);
                        self.heart_action_message(handle);
                    }
                    self.machine.handle_message(&event, handle);
                    self.swimming_machine.handle_message(&event, handle);

                }
            }
            Err(error) => eprintln!("Midnight playback: {error}"),
        }
        for receipt in self.body.midnight.drain_animation_sounds() {
            use crate::gameplay::sound::{Emitter, Request};
            let emitter = if let Some(joint) = receipt.sound.joint {
                let entity = match receipt.emitter { Emitter::Equipment { entity, .. } => Some(entity), _ => None };
                Emitter::PlayerJoint { entity, part: receipt.emitter.player_part(), joint }
            } else { receipt.emitter };
            if matches!(emitter, Emitter::PlayerJoint { .. }) && !self.animation_sound_emitters.contains(&emitter) {
                self.animation_sound_emitters.push(emitter);
            }
            self.body.sound_requests.push(Request::Post { event: receipt.sound.event.id, emitter });
        }
        if let (Some(weapon), Some(inventory)) = (&mut self.weapon, inventory.as_deref_mut()) {
            weapon.update_inventory(inventory.quantity(&weapon.ammunition().item));
        }
        // 8ad260 flushes Midnight messages before component/state updates.
        // Spending a final-frame key after state exit would lose the hit.
        for event in self.body.midnight.drain_point_events() {
            if let (Some(weapon), Some(inventory)) = (&mut self.weapon, inventory.as_deref_mut()) {
                if let Err(error) = weapon.handle_event(&event, inventory, &mut self.random) { eprintln!("Weapon animation event: {error}"); }
            }
            self.animation_events.push(event);
        }
    }

    fn frame_impl(&mut self, world: &dyn PlayerWorld, events: &[ActionEvent], settings: &LookSettings, dt: f32, frame_delta: f32, now: f64, base_fov: f32,
        mut inventory: Option<&mut dyn weapons::AmmunitionInventory>, marker: Option<i32>) {
        let prepared = self.prepared_animation_frame.take();
        if marker.is_none() || prepared != marker {
            self.advance_animation_events(dt, now, &mut inventory);
        }
        if !self.health.is_dead() { self.controller.process_input(events, settings); }
        // QUERY_PHYSICS, then the DEFAULT stage: sensor and physics results
        // before the controller (it depends on both), then the states.
        self.physics.update_queries(world);
        let sliding = self.machine.is_active(StateId::SLIDING);
        self.autocrouch.update(world, &self.physics, self.requests.linear_priority0, dt, sliding);
        self.lean_sensor.update(world, self.camera.eye, self.physics.position(), self.physics.axis());
        let mantle_surface = self.swimming_contact.filter(|water|
            self.swimming_active && self.camera.eye.z > water.head).map(|water| water.plane);
        self.mantle_sensor.update(world, &self.physics, mantle_surface);
        self.vault_sensor.update(world, &self.physics, dt);
        self.catfall_sensor.update(world, &self.physics);
        if !self.health.is_dead() { self.controller.think(settings, frame_delta); }
        self.requests = MotionRequests::default();
        let mut origin = self.physics.position();
        let axis = self.physics.axis();
        let velocity = self.physics.velocity;
        let underwater = self.is_underwater();
        let mut swim_mantle_exit = false;
        if let Some(water) = self.swimming_contact {
            let (height, normal) = self.swimming_settings.wave_target(
                water.plane, water.ocean_level.unwrap_or(water.plane),
                origin.z + self.swimming_eye_height, water.samples.as_ref(),
            );
            self.swimming_wave_pose = self.swimming_waves.update(height, normal, dt);
        } else {
            self.swimming_waves = swimming::WaveFilters::default();
            self.swimming_wave_pose = (0.0, Vec3::Z);
        }
        let equipment_transitioning = std::array::from_fn(|side| self.equipment_changes.transitioning(side)
            || self.equipment_action_handles[side].is_some_and(|handle| !self.body.midnight.finished(handle)));
        let camera_axis = self.camera_axis;
        macro_rules! frame {
            () => {
                Frame {
                    dt,
                    world,
                    controller: &mut self.controller,
                    physics: &mut self.physics,
                    body: &mut self.stances,
                    footstep: &mut self.body.footstep,
                    footstep_clock: &mut self.body.footsteps,
                    contact_requests: &mut self.body.contact_requests,
                    sound_requests: &mut self.body.sound_requests,
                    body_speed: self.body.speed,
                    body_additives: &self.body.additives,
                    body_additive_command: &mut self.body.additive_command,
                    camera: &mut self.camera,
                    camera_axis,
                    body_parts: &mut self.body_parts,
                    follow_slope: &mut self.body.follow_slope,
                    requests: &mut self.requests,
                    autocrouch: &mut self.autocrouch,
                    origin,
                    axis,
                    velocity,
                    attributes: &self.attributes,
                    attribute_requests: &mut self.attribute_requests,
                    powers: &mut self.powers,
                    equipment_changes: &mut self.equipment_changes,
                    carrying: &mut self.carrying,
                    mana: &mut self.mana,
                    mana_replenishes: self.mana_replenishes,
                    weapon: &mut self.weapon,
                    throwable: &mut self.throwable,
                    holstered_throwables: &mut self.holstered_throwables,
                    equipment_transitioning,
                    equipment_archetypes: self.equipment_archetypes,
                    action_time: self.entity_event_time,
                    sprint_toggle: settings.sprint_toggle,
                    look_stick_sensitivity: settings.look_stick_sensitivity,
                    lean: &mut self.body.lean,
                    lean_sensor: &mut self.lean_sensor,
                    catfall_sensor: &mut self.catfall_sensor,
                    arms_shown: &mut self.body.arms_shown,
                    desired_legs: &mut self.body.desired_legs,
                    jump_anim: &mut self.body.jump,
                    midnight: &mut self.body.midnight,
                    scenes: &self.scenes,
                    md6: body::cast_md6(&self.md6, &self.body.arms, &self.body.equipment),
                    animations: body::cast(&self.body.animations, &self.body.arms, &self.body.equipment),
                    mantle_sensor: &mut self.mantle_sensor,
                    vault_sensor: &mut self.vault_sensor,
                    retargeting: &mut self.retargeting,
                    health: self.health.maximum(&self.attributes),
                    underwater,
                    swim_mantle_exit: &mut swim_mantle_exit,
                    game_over_requested: &mut self.game_over_requested,
                    drowning_fx_requested: &mut self.drowning_fx_requested,
                    random: &mut self.random,
                    heart_entity:self.heart_entity,
                    heart_target:self.ranged_target_entity,
                    heart_bark_playing:self.heart_bark_playing,
                    heart_requests:&mut self.heart_requests,
                    small_fall_cooldown: self.body_settings.small_fall_cooldown,
                    lean_toggle: settings.lean_toggle,
                    // The host reads keyboard and mouse only.
                    keyboard: true,
                }
            };
        }
        let in_water = if self.health.is_dead() { self.swimming_active } else { (!self.machine.is_active(StateId::MANTLE) || self.swimming_active) && self.swimming_contact.is_some_and(|water| {
            let depth = water.plane - origin.z + self.physics.ground_ray_distance_min;
            let head_underwater = self.camera.eye.z <= water.head;
            let surface = water.plane + self.swimming_settings.distance_above_surface;
            if self.swimming_active {
                let has_ground = self.physics.ground_distance < f32::MAX
                    || self.physics.ground.state != SupportState::Unsupported;
                // 889f10 raises the exit comparison by a negative ground
                // distance while jump is held and the head is above water.
                let ground = self.physics.ground_distance;
                let jump_ground = if !head_underwater && ground < 0.0
                    && self.controller.down(controller::ACTION_JUMP) { -ground } else { 0.0 };
                let above_surface = !head_underwater && surface + 0.1 < origin.z + self.swimming_eye_height + jump_ground;
                let too_shallow = depth < self.swimming_settings.minimum_depth;
                !(has_ground && (above_surface || too_shallow)
                    && self.controller.slipping.slope_cos < self.physics.shortest_ground_ray_normal().z)
            } else {
                depth >= self.swimming_settings.minimum_depth
                    && (head_underwater || origin.z + self.swimming_eye_height <= surface)
            }
        }) };
        if in_water != self.swimming_active && self.swimming_available {
            if !in_water {
                // 88a2e0: before restoring the previous mode, keep the
                // standing shape's step hull above the sampled ground.
                let step = self.physics.posture_settings(shape::POSTURE_STANDING).step_height;
                let ground = self.physics.ground_distance;
                if ground < -step {
                    self.physics.proxy.position.z -= ground + step;
                    origin = self.physics.position();
                }
            }
            self.machine.leave_mode(&mut frame!());
            std::mem::swap(&mut self.machine, &mut self.swimming_machine);
            self.swimming_active = in_water;
            if in_water {
                self.swimming_current = swimming::WaterCurrent::default();
                self.breath.reset_for_water_entry();
                let swim_height = self.physics.posture_settings(shape::POSTURE_SWIMMING).height;
                let old_height = self.physics.posture_settings(self.physics.posture).height;
                self.physics.proxy.position.z -= swim_height - old_height;
                self.physics.set_posture(shape::POSTURE_SWIMMING, 0.0);
                origin = self.physics.position();
            }
        }
        if !self.swimming_active {
            if !self.health.is_dead() { self.machine.controller_support(&mut frame!()); }
        } else if !self.health.is_dead() {
            if let Some(water) = self.swimming_contact {
                let current = self.swimming_current.update(water.current, dt, &self.swimming_settings);
                self.physics.set_water_motion(current,
                    Some(water.plane + self.swimming_settings.distance_above_surface - self.swimming_eye_height));
            }
        }
        if !self.health.is_dead() { self.controller.apply_mapping(&mut self.machine); }
        self.machine.think(&mut frame!());
        self.apply_equipment_changes();
        self.machine.advance_equipment_changes(&mut frame!());
        let mut attribute_requests = std::mem::take(&mut self.attribute_requests);
        for request in attribute_requests.drain(..) {
            self.apply_attribute_modifier(&request.owner, request.index, &request.tag, request.increase, request.percentage,
                request.duration, request.added, now);
        }
        self.attribute_requests = attribute_requests;
        if let (Some(weapon), Some(inventory)) = (&mut self.weapon, inventory.as_deref_mut()) {
            weapon.flush_pending_fire(inventory, &mut self.random);
        }
        self.update_power_camera_modifier();
        // 1407f8e70 posts the player's forceGameOver event after 7 seconds.
        // Its handler 140784590 requests game-over when the player is dead.
        if self.health.is_dead()
            && self.game_over_fallback.is_some_and(|deadline| deadline <= self.entity_event_time) {
            self.game_over_requested = true;
            self.game_over_fallback = None;
        }
        if self.swimming_active && swim_mantle_exit {
            self.machine.leave_mode(&mut frame!());
            std::mem::swap(&mut self.machine, &mut self.swimming_machine);
            self.swimming_active = false;
            self.machine.request(StateId::MANTLE);
            self.machine.think(&mut frame!());
        }
        if self.swimming_active && self.is_underwater() {
            let cost = self.attributes.get("ARK_ATTRIBUTE_SWIMMING_BREATH_COST_PER_SECOND");
            if cost != 0.0 { self.breath.consume(dt * cost); }
        }
        if let Some(weapon) = &mut self.weapon {
            weapon.think(self.machine.ranged_is_charging());
            weapon.update_aim(&self.attributes, dt);
        }
        //633990 ->63bfc0 requests the real ReloadWeapon action when empty;
        //it never refills a cancelled/unequipped grenade directly.
        if self.throwable.as_ref().is_some_and(|weapon|weapon.magazine==0&&weapon.total_ammo>0){
            self.machine.request(StateId::RELOAD_WEAPON);
        }
        let mut equipment_events = std::mem::take(&mut self.powers.grapple_equipment_events);
        for event in equipment_events.drain(..) {
            self.set_owned_equipment_hidden(matches!(event, grapple_state::EquipmentEvent::Stick));
        }
        self.powers.grapple_equipment_events = equipment_events;
        self.breath.update(dt);
        self.mana.update(dt, &self.attributes, self.mana_replenishes);
        self.adrenaline.update(dt, &self.attributes, self.adrenaline_in_combat);
        self.health.update_with_healing(dt, &self.attributes, self.health_regen_factor, self.health_heal_factor);
        if let Some(effect) = &mut self.combat_fx {
            //81fda0 reads actual Slash+d1, suppressed by state4c.
            let focused = !self.machine.is_active(StateId(0x4c)) && self.machine.melee_focus_active();
            if let Err(error) = effect.advance(focused, dt) { eprintln!("Player combat screen effect: {error}"); }
        }
        // 1404c0cf0 spends the mana-first share before dispatching the
        // remaining authored water damage to health.
        if self.is_underwater() {
            let adrenaline_gain = dt * self.attributes.get("ARK_ATTRIBUTE_PLAYER_ADRENALINE_GAIN_UNDERWATER");
            self.adrenaline.gain(adrenaline_gain, &self.attributes, self.bloodthirsty_active);
            let mana_factor = self.attributes.get("ARK_ATTRIBUTE_DROWNING_DAMAGES_MANA_FIRST_FACTOR");
            {
                let mut mana_delta = dt * self.attributes.get("ARK_ATTRIBUTE_PLAYER_MANA_GAIN_UNDERWATER");
                let mut raw_damage = if self.breath.remaining <= 0.0 && self.drowning_damage_enabled {
                    dt * self.attributes.get("ARK_ATTRIBUTE_DROWNING_DAMAGE_PER_SECOND")
                } else { 0.0 };
                if mana_factor > 0.0 && raw_damage > 0.0 {
                    mana_delta -= raw_damage * mana_factor;
                    if -mana_delta < self.mana.current() {
                        raw_damage = 0.0;
                    } else {
                        mana_delta = -self.mana.current();
                        raw_damage -= self.mana.current() / mana_factor;
                    }
                }
                if mana_delta < 0.0 { self.mana.consume(-mana_delta, &self.attributes, self.mana_replenishes); }
                if mana_delta > 0.0 { self.mana.gain(mana_delta, &self.attributes); }
                if !self.health.accepts_water() { raw_damage = 0.0; }
                if raw_damage > 0.0 {
                    if let Some(declaration) = &self.drowning_damage {
                        // 4c0cf0 -> 780a80 retains authored type/bounds and
                        // supplies the actual player as attacker and inflictor.
                        let mut parameters = crate::gameplay::damage::Parameters::from_declaration(declaration);
                        parameters.set_amount(declaration, raw_damage, true);
                        parameters.attacker = self.managed_entity;
                        parameters.inflictor = self.managed_entity;
                        parameters.hit_location = Some(self.physics.position());
                        parameters.hit_direction = -Vec3::Z;
                        parameters.affected_regions = 0;
                        self.receive_damage(&parameters);
                    }
                }
            }
        }
        if let Some(amount) = self.machine.take_fall_damage() {
            if let Some(declaration) = &self.fall_damage {
                let mut parameters = crate::gameplay::damage::Parameters::from_declaration(declaration);
                parameters.set_amount(declaration, amount, true);
                parameters.attacker = self.managed_entity;
                // 8658c0 writes the authored source UP direction at +3c.
                parameters.hit_direction = Vec3::Z;
                if amount == f32::MAX {
                    // 8658c0 calls 780ae0: preserve authored FALL and add
                    // INSTANTKILL for the fatal-height sentinel.
                    parameters.damage_type.0 |= crate::gameplay::damage::DamageType::INSTANT_KILL.0;
                }
                self.receive_damage(&parameters);
            }
        }
        // Camera DEFAULT (FUN_1407f6140): modes, look integration, FOV.
        if dt != 0.0 {
            let yaw_offset = &mut self.body.follow_slope.yaw_offset;
            let yaw_rate = self.physics.angular_velocity;
            self.camera.update(dt, self.controller.input_look, self.controller.look_is_analog, &mut self.physics, yaw_offset, yaw_rate);
            let speed_state = self.machine.is_active(StateId::SPRINTING)
                || self.machine.from_sprinting(StateId::JUMPING)
                || self.machine.from_sprinting(StateId::FALLING);
            self.camera.update_fov(dt, base_fov, speed_state);
        }
        // Body DEFAULT (FUN_140777280).
        //62b990 switches the crossbow's own default input on magazine changes;
        //its loaded child's visibility remains a separate actual entity flag.
        if let Some(alias) = self.weapon.as_ref().and_then(weapons::Runtime::default_animation) {
            if let Some(tree) = self.body.equipment[0].as_mut() {
                if let Err(error) = tree.set_default_alias(alias) { eprintln!("Weapon default animation: {error}"); }
            }
        }
        self.body.set_grapple_control(self.camera.grapple_control());
        let input_velocity = self.physics.input_velocity();
        let supported = self.controller.support_state != SupportState::Unsupported;
        let ground_normal = self.physics.ground_ray_normal_average;
        self.body.update(dt, &self.stances, input_velocity, self.physics.axis(), self.body_settings, ground_normal, supported);
        self.body.update_footsteps(world, &self.physics, &self.attributes, dt, now);
        self.physics.lean_offset = self.body.apply_lean(dt, &mut self.lean_sensor, self.physics.axis());
        // ANIMATION (arkCpntAnimEvolved FUN_14054f500): the midnight root
        // motion as priority-1 velocities.
        if let (Some((delta, yaw)), true) = (self.body.midnight.root, dt != 0.0) {
            let axis = self.physics.axis();
            self.requests.push_linear(1, axis * delta / dt);
            self.requests.push_angular(1, Vec3::new(0.0, 0.0, yaw * 0.017453292 / dt));
        }
        self.retarget(dt);
        if dt != 0.0 {
            let input = MotionInput { linear: self.requests.linear, angular: self.requests.angular };
            self.physics.step(world, dt, input);
        }
        if self.machine.is_active(StateId::GRAPPLE) {
            if let Some(destination) = self.grapple_state().and_then(|run| run.beam_destination()) {
                if let Err(error) = self.retarget_grapple_beam(destination) { eprintln!("Grapple beam retarget: {error}"); }
            }
        }
        self.update_grapple_beam(dt);
    }

    /// FUN_1407583e0 for the player slave of the mantle dummy (orientation
    /// mode 0, no tag offset, a static dummy): priority-3 velocities that
    /// blend the player from its start onto the anchor's target.
    fn retarget(&mut self, dt: f32) {
        let Some(mut r) = self.retargeting else { return };
        if dt == 0.0 {
            return;
        }
        let t = if r.duration > 0.0 && r.timer / r.duration <= 1.0 { r.timer / r.duration } else { 1.0 };
        let master = Mat3::from_rotation_z(r.target_yaw);
        // The slave joint relative to joint 0 with the origin translation
        // removed (FUN_14024b280), in the slave's current anim.
        let mut joint = (Vec3::ZERO, Mat3::IDENTITY);
        if r.anchor {
            let current = self.body.midnight.main_clip();
            if let Some((clip, _)) = current {
                r.last_clip = Some(clip);
            }
            let sampled = current.or_else(|| r.last_clip.map(|c| (c, (self.body.animations[c].frame_count as f32 - 1.0).max(0.0))));
            if let (Some((clip, frame)), Some(anchor)) = (sampled, self.body.skeleton.anchor) {
                if let Some(local) = self.body.anchor_transform(clip, frame, anchor) {
                    joint = local;
                }
            }
        }
        let target_axis = joint.1.transpose() * master;
        let target = r.target - target_axis * joint.0;
        let snap = |v: Vec3| Vec3::from_array(v.to_array().map(|c| if c.abs() <= 1e-18 { 0.0 } else { c }));
        let p = snap(r.start) * (1.0 - t) + snap(target) * t;
        self.requests.push_linear(3, (p - self.physics.position()) / dt);
        if r.orientation {
            let target_yaw = target_axis.x_axis.y.atan2(target_axis.x_axis.x);
            let yaw = r.start_yaw + physics::wrap_angle(target_yaw - r.start_yaw) * t;
            let rate = physics::wrap_angle(yaw - self.physics.yaw) / dt;
            self.requests.push_angular(3, Vec3::new(0.0, 0.0, rate));
        } else {
            // The existing angular request, re-pushed.
            let current = self.requests.angular.unwrap_or(Vec3::ZERO);
            self.requests.push_angular(3, Vec3::new(0.0, 0.0, current.z));
        }
        r.timer += dt;
        self.retargeting = Some(r);
    }

    /// PRE_RENDER view placement.

    pub fn view(&mut self) -> View {
        // A pose that fails to sample keeps the previous one (sampling of
        // every clip is validated when the body loads).
        let sampled = if let Ok(pose) = self.body.pose() {
            self.pose = pose;
            true
        } else { false };
        if sampled && self.swimming_active {
            // 140846450 applies the wave height at the root and rotates the
            // arms joint to the smoothed water normal before camera controls.
            let (height, normal) = self.swimming_wave_pose;
            for position in &mut self.pose.positions { position.z += height; }
            let axis = self.physics.axis();
            let left = normal.cross(axis.x_axis).normalize_or_zero();
            let forward = left.cross(normal).normalize_or_zero();
            let rotation = axis.transpose() * Mat3::from_cols(forward, left, normal);
            let joint = self.pose.controls[2];
            let pivot = self.pose.positions[joint];
            for i in 0..self.pose.positions.len() {
                if self.pose.descends_from(i, joint) {
                    self.pose.positions[i] = pivot + rotation * (self.pose.positions[i] - pivot);
                    self.pose.rotations[i] = rotation * self.pose.rotations[i];
                }
            }
        }
        let allow = self.machine.allow_camera_out_of_shape();
        let body_yaw = self.body.follow_slope.yaw_offset;
        let body_controls = self.camera.controls(&self.pose, body_yaw);
        self.update_arms(body_yaw, &body_controls);
        let view = self.camera.view(&self.pose, &self.physics, allow, &body_controls);
        self.camera_axis = view.axis;
        for (slot, id) in [StateId::MANTLE, StateId::CATFALL].into_iter().enumerate() {
            let active = self.machine.is_active(id);
            if active {
                if !self.arm_action_active[slot] {
                    self.arm_action_history[slot] = Some(ArmActionDiagnostic {
                        selected_hands: if id == StateId::MANTLE { self.mantle_sensor.last_selected_hands } else { self.catfall_sensor.last_selected_hands },
                        scenes: Vec::new(),
                        scene_start_error: self.body.midnight.last_start_error.clone(),
                        frames: 0, tree_drove: [false; 3], published: [false; 2], visible: [false; 2],
                        ever_in_front: [false; 2], camera_slopes: [[f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY]; 2], pose_error: [false; 2],
                        tree_frames: [0; 3], visible_frames: [0; 2], in_front_frames: [0; 2],
                    });
                }
                if let Some(history) = &mut self.arm_action_history[slot] {
                    self.body.midnight.capture_action_scenes(&self.scenes, &mut history.scenes);
                    if history.scene_start_error.is_none() {
                        history.scene_start_error = self.body.midnight.last_start_error.clone();
                    }
                    history.frames = history.frames.saturating_add(1);
                    for target in 0..3 {
                        let driven = self.body.midnight.drives(target);
                        history.tree_drove[target] |= driven;
                        history.tree_frames[target] = history.tree_frames[target].saturating_add(u32::from(driven));
                    }
                    for side in 0..2 {
                        history.pose_error[side] |= self.arm_pose_errors[side].is_some();
                        if let Some(arm) = self.body.arms.get(side).and_then(|source| self.arm_render.iter().find(|arm| arm.model == source.model)) {
                            history.published[side] = true;
                            history.visible[side] |= arm.visible;
                            history.visible_frames[side] = history.visible_frames[side].saturating_add(u32::from(arm.visible));
                            let mut in_front = false;
                            for joint in &arm.joints {
                                let p = Vec3::new(joint[0][3], joint[1][3], joint[2][3]);
                                let relative = p - view.origin;
                                let forward = relative.dot(view.axis.x_axis);
                                if forward > 0.0 {
                                    history.ever_in_front[side] = true;
                                    in_front = true;
                                    let horizontal = relative.dot(view.axis.y_axis) / forward;
                                    let vertical = relative.dot(view.axis.z_axis) / forward;
                                    let bounds = &mut history.camera_slopes[side];
                                    bounds[0] = bounds[0].min(horizontal); bounds[1] = bounds[1].max(horizontal);
                                    bounds[2] = bounds[2].min(vertical); bounds[3] = bounds[3].max(vertical);
                                }
                            }
                            history.in_front_frames[side] = history.in_front_frames[side].saturating_add(u32::from(in_front));
                        }
                    }
                }
            }
            self.arm_action_active[slot] = active;
        }
        view
    }

    /// The arm entities' joints: each arm bound to its body tag (spine03 of
    /// the body pose after the camera's joint controls, FUN_1407506d0), its
    /// tree pose, then the camera's shoulder controls (FUN_14053bcd0: world
    /// rotations about the shoulder, children carried).
    fn update_arms(&mut self, body_yaw: f32, body_controls: &[camera::JointControl]) {
        let shoulder_controls = self.camera.arm_controls(body_yaw);
        let (position, axis) = (self.physics.position(), self.physics.axis());
        let mut render = Vec::with_capacity(self.body.arms.len() + 2 + usize::from(self.body.legs_visible));
        let poses: Vec<_> = {
            let Body { arms, midnight, .. } = &mut self.body;
            arms.iter().enumerate().map(|(side, arm)| arm.pose(midnight, side + 1)).collect()
        };
        for ((side, arm), joints) in self.body.arms.iter().enumerate().zip(poses) {
            let joints = match joints {
                Ok(joints) => {
                    self.arm_pose_errors[side.min(1)] = None;
                    joints
                }
                Err(error) => {
                    let previous = &mut self.arm_pose_errors[side.min(1)];
                    if previous.as_deref() != Some(error.as_str()) {
                        eprintln!("Player arm {} pose: {error}", arm.model);
                    }
                    *previous = Some(error);
                    continue;
                }
            };
            let (frame_p, frame_r) = self.arm_frame(arm, body_controls);
            let shoulder = arm.skeleton.shoulder;
            let pivot = frame_p + frame_r * joints[shoulder].0;
            let rows = joints
                .iter()
                .enumerate()
                .map(|(j, &(p, r))| {
                    let (mut p, mut r) = (frame_p + frame_r * p, frame_r * r);
                    if arm.descends_from(j, shoulder) {
                        for c in &shoulder_controls[side.min(1)] {
                            p = pivot + *c * (p - pivot);
                            r = *c * r;
                        }
                    }
                    let (p, r) = (position + axis * p, axis * r);
                    std::array::from_fn(|i| [r.x_axis[i], r.y_axis[i], r.z_axis[i], p[i]])
                })
                .collect();
            render.push(ArmRender { model: arm.model.clone(), managed_entity: None, joints: rows, visible: arm.visible && self.body.arms_shown[side.min(1)] });
        }
        // Equipment binds to the final controlled owner pose. In particular,
        // arm shoulder controls have already carried the hand's attachment tag;
        // applying those controls again would rotate a held weapon twice.
        self.equipment_frames = [None; 2];
        self.equipment_entity_frames.clear();
        let equipment_poses: [_; 2] = {
            let Body { equipment, midnight, .. } = &mut self.body;
            std::array::from_fn(|side| equipment[side].as_ref().map(|tree| tree.pose(midnight, side + 3)))
        };
        for (side, pose) in equipment_poses.into_iter().enumerate() {
            let Some(tree) = &self.body.equipment[side] else { continue };
            let Some((p, r)) = self.equipment_root_frame(tree, body_controls, &render) else { continue };
            self.equipment_frames[side] = Some((p, r));
            if let Some(entity) = tree.managed_entity {
                self.equipment_entity_frames.push((crate::gameplay::sound::Emitter::Equipment { entity, part: 1 << (side + 3) }, p, r, tree.sound_settings));
            }
            let Some(Ok(pose)) = pose else { continue };
            let joints = pose.into_iter().map(|(local_p, local_r)| {
                let (p, r) = (p + r * local_p, r * local_r);
                std::array::from_fn(|i| [r.x_axis[i], r.y_axis[i], r.z_axis[i], p[i]])
            }).collect();
            render.push(ArmRender { model: tree.model.clone(), managed_entity: tree.managed_entity, joints,
                visible: tree.visible && self.body.arms_shown[side] });
        }
        for target in self.equipment_changes.inactive_targets() {
            let Some(tree) = &target.tree else { continue };
            let Some(part) = (match target.equipment.slot { 1 => Some(8), 0 => Some(16), _ => None }) else { continue };
            let Some((p, r)) = self.equipment_root_frame(tree, body_controls, &render) else { continue };
            if let Some(entity) = tree.managed_entity {
                self.equipment_entity_frames.push((crate::gameplay::sound::Emitter::Equipment { entity, part }, p, r, tree.sound_settings));
            }
            // 61b010 hides only equipment whose declaration requests it.
            // The stopped entity keeps its own base pose; another item now
            // using Midnight target3/4 must not write into this skeleton.
            let Ok(pose) = tree.pose_base() else { continue };
            let joints = pose.into_iter().map(|(local_p, local_r)| {
                let (p, r) = (p + r * local_p, r * local_r);
                std::array::from_fn(|i| [r.x_axis[i], r.y_axis[i], r.z_axis[i], p[i]])
            }).collect();
            render.push(ArmRender { model: tree.model.clone(), managed_entity: tree.managed_entity, joints, visible: tree.visible });
        }
        // The body entity's own model: hidden-legs materials (nothing drawn)
        // unless the legs are visible (FUN_140776e60).
        if self.body.legs_visible {
            let joints = (0..self.pose.positions.len())
                .map(|j| {
                    let (p, r) = self.pose.joint_with_controls(j, body_controls);
                    let (p, r) = (position + axis * p, axis * r);
                    std::array::from_fn(|i| [r.x_axis[i], r.y_axis[i], r.z_axis[i], p[i]])
                })
                .collect();
            render.push(ArmRender { model: self.md6.clone(), managed_entity: None, joints, visible: true });
        }
        self.arm_render = render;
    }

    fn arm_frame(&self, arm: &arms::Arm, controls: &[camera::JointControl]) -> (Vec3, Mat3) {
        let (tag_p, tag_r) = self.pose.joint_with_controls(arm.bind_joint, controls);
        (tag_p + tag_r * arm.bind.0, tag_r * arm.bind.1)
    }

    /// The player's entity and model-tag frames, before render-camera collision
    /// adjustments. Uses the body pose/control matrices shared with its arms.
    pub fn reference_tag_binding(&self, tag: &str) -> Result<(i16, [f32; 3], [f32; 4]), String> {
        if tag.is_empty() { return Ok((-1, [0.0; 3], [0.0, 0.0, 0.0, 1.0])); }
        let (joint, offset, orientation) = if let Some(&(joint, offset, orientation)) = self.body.tags.get(tag) {
            (joint, offset, orientation)
        } else {
            let joint = self.body.skeleton.joint_names.iter().position(|name| name == tag)
                .ok_or_else(|| format!("player body has no tag or joint {tag}"))?;
            (joint, Vec3::ZERO, Mat3::IDENTITY)
        };
        Ok((i16::try_from(joint).map_err(|_| "player joint exceeds reference-frame index")?,
            offset.to_array(), bevy_math::Quat::from_mat3(&orientation).to_array()))
    }
    pub fn reference_local_joint(&self, joint: i16) -> Result<crate::gameplay::reference_frame::ReferenceFrame, String> {
        if joint == -1 { return Ok(Default::default()); }
        let index = usize::try_from(joint).map_err(|_| "invalid player reference joint")?;
        if index >= self.pose.positions.len() { return Err("player reference joint is outside its actual skeleton".into()); }
        let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
        let (position, rotation) = self.pose.joint_with_controls(index, &controls);
        Ok(crate::gameplay::reference_frame::ReferenceFrame { position: position.to_array(), basis: rotation.to_cols_array_2d() })
    }
    pub fn reference_frames(&self) -> (crate::gameplay::reference_frame::ReferenceFrame,
        hashbrown::HashMap<String, crate::gameplay::reference_frame::ReferenceFrame>) {
        use crate::gameplay::reference_frame::ReferenceFrame;
        let origin = self.physics.position();
        let axis = self.physics.axis();
        let root = ReferenceFrame { position: origin.to_array(), basis: axis.to_cols_array_2d() };
        let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
        let mut frames = hashbrown::HashMap::new();
        let joint_frame = |joint| {
            let (position, rotation) = self.pose.joint_with_controls(joint, &controls);
            (position, rotation)
        };
        let world_frame = |position: Vec3, rotation: Mat3| ReferenceFrame {
            position: (origin + axis * position).to_array(),
            basis: (axis * rotation).to_cols_array_2d(),
        };
        for (joint, name) in self.body.skeleton.joint_names.iter().enumerate() {
            let (position, rotation) = joint_frame(joint);
            frames.insert(name.clone(), world_frame(position, rotation));
        }
        // Tag lookup precedes bare joint lookup in 4b19b0.
        for (name, &(joint, offset, orientation)) in &self.body.tags {
            let (position, rotation) = joint_frame(joint);
            frames.insert(name.clone(), world_frame(position + rotation * offset, rotation * orientation));
        }
        (root, frames)
    }

    /// The bound entity frame precedes the model's local animation pose. This
    /// is shared by rendering and audio/entity publication, not inferred from
    /// a model's first joint or the camera.
    fn equipment_root_frame(&self, tree: &equipment::EquipmentTree, controls: &[camera::JointControl], render: &[ArmRender]) -> Option<(Vec3, Mat3)> {
        let (p, r) = match tree.binding {
            equipment::EquipmentBinding::BodyEntity => (self.physics.position(), self.physics.axis()),
            equipment::EquipmentBinding::ArmEntity(owner) => {
                let (p, r) = self.arm_frame(self.body.arms.get(owner)?, controls);
                (self.physics.position() + self.physics.axis() * p, self.physics.axis() * r)
            }
            equipment::EquipmentBinding::Body => {
                if tree.bind_joint >= self.pose.positions.len() { return None; }
                let (p, r) = self.pose.joint_with_controls(tree.bind_joint, controls);
                (self.physics.position() + self.physics.axis() * p, self.physics.axis() * r)
            }
            equipment::EquipmentBinding::Arm(owner) => {
                let arm = self.body.arms.get(owner)?;
                let rows = render.iter().find(|entry| entry.managed_entity.is_none() && entry.model == arm.model)?.joints.get(tree.bind_joint)?;
                (Vec3::new(rows[0][3], rows[1][3], rows[2][3]), Mat3::from_cols(
                    Vec3::new(rows[0][0], rows[1][0], rows[2][0]), Vec3::new(rows[0][1], rows[1][1], rows[2][1]), Vec3::new(rows[0][2], rows[1][2], rows[2][2])))
            }
        };
        Some((p + r * tree.bind.0, r * tree.bind.1))
    }

    /// Publish every actual owned equipment audio component from its entity
    /// binding. Inactive actors remain bound to their authored body tag.
    pub fn visit_equipment_audio_frames(&self, mut visit: impl FnMut(crate::gameplay::sound::Emitter, Vec3, Mat3, crate::gameplay::sound::component::Settings)) {
        for &(emitter, p, r, settings) in &self.equipment_entity_frames {
            if let Some(settings) = settings { visit(emitter, p, r, settings); }
        }
    }

    /// Entity publication also includes actors without an audio component.
    pub fn visit_equipment_entity_frames(&self, mut visit: impl FnMut(crate::gameplay::entity_lifetime::EntityId, Vec3, Mat3)) {
        for &(emitter, p, r, _) in &self.equipment_entity_frames {
            if let crate::gameplay::sound::Emitter::Equipment { entity, .. } = emitter { visit(entity, p, r); }
        }
    }

    pub fn sound_rate(&self, emitter: crate::gameplay::sound::Emitter, clock_rate: f32) -> Option<f32> {
        use crate::gameplay::sound::Emitter;
        let emitter = emitter.player_owner();
        let settings = match emitter {
            Emitter::PlayerBody => self.body.sound_settings,
            Emitter::PlayerLeftArm => self.body.arms.first()?.sound_settings?,
            Emitter::PlayerRightArm => self.body.arms.get(1)?.sound_settings?,
            Emitter::PlayerLeftEquipment => self.body.equipment[0].as_ref()?.sound_settings?,
            Emitter::PlayerRightEquipment => self.body.equipment[1].as_ref()?.sound_settings?,
            Emitter::Equipment { entity, .. } => self.body.equipment.iter().flatten()
                .chain(self.equipment_changes.inactive_targets().filter_map(|target| target.tree.as_ref()))
                .find(|tree| tree.managed_entity == Some(entity))?.sound_settings?,
            Emitter::Menu | Emitter::Effect(_) | Emitter::Component { .. } | Emitter::PlayerJoint { .. } => return None,
        };
        Some(settings.rate(clock_rate))
    }

    pub fn visit_animation_sound_emitters(&self, mut visit: impl FnMut(crate::gameplay::sound::Emitter)) {
        for &emitter in &self.animation_sound_emitters {
            if self.sound_transform(emitter).is_some() { visit(emitter); }
        }
    }

    pub fn forget_animation_sound_owner(&mut self, owner: crate::gameplay::sound::Emitter) {
        self.animation_sound_emitters.retain(|emitter| emitter.player_owner() != owner);
    }

    /// 53dac0 -> 53d800 -> 53d3f0 reads the animation component's retained
    /// palette and the entity transform. Reuse the same published world joints
    /// here; socket queries never decode or pose an additional model instance.
    pub fn equipment_tag_world(&self, side: usize, tag: &str) -> Option<(Vec3, Mat3)> {
        let tree = self.body.equipment.get(side)?.as_ref()?;
        let (joint, local_position, local_rotation) = tree.rig.tag(tag).ok()?;
        let rows = self.arm_render.iter().find(|entry| entry.managed_entity == tree.managed_entity && entry.model == tree.model)?.joints.get(joint)?;
        let position = Vec3::new(rows[0][3], rows[1][3], rows[2][3]);
        let rotation = Mat3::from_cols(Vec3::new(rows[0][0], rows[1][0], rows[2][0]),
            Vec3::new(rows[0][1], rows[1][1], rows[2][1]), Vec3::new(rows[0][2], rows[1][2], rows[2][2]));
        Some((position + rotation * local_position, rotation * local_rotation))
    }

    /// 752900's bone-only binding fallback reads the same retained world
    /// palette as tagged attachments. It never evaluates another pose.
    pub fn equipment_joint_world(&self, entity: crate::gameplay::entity_lifetime::EntityId, joint: usize) -> Option<(Vec3, Mat3)> {
        let rows = self.arm_render.iter().find(|entry| entry.managed_entity == Some(entity))?.joints.get(joint)?;
        Some((Vec3::new(rows[0][3], rows[1][3], rows[2][3]), Mat3::from_cols(
            Vec3::new(rows[0][0], rows[1][0], rows[2][0]), Vec3::new(rows[0][1], rows[1][1], rows[2][1]), Vec3::new(rows[0][2], rows[1][2], rows[2][2]))))
    }

    /// Audio without a joint attachment uses its entity's origin and axis
    /// (140709ee0 / 1400c5b70). Bound arms use the same frame as their model
    /// (1407506d0), before shoulder controls modify individual joints.
    pub fn sound_transform(&self, emitter: crate::gameplay::sound::Emitter) -> Option<(Vec3, Mat3)> {
        use crate::gameplay::sound::Emitter;
        if let Emitter::PlayerJoint { entity, part, joint } = emitter {
            let joint = usize::from(joint);
            if let Some(entity) = entity { return self.equipment_joint_world(entity, joint); }
            if part == 1 {
                self.pose.positions.get(joint)?;
                self.pose.rotations.get(joint)?;
                let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
                let (p, r) = self.pose.joint_with_controls(joint, &controls);
                return Some((self.physics.position() + self.physics.axis() * p, self.physics.axis() * r));
            }
            let side = match part { 2 => 0, 4 => 1, _ => return None };
            let rows = self.arm_render.get(side)?.joints.get(joint)?;
            return Some((Vec3::new(rows[0][3], rows[1][3], rows[2][3]), Mat3::from_cols(
                Vec3::new(rows[0][0], rows[1][0], rows[2][0]), Vec3::new(rows[0][1], rows[1][1], rows[2][1]),
                Vec3::new(rows[0][2], rows[1][2], rows[2][2]))));
        }
        let origin = self.physics.position();
        let axis = self.physics.axis();
        let side = match emitter {
            Emitter::PlayerBody => return Some((origin, axis)),
            Emitter::PlayerLeftArm => 0,
            Emitter::PlayerRightArm => 1,
            Emitter::PlayerLeftEquipment => return self.equipment_frames[0],
            Emitter::PlayerRightEquipment => return self.equipment_frames[1],
            Emitter::Equipment { entity, .. } => return self.equipment_entity_frames.iter()
                .find(|(emitter, ..)| matches!(emitter, Emitter::Equipment { entity: owner, .. } if *owner == entity))
                .map(|(_, p, r, _)| (*p, *r)),
            Emitter::Menu | Emitter::Effect(_) | Emitter::Component { .. } | Emitter::PlayerJoint { .. } => return None,
        };
        let arm = self.body.arms.get(side)?;
        let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
        let (position, rotation) = self.arm_frame(arm, &controls);
        Some((origin + axis * position, axis * rotation))
    }

    /// 14070a7b0: the listener uses camera_jnt for orientation and its
    /// declared main/peeking joint for position. This precedes render-camera
    /// collision corrections and retains the entity fallback for absent tags.
    pub fn sound_listener(&self) -> Option<(crate::gameplay::sound::spatial::Listener, &crate::gameplay::sound::listener::Settings)> {
        let binding = self.body.sound_listener.as_ref()?;
        let origin = self.physics.position();
        let axis = self.physics.axis();
        let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
        let listener = binding.transform(origin, axis, self.machine.is_active(StateId(0x18)), |index| {
            self.pose.positions.get(index)?;
            self.pose.rotations.get(index)?;
            let (position, rotation) = self.pose.joint_with_controls(index, &controls);
            Some((origin + axis * position, axis * rotation))
        })?;
        Some((listener, &binding.settings))
    }

    pub fn teleport(&mut self, feet: Vec3, yaw: f32) {
        self.physics.teleport(feet, yaw);
    }

    /// Both arms (left, right) as of the last `view`; empty until the arms
    /// are loaded.
    pub fn arms(&self) -> &[ArmRender] {
        &self.arm_render
    }

    pub fn feet(&self) -> Vec3 {
        self.physics.position()
    }

    pub fn axis(&self) -> Mat3 {
        self.physics.axis()
    }
}
