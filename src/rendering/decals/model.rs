//! Model-local projection rows: 233a30, 234ab0, 235680 and 1c0a10.
//! Global decal/material resources remain owned by DecalPool.
use crate::render_shadow::{invert_rows, multiply_rows};

///233a30 initializes both staging and evaluated lists with 16 inline rows.
pub const MODEL_PROJECTION_LIMIT: usize = 16;
#[derive(Clone, Debug, PartialEq)]
pub struct ModelProjection {
    pub slot: u32,
    pub owner: u64,
    pub local_to_decal: [[f32; 4]; 4],
    pub world_to_decal: [[f32; 4]; 4],
    pub surfaces: u32,
    pub pending: bool,
    pub inherited: bool,
}
impl ModelProjection {
    pub fn new(
        slot: u32,
        owner: u64,
        world_to_decal: [[f32; 4]; 4],
        model: [[f32; 4]; 4],
        surfaces: u32,
    ) -> Self {
        Self {
            slot,
            owner,
            local_to_decal: multiply_rows(world_to_decal, model),
            world_to_decal,
            surfaces,
            pending: false,
            inherited: false,
        }
    }
    ///1c0a10 retains the copied local matrix (state1), and only publishes its
    ///new world matrix after the child's actual surface bounds admit it.
    pub fn place(&mut self, model: [[f32; 4]; 4]) -> Result<(), String> {
        let mut inverse = [[0.; 4]; 4];
        if !invert_rows(&model, &mut inverse) {
            return Err("model projection transform is singular".into());
        }
        self.world_to_decal = multiply_rows(self.local_to_decal, inverse);
        Ok(())
    }
}
