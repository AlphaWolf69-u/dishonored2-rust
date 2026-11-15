//! Far Reach's retained typed acquisition. World targeting is admitted only
//! after sensor27's actual safe-position and player-state checks succeed.
use crate::{
    assets::entities::Value,
    gameplay::player::{
        Player,
        camera::View,
        character_proxy::PlayerWorld,
        grapple::{Decl, GeometryTarget, Sensor, SensorDecl, Target},
        grapple_state::Request,
        physics::PlayerPhysics,
        states::StateId,
    },
};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct PlayerSettings {
    pub motion: Decl,
    pub sensor: SensorDecl,
}

pub(super) struct Runtime {
    sensor: Sensor,
    target: Option<GeometryTarget>,
    pending: bool,
    was_active: bool,
}
impl Runtime {
    pub fn new(prepared: Prepared, _physics: &PlayerPhysics) -> Self {
        Self {
            sensor: Sensor::new(prepared.player.sensor),
            target: None,
            pending: false,
            was_active: false,
        }
    }
    /// Source sensor27 queries the existing geometry owner after the real
    /// player camera exists; no synthetic grapple target actor is spawned.
    pub fn aim(
        &mut self,
        player: &Player,
        view: &View,
        queries: &dyn PlayerWorld,
        snapping: bool,
    ) -> bool {
        if self.has_activity() {
            return false;
        }
        self.target = self.sensor.geometry_target(
            queries,
            &player.physics,
            view.origin,
            view.axis,
            player
                .attributes
                .get("ARK_ATTRIBUTE_GRAPPLE_HORIZONTAL_RANGE"),
            player
                .attributes
                .get("ARK_ATTRIBUTE_GRAPPLE_VERTICAL_RANGE"),
            snapping,
            player.machine.is_active(StateId::SNEAKING),
            player.machine.is_active(StateId::AUTOCROUCH),
        );
        self.target.is_some()
    }
    pub fn execute(&mut self, player: &mut Player, _view: &View) -> bool {
        if self.has_activity() {
            return false;
        }
        let Some(target) = self.target else {
            return false;
        };
        let exit_state = if player.machine.is_active(StateId::SNEAKING) {
            StateId::SNEAKING
        } else if player.machine.is_active(StateId::AUTOCROUCH) {
            StateId::AUTOCROUCH
        } else {
            StateId::RUNNING
        };
        if !player.begin_grapple(Request {
            target: Target::from_normal(target.normal),
            target_position: target.position,
            target_normal: target.normal,
            safe_position: target.safe_position,
            target_velocity: bevy_math::Vec3::ZERO,
            start_posture: target.posture,
            exit_state,
            to_mantle: false,
            can_jump: target.can_jump,
            snapped_ground: target.snapped_ground,
            jump_to_target: false,
            sensor: Arc::clone(&self.sensor.prepared),
        }) {
            return false;
        }
        self.pending = true;
        true
    }
    pub fn has_activity(&self) -> bool {
        self.pending || self.was_active
    }
    pub fn cancel(&mut self, _player: &mut Player) {
        if !self.has_activity() {
            self.target = None;
        }
    }
    pub fn service(&mut self, player: &mut Player) -> bool {
        let active = player.machine.is_active(StateId::GRAPPLE);
        if active {
            self.pending = false;
            self.was_active = true;
        }
        if player.take_finished_grapple().is_some() || self.was_active && !active {
            self.pending = false;
            self.was_active = false;
            self.target = None;
        }
        self.has_activity()
    }
    pub fn retire(&mut self, player: &mut Player) {
        if self.has_activity() {
            player.cancel_grapple();
        }
        self.pending = false;
        self.was_active = false;
        self.target = None;
    }
}
#[derive(Clone)]
pub(super) struct Prepared {
    pub player: PlayerSettings,
}
impl Prepared {
    pub fn player(
        grapple: &[(String, Value)],
        sensor: &[(String, Value)],
    ) -> Result<PlayerSettings, String> {
        if grapple.is_empty() || sensor.is_empty() {
            return Err("Far Reach requires its Grapple state and Grapple sensor component".into());
        }
        Ok(PlayerSettings {
            motion: Decl::read(grapple)?,
            sensor: SensorDecl::read(sensor)?,
        })
    }
    pub fn with_power(
        _component: &[(String, Value)],
        player: &PlayerSettings,
    ) -> Result<Self, String> {
        Ok(Self {
            player: player.clone(),
        })
    }
    /// Traversal scenes are canonical Midnight resources and are acquired
    /// alongside the player state. Power FX stay with their established owner.
    pub fn scene_paths(&self) -> impl Iterator<Item = &str> {
        [
            self.player.motion.translate_midnight.as_deref(),
            self.player.motion.wall_midnight.as_deref(),
            self.player.motion.ceil_midnight.as_deref(),
        ]
        .into_iter()
        .flatten()
    }
}
