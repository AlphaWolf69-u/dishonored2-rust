//! Actual Observable5fef00/6002e0 and AttentionSource5d90c0/5d9430 state.
use super::{
    component_state::{ComponentInstance, ComponentOwner},
    pickable_component::CheckedComponent,
};
use crate::assets::ai_cues::{AttentionDefinition, ObservableDefinition};
use std::sync::Arc;

pub struct Observable {
    pub owner: ComponentOwner,
    pub definition: Arc<ObservableDefinition>,
    pub attachment: Option<CheckedComponent>,
    ///6002e0 copies the real PhysicsBase+d0/+d8 body ID list.
    pub bodies: Vec<u32>,
    pub enabled_count: i32,
    pub disabled_remaining: f32,
    pub initialized: bool,
    pub started: bool,
}
impl Observable {
    pub fn new(definition: Arc<ObservableDefinition>) -> Self {
        let mut owner = ComponentOwner::default();
        //5fef00 constructor7e7ab0(1), independent of enabled_count.
        owner.request_update(1);
        Self {
            owner,
            definition,
            attachment: None,
            bodies: Vec::new(),
            enabled_count: 0,
            disabled_remaining: 0.,
            initialized: false,
            started: false,
        }
    }
}
pub struct AttentionSource {
    pub owner: ComponentOwner,
    pub definition: Arc<AttentionDefinition>,
    pub meter: i32,
    pub initialized: bool,
    pub started: bool,
}
impl AttentionSource {
    pub fn new(definition: Arc<AttentionDefinition>) -> Self {
        Self {
            owner: ComponentOwner::default(),
            definition,
            meter: -1,
            initialized: false,
            started: false,
        }
    }
}
macro_rules! component {($($ty:ty),*)=>{$(impl ComponentInstance for $ty {
    fn owner(&self)->&ComponentOwner{&self.owner}
    fn owner_mut(&mut self)->&mut ComponentOwner{&mut self.owner}
})*}}
component!(Observable, AttentionSource);
