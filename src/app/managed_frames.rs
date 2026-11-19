//!Borrow canonical generation/frame/palette owners for demanded script refs.
use crate::{
    gameplay::{
        entity_lifetime::{EntityId, LiveEntities},
        event_graph::EntityReference,
        reference_frame::ReferenceFrame,
    },
    rendering::bevy_backend::world_effects::ManagedFrames,
};
pub(super) struct Resolver<'a> {
    pub clocks: Option<&'a crate::gameplay::game_clock_runtime::ClockRuntime>,
    pub live: &'a LiveEntities,
    pub scene: &'a super::physics_scene::PhysicsScene,
    pub player: Option<&'a crate::gameplay::player::Player>,
}
impl ManagedFrames for Resolver<'_> {
    fn graph_clock(&self, owner: &str) -> Result<f64, String> {
        let id = self.live.find(owner).ok_or_else(|| format!("script clock owner {owner} is absent"))?;
        let clock = self.clocks.ok_or("game clock unavailable")?;
        let (selector, _) = self.scene.entity_time_binding(self.live, id)?;
        clock.entity_time_from_definition(id, selector).map_err(str::to_owned)
    }
    fn entity(&self, reference: &EntityReference) -> Result<Option<EntityId>, String> {
        Ok(match reference {
            EntityReference::Managed(id) => self.live.get(*id).map(|_| *id),
            EntityReference::Named(name) => self.live.find(name),
            EntityReference::Player => self
                .player
                .and_then(|player| player.managed_entity())
                .filter(|id| self.live.get(*id).is_some()),
            EntityReference::Invalid => None,
        })
    }
    fn name(&self, id: EntityId) -> Option<&str> {
        self.live.get(id).map(|(name, _)| name)
    }
    fn spawn_tag(&self, id: EntityId) -> Result<String, String> {
        Ok(self.live.spawn_tag_text(id)?.to_owned())
    }
    fn animated_model(&self, id: EntityId) -> Result<bool, String> {
        if self.live.get(id).is_none() { return Ok(false); }
        Ok(self.scene.runtime_animation_rig(id)?.is_some())
    }
    fn frame(&self, id: EntityId, tag: &str) -> Result<Option<ReferenceFrame>, String> {
        if self.live.get(id).is_none() {
            return Ok(None);
        }
        if let Some(player) = self.player {
            if let Some(frame) = player.animation_entity_tag_world(id, tag)? {
                return Ok(Some(frame));
            }
        }
        let Some(frame) = self.scene.entity_frame(self.live, id)? else {
            return Ok(None);
        };
        if tag.is_empty() {
            return Ok(Some(frame));
        }
        let rig = self
            .scene
            .runtime_animation_rig(id)?
            .ok_or("managed FX tag has no actual animation owner")?;
        let matrices = self
            .scene
            .runtime_animation_matrices(id)?
            .ok_or("managed FX tag has no actual palette")?;
        let published = self.scene.published_entity_frame(self.live, id)?;
        super::joint_frames::current_tag_frame(
            &rig.rig,
            matrices,
            "_info",
            tag,
            published.scale,
            frame,
        )
    }
}
