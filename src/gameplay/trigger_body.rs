//! Body creation parameters shared by arkCpntTrigger instances (779530).
use crate::assets::physworld::{body::BodyInfo, rotation_quaternion};

pub mod transform;
mod component;
pub use component::TriggerComponent;
mod store;
pub use store::{DeferredPhysicsState, TriggerComponentStore, TriggerDataStore};
mod publication;
pub(crate) mod startup;
pub mod initialization;
pub use publication::TriggerPublication;

pub struct TriggerShapeRequest {
    pub name: String,
    pub basis: [[f32; 3]; 3],
}

impl TriggerShapeRequest {
    pub fn asset_path(&self) -> String {
        use crate::assets::physworld::{physics_shape_asset_path, physics_shape_key};
        let key = physics_shape_key(&self.name, 4, [1.; 3], 0., false)
            .expect("fixed trigger shape recipe has finite scale and radius");
        physics_shape_asset_path(&key)
    }
}

/// Resource name and shape basis selected by 779530 before loading kind 4,
/// with unit scale, zero convex radius and the HD flag disabled.
pub fn trigger_shape_request(
    model: &str,
    map: &str,
    entity: &str,
    basis: [[f32; 3]; 3],
) -> TriggerShapeRequest {
    use crate::assets::physworld::{shape_extension, strip_shape_extension};
    let cstr = |s: &str| s.split('\0').next().unwrap().to_owned();
    let model = cstr(model);
    if shape_extension(&model) == "lwo" {
        return TriggerShapeRequest { name: model, basis };
    }
    // 140530 appends verbatim; 145000 strips the map extension. Only the
    // entity suffix uses 140bd0's path append and backslash conversion.
    let map = format!("maps/{}", cstr(map));
    let mut name = strip_shape_extension(&map).to_owned();
    let entity = cstr(entity);
    if !entity.is_empty() {
        if !name.ends_with(['/', '\\']) {
            name.push('/');
        }
        let suffix = entity.strip_prefix(['/', '\\']).unwrap_or(&entity);
        name.push_str(&suffix.replace('\\', "/"));
    }
    TriggerShapeRequest {
        name,
        basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TriggerCollisionSettings {
    pub debris: bool,
    pub rigid_bodies: bool,
    pub npc_ragdolls: bool,
    pub npc_capsules: bool,
    pub keyframed: bool,
}
impl Default for TriggerCollisionSettings {
    /// arkDeclCpntTrigger779350 and reflected field offsets68..6c.
    fn default() -> Self {
        Self {
            debris: false,
            rigid_bodies: false,
            npc_ragdolls: false,
            npc_capsules: true,
            keyframed: false,
        }
    }
}
pub struct TriggerBodyInput {
    pub shape: usize,
    pub position: [f32; 3],
    pub basis: [[f32; 3]; 3],
    pub material_id: u16,
    /// The world owner's stable handle for this trigger's user-data object.
    pub user_data: u64,
    pub water_component: bool,
    pub wind_component: bool,
    pub collision: TriggerCollisionSettings,
}

pub fn prepare_trigger_body(input: TriggerBodyInput) -> BodyInfo {
    // 81a5d0's actual table results for trigger layer16 and water layer22.
    // Water bypasses the ordinary trigger's per-declaration mask edits.
    let mut filter = if input.water_component {
        0x0009_cb16
    } else {
        0x0008_cb10
    };
    if !input.water_component {
        if !input.collision.npc_ragdolls {
            filter &= 0xfff7_7fff;
        }
        if !input.collision.npc_capsules {
            filter &= 0xffff_bfff;
        }
        if !input.collision.debris {
            filter &= 0xffff_fdff;
        }
        if !input.collision.rigid_bodies {
            filter &= 0xffff_feff;
        }
        if input.wind_component {
            filter |= 0x000a_0300;
        }
    }
    BodyInfo {
        shape: input.shape,
        flags: 0x10,
        filter_info: filter | 0x400,
        material_id: input.material_id,
        user_data: input.user_data,
        motion_type: u8::from(input.collision.keyframed),
        position: [input.position[0], input.position[1], input.position[2], 0.],
        orientation: rotation_quaternion(input.basis.map(|v| [v[0], v[1], v[2], 0.])),
        ..BodyInfo::default()
    }
}

#[cfg(test)]
#[path = "trigger_body/tests.rs"]
mod tests;
