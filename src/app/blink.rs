//! Blink's real player/world adapter. Prepared selected declarations stay
//! with the selection acquisition; casts reuse the player collision shapes,
//! shared PlayerWorld queries and native sensor2 mantle instance.
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;
use crate::gameplay::{game_clock_runtime::ClockRuntime, powers::{world_effects::{TimeLease,TimeRequest}, traversal::BlinkContact},
    reference_frame::{self, ReferenceFrame}, player::states::StateId, event_graph::EntityReference};
use super::usable_effects::{EffectRequests,EntityPlacement};
use crate::{assets::entities::Value, gameplay::{
    collision_filter::QueryFilter,
    entity_lifetime::{EntityId, LiveEntities},
    navigation::CollisionWorld,
    player::{Player, physics::PlayerPhysics, camera::View, sensors::{MantleDecl, MantleSensor},
        character_proxy::{PlayerWorld, WorldHit}},
    powers::traversal::{Blink, BlinkSettings, BlinkMotionSettings, BlinkMotion, BlinkInput,
        BlinkWorld, Hit, MantleTarget, ShapeInfo},
}};
use super::{interactions::Interactions, physics_scene::PhysicsScene};

#[derive(Clone)]
pub(super) struct Prepared {
    settings: BlinkSettings,
    motion: BlinkMotionSettings,
    mantle: MantleDecl,
    helpers: [Option<String>;3],
    screen: Option<String>,
}
#[derive(Clone)]
pub(super) struct PlayerSettings {
    motion: BlinkMotionSettings,
    mantle: MantleDecl,
}
impl Prepared {
    /// Actual authored helper/screen dependencies; reserve their sources
    /// without starting the aiming actor or a particle instance.
    pub fn effect_paths(&self) -> impl Iterator<Item = &str> {
        self.helpers.iter().flatten().map(String::as_str).chain(self.screen.as_deref())
    }
    /// Compact typed projection retained by the existing player-action
    /// acquisition owner; no generic declaration tree or rig is duplicated.
    pub fn player(blinking: &[(String, Value)], mantle: &[(String, Value)]) -> Result<PlayerSettings, String> {
        if blinking.is_empty() || mantle.is_empty() { return Err("Blink requires its Blinking state and BlinkToMantle component".into()); }
        Ok(PlayerSettings { motion: BlinkMotionSettings::read(blinking)?, mantle: crate::gameplay::player::decls::mantle_sensor(mantle)? })
    }
    pub fn with_power(power: &[(String, Value)], player: &PlayerSettings) -> Result<Self, String> {
        let mask = crate::gameplay::player::decls::layer_mask(power, "m_collisionMask", 0x20b)?;
        let path = |key| crate::gameplay::options::decl::atom(power,key).filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL")).map(str::to_owned);
        Ok(Self { settings: BlinkSettings::read(power, mask)?, motion: player.motion.clone(), mantle: player.mantle,
            helpers: [path("m_blinkTopTargetFx"),path("m_blinkBottomTargetFx"),path("m_blinkMantleFx")], screen: path("m_blinkScreenFx") })
    }
    /// Called by the selected equipment acquisition worker. Sensor2's
    /// 1403a52a0 ctor derives the ordinary MantleDecl and sets sensorID2.
    pub fn read(power: &[(String, Value)], blinking: &[(String, Value)], mantle: &[(String, Value)]) -> Result<Self, String> {
        Self::with_power(power, &Self::player(blinking, mantle)?)
    }
}

pub(super) struct Runtime {
    pub blink: Blink,
    motion: BlinkMotionSettings,
    mantle: Option<MantleSensor>,
    hits: Vec<WorldHit>,
    contacts: Vec<BlinkContact>,
    helpers: [Option<String>;3],
    helper_keys: [Arc<str>;3],
    helper_started: [bool;3],
    screen: Option<String>,
    screen_started: bool,
    pending: bool,
    was_active: bool,
    time: Option<TimeLease>,
}
impl Runtime {
    pub fn new(prepared: Prepared, physics: &PlayerPhysics) -> Self {
        Self { blink: Blink::new(prepared.settings), motion: prepared.motion,
            mantle: Some(MantleSensor::new(prepared.mantle, physics)), hits: Vec::new(), contacts: Vec::new(),
            helpers: prepared.helpers, helper_keys: [Arc::from("blink:top"),Arc::from("blink:bottom"),Arc::from("blink:mantle")],
            helper_started:[false;3], screen:prepared.screen,screen_started:false,pending:false,was_active:false,time:None }
    }
    /// Run after the real player view exists, while UsePower is aiming.
    /// The returned target is consumed by the established FX owner; it does
    /// not allocate a target actor or mutate player position.
    pub fn aim(&mut self, player: &mut Player, view: &View, queries: &dyn PlayerWorld,
        _collision: &CollisionWorld, _live: &LiveEntities, _interactions: Option<&Interactions>, _scene: Option<&PhysicsScene>) -> bool {
        if self.mantle.is_none() { self.mantle = player.take_blink_mantle(); }
        let Some(mantle) = self.mantle.as_mut() else { return false };
        let shapes = shape_info(&player.physics);
        let input = input(player, view, &shapes);
        let mut world = World { queries, physics: &player.physics, mantle, hits: &mut self.hits };
        self.blink.aim(&mut world, &input);
        true
    }
    /// Execute the actual release callback. State12 owns motion and all
    /// proxy restoration; the central dispatcher charges mana only when
    /// this accepts the prepared native state request.
    pub fn execute(&mut self, player: &mut Player, view: &View) -> bool {
        let shapes = shape_info(&player.physics);
        let request = { let input = input(player, view, &shapes); self.blink.cast_request(&input) };
        let Some(request) = request else { return false };
        let motion = BlinkMotion::new(self.motion.clone(), request, player.physics.velocity);
        if !player.begin_blink(motion) { return false; }
        self.pending = true;
        if let Some(mantle) = self.mantle.take() { player.set_blink_mantle(mantle); }
        true
    }
    pub fn cancel(&mut self, player: &mut Player) {
        if !self.has_activity() { self.blink.cancel(); }
        if self.mantle.is_none() { self.mantle = player.take_blink_mantle(); }
    }
    pub fn has_activity(&self) -> bool { self.pending || self.was_active || self.time.is_some() }
    pub fn breakable_damage_path(&self) -> Option<&str> { self.motion.breakable_damage.as_deref() }
    /// The real state owns its previous proxy contacts. Body0 is the
    /// canonical GameCollision level-static body; placed bodies resolve their
    /// actual admitted rigid generation, so dynamic bodies retain their flags.
    pub fn service(&mut self, player: &mut Player, clock: &mut ClockRuntime, effects: &mut EffectRequests,
        collision: &CollisionWorld, scene: Option<&PhysicsScene>, live: &LiveEntities) -> Result<bool,String> {
        let active = player.machine.is_active(StateId::BLINKING);
        if active {
            self.pending=false; self.was_active=true;
            if self.time.is_none() {
                let world = if player.attributes.get("ARK_ATTRIBUTE_STOP_TIME_DURING_BLINK") > 0.0 {0.0} else {self.motion.world_time_scale};
                self.time=Some(clock.acquire_time_request(TimeRequest {world,unbent:self.motion.unbent_time_scale,ease:self.motion.ease_in_duration})?);
                clock.set_player_time_override(f32::from_bits(0x7149_f2ca))?;
            }
            self.stop_helpers(effects);
            self.contacts.clear();
            let has_placed_contacts = player.physics.proxy.contact_hits().any(|contact| contact.body != 0);
            let shared = if has_placed_contacts { scene.map(PhysicsScene::point_queries).transpose()? } else { None };
            let storage = shared.as_ref().map(|world| world.read().map_err(|_| "Blink contact world is poisoned".to_owned())).transpose()?;
            for contact in player.physics.proxy.contact_hits() {
                let flags = if contact.body==0 {1} else {
                    let owner=collision.owner_name(contact.body-1).ok_or("Blink contact has no collision owner")?;
                    let scene=scene.ok_or("Blink contact has no actual physics component owner")?;
                    let body=scene.blink_collision_body(live,owner)?.ok_or_else(||format!("Blink contact {owner} has no admitted collision body"))?;
                    storage.as_ref().and_then(|world|world.storage.bodies.get(body)).ok_or("Blink contact body generation is stale")?.flags as u8
                };
                self.contacts.push(BlinkContact {body_flags:flags,normal:contact.normal});
            }
            player.set_blink_contacts(&self.contacts);
        }
        if player.take_finished_blink().is_some() || (self.was_active && !active) {
            self.retire(player,clock,effects)?;
        }
        Ok(self.has_activity())
    }
    /// Safe before map clock/FX owners retire. Repeated calls release no
    /// other component's lease and never clear another power's time request.
    pub fn retire(&mut self, player:&mut Player, clock:&mut ClockRuntime, effects:&mut EffectRequests)->Result<(),String>{
        if let Some(lease)=self.time.take(){clock.release_time_request(lease,self.motion.ease_out_duration)?;}
        // Native80f040 restores the player's group when no retained world
        // bending applies. Do not erase an override held by another power.
        self.pending=false;self.was_active=false;
        self.stop_helpers(effects);self.cancel(player);Ok(())
    }
    pub fn stop_helpers(&mut self,effects:&mut EffectRequests){
        for index in 0..3 {if self.helper_started[index]{effects.stop_entity(self.helper_keys[index].clone());self.helper_started[index]=false;}}
        if self.screen_started{effects.stop_player("blink:screen");self.screen_started=false;}
    }
    /// Explicit87dc10 placement updates on the same canonical FX instances.
    /// The parent is the real player frame; local transforms resolve to the
    /// native world-space helper poses without a fabricated target entity.
    pub fn publish_helpers(&mut self,effects:&mut EffectRequests,player:&Player){
        let Some(target)=self.blink.target.as_ref() else {self.stop_helpers(effects);return};
        let positions=[target.top_fx.map(|position|(position,Mat3::IDENTITY)),target.bottom_fx.map(|position|(position,Mat3::IDENTITY)),
            target.mantle.map(|mantle|(mantle.position,mantle.axis))];
        let root=ReferenceFrame{position:player.physics.position().to_array(),basis:player.physics.axis().to_cols_array_2d()};
        let mut inverse=root.basis;if !reference_frame::invert_basis(&mut inverse){return;}
        for index in 0..3{
            let Some((position,axis))=positions[index] else{if self.helper_started[index]{effects.stop_entity(self.helper_keys[index].clone());self.helper_started[index]=false;}continue};
            let Some(path)=self.helpers[index].as_deref() else{continue};
            let world=ReferenceFrame{position:position.to_array(),basis:axis.to_cols_array_2d()};
            let local=ReferenceFrame{position:reference_frame::relative_binding(root,world).0,basis:reference_frame::basis_product(world.basis,inverse)};
            let placement=EntityPlacement{target:EntityReference::Player,tag:String::new(),local,attached:false};
            if self.helper_started[index]{effects.update_entity(self.helper_keys[index].clone(),placement);}
            else{effects.start_entity(self.helper_keys[index].clone(),path,placement);self.helper_started[index]=true;}
        }
        if !self.screen_started{if let Some(path)=&self.screen{effects.start_player("blink:screen",path);self.screen_started=true;}}
    }
    pub fn visit_breakable_hits(&self,player:&Player,dt:f32,mut visit:impl FnMut(EntityId,Vec3,&str)){
        if !self.was_active{return;}
        let Some(path)=self.motion.breakable_damage.as_deref() else{return;};
        let reach=(self.motion.speed*dt).max(self.motion.breakable_anticipation_distance);
        for hit in &self.blink.breakables{
            let projection=player.physics.axis().x_axis.dot(hit.hit_position-player.physics.position());
            let projected=player.physics.axis().x_axis.truncate()*projection;
            if projected.length_squared()<reach*reach*reach*reach{visit(hit.entity,hit.hit_position,path);}
        }
    }
}

fn shape_info(physics: &PlayerPhysics) -> [ShapeInfo; 20] {
    std::array::from_fn(|slot| { let settings = physics.posture_settings(slot);
        // Native +0x0c is z_offset, historically named margin by the
        // traversal projection. It is not proxy keep distance.
        ShapeInfo { radius: settings.radius, height: settings.height, margin: settings.z_offset }
    })
}
fn input<'a>(player: &Player, view: &View, shapes: &'a [ShapeInfo; 20]) -> BlinkInput<'a> {
    BlinkInput { shapes, current_shape: player.physics.posture, physical_position: player.physics.position(),
        camera_position: view.origin, camera_direction: view.axis.x_axis, player_axis: player.physics.axis(),
        horizontal_distance: player.attributes.get("ARK_ATTRIBUTE_BLINK_HORIZONTAL_DISTANCE"),
        clearance: player.physics.proxy.settings.keep_distance + player.physics.proxy.settings.keep_contact_tolerance + 0.1,
        // The assassination target owner is not instantiated by the current
        // player. Never manufacture an NPC target for Blink attack.
        assassination_target: None }
}

struct World<'a> {
    queries: &'a dyn PlayerWorld,
    physics: &'a PlayerPhysics,
    mantle: &'a mut MantleSensor,
    hits: &'a mut Vec<WorldHit>,
}
impl BlinkWorld for World<'_> {
    fn sweep(&mut self, slot: usize, mask: u32, from: Vec3, direction: Vec3, length: f32) -> Option<Hit> {
        self.hits.clear();
        self.queries.cast_shape(QueryFilter::layers(mask, 0x40), self.physics.posture_body(slot),
            Mat3::IDENTITY, from, direction * length, 0.0, self.hits, None);
        self.hits.iter().min_by(|a,b| a.value.total_cmp(&b.value)).map(hit)
    }
    fn sweep_all(&mut self, slot: usize, mask: u32, from: Vec3, direction: Vec3, length: f32, out: &mut Vec<Hit>) {
        self.hits.clear();
        self.queries.cast_shape(QueryFilter::layers(mask, 0x40), self.physics.posture_body(slot),
            Mat3::IDENTITY, from, direction * length, 0.0, self.hits, None);
        out.extend(self.hits.iter().map(hit));
    }
    fn blink_breakable(&self, _hit: &Hit) -> Option<EntityId> {
        // 14061e1a0 requires material bit0x04 AND an actual started
        // arkCpntBreakable from 140877720. PhysicsScene has no Breakable
        // pool/lifecycle yet. Authored metadata and a Health receiver do
        // not admit this component, so its geometry remains blocking.
        None
    }
    fn mantle(&mut self, origin: Vec3, axis: Mat3) -> Option<MantleTarget> {
        self.mantle.update_at(self.queries, self.physics, origin, axis);
        self.mantle.can_mantle().then_some(MantleTarget { position: self.mantle.destination, axis: self.mantle.destination_axis })
    }
}
fn hit(value: &WorldHit) -> Hit { Hit { fraction: value.value, position: value.position, normal: value.normal, body: value.body, key: value.key } }
