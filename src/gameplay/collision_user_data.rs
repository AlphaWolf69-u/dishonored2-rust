//! Collision-data ownership, independent of a movement body's actor link.
//! Trigger setup779530 publishes trigger+40, whose +8 owner is the trigger
//! component. 751050 instead changes the separate movement interface at+20.
use super::component_state::{entity::ComponentHandle, ComponentOwner};
pub mod velocity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionUserData {
    pub component: ComponentHandle,
}

#[cfg(test)]
#[path = "collision_user_data/tests.rs"]
mod tests;
impl CollisionUserData {
    /// Stable ABI identity for the component's collision-data subobject. Zero
    /// keeps its null meaning; no body-ID or entity-ID lookup table is needed.
    pub fn encode(self) -> u64 {
        self.component.packed() + 1
    }
    pub fn decode(value: u64) -> Result<Self, String> {
        let component = value
            .checked_sub(1)
            .ok_or("collision hit has no user-data owner")?;
        Ok(Self {
            component: ComponentHandle::from_packed(component)?,
        })
    }
    pub fn entity<'a>(
        self,
        mut component: impl FnMut(ComponentHandle) -> Option<&'a ComponentOwner>,
    ) -> Result<usize, String> {
        component(self.component)
            .ok_or("collision component owner is missing")?
            .entity
            .ok_or_else(|| "collision component has no owning entity".into())
    }
}
