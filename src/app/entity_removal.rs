//! GameLocal8ab380's ordinary removal FIFO. LiveEntities retains the sole
//! active batch; this host state stores callback receipts, never another queue.
use crate::gameplay::{
    component_state::entity::ComponentHandle,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
    entity_release::{EntityReleaseBatchWorld, EntityReleaseWorld},
};
use bevy::prelude::*;

#[derive(Resource, Default)]
pub(super) struct EntityRemoval {
    components: super::physics_scene::lifetime::Retirement,
}

/// Publish with the other level owners so world shutdown owns this cursor.
pub(super) fn initialize(world: &mut World) {
    world.init_resource::<EntityRemoval>();
}

///Resource owners disposing an ordinary actor must enter8794b0/8ab380's
///actual FIFO, including save-name preparation, attachment/component release
///and final managed-ID acknowledgement. Never call the batch's final-retire
///callback directly after only draining components. The sole existing batch
///retains every pending cursor; this helper creates no second removal queue.
pub(super) fn request_and_wait(world: &mut World, entity: EntityId) -> Result<bool, String> {
    let mut live = world.get_resource_mut::<LiveEntities>()
        .ok_or("entity removal managed owner is absent")?;
    if live.get(entity).is_none() { return Ok(true); }
    live.request_removal(entity)?;
    drop(live);
    update(world)?;
    Ok(world.resource::<LiveEntities>().get(entity).is_none())
}

fn with_scene<T>(
    world: &mut World,
    callback: impl FnOnce(
        &mut World,
        &mut super::physics_scene::PhysicsScene,
        &mut LiveEntities,
        &mut EntityEvents,
    ) -> Result<T, String>,
) -> Result<T, String> {
    let mut scene = world
        .remove_resource::<super::physics_scene::PhysicsScene>()
        .ok_or("entity removal scene owner is absent")?;
    let Some(mut live) = world.remove_resource::<LiveEntities>() else {
        world.insert_resource(scene);
        return Err("entity removal managed owner is absent".into());
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("entity removal event owner is absent".into());
    };
    let result = callback(world, &mut scene, &mut live, &mut events);
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    result
}
struct Host<'a> {
    world: &'a mut World,
    components: &'a mut super::physics_scene::lifetime::Retirement,
}
impl EntityReleaseWorld for Host<'_> {
    fn child_count(&self, entity: EntityId) -> Result<usize, String> {
        Ok(self
            .world
            .resource::<LiveEntities>()
            .children(entity)?
            .len())
    }
    fn child_at(&self, entity: EntityId, index: usize) -> Result<EntityId, String> {
        self.world
            .resource::<LiveEntities>()
            .children(entity)?
            .get(index)
            .copied()
            .ok_or_else(|| "entity removal child index is absent".into())
    }
    fn detach_child(&mut self, entity: EntityId, child: EntityId) -> Result<(), String> {
        if self.world.resource::<LiveEntities>().parent(child)? != Some(entity) {
            return Err("entity removal child parent differs".into());
        }
        if super::equipment_identity::runtime_flags(self.world, child).is_some() {
            super::equipment_identity::detach(self.world, child)
        } else {
            with_scene(self.world, |_, scene, live, events| {
                scene.detach_removal_child(live, events, entity, child)
            })
        }
    }
    fn component_count(&self, entity: EntityId) -> Result<usize, String> {
        self.world
            .resource::<super::physics_scene::PhysicsScene>()
            .removal_component_count(self.world.resource::<LiveEntities>(), entity)
    }
    fn component_at(
        &self,
        entity: EntityId,
        index: usize,
    ) -> Result<(ComponentHandle, u32), String> {
        self.world
            .resource::<super::physics_scene::PhysicsScene>()
            .removal_component_at(self.world.resource::<LiveEntities>(), entity, index)
    }
    fn request_component_release(
        &mut self,
        entity: EntityId,
        component: ComponentHandle,
        generation: u32,
    ) -> Result<(), String> {
        with_scene(self.world, |world, scene, live, events| {
            scene.request_removal_component(
                world,
                live,
                events,
                entity,
                component,
                generation,
                self.components,
            )
        })
    }
    fn retire_entity(&mut self, entity: EntityId) -> Result<(), String> {
        if self.poll_retire_entity(entity)? {
            Ok(())
        } else {
            Err("entity removal awaits actual owner acknowledgements".into())
        }
    }
    fn poll_retire_entity(&mut self, entity: EntityId) -> Result<bool, String> {
        with_scene(self.world, |_, scene, live, _| {
            scene.finish_requested_removal(live, entity, self.components)
        })?;
        //Renderer/audio/graph callbacks remain resumable in their canonical
        //owner. Identity reclamation follows their actual completion only.
        if !super::runtime_entities::retire_components(self.world, entity)? {
            return Ok(false);
        }
        super::entity_functions::retire_entity(self.world, entity);
        super::runtime_entities::retire(self.world, entity)
    }
}
impl EntityReleaseBatchWorld for Host<'_> {
    fn prepare_entity_release(&mut self, entity: EntityId) -> Result<(), String> {
        let flags =
            if let Some(flags) = super::equipment_identity::runtime_flags(self.world, entity) {
                flags
            } else {
                self.world
                    .resource::<super::physics_scene::PhysicsScene>()
                    .removal_runtime_flags(self.world.resource::<LiveEntities>(), entity)?
            };
        self.world
            .resource_mut::<LiveEntities>()
            .prepare_entity_release(entity, flags)
    }
    fn drain_entity_component_releases(&mut self) -> Result<(), String> {
        if self.poll_entity_component_releases()? {
            Ok(())
        } else {
            Err("entity component release awaits actual host completion".into())
        }
    }
    fn poll_entity_component_releases(&mut self) -> Result<bool, String> {
        with_scene(self.world, |world, scene, live, events| {
            super::physics_scene::lifetime::poll_requested_components(
                world,
                scene,
                live,
                events,
                self.components,
            )
        })
    }
}

/// After delayed entity functions, before source entity-slot reuse. Pending
/// GPU/audio callbacks keep the same LiveEntities batch through later frames.
pub(super) fn update(world: &mut World) -> Result<(), String> {
    // 841030 delivers inventory and named pickedUp callbacks synchronously.
    // A retained host callback must finish before the ordinary destructor
    // batch can stop its recipients or reuse their managed identities.
    if super::pickups::accepted_pending(world)||super::script_spawner::startup_pending(world) {
        return Ok(());
    }
    let Some(mut state) = world.remove_resource::<EntityRemoval>() else {
        return Err("entity removal owner was not initialized".into());
    };
    let result = (|| {
        if !world.resource::<LiveEntities>().active_removal_pending() {
            //The accepted ordinary7ea870 drain uses its existing Scene
            //cursor. Complete it before8ab380 takes this frame's snapshot;
            //unrelated retained actor receipts stay with their real owner.
            if !with_scene(world, |world, scene, live, events| {
                super::physics_scene::lifetime::prepare_entity_removal(
                    world,
                    scene,
                    live,
                    events,
                    &mut state.components,
                )
            })? {
                return Ok(());
            }
        }
        let Some(mut batch) = world
            .resource_mut::<LiveEntities>()
            .take_active_removal_batch()?
        else {
            return Ok(());
        };
        let advanced = batch.poll_advance(&mut Host {
            world,
            components: &mut state.components,
        });
        let completed = matches!(advanced, Ok(true));
        //Restore the source batch even when a real callback fails.
        world
            .resource_mut::<LiveEntities>()
            .finish_active_removal_batch(batch, completed)?;
        match advanced {
            Ok(true) => state.components = Default::default(),
            Ok(false) => return Ok(()),
            Err(error) => return Err(error),
        }
        Ok(())
    })();
    world.insert_resource(state);
    result
}

/// Whole-world destruction may finish the already accepted ordinary snapshot,
/// but does not admit newly queued removals into that different shutdown path.
pub(super) fn finish_accepted(world: &mut World) -> Result<bool, String> {
    if !world
        .get_resource::<LiveEntities>()
        .is_some_and(LiveEntities::active_removal_pending)
    {
        return Ok(true);
    }
    update(world)?;
    Ok(!world.resource::<LiveEntities>().active_removal_pending())
}
