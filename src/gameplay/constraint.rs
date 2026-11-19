//!7125d0 component state; joint arithmetic belongs to the shared world owner.
use crate::assets::constraints::Definition;
use crate::gameplay::{
    component_state::{ComponentInstance, ComponentOwner},
    physics_world::storage::JointId,
};
use std::sync::Arc;

pub(crate) struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub joint: Option<JointId>,
    /// A failed host release retains the same Removed receipt for retry.
    pub retiring_joint: Option<JointId>,
    /// Full-generation canonical IDs, including the actual world body0.
    pub bodies: [u32; 2],
    ///Map source proves these named declarations are actually absent, rather
    ///than an existing actor whose provider the host has not admitted.
    pub missing_endpoint_world: [bool; 2],
    pub endpoints_resolved: u8,
    pub initialized: bool,
    pub started: bool,
    pub pending_enable: bool,
    ///Accepted Start journal: 0=endpoint fence,1=enable,2=timespace sync.
    pub enable_phase: u8,
    /// Host retry cursor for registration/withdrawal of the actual observers.
    pub observer_phase: u8,
    ///The actual852040 filter request, continued before endpoint retirement.
    pub collision_ignore: crate::gameplay::physics_world::storage::collision_pairs::CollisionIgnoreProgress,
}
impl Component {
    pub(crate) fn new(definition: Arc<Definition>) -> Self {
        Self {
            owner: ComponentOwner::default(), definition,
            joint: None, retiring_joint: None, bodies: [0; 2], missing_endpoint_world: [false; 2], endpoints_resolved: 0, initialized: false,
            started: false, pending_enable: false, enable_phase: 0, observer_phase: 0,
            collision_ignore: Default::default(),
        }
    }
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}
