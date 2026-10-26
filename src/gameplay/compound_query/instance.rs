//! Unfiltered compound leaf preparation shared by 14123b5e0 and14123acd0.
use crate::assets::physworld::compound::CompoundInstance;
use crate::gameplay::{
    convex_support::scaling::adjust_scale,
    shape_transform::{compose, inverse_rigid, rotate_vector, ScaledShapeTransform, ShapeMatrix},
};

#[derive(Clone, Copy, Debug)]
pub struct ShapeKey {
    pub value: u32,
    pub bits: u32,
}
impl ShapeKey {
    /// Preserve the consumed prefix and append the instance key, with trailing
    /// one bits as in141ec47d0. Call before the tag/filter callbacks.
    pub fn compound_child(self, instance: u16, key_bits: u8) -> Result<Self, String> {
        let bits = self.bits + u32::from(key_bits);
        if self.bits > 32 || bits > 32 {
            return Err("compound shape key exceeds32 bits".into());
        }
        let mask = if self.bits == 0 {
            0
        } else {
            u32::MAX << (32 - self.bits)
        };
        let value = (u32::from(instance) + 1)
            .wrapping_shl(32u32.wrapping_sub(bits))
            .wrapping_sub(1)
            | (self.value & mask);
        Ok(Self { value, bits })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompoundShapeState {
    pub transform: ScaledShapeTransform,
    pub scaled: bool,
    pub keep_radius: bool,
    pub radius: f32,
}
pub struct CompoundChildGeometry<'a> {
    pub convex: bool,
    pub shape_type: u8,
    pub vertices: &'a [[f32; 4]],
    pub radius: f32,
}
pub struct PreparedCompoundInstance {
    pub state: CompoundShapeState,
    pub relative: ShapeMatrix,
}

/// Run only after enabled-bit, child mask, shape tag and query-filter admission.
/// World-body identity and the nested mask remain with the caller's shape info.
pub fn prepare_instance(
    instance: &CompoundInstance,
    parent: CompoundShapeState,
    parent_relative: ShapeMatrix,
    child: CompoundChildGeometry<'_>,
) -> Result<PreparedCompoundInstance, String> {
    let mut local = instance.transform;
    let mut scale = instance.scale;
    if parent.scaled {
        for i in 0..4 {
            local[3][i] *= parent.transform.scale[i];
            scale[i] *= parent.transform.scale[i];
        }
    }
    let mut state = parent;
    if parent.scaled || instance.flags & 8 != 0 {
        state.scaled = true;
        state.keep_radius = if instance.flags & 8 != 0 {
            instance.flags & 32 == 0
        } else {
            parent.keep_radius
        };
        state.radius = child.radius;
        state.transform.scale = scale;
        state.transform.offset = [0.; 4];
        if child.convex {
            state.transform.offset = adjust_scale(
                child.shape_type,
                child.vertices,
                state.keep_radius,
                &mut state.transform.scale,
                &mut state.radius,
            )?;
        }
    }
    state.transform.world_from_local = compose(parent.transform.world_from_local, local);
    let inverse = inverse_rigid(instance.transform);
    let mut relative = parent_relative.map(|column| rotate_vector(inverse, column));
    relative[3] = rotate_vector(
        inverse,
        std::array::from_fn(|i| parent_relative[3][i] - local[3][i]),
    );
    Ok(PreparedCompoundInstance { state, relative })
}
