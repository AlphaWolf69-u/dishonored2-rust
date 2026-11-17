//! Actual AIDangerZone pool payload and41e090/41e220/41e330/41e1d0 lifecycle.
use super::*;
use crate::gameplay::{ai_world::{DangerZones, Worlds}, entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities}, pickable_component::CheckedComponent};
use bevy::prelude::World;

impl PhysicsScene {
    fn ai_danger_identity(&self, handle: ComponentHandle) -> Result<(EntityId, CheckedComponent), String> {
        if handle.class_id != self.classes[20] { return Err("AI danger component class differs".into()); }
        let component = self.ai_danger_zones.get(handle.slot).ok_or("AI danger payload absent")?;
        let index = component.owner.entity.ok_or("AI danger component has no parent")?;
        let id = self.entities.get(index).ok_or("AI danger parent absent")?.shared;
        Ok((id, CheckedComponent { handle, generation: component.owner.generation }))
    }
    pub(super) fn initialize_ai_danger(&mut self, world: &mut World, live: &LiveEntities, handle: ComponentHandle) -> Result<(), String> {
        let (id, checked) = self.ai_danger_identity(handle)?;
        if live.get(id).is_none() { return Err("AI danger parent identity is stale".into()); }
        if self.ai_danger_zones.get(handle.slot).unwrap().initialized { return Ok(()); }
        let mut worlds = world.get_resource_mut::<Worlds>().ok_or("AI danger Initialize has no canonical AIWorld list")?;
        if worlds.danger(id)?.is_some() { return Err("AIWorld already has its actual Danger component".into()); }
        worlds.set_danger(id, Some(checked))?;
        let component = self.ai_danger_zones.get_mut(handle.slot).unwrap();
        component.owner.mark_initialized();
        component.initialized = true;
        Ok(())
    }
    pub(super) fn start_ai_danger(&mut self, handle: ComponentHandle, events: &mut EntityEvents) -> Result<(), String> {
        let (id, checked) = self.ai_danger_identity(handle)?;
        let component = self.ai_danger_zones.get_mut(handle.slot).unwrap();
        if !component.initialized { return Err("AI danger Start precedes Initialize".into()); }
        if !component.started { super::super::ai_world::bind(id, checked, events); component.started = true; }
        Ok(())
    }
    pub(super) fn stop_ai_danger(&mut self, handle: ComponentHandle, events: &mut EntityEvents) -> Result<(), String> {
        let (id, checked) = self.ai_danger_identity(handle)?;
        let component = self.ai_danger_zones.get_mut(handle.slot).unwrap();
        if component.started { super::super::ai_world::unbind(id, checked, events); component.started = false; }
        Ok(())
    }
    pub(super) fn deinitialize_ai_danger(&mut self, world: &mut World, handle: ComponentHandle) -> Result<(), String> {
        let (id, checked) = self.ai_danger_identity(handle)?;
        let component = self.ai_danger_zones.get_mut(handle.slot).unwrap();
        if component.started { return Err("AI danger Deinitialize precedes Stop".into()); }
        if component.initialized {
            let mut worlds = world.get_resource_mut::<Worlds>().ok_or("AI danger Deinitialize has no canonical AIWorld list")?;
            let stored = worlds.danger(id)?.ok_or("AI danger parent binding disappeared")?;
            if stored.handle != checked.handle || stored.generation != checked.generation { return Err("AI danger parent binding was replaced".into()); }
            worlds.set_danger(id, None)?;
            component.initialized = false;
        }
        Ok(())
    }
    pub(in crate::app) fn ai_danger_rows(&mut self, live: &LiveEntities, id: EntityId,
        checked: CheckedComponent) -> Result<&mut DangerZones, String> {
        let (parent, actual) = self.ai_danger_identity(checked.handle)?;
        if parent != id || actual.generation != checked.generation || live.get(id).is_none() { return Err("AI danger callback owner is stale".into()); }
        let component = self.ai_danger_zones.get_mut(checked.handle.slot).unwrap();
        if !component.started { return Err("AI danger callback has no started receiver".into()); }
        Ok(&mut component.zones)
    }
    pub(in crate::app) fn resume_ai_danger(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        if self.owner(handle)?.flags as u8 & 0x30 > 0x1f { registry.resume_updates(handle, self)?; }
        Ok(())
    }
    pub(in crate::app) fn update_ai_danger_sources(&mut self, live: &LiveEntities) -> Result<(), String> {
        let mut registry = self.registry.take().ok_or("AI danger update registry is borrowed")?;
        let result = (|| {
            for index in 0..self.ai_danger_zones.roster.sorted_start as usize {
                let slot = self.ai_danger_zones.roster.indices[index];
                let Some(component) = self.ai_danger_zones.get_mut(slot).filter(|component| component.started) else { continue };
                component.zones.retire_stale(live);
                if component.zones.spheres().is_empty() {
                    registry.pause_updates(ComponentHandle { class_id: self.classes[20], slot }, self)?;
                }
            }
            Ok(())
        })();
        self.registry = Some(registry);
        result
    }
}
