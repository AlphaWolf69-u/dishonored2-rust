//! Real shared AISmartObject5b2e90 slot state and concrete Cue subtypes.
use crate::{
    assets::ai_smart_objects::Definition,
    gameplay::{
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
    },
};
use std::sync::Arc;
pub struct Slot {
    /// Managed user entities5b5450, never copies of NPC runtime objects.
    pub users: Vec<EntityId>,
    pub position_override: Option<[f32; 3]>,
    pub face_to: Option<EntityId>,
}
pub struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub slots: Vec<Slot>,
    pub fallback_slot: Option<usize>,
    pub active: bool,
    pub initialized: bool,
    pub started: bool,
    /// SearchSpot5eb7c0's actual ushort, absent meansffff.
    pub influence_cell: Option<u16>,
    pub registered: bool,
    /// Dynamic5af530's domain-aware source time and position snapshot.
    pub deadline: Option<(f64, u32)>,
    pub position_snapshot: [f32; 3],
    /// Host acknowledgement of the accepted source stage0 callback.
    pub updated_frame: Option<i32>,
}
impl Component {
    pub fn new(definition: Arc<Definition>) -> Self {
        let mut owner = ComponentOwner::default();
        owner.request_update(1); //5b2e90 +35bit0 ->7e7ab0(1).
        Self {
            owner,
            definition,
            slots: Vec::new(),
            fallback_slot: None,
            active: false,
            initialized: false,
            started: false,
            influence_cell: None,
            registered: false,
            deadline: None,
            position_snapshot: [0.; 3],
            updated_frame: None,
        }
    }
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
