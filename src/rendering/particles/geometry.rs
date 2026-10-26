//! Shared sprite grids from 1dc200/1dc5b0. The installed renderer retains six
//! levels with side lengths 2,3,5,9,17,33 and selects a level per sprite.
use crate::float::{add, divide, multiply, subtract};

pub struct SpriteVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}
pub struct SpriteGrid {
    pub vertices: Vec<SpriteVertex>,
    pub indices: Vec<u16>,
}
impl SpriteGrid {
    /// Level selection is capped at five by 1dc9e0. Preserve incremental
    /// arithmetic along each row; recomputing index*spacing changes the grid.
    pub fn new(level: u32) -> Self {
        let side = (1usize << level.min(5)) + 1;
        let step = divide(1., (side - 1) as f32);
        let position_step = multiply(step, 2.);
        let mut vertices = Vec::with_capacity(side * side);
        let mut z = 1.;
        let mut v = 0.;
        for _ in 0..side {
            let mut y = -1.;
            let mut u = 0.;
            for _ in 0..side {
                vertices.push(SpriteVertex {
                    position: [0., y, z],
                    uv: [u, v],
                });
                y = add(y, position_step);
                u = add(u, step);
            }
            z = subtract(z, position_step);
            v = add(v, step);
        }
        let mut indices = Vec::with_capacity((side - 1) * (side - 1) * 6);
        for row in 0..side - 1 {
            for column in 0..side - 1 {
                let first = (row * side + column) as u16;
                let next_row = first + side as u16;
                indices.extend_from_slice(&[
                    first,
                    first + 1,
                    next_row,
                    first + 1,
                    next_row + 1,
                    next_row,
                ]);
            }
        }
        Self { vertices, indices }
    }
}
