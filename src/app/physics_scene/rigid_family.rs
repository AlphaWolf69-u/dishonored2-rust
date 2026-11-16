//! Actual RigidBody-derived payload pools. Handles retain the constructed
//! component class and its own slot, roster and generation throughout dispatch.
//! Throwable inherits the body owner (4c6600); Breakable/Projectile/Holdable
//! have distinct payload owners and must not be reinterpreted as this type.
use super::*;
macro_rules! rigid_pool {
    ($scene:expr, $class:expr) => {{
        let class = $class;
        if class == ($scene).classes[8] { Ok::<_, String>(&($scene).rigid_bodies) }
        else if class == ($scene).classes[12] { Ok::<_, String>(&($scene).throwables) }
        else { Err(format!("component class {class} has no admitted rigid payload pool")) }
    }};
}
macro_rules! rigid_pool_mut {
    ($scene:expr, $class:expr) => {{
        let class = $class;
        if class == ($scene).classes[8] { Ok::<_, String>(&mut ($scene).rigid_bodies) }
        else if class == ($scene).classes[12] { Ok::<_, String>(&mut ($scene).throwables) }
        else { Err(format!("component class {class} has no admitted rigid payload pool")) }
    }};
}
impl PhysicsScene {
    pub(super) fn is_rigid_class(&self, class: i32) -> bool { class == self.classes[8] || class == self.classes[12] }
    pub(super) fn rigid_pool(&self, class: i32) -> Result<&ComponentPool<super::super::rigid_body::RigidBodyComponent>, String> {
        rigid_pool!(self, class)
    }
    pub(super) fn rigid_pool_mut(&mut self, class: i32) -> Result<&mut ComponentPool<super::super::rigid_body::RigidBodyComponent>, String> {
        rigid_pool_mut!(self, class)
    }
    pub(super) fn rigid_component(&self, handle: ComponentHandle) -> Result<&super::super::rigid_body::RigidBodyComponent, String> {
        self.rigid_pool(handle.class_id)?.get(handle.slot).ok_or("rigid component slot is absent".into())
    }
    pub(super) fn rigid_component_mut(&mut self, handle: ComponentHandle) -> Result<&mut super::super::rigid_body::RigidBodyComponent, String> {
        self.rigid_pool_mut(handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent".into())
    }
    pub(super) fn rigid_class_range(&self) -> Result<[i32; 2], String> {
        self.types.classes().hierarchy().type_range(self.types.factory(self.factories[8])?.class)
    }
}
