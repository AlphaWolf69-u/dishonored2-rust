//! Source TIProvide context for a single DEVICE_BARK rule selection.
//! Immutable providers borrow the canonical world/graph owners; there is no
//! second mutable fact cache or per-frame acquisition path.
use crate::gameplay::entity_lifetime::{EntityId, LiveEntities};
use bevy::prelude::*;

pub(super) fn sanitize_target(
    world: &World,
    target: Option<EntityId>,
) -> Result<Option<EntityId>, String> {
    world
        .get_resource::<super::physics_scene::PhysicsScene>()
        .ok_or("Heart provider physics owner absent")?
        .heart_context_target(
            world
                .get_resource::<LiveEntities>()
                .ok_or("Heart provider live owner absent")?,
            target,
        )
}

pub(super) struct Facts<'w> {
    world: &'w World,
    target_alive: bool,
    already_asked: bool,
}
impl<'w> Facts<'w> {
    ///Native registry6cc860 ->14b970 identifies the actual typed methods:
    ///9e9530/9550/9570/9590/95d0/9610/9630/9890. They consume the real
    ///sensor9 result and Heart611ea0 answer state, not the pulse sensor20.
    pub(super) fn read(
        world: &'w World,
        heart: EntityId,
        target: Option<EntityId>,
        already_asked: bool,
    ) -> Result<Self, String> {
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("Heart provider identity owner absent")?;
        live.get(heart)
            .ok_or("Heart provider equipment identity stale")?;
        let scene = world
            .get_resource::<super::physics_scene::PhysicsScene>()
            .ok_or("Heart provider scene owner absent")?;
        let target = scene.heart_context_target(live, target)?;
        if let Some(target) = target {
            if scene.heart_context_component_present(target, b"arkCpntAIIdentity")? {
                return Err(
                    "Heart AIIdentity provider requires its actual admitted payload".into(),
                );
            }
        }
        //5da5a0 caches an own-entity AreaLocator with mode0.877720 never
        //falls back to the bound parent. All Heart variants' actual component
        //lists omit it;5db8c0 consequently preserves782250's invalid tag.
        if scene.heart_context_component_present(heart, b"arkCpntAreaLocator")? {
            return Err("Heart AreaLocator provider requires its actual admitted payload".into());
        }
        Ok(Self {
            world,
            target_alive: scene.heart_context_target_alive(live, target)?,
            already_asked,
        })
    }

    pub(super) fn boolean(&mut self, variable: &str) -> Result<bool, String> {
        match variable {
            "lib.true" => Ok(true), //6cc9a0 creates the constant true provider.
            "bark.device.player.isTargetAlive" => Ok(self.target_alive),
            "bark.device.player.alreadyAskedEntity" => Ok(self.already_asked),
            _ => {
                let qualified = variable
                    .strip_prefix("kiscule.story.bool.")
                    .ok_or_else(|| format!("unported Heart bool provider {variable}"))?;
                let effects = self
                    .world
                    .get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
                    .ok_or("Heart story provider Kiscule manager absent")?;
                //5134e0 returns null for a genuinely missing graph/variable;
                //6cbf20 then reads143985210, the zero-initialized bool.
                let worldspawn = self
                    .world
                    .get_resource::<crate::gameplay::entity_lifetime::WorldspawnEntity>()
                    .ok_or("Heart story provider actual worldspawn absent")?
                    .0;
                Ok(effects
                    .story_boolean(worldspawn, qualified)?
                    .unwrap_or(false))
            }
        }
    }
    pub(super) fn area_tag(&mut self, variable: &str) -> Result<u32, String> {
        if variable != "bark.base.ownerAreaTag" {
            return Err(format!("unported Heart area provider {variable}"));
        }
        Ok(u32::MAX) //5db8c0 ->782250, actual absent own AreaLocator.
    }
    pub(super) fn symbol(&mut self, variable: &str) -> Result<&str, String> {
        match variable {
            "bark.base.trigger" => Ok("AI_BARK_TRIGGER_AMBIENT"), //actual request1
            "bark.device.player.targetName" | "bark.device.player.targetFaction" => Ok(""), //572fe0/572ed0 nullIdentity
            "bark.device.player.targetGender" => Ok("GENDER_UNDEFINED"), //572f30=5;1419fd460
            "bark.device.player.targetMorality" => Ok("MORALITY_UNDEFINED"), //572fa0=3;141a0d3d0
            "world.chaosLevel" => {
                let player = self
                    .world
                    .get_resource::<super::player::PlayerState>()
                    .and_then(|state| state.player.as_ref())
                    .ok_or("Heart chaos provider actual player absent")?;
                match player.chaos_level()? {
                    0 => Ok("CHAOS_LEVEL_LOW"),
                    1 => Ok("CHAOS_LEVEL_HIGH"),
                    2 => Ok("CHAOS_LEVEL_VERY_HIGH"),
                    value => Err(format!("actual PlayerChaos level outside enum: {value}")),
                }
            }
            _ => Err(format!("unported Heart symbol provider {variable}")),
        }
    }
}
