//!779530 initialization on the level's live component and physics owners.
use super::{prepare_trigger_body, TriggerBodyInput, TriggerCollisionSettings, TriggerComponent};
use crate::{
    assets::physworld::body::BodyInfo,
    gameplay::{collision_user_data::CollisionUserData, component_state::entity::ComponentHandle},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerDependency {
    Water,
    Wind,
}

pub struct LoadedTriggerShape {
    pub shape: usize,
    pub basis: [[f32; 3]; 3],
}

/// Each boundary reads the actual owner again at the source call site. In
/// particular, component lookup can change declaration/pose/body state.
pub trait TriggerInitializationWorld {
    fn trigger(&self, reference: ComponentHandle) -> Result<&TriggerComponent, String>;
    fn trigger_mut(&mut self, reference: ComponentHandle) -> Result<&mut TriggerComponent, String>;
    /// Resolve the resource recipe from trigger_shape_request and retain its
    /// actual loaded shape. Absence follows the source's failed-init branch.
    fn load_shape(&mut self, entity: usize) -> Result<Option<LoadedTriggerShape>, String>;
    ///166830's error reporting. The debug trap policy may stop here; otherwise
    /// the routine continues to7e72c0, despite Ghidra's noreturn annotation.
    fn report_missing_shape(&mut self, entity: usize) -> Result<(), String>;
    fn release_failed(&mut self, reference: ComponentHandle) -> Result<(), String>;
    ///877720, policy1. Do not infer presence merely from an authored class name.
    fn dependency(
        &mut self,
        entity: usize,
        kind: TriggerDependency,
    ) -> Result<Option<ComponentHandle>, String>;
    fn position(&self, entity: usize) -> Result<[f32; 3], String>;
    fn declaration(&self, reference: ComponentHandle) -> Result<usize, String>;
    fn collision_settings(&self, declaration: usize) -> Result<TriggerCollisionSettings, String>;
    fn trigger_material(&self) -> Result<u16, String>;
    ///c8e5b0 with BodyAddition::Disabled and BodyActivation::Activate.
    fn create_disabled_body(
        &mut self,
        reference: ComponentHandle,
        info: &BodyInfo,
    ) -> Result<u32, String>;
    /// Publish into the trigger's shared movement interface, not another map.
    fn set_body(&mut self, reference: ComponentHandle, body: u32) -> Result<(), String>;
    fn mark_entity_keyframed(&mut self, entity: usize) -> Result<(), String>;
}

pub fn initialize_trigger(
    reference: ComponentHandle,
    world: &mut impl TriggerInitializationWorld,
) -> Result<(), String> {
    let trigger = world.trigger(reference)?;
    let entity = trigger.owner.entity.ok_or("trigger has no owning entity")?;
    let bit30 = trigger.owner.flags >> 30 & 1 != 0;
    world.trigger_mut(reference)?.spawn_owner_bit30 = bit30;
    let Some(shape) = world.load_shape(entity)? else {
        world.report_missing_shape(entity)?;
        world.release_failed(reference)?;
        return Ok(());
    };
    world.trigger_mut(reference)?.owner.mark_initialized();
    let water = world.dependency(entity, TriggerDependency::Water)?;
    world.trigger_mut(reference)?.water = water;
    let wind = world.dependency(entity, TriggerDependency::Wind)?;
    world.trigger_mut(reference)?.wind = wind;
    let declaration = world.declaration(reference)?;
    let collision = world.collision_settings(declaration)?;
    let input = prepare_trigger_body(TriggerBodyInput {
        shape: shape.shape,
        basis: shape.basis,
        position: world.position(entity)?,
        material_id: world.trigger_material()?,
        user_data: CollisionUserData {
            component: reference,
        }
        .encode(),
        water_component: world.trigger(reference)?.water.is_some(),
        wind_component: world.trigger(reference)?.wind.is_some(),
        collision,
    });
    let body = world.create_disabled_body(reference, &input)?;
    world.set_body(reference, body)?;
    // Body creation observers can edit this same declaration. The source keeps
    // its declaration pointer, then reads keyframed again after publication.
    if world.collision_settings(declaration)?.keyframed {
        world.mark_entity_keyframed(entity)?;
    } else {
        world.trigger_mut(reference)?.owner.request_update(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
pub(crate) mod physics;
