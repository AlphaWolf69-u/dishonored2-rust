//! Actual entity Enable87caa0/Disable87cd60, distinct from875290 Activate.
use super::*;
use crate::gameplay::{
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
    game_clock_runtime::ClockRuntime,
};
use bevy::prelude::World;
impl PhysicsScene {
    pub(in crate::app) fn set_entity_enabled(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        enabled: bool,
    ) -> Result<(), String> {
        live.get(id)
            .ok_or("entity Enable/Disable generation stale")?;
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("entity Enable/Disable header absent")?;
        let was_disabled = self.entities[index].entity_flags & 0x100;
        let was_deferred = self.entities[index].components.start_deferred;
        if (self.entities[index].entity_flags & 0x100 == 0) == enabled {
            return Ok(());
        }
        let handles = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .map(|&word| ComponentHandle::from_packed(word))
            .collect::<Result<Vec<_>, _>>()?;
        let mut registry = self
            .registry
            .take()
            .ok_or("entity Enable/Disable registry borrowed")?;
        let result = (|| {
            if enabled {
                //875cf0 resets only selector+2ac before87c500 queues matching
                //Start/Stop; +2a0 and the source root countdown remain retained.
                let (group, offset) = {
                    let mut clock = world
                        .get_resource_mut::<ClockRuntime>()
                        .ok_or("entity Enable clock owner absent")?;
                    clock.reset_entity_timespace(id)?;
                    clock.entity_model_clock(id).map_err(str::to_owned)?
                };
                super::super::model_motion::set_shader_clock_with_live(
                    world, live, id, group, offset,
                )?;
                self.entities[index].timespace = 0;
                self.entities[index].components.start_deferred = false;
                let context = live.entity_context(id)?.requested;
                let mut starts_movie = false;
                for handle in handles {
                    let scheduling = &self
                        .declarations
                        .get(&handle.packed())
                        .ok_or("enabled component declaration absent")?
                        .scheduling;
                    if scheduling.start_phases >> (context & 31) & 1 == 0 {
                        registry.request_stop(
                            handle,
                            &mut super::interaction_components::Startup {
                                world: Some(world),
                                scene: self,
                                live,
                                events,
                            },
                        )?;
                    } else if scheduling.starts_on_spawn() && self.owner(handle)?.flags & 3 != 3 {
                        starts_movie |= self.is_surface_class(handle.class_id);
                        registry.request_start(
                            self.owner_mut(handle)?,
                            handle.packed() as usize,
                            false,
                            |_, _| Ok(()),
                        )?;
                    }
                }
                // FlashSurface Start (4b0d70) hides the actor while the movie
                // is pending. Enabling it must not expose its default quad.
                self.set_runtime_actor_visible(world, live, id, !starts_movie)?;
                self.entities[index].entity_flags &= !0x100;
            } else {
                self.set_runtime_actor_visible(world, live, id, false)?;
                for handle in handles {
                    registry.request_stop(
                        handle,
                        &mut super::interaction_components::Startup {
                            world: Some(world),
                            scene: self,
                            live,
                            events,
                        },
                    )?;
                }
                self.entities[index].entity_flags |= 0x100;
                self.entities[index].components.start_deferred = true;
            }
            //The same accepted registry owns actual callbacks; no second queue
            //or flag-only component stand-in acknowledges an Enable/Disable.
            self.run_runtime_startup(world, live, events, &mut registry)
        })();
        self.registry = Some(registry);
        if result.is_err() {
            //A failed synchronous host callback retains its registry cursor.
            //Keep the public enabled predicate pending so the accepted source
            //caller retries that cursor before issuing its following notice.
            self.entities[index].entity_flags =
                (self.entities[index].entity_flags & !0x100) | was_disabled;
            self.entities[index].components.start_deferred = was_deferred;
        }
        result
    }
}
