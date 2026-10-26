//! Runtime query-body state, distinct from the authored BodyInfo recipe.
use super::{broadphase::PackedBodyBounds, shape_transform::ShapeMatrix};
#[path = "collision_body/motion_expansion.rs"]
mod motion_expansion;
pub use motion_expansion::MotionExpansion;
#[path = "collision_body/predicted_frame.rs"]
mod predicted_frame;
#[path = "collision_body/swept_bounds.rs"]
mod swept_bounds;
pub use swept_bounds::SweptBoundsStep;
#[path = "collision_body/motion_cache.rs"]
mod motion_cache;
pub use motion_cache::BodyCacheSettings;
use crate::assets::physworld::{
    body::BodyInfo, quaternion_basis, rotation_quaternion, shape::ShapeHeader,
};

#[derive(Clone, Copy, Debug)]
pub struct ShapeSlots {
    pub shapes: [usize; 8],
    pub selectors: [u8; 8],
}

/// Meaningful runtime fields; allocator padding and process pointers are not
/// exposed. Shape handles refer to the physics world's retained shape assets.
#[derive(Clone, Debug, Default)]
pub struct CollisionBody {
    pub transform: ShapeMatrix,
    pub motion_id: u32,
    pub flags: u32,
    pub status: u16,
    pub bounds: PackedBodyBounds,
    pub shape: usize,
    pub lod_flags: u8,
    pub quality_id: u8,
    pub material_id: u16,
    pub filter_info: u32,
    pub id: u32,
    pub next_attached_body_id: u32,
    pub broadphase_id: u32,
    /// Farthest shape-bound distance from the motion center, stored as bfloat16.
    pub packed_radius: u16,
    pub max_contact_distance: u16,
    pub max_tim_distance: u16,
    /// Accumulated transform rotation, source +4a (41720.875 units/radian).
    pub tim_angle: u16,
    /// Radius-scaled rotational contribution to TIM, source +4e.
    pub angular_tim_distance: u16,
    pub user_data: u64,
    /// Source +98, initialized to the invalid active-list index on creation.
    pub active_list_index: u32,
    /// Static bodies cache inverse world rotation; attached bodies cache the
    /// motion-to-body rotation. Both occupy the same source +a0 member.
    pub motion_to_body_orientation: [f32; 4],
    /// Source avgSurfaceVelocity (+9c), updated by cc8b40 above its threshold.
    pub packed_average_surface_velocity: u16,
    pub shape_selectors: [u8; 8],
    pub shape_slots: Option<[usize; 8]>,
}

impl CollisionBody {
    /// ca6a20 copies XYZ of all four columns. W lanes hold body caches and
    /// collision look-ahead distance; the incoming pose must not replace them.
    pub fn set_transform(&mut self, frame: ShapeMatrix) {
        for (column, input) in self.transform.iter_mut().zip(frame) {
            column[..3].copy_from_slice(&input[..3]);
        }
    }

    /// ca6810, ca6a20 and ca6910, using owned shape metadata instead of pointers.
    /// A fresh reservation clears the record first; this method also preserves
    /// existing W lanes and status bits when reinitializing a retained record.
    pub fn initialize(
        &mut self,
        id: u32,
        info: &BodyInfo,
        header: ShapeHeader,
        wrapped: Option<ShapeSlots>,
    ) -> Result<(), String> {
        let wrapped = if header.shape_type == 18 {
            Some(wrapped.ok_or("shape type18 requires its actual slots and selectors")?)
        } else {
            None
        };
        self.bounds = PackedBodyBounds {
            words: [0xff7f_ffff, 0x7fff_ff7f, 0, 0],
        };
        self.motion_id = 0x7fff_ffff;
        self.id = id;
        self.next_attached_body_id = 0x00ff_ffff;
        self.broadphase_id = u32::MAX;
        self.active_list_index = u32::MAX;
        self.packed_radius = 0x3f80;
        self.flags = info.flags & 0xffff_fff0;
        let basis = quaternion_basis(info.orientation);
        self.set_transform([basis[0], basis[1], basis[2], info.position]);
        self.transform[3][3] = info.collision_look_ahead_distance;
        self.shape = info.shape;
        if let Some(wrapped) = wrapped {
            self.shape_slots = Some(wrapped.shapes);
            self.shape_selectors = wrapped.selectors;
        } else {
            self.shape_slots = None;
            self.shape_selectors = [0x80; 8];
        }
        self.flags &= !0x0800_0000;
        if header.flags & 0x8000 != 0 {
            self.flags |= 0x0800_0000;
        }
        self.material_id = info.material_id;
        self.quality_id = info.quality_id;
        self.filter_info = info.filter_info;
        self.status |= 1;
        self.lod_flags = 0;
        self.user_data = info.user_data;
        Ok(())
    }

    /// The motion-kind/quality portion of caf730. Name, mass and motion-property
    /// sidecars remain the body manager's responsibility.
    pub fn assign_motion_kind(&mut self, dynamic: bool) {
        if dynamic {
            if self.quality_id == u8::MAX {
                self.quality_id = 3;
            }
            self.flags |= 2;
            self.active_list_index = u32::MAX;
        } else {
            if self.quality_id == u8::MAX {
                self.quality_id = 0;
            }
            self.motion_id = 0;
            self.flags |= 1;
        }
    }

    pub fn selected_shape(&self, channel: usize) -> Result<usize, String> {
        let selector = *self
            .shape_selectors
            .get(channel)
            .ok_or("shape channel outside selector array")?;
        match self.shape_slots {
            Some(slots) => Ok(slots[usize::from(selector & 7)]),
            None if selector & 7 == 0 => Ok(self.shape),
            None => Err("single-shape body selected a nonzero slot".into()),
        }
    }

    /// ca6d50 and ca6a70: inverse frame cached in quaternion and matrix W lanes.
    pub fn refresh_inverse_frame(&mut self) {
        let mut q = rotation_quaternion([self.transform[0], self.transform[1], self.transform[2]]);
        for value in &mut q[..3] {
            *value = -*value;
        }
        self.motion_to_body_orientation = q;
        let p = self.transform[3];
        let dot = (q[1] * p[1] + q[0] * p[0]) + q[2] * p[2];
        let factor = q[3] * q[3] - 0.5;
        let cross = [
            (p[2] * q[1] - q[2] * p[1]) * q[3],
            (p[0] * q[2] - q[0] * p[2]) * q[3],
            (p[1] * q[0] - q[1] * p[0]) * q[3],
        ];
        for i in 0..3 {
            let value = (dot * q[i] + factor * p[i]) + cross[i];
            self.transform[i][3] = -(value + value);
        }
    }

    /// cc8bd0 changes only bits1/2 of the LOD byte; unordered compares do
    /// not set bit1. Motion ownership is supplied by the actual world caller.
    pub fn update_size_and_static_lod(&mut self, threshold: f32, motion_present: bool) {
        let value = f32::from_bits(u32::from(self.packed_radius) << 16);
        let mut flags = if motion_present { 0 } else { 4 };
        if threshold < value {
            flags |= 2;
        }
        self.lod_flags = (self.lod_flags & !6) | flags;
    }
}

#[path = "collision_body/motion_frame.rs"]
mod motion_frame;

#[cfg(test)]
#[path = "collision_body/tests.rs"]
mod tests;
