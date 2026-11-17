//! Throwable subtype hooks over the one real rigid body and entity registry.
//! Pool/factory admission is PhysicsScene's responsibility; no second body or
//! per-projectile physics world is created here.
use crate::gameplay::{
    component_state::entity::ComponentHandle,
    entity_events::{EntityEvents, Listener},
    entity_lifetime::EntityId,
};
use std::sync::Arc;
pub(super) struct Prepared {
    pub definition: Arc<crate::assets::throwable::Definition>,
    pub resources: super::rigid_body::RigidBodyResources,
}
impl Prepared {
    /// Each source component takes its own shape-reference/material-table
    /// lease. Geometry payload and resource wrappers are shared unchanged.
    pub(super) fn instantiate_resources(&self) -> super::rigid_body::RigidBodyResources {
        self.resources.clone_refs()
    }
}
///4c7fc0 installs125 only after the real base Start has completed. Retrying
/// an already registered same component is idempotent; other owners reject.
pub(super) fn start_listener(
    component: &mut super::rigid_body::RigidBodyComponent,
    events: &mut EntityEvents,
    owner: usize,
    handle: ComponentHandle,
    entity: EntityId,
) -> Result<(), String> {
    let generation = component.owner.generation;
    let state = component.throwable_mut()?;
    let expected = (owner, handle, generation, entity);
    if let Some(previous) = state.listener() {
        return if previous == expected {
            Ok(())
        } else {
            Err("Throwable listener identity changed".into())
        };
    }
    events.register(
        0x125,
        Some(entity),
        Listener::Component {
            owner,
            handle,
            generation,
            handler: 0x125,
        },
    );
    state.set_listener(Some(expected));
    Ok(())
}
///4c8140 removes exactly125 before80a260 unpublishes the base body's state.
pub(super) fn stop_listener(
    component: &mut super::rigid_body::RigidBodyComponent,
    events: &mut EntityEvents,
) -> Result<(), String> {
    let state = component.throwable_mut()?;
    if let Some((owner, handle, generation, entity)) = state.listener() {
        events.unregister(
            0x125,
            Some(entity),
            Listener::Component {
                owner,
                handle,
                generation,
                handler: 0x125,
            },
        );
        state.set_listener(None);
    }
    state.clear_contacts();
    Ok(())
}

///4c7930 follows the real base Initialize, using its actual selected shape and
/// motion property. No render bounds or material-derived substitute is used.
pub(super) fn initialize<G: crate::gameplay::physics_world::body_creation::BodyCreationGeometry>(
    component: &mut super::rigid_body::RigidBodyComponent,
    storage: &mut crate::gameplay::physics_world::storage::WorldStorage,
    geometry: &mut G,
    timespace: usize,
    contact_damage_receiver: bool,
) -> Result<(), String> {
    if component.throwable()?.initialized {
        return Ok(());
    }
    let body_id = component
        .body()
        .ok_or("Throwable has no initialized real body")?;
    let body = storage
        .bodies
        .get(body_id)
        .ok_or("Throwable body identity is stale")?;
    let selected = body.selected_shape(3)?;
    let bounds = geometry.bounds(
        selected,
        [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ],
    )?;
    let center = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
    let damping = component
        .throwable()?
        .source
        .settings
        .damping_after_first_hit;
    let property = if damping > 0. && body.flags & 5 == 0 {
        if !storage.motion_properties.added.is_empty() {
            return Err("Throwable damping profile requires its real added-signal observer".into());
        }
        let motion = storage
            .motions
            .get(body.motion_id)
            .ok_or("Throwable dynamic motion is absent")?;
        let mut words = *storage
            .motion_properties
            .get(motion.properties_id)
            .ok_or("Throwable motion profile is absent")?
            .words();
        //4c7930 clears source flag26, speed limiter metadata, and supplies
        //1.0 at+0c (DAT141f697b0), then its authored angular damping+1c.
        words[0] = 0;
        words[1] &= !0x0400_0000;
        words[3] = 1.0f32.to_bits();
        words[7] = damping.to_bits();
        let original = storage.motion_properties.add(
            crate::gameplay::motion_properties::MotionProfile::from_words(words),
            |_, _, _, _| unreachable!("motion-property observers checked absent"),
        )?;
        Some(
            storage
                .motion_timespaces
                .as_mut()
                .ok_or("Throwable physics timespaces are absent")?
                .resolve(
                    &mut storage.motion_properties,
                    original,
                    timespace,
                    |_, _, _, _| unreachable!("motion-property observers checked absent"),
                )?,
        )
    } else {
        None
    };
    let state = component.throwable_mut()?;
    state.center = center;
    state.damping_profile = property;
    state.set_contact_damage_receiver(contact_damage_receiver);
    state.initialized = true;
    Ok(())
}
///4c7fc0/4c8f00/4c88e0. The source base Start/behavior transition must
/// have finished before this call, so a dormant body is never implicitly woken.
pub(super) fn configure_body<
    G: crate::gameplay::physics_world::body_creation::BodyCreationGeometry,
    E: crate::gameplay::physics_world::PublicationEvents
        + crate::gameplay::physics_world::body_creation::BodyCreationCallbacks,
>(
    component: &mut super::rigid_body::RigidBodyComponent,
    storage: &mut crate::gameplay::physics_world::storage::WorldStorage,
    geometry: &mut G,
    events: &mut E,
    quality_enabled: bool,
) -> Result<(), String> {
    let id = component
        .body()
        .ok_or("Throwable policy has no real body")?;
    let quality = component.throwable()?.quality(quality_enabled);
    if let Some(quality) = quality {
        storage.set_body_quality(geometry, events, id, quality, 0)?;
    }
    component.throwable_mut()?.quality_applied(quality_enabled);
    let flags = storage
        .bodies
        .get(id)
        .ok_or("Throwable policy body retired")?
        .flags;
    let mode = if flags & 1 != 0 {
        0
    } else if flags & 4 != 0 {
        1
    } else {
        2
    };
    let contact = component.throwable()?.contact_events(mode);
    storage.set_body_user_flags(geometry, events, id, 0x40, contact, 0)?;
    Ok(())
}

///125 ->4c9980 ->4c7570 observes the actual managed binding AFTER875e80
/// or8762b0. It never manufactures a parent or motion transition.
pub(super) fn binding_changed<
    G: crate::gameplay::physics_world::body_creation::BodyCreationGeometry,
    E: crate::gameplay::physics_world::PublicationEvents
        + crate::gameplay::physics_world::body_creation::BodyCreationCallbacks,
>(
    component: &mut super::rigid_body::RigidBodyComponent,
    storage: &mut crate::gameplay::physics_world::storage::WorldStorage,
    geometry: &mut G,
    events: &mut E,
    bound: bool,
) -> Result<bool, String> {
    let state = component.throwable()?;
    if !state.stuck() && !state.collision_update_pending() {
        return Ok(false);
    }
    let mut settings = state.source.rigid.settings.clone();
    if bound {
        settings.collision = Some(state.source.settings.attached_collision);
    }
    let filter = settings
        .collision_filter()
        .ok_or("Throwable binding collision filter is invalid")?;
    let body = component
        .body()
        .ok_or("Throwable binding has no actual body")?;
    storage.set_body_filter(geometry, events, body, filter)?;
    if !bound {
        component.throwable_mut()?.detached();
    }
    Ok(!bound)
}
