//! Blink's real player/world adapter. Prepared selected declarations stay
//! with the selection acquisition; casts reuse the player collision shapes,
//! shared PlayerWorld queries and native sensor2 mantle instance.
use bevy_math::{Mat3, Vec3};
use crate::{assets::entities::Value, gameplay::{
    collision_filter::QueryFilter,
    entity_lifetime::{EntityId, LiveEntities, Stage},
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
}
#[derive(Clone)]
pub(super) struct PlayerSettings {
    motion: BlinkMotionSettings,
    mantle: MantleDecl,
}
impl Prepared {
    /// Compact typed projection retained by the existing player-action
    /// acquisition owner; no generic declaration tree or rig is duplicated.
    pub fn player(blinking: &[(String, Value)], mantle: &[(String, Value)]) -> Result<PlayerSettings, String> {
        if blinking.is_empty() || mantle.is_empty() { return Err("Blink requires its Blinking state and BlinkToMantle component".into()); }
        Ok(PlayerSettings { motion: BlinkMotionSettings::read(blinking)?, mantle: crate::gameplay::player::decls::mantle_sensor(mantle)? })
    }
    pub fn with_power(power: &[(String, Value)], player: &PlayerSettings) -> Result<Self, String> {
        let mask = crate::gameplay::player::decls::layer_mask(power, "m_collisionMask", 0x20b)?;
        Ok(Self { settings: BlinkSettings::read(power, mask)?, motion: player.motion.clone(), mantle: player.mantle })
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
}
impl Runtime {
    pub fn new(prepared: Prepared, physics: &PlayerPhysics) -> Self {
        Self { blink: Blink::new(prepared.settings), motion: prepared.motion,
            mantle: Some(MantleSensor::new(prepared.mantle, physics)), hits: Vec::new() }
    }
    /// Run after the real player view exists, while UsePower is aiming.
    /// The returned target is consumed by the established FX owner; it does
    /// not allocate a target actor or mutate player position.
    pub fn aim(&mut self, player: &mut Player, view: &View, queries: &dyn PlayerWorld,
        collision: &CollisionWorld, live: &LiveEntities, interactions: Option<&Interactions>, scene: Option<&PhysicsScene>) -> bool {
        if self.mantle.is_none() { self.mantle = player.take_blink_mantle(); }
        let Some(mantle) = self.mantle.as_mut() else { return false };
        let shapes = shape_info(&player.physics);
        let input = input(player, view, &shapes);
        let mut world = World { queries, collision, live, interactions, scene, physics: &player.physics, mantle, hits: &mut self.hits };
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
        if let Some(mantle) = self.mantle.take() { player.set_blink_mantle(mantle); }
        true
    }
    pub fn cancel(&mut self, player: &mut Player) {
        self.blink.cancel();
        if self.mantle.is_none() { self.mantle = player.take_blink_mantle(); }
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
    collision: &'a CollisionWorld,
    live: &'a LiveEntities,
    interactions: Option<&'a Interactions>,
    scene: Option<&'a PhysicsScene>,
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
    fn blink_breakable(&self, hit: &Hit) -> Option<EntityId> {
        if !self.collision.physics_material(hit.key).is_some_and(|material| material.can_blink_through()) { return None; }
        let name = self.collision.owner_name(hit.body.checked_sub(1)?)?;
        if !self.interactions?.is_breakable(name) { return None; }
        // The actual admitted Health receiver is required until the native
        // Breakable owner can accept its chunk/FX destruction path. A source
        // name without a real receiver never becomes a fabricated target.
        let id = self.scene?.named_damage_target(self.live, name).ok()??;
        (self.live.stage(id) == Some(Stage::Spawned)).then_some(id)
    }
    fn mantle(&mut self, origin: Vec3, axis: Mat3) -> Option<MantleTarget> {
        self.mantle.update_at(self.queries, self.physics, origin, axis);
        self.mantle.can_mantle().then_some(MantleTarget { position: self.mantle.destination, axis: self.mantle.destination_axis })
    }
}
fn hit(value: &WorldHit) -> Hit { Hit { fraction: value.value, position: value.position, normal: value.normal, body: value.body, key: value.key } }
