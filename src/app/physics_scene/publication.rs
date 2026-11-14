//! Borrow each canonical publication owner once; callbacks may reach the same
//! live component pools without reconstructing a query world or flag shadow.
macro_rules! scene_publication_events {
    ($scene:ident, $registry:expr, $events:ident) => {
        let d4 = &mut $scene.entity_flags_d4;
        let post_task = &$scene.post_updates;
        let post_frame = $scene.post_frame;
        let mut entity_activation = |entity: usize,
            component: &mut crate::app::rigid_body::RigidBodyComponent,
            active: bool, flags: u32, registry: &mut ComponentRegistry,
            actors: &mut PhysicsActorLinks| -> Result<(), String> {
            let movement = component.movement_body()?;
            let flags_d4 = d4.get_mut(entity).ok_or("rigid activation entity flags are absent")?;
            component.on_body_activation(active, flags, flags_d4, registry, || {
                let actor = actors.bodies.get(movement).and_then(|body| body.actor)
                    .ok_or("rigid activation reference actor is absent")?;
                actors.request_phase_update(actor, post_frame,
                    crate::gameplay::physics_actor::ActorUpdatePhase::PostPhysics, |actors, actor| {
                        let generation = actors.actors.get_at(actor).ok_or("queued actor is absent")?.owner.generation;
                        post_task.updates.enqueue(crate::gameplay::physics_world::update_queue::ObjectReference::capture(Some((generation, actor))))?;
                        Ok(())
                    })?;
                Ok(())
            })
        };
        let mut observer = startup::WorldActivationObserver { owner: $scene.owner_id,
            rigid_class: $scene.classes[8], rigid: &mut $scene.rigid_bodies,
            body_events: &mut $scene.body_activation_events, registry: &mut *$registry,
            entity_activation: &mut entity_activation };
        let mut $events = ComponentPublicationEvents { components: TriggerComponentStore {
            trigger_class: $scene.classes[0], water_class: $scene.classes[1],
            triggers: &mut $scene.triggers, waters: &mut $scene.waters,
            actors: &mut $scene.actors, deferred: &mut $scene.deferred },
            signals: &mut $scene.signals,
            dispatch: |_, subscriber, callback, owners, event, components| {
                observer.dispatch(subscriber, callback, owners, event, components)
            } };
    };
}
