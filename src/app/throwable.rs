//! Throwable subtype hooks over the one real rigid body and entity registry.
//! Pool/factory admission is PhysicsScene's responsibility; no second body or
//! per-projectile physics world is created here.
use crate::gameplay::{
    component_state::entity::ComponentHandle,
    entity_events::{EntityEvents, Listener},
    entity_lifetime::EntityId,
};
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
