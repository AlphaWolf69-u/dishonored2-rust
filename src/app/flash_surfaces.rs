//! World FlashSurface callbacks over the canonical component and movie owners.
use crate::gameplay::{
    component_state::entity::ComponentHandle,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
};
use bevy::prelude::World;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pub entity: EntityId,
    pub handle: ComponentHandle,
    pub generation: u32,
}
#[derive(Clone, Copy)]
pub(super) enum CookingCommand { Initialize { value: f32, kind: i32 }, Update(f32) }
impl std::hash::Hash for Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.entity, state);
        std::hash::Hash::hash(&self.handle.packed(), state);
        std::hash::Hash::hash(&self.generation, state);
    }
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    super::flash_surface_movie::poll(world)?;
    if !world
        .get_resource::<super::physics_scene::PhysicsScene>()
        .is_some_and(|s| s.has_flash_surfaces())
    {
        return Ok(());
    }
    let dt = world
        .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .filter(|c| c.advanced())
        .map(|c| c.clock().map(|c| c.delta(2)))
        .transpose()?
        .flatten()
        .unwrap_or(0.);
    //Keep the large Scene in its canonical resource slot. Only the small
    //generational command/status crosses the movie-owner borrow boundary.
    let mut position = 0;
    loop {
        let Some(key) = world
            .resource::<super::physics_scene::PhysicsScene>()
            .flash_update_key(position)?
        else {
            break;
        };
        position += 1;
        if world.resource::<LiveEntities>().get(key.entity).is_none() {
            return Err("world FlashSurface entity generation stale".into());
        }
        //0310c0 releases playback on ordinary Stop, not only on eventual
        //component destruction. Re-enable must await this exact real ACK.
        if world.resource::<super::physics_scene::PhysicsScene>().flash_stopped_pending(key)? {
            if super::flash_surface_movie::status(world,key)?.is_some_and(|status|status.retired) {
                super::flash_surface_movie::forget(world,key)?;
                world.resource_mut::<super::physics_scene::PhysicsScene>().flash_stopped_acknowledged(key)?;
            }
            continue;
        }
        let request = world
            .resource::<super::physics_scene::PhysicsScene>()
            .flash_request(key)?;
        let Some(request) = request else { continue };
        if let Some((definition, requires_configuration)) = request {
            super::runtime_entities::with_scene(world, |world, scene, live, _| {
                scene.set_runtime_actor_visible(world, live, key.entity, false)
            })?;
            super::flash_surface_movie::request(world, key, definition, requires_configuration)?;
            world
                .resource_mut::<super::physics_scene::PhysicsScene>()
                .flash_requested(key)?;
        }
        let Some(status) = super::flash_surface_movie::status(world, key)? else {
            continue;
        };
        let (show, stop) =
            world.resource_scope(|world, mut events: bevy::prelude::Mut<EntityEvents>| {
                world
                    .resource_mut::<super::physics_scene::PhysicsScene>()
                    .observe_flash_status(key, status, dt, &mut events)
            })?;
        if show {
            super::runtime_entities::with_scene(world, |world, scene, live, _| {
                if std::env::var_os("DH2_DEBUG_ACTION_SCENARIO").is_some() {
                    let was_visible = scene.runtime_actor_visible(live, key.entity)?;
                    eprintln!("ACTION_SMOKE_WORLD_MOVIE_FIRST_DRAW entity={:?} generation={} previously_visible={was_visible} configured={} target_rendered_and_bound={}", key.entity, key.generation, status.configured, status.captured);
                }
                scene.set_runtime_actor_visible(world, live, key.entity, true)?;
                scene.acknowledge_flash_visibility(key)
            })?;
        }
        if stop {
            super::flash_surface_movie::stop(world, key)?;
            world
                .resource_mut::<super::physics_scene::PhysicsScene>()
                .flash_static_released(key)?;
        }
    }
    Ok(())
}

///After the shared199 callback pump, before renderer extraction. `_C_Show`
///must populate the same target that is about to be published this frame.
pub(super) fn finish(world: &mut World) -> Result<(), String> {
    let mut position = 0;
    while let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() {
        let Some(key) = scene.flash_update_key(position)? else {
            break;
        };
        position += 1;
        if let Some(map) = scene.flash_pending_map(key)? {
            super::flash_surface_movie::show_map(world, key, map)?;
            world
                .resource_mut::<super::physics_scene::PhysicsScene>()
                .flash_map_submitted(key)?;
        }
        let command = world.resource::<super::physics_scene::PhysicsScene>().flash_pending_cooking(key)?;
        if let Some(command) = command {
            if super::flash_surface_movie::cooking_command(world, key, command)? {
                world.resource_mut::<super::physics_scene::PhysicsScene>().flash_cooking_submitted(key)?;
            }
        }
    }
    //UI playback shares the existing real-time movie clock. Only the
    //component's static-capture countdown uses GameClock group2 above.
    let dt = world
        .get_resource::<bevy::prelude::Time<bevy::time::Real>>()
        .map_or(0., |time| time.delta_secs());
    super::flash_surface_movie::tick(world, dt)
}
pub(super) fn ready(world: &World) -> bool {
    world
        .get_resource::<super::physics_scene::PhysicsScene>()
        .is_none_or(|scene| scene.flash_surfaces_ready())
}
pub(super) fn write_diagnostics(world: &World, report: &mut String) {
    if let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() {
        scene.write_flash_surface_diagnostics(report);
    }
}
