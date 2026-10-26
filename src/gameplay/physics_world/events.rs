//! World observer signals share the component owner used by body publication.
use super::body_creation::BodyCreationCallbacks;
use super::{MotionCreationCallbacks, PublicationEvents};
use crate::assets::physworld::body::BodyInfo;
use crate::gameplay::{
    body_activation::{ActivationOwners, ActivationSignals},
    collision_user_data::velocity::{CollisionVelocities, VelocityDataStore},
    component_state::entity::ComponentHandle,
    motion_info::MotionInfo,
    signal::Signal,
    trigger_body::{TriggerComponentStore, TriggerDataStore},
    water_component::WaterComponent,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BodyEventCallback {
    TriggerContact,
}

pub struct PublicationSignals<S, C> {
    pub activation: ActivationSignals<S, C>,
    pub added: Signal<S, C>,
    pub static_transform: Signal<S, C>,
    pub transform: Signal<S, C>,
    pub motion_capacity_requested: Signal<S, C>,
    pub motion_created: Signal<S, C>,
    pub body_capacity_requested: Signal<S, C>,
    pub body_created: Signal<S, C>,
}
impl<S, C> Default for PublicationSignals<S, C> {
    fn default() -> Self {
        Self {
            activation: ActivationSignals::default(),
            added: Signal::default(),
            static_transform: Signal::default(),
            transform: Signal::default(),
            motion_capacity_requested: Signal::default(),
            motion_created: Signal::default(),
            body_capacity_requested: Signal::default(),
            body_created: Signal::default(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum PublicationEvent<'a> {
    Activation { ids: &'a [u32], active: bool },
    Added(&'a [u32]),
    Transform { body: u32, static_only: bool },
    MotionCapacityRequested,
    MotionCreated { info: &'a MotionInfo, motion: u32 },
    BodyCapacityRequested,
    BodyCreated { info: &'a BodyInfo, body: u32 },
}

impl<'a, S: Copy + Eq, C: Copy + Eq, F> BodyCreationCallbacks
    for ComponentPublicationEvents<'a, S, C, F>
where
    F: FnMut(
        &mut Signal<S, C>,
        S,
        C,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'a>,
    ) -> Result<(), String>,
{
    fn body_capacity_requested(&mut self, owners: ActivationOwners<'_>) -> Result<(), String> {
        emit(
            &mut self.signals.body_capacity_requested,
            &mut self.components,
            &mut self.dispatch,
            owners,
            PublicationEvent::BodyCapacityRequested,
        )
    }
    fn body_created(
        &mut self,
        owners: ActivationOwners<'_>,
        info: &BodyInfo,
        body: u32,
    ) -> Result<(), String> {
        emit(
            &mut self.signals.body_created,
            &mut self.components,
            &mut self.dispatch,
            owners,
            PublicationEvent::BodyCreated { info, body },
        )
    }
}

impl<'a, S: Copy + Eq, C: Copy + Eq, F> MotionCreationCallbacks
    for ComponentPublicationEvents<'a, S, C, F>
where
    F: FnMut(
        &mut Signal<S, C>,
        S,
        C,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'a>,
    ) -> Result<(), String>,
{
    fn motion_capacity_requested(&mut self, owners: ActivationOwners<'_>) -> Result<(), String> {
        emit(
            &mut self.signals.motion_capacity_requested,
            &mut self.components,
            &mut self.dispatch,
            owners,
            PublicationEvent::MotionCapacityRequested,
        )
    }
    fn motion_created(
        &mut self,
        owners: ActivationOwners<'_>,
        info: &MotionInfo,
        motion: u32,
    ) -> Result<(), String> {
        emit(
            &mut self.signals.motion_created,
            &mut self.components,
            &mut self.dispatch,
            owners,
            PublicationEvent::MotionCreated { info, motion },
        )
    }
}

pub struct ComponentPublicationEvents<'a, S, C, F> {
    pub components: TriggerComponentStore<'a>,
    pub signals: &'a mut PublicationSignals<S, C>,
    pub dispatch: F,
}

fn emit<'a, S: Copy + Eq, C: Copy + Eq>(
    signal: &mut Signal<S, C>,
    components: &mut TriggerComponentStore<'a>,
    dispatch: &mut impl FnMut(
        &mut Signal<S, C>,
        S,
        C,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'a>,
    ) -> Result<(), String>,
    owners: ActivationOwners<'_>,
    event: PublicationEvent<'_>,
) -> Result<(), String> {
    let mut error = None;
    signal.emit(&event, |signal, subscriber, callback, &event| {
        if error.is_none() {
            let live = ActivationOwners {
                bodies: &mut *owners.bodies,
                motions: &mut *owners.motions,
                deactivation: &mut *owners.deactivation,
                broadphase: &mut *owners.broadphase,
            };
            if let Err(failure) = dispatch(signal, subscriber, callback, live, event, components) {
                error = Some(failure);
            }
        }
    });
    error.map_or(Ok(()), Err)
}

impl<'a, S: Copy + Eq, C: Copy + Eq, F> PublicationEvents
    for ComponentPublicationEvents<'a, S, C, F>
where
    F: FnMut(
        &mut Signal<S, C>,
        S,
        C,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'a>,
    ) -> Result<(), String>,
{
    fn activation(
        &mut self,
        owners: ActivationOwners<'_>,
        ids: &[u32],
        active: bool,
    ) -> Result<(), String> {
        let signal = if active {
            &mut self.signals.activation.activated
        } else {
            &mut self.signals.activation.deactivated
        };
        emit(
            signal,
            &mut self.components,
            &mut self.dispatch,
            owners,
            PublicationEvent::Activation { ids, active },
        )
    }
    fn added(&mut self, owners: ActivationOwners<'_>, ids: &[u32]) -> Result<(), String> {
        emit(
            &mut self.signals.added,
            &mut self.components,
            &mut self.dispatch,
            owners,
            PublicationEvent::Added(ids),
        )
    }
    fn transform_changed(
        &mut self,
        owners: ActivationOwners<'_>,
        body: u32,
        static_only: bool,
    ) -> Result<(), String> {
        let signal = if static_only {
            &mut self.signals.static_transform
        } else {
            &mut self.signals.transform
        };
        emit(
            signal,
            &mut self.components,
            &mut self.dispatch,
            owners,
            PublicationEvent::Transform { body, static_only },
        )
    }
}

impl<S, C, F> VelocityDataStore for ComponentPublicationEvents<'_, S, C, F> {
    type Reference = ComponentHandle;
    fn referenced_body(&self, reference: ComponentHandle) -> Result<u32, String> {
        self.components.referenced_body(reference)
    }
    fn velocities(&mut self, user_data: u64) -> Result<&mut CollisionVelocities, String> {
        self.components.velocities(user_data)
    }
}
impl<S, C, F> TriggerDataStore for ComponentPublicationEvents<'_, S, C, F> {
    fn deferred(&self) -> bool {
        self.components.deferred()
    }
    fn water_component(&self, trigger: ComponentHandle) -> Result<Option<ComponentHandle>, String> {
        self.components.water_component(trigger)
    }
    fn water_mut(&mut self, water: ComponentHandle) -> Result<&mut WaterComponent, String> {
        self.components.water_mut(water)
    }
}
