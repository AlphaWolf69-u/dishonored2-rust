//! SoundVolumeControl actual738b30 Initialize, before7e7670 base Start.
use super::*;
use crate::gameplay::entity_lifetime::LiveEntities;
impl PhysicsScene {
    pub(super) fn initialize_sound_volume(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &LiveEntities,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[17] {
            return Err("SoundVolumeControl class differs".into());
        }
        let component = self
            .sound_volumes
            .get(handle.slot)
            .ok_or("SoundVolumeControl initialization slot absent")?;
        let index = component
            .owner
            .entity
            .ok_or("SoundVolumeControl actual entity absent")?;
        let entity = self
            .entities
            .get(index)
            .ok_or("SoundVolumeControl entity header absent")?;
        if entity.retired
            || self.shared_indices.get(&entity.shared) != Some(&index)
            || live.get(entity.shared).is_none()
        {
            return Err("SoundVolumeControl entity generation stale".into());
        }
        let bounds = super::super::world_models::entity_bounds(world, live, entity.shared)?
            .ok_or("SoundVolumeControl738bf0 requires its actual render model bounds")?;
        let acoustic = world
            .get_resource::<super::super::audio::level::LevelAudio>()
            .ok_or("SoundVolumeControl initialized before the canonical acoustic world")?
            .acoustic_world();
        let mut acoustic = acoustic
            .lock()
            .map_err(|_| "SoundVolumeControl acoustic world poisoned")?;
        let component = self
            .sound_volumes
            .get_mut(handle.slot)
            .ok_or("SoundVolumeControl slot retired during initialization")?;
        let definition = component
            .definition
            .as_ref()
            .ok_or("SoundVolumeControl typed definition absent")?;
        let flags = component.owner.flags;
        let restored = flags & (1 << 30) != 0;
        component.owner.mark_initialized(); //738b30 begins with7e7400.
        if let Err(error) = component
            .state
            .initialize(definition, &mut acoustic, bounds, restored)
        {
            // Host fallibility must not publish a successful initialization
            //ack before the real zone/strength owner accepted its operation.
            component.owner.flags = flags;
            return Err(error);
        }
        Ok(())
    }
}
