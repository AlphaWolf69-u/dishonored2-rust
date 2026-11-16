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
    damage: Option<Arc<crate::assets::damage::Declaration>>,
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
            helpers: [path("m_blinkTopTargetFx"),path("m_blinkBottomTargetFx"),path("m_blinkMantleFx")], screen: path("m_blinkScreenFx"), damage: None })
    }
    /// Acquire with the selected equipment's existing dependency worker. The
    /// declaration follows the same canonical registry/selection lease.
    pub fn acquire_damage(&mut self, catalog: &crate::archive::Catalog) -> Result<(), String> {
        self.damage = self.motion.breakable_damage.as_deref()
            .map(|path| crate::assets::damage::Declaration::acquire(catalog, path)).transpose()?;
        Ok(())
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
    screen_key: String,
    pending: bool,
    was_active: bool,
    time: Option<TimeLease>,
    damage: Option<Arc<crate::assets::damage::Declaration>>,
}
impl Runtime {
    pub fn new(prepared: Prepared, physics: &PlayerPhysics) -> Self {
        Self { blink: Blink::new(prepared.settings), motion: prepared.motion,
            mantle: Some(MantleSensor::new(prepared.mantle, physics)), hits: Vec::new(), contacts: Vec::new(),
            helpers: prepared.helpers, helper_keys: [Arc::from("blink:top"),Arc::from("blink:bottom"),Arc::from("blink:mantle")],
            helper_started:[false;3], screen:prepared.screen,screen_started:false,screen_key:"blink:screen".into(),pending:false,was_active:false,time:None,damage:prepared.damage }
    }
    /// The actual owned-component admission generation identifies its FX
    /// producer. Set once before publication; equipment reselection reuses it.
    pub fn set_owner_key(&mut self, owner: Arc<str>) -> Result<(), String> {
        if self.has_activity() || self.screen_started || self.helper_started.iter().any(|started| *started) {
            return Err("cannot replace a live Blink component's FX producer identity".into());
        }
        self.helper_keys = [Arc::from(format!("{owner}:top")), Arc::from(format!("{owner}:bottom")), Arc::from(format!("{owner}:mantle"))];
        self.screen_key = format!("{owner}:screen");
        Ok(())
    }
    /// Run after the real player view exists, while UsePower is aiming.
    /// The returned target is consumed by the established FX owner; it does
    /// not allocate a target actor or mutate player position.
    pub fn aim(&mut self, player: &mut Player, view: &View, queries: &dyn PlayerWorld,
        _collision: &CollisionWorld, live: &LiveEntities, _interactions: Option<&Interactions>, scene: Option<&PhysicsScene>) -> Result<bool, String> {
        if self.mantle.is_none() { self.mantle = player.take_blink_mantle(); }
        let Some(mantle) = self.mantle.as_mut() else { return Ok(false) };
        let shapes = shape_info(&player.physics);
        let input = input(player, view, &shapes);
        let mut world = World { queries, physics: &player.physics, mantle, hits: &mut self.hits,
            live, scene, error: std::cell::RefCell::new(None) };
        self.blink.aim(&mut world, &input);
        if let Some(error) = world.error.into_inner() { return Err(error); }
        Ok(true)
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
    pub fn has_motion(&self) -> bool { self.pending || self.was_active }
    pub fn breakable_damage_path(&self) -> Option<&str> { self.motion.breakable_damage.as_deref() }
    /// The real state owns its previous proxy contacts. Preserve the body
    /// flags captured by that query; body IDs are not navigation owner indexes.
    pub fn service(&mut self, player: &mut Player, clock: &mut ClockRuntime, effects: &mut EffectRequests,
        _collision: &CollisionWorld, _scene: Option<&PhysicsScene>, _live: &LiveEntities) -> Result<bool,String> {
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
            for contact in player.physics.proxy.contact_hits() {
                self.contacts.push(BlinkContact {body_flags:contact.body_flags,normal:contact.normal});
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
        if self.screen_started{effects.stop_player(&self.screen_key);self.screen_started=false;}
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
        if !self.screen_started{if let Some(path)=&self.screen{effects.start_player(&self.screen_key,path);self.screen_started=true;}}
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
    ///8584c0 runs before the state's motion step. It constructs damage from
    ///the authored declaration, sets780ae0's INSTANTKILL bit and dispatches
    ///only still-started Breakables in the source anticipation bound. The
    ///source leaves attacker, inflictor and hit-location fields at defaults.
    pub fn damage_breakables(&self, player: &Player, dt: f32, scene: &mut PhysicsScene,
        live: &mut LiveEntities, events: &mut crate::gameplay::entity_events::EntityEvents) -> Result<(), String> {
        if !player.machine.is_active(StateId::BLINKING) { return Ok(()); }
        let Some(declaration) = self.damage.as_ref() else { return Ok(()); };
        let mut parameters = crate::gameplay::damage::Parameters::from_declaration(declaration);
        parameters.damage_type.0 |= crate::gameplay::damage::DamageType::INSTANT_KILL.0;
        let mut result = Ok(());
        self.visit_breakable_hits(player, dt, |entity, _, _| {
            if result.is_err() || live.get(entity).is_none() { return; }
            result = (|| {
                if scene.breakable_component(live, entity)?.is_some() {
                    scene.damage_entity(live, events, entity, &parameters)?;
                }
                Ok(())
            })();
        });
        result
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
    live: &'a LiveEntities,
    scene: Option<&'a PhysicsScene>,
    error: std::cell::RefCell<Option<String>>,
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
    fn blink_breakable(&self, hit: &Hit) -> Option<EntityId> {
        // 61e1a0: the actual hit material permits Blink, then the collider's
        // userData resolves a started Breakable (877720 mode2). Neither a
        // navigation owner number nor a Health receiver admits a Breakable.
        let witness = self.hits.iter().find(|value| value.body == hit.body && value.key == hit.key)?;
        if !self.queries.physics_material(witness)?.can_blink_through() { return None; }
        let scene = self.scene?;
        let result = (|| {
            let target = crate::gameplay::collision_hit::ShapeHitInfo {
                body_id: Some(witness.body), material_id: witness.material_id.unwrap_or(u16::MAX),
                shape_key: witness.key, filter_info: 0, user_data: witness.user_data,
            };
            let Some(entity) = scene.collision_entity(self.live, &target)? else { return Ok(None); };
            Ok(scene.breakable_component(self.live, entity)?.map(|_| entity))
        })();
        match result {
            Ok(entity) => entity,
            Err(error) => { self.error.borrow_mut().get_or_insert(error); None }
        }
    }
    fn mantle(&mut self, origin: Vec3, axis: Mat3) -> Option<MantleTarget> {
        self.mantle.update_at(self.queries, self.physics, origin, axis);
        self.mantle.can_mantle().then_some(MantleTarget { position: self.mantle.destination, axis: self.mantle.destination_axis })
    }
}
fn hit(value: &WorldHit) -> Hit { Hit { fraction: value.value, position: value.position, normal: value.normal, body: value.body, key: value.key } }
