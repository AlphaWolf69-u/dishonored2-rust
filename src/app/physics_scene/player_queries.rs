//! Player-query ownership of real rigid-family bodies and legacy snapshots.
use super::*;
use crate::gameplay::{
    entity_lifetime::{EntityId, LiveEntities},
    navigation::CollisionWorld,
};
impl PhysicsScene {
    pub(in crate::app) fn player_rigid_classes(&self) -> [i32; 2] {
        [self.classes[8], self.classes[12]]
    }

    ///Run once after actual body initialization and before level publication.
    ///Only a constructed body proves replacement; a declaration alone does not.
    pub(in crate::app) fn publish_navigation_ownership(
        &self,
        navigation: &mut CollisionWorld,
    ) -> Result<(), String> {
        for pool in [&self.rigid_bodies, &self.throwables] {
            for component in pool.iter() {
                if component.body().is_none() {
                    continue;
                }
                let index = component
                    .owner
                    .entity
                    .ok_or("admitted rigid navigation owner absent")?;
                let entity = self
                    .entities
                    .get(index)
                    .ok_or("admitted rigid navigation header absent")?;
                navigation.mark_physics_owned(&entity.name);
            }
        }
        Ok(())
    }
    ///Late component/body admission uses the same real entity identity and
    ///typed payload pools; the admission bit persists until the level drops.
    pub(in crate::app) fn publish_entity_navigation_ownership(
        &self,
        live: &LiveEntities,
        id: EntityId,
        navigation: &mut CollisionWorld,
    ) -> Result<(), String> {
        if live.get(id).is_none() {
            return Err("navigation admission entity generation stale".into());
        }
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("navigation admission scene header absent")?;
        let entity = self
            .entities
            .get(index)
            .ok_or("navigation admission scene slot absent")?;
        for &packed in entity.components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            if self.is_rigid_class(handle.class_id)
                && self.rigid_component(handle)?.body().is_some()
            {
                navigation.mark_physics_owned(&entity.name);
                break;
            }
        }
        Ok(())
    }
}
