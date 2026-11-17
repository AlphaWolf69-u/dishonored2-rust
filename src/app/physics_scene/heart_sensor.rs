//!76e8b0/76eaf0 register and remove actual started Pickable Rune/Charm rows.
use super::*;
impl PhysicsScene {
    pub(in crate::app) fn heart_pickable_started(
        &self,
        world: &mut bevy::prelude::World,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[6] {
            return Err("RuneCharms Start requires actual Pickable class".into());
        }
        let component = self
            .pickables
            .get(handle.slot)
            .ok_or("RuneCharms Start Pickable absent")?;
        let Some(item) = &component.item else {
            return Ok(());
        }; //Source nullable inventory declaration.
        let index = component
            .owner
            .entity
            .ok_or("RuneCharms Pickable owner absent")?;
        let entity = self
            .entities
            .get(index)
            .ok_or("RuneCharms Pickable entity absent")?;
        super::super::heart::register_pickable(
            world,
            entity.shared,
            item.type_number,
            item.hud_index,
            component.definition.invisible_npc_loot,
        );
        Ok(())
    }
    pub(in crate::app) fn heart_pickable_stopped(
        &self,
        world: &mut bevy::prelude::World,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[6] {
            return Err("RuneCharms Stop requires actual Pickable class".into());
        }
        let component = self
            .pickables
            .get(handle.slot)
            .ok_or("RuneCharms Stop Pickable absent")?;
        let Some(item) = &component.item else {
            return Ok(());
        };
        if !matches!(item.type_number, 5 | 7 | 8) {
            return Ok(());
        }
        let index = component
            .owner
            .entity
            .ok_or("RuneCharms stopped Pickable owner absent")?;
        let entity = self
            .entities
            .get(index)
            .ok_or("RuneCharms stopped Pickable entity absent")?;
        super::super::heart::unregister_pickable(world, entity.shared);
        Ok(())
    }
    ///bevy_backend: CPU startup has no published audio/view owner. Transfer
    ///the already-started registration roster once when its real Player binds.
    ///Subsequent transitions use the exact Start/Stop hooks above.
    pub(in crate::app) fn seed_heart_pickables(
        &self,
        world: &mut bevy::prelude::World,
    ) -> Result<(), String> {
        for component in self.pickables.iter() {
            if component.owner.flags & 12 != 8 {
                continue;
            }
            let Some(item) = &component.item else {
                continue;
            };
            let index = component
                .owner
                .entity
                .ok_or("RuneCharms seeded Pickable owner absent")?;
            let entity = self
                .entities
                .get(index)
                .ok_or("RuneCharms seeded Pickable entity absent")?;
            super::super::heart::register_pickable(
                world,
                entity.shared,
                item.type_number,
                item.hud_index,
                component.definition.invisible_npc_loot,
            );
        }
        Ok(())
    }
}
