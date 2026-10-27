//! 1eb4c0 stable mesh slots and 1ebc50 lifetime rotation/animation updates.
use super::{
    parameters::{ScalarParameter, VectorParameter},
    sprite::{InitialAnimationFrame, SpriteAppearance, SpriteFlipbook, TextureFlip},
};
use crate::{
    colour::{pack_rgba8, unpack_rgba8},
    float::{add, multiply},
};

pub struct MeshSpawnDefinition {
    pub base_colour: [f32; 4],
    pub use_spawn_mesh_colour: bool,
    pub colour: VectorParameter,
    pub colour_lods: Vec<[[f32; 3]; 2]>,
    pub alpha: ScalarParameter,
    pub alpha_lods: Vec<[f32; 2]>,
    pub size: VectorParameter,
    pub size_lods: Vec<[[f32; 3]; 2]>,
    pub pivot: VectorParameter,
    pub rotation: VectorParameter,
}
impl Default for MeshSpawnDefinition {
    fn default() -> Self {
        Self {
            base_colour: [1.; 4],
            use_spawn_mesh_colour: false,
            colour: VectorParameter::constant([1.; 3]),
            colour_lods: Vec::new(),
            alpha: ScalarParameter::constant(1.),
            alpha_lods: Vec::new(),
            size: VectorParameter::constant([1.; 3]),
            size_lods: Vec::new(),
            pivot: VectorParameter::constant([0.; 3]),
            rotation: VectorParameter::constant([0.; 3]),
        }
    }
}
pub struct MeshAppearance {
    pub colour_over_life: VectorParameter,
    pub alpha_over_life: ScalarParameter,
    pub size_over_life: VectorParameter,
    pub size_over_velocity: VectorParameter,
    pub velocity_offset_over_life: ScalarParameter,
    pub pivot_offset_over_life: VectorParameter,
    pub rotation_over_life: VectorParameter,
    pub rotation_rate_over_life: VectorParameter,
}
impl Default for MeshAppearance {
    fn default() -> Self {
        let shared = SpriteAppearance::default();
        let mut rotation = VectorParameter::constant([0.; 3]);
        rotation.curve.maximum = Default::default();
        rotation.curve.maximum.append_key(0., [0.; 3], 4);
        rotation.curve.maximum.append_key(1., [1.; 3], 4);
        Self {
            colour_over_life: shared.colour_over_life,
            alpha_over_life: shared.alpha_over_life,
            size_over_life: shared.size_over_life,
            size_over_velocity: shared.size_over_velocity,
            velocity_offset_over_life: shared.velocity_offset_over_life,
            pivot_offset_over_life: VectorParameter::constant([0.; 3]),
            rotation_over_life: rotation,
            rotation_rate_over_life: VectorParameter::constant([0.; 3]),
        }
    }
}
#[derive(Clone, Copy)]
pub struct MeshInitialState {
    pub colour: u32,
    pub flip_bits: u8,
    pub rotation: [f32; 3],
    pub size: [f32; 3],
    pub pivot: [f32; 3],
    pub animation_frame: Option<f32>,
}
impl MeshInitialState {
    pub fn new(
        definition: &MeshSpawnDefinition,
        fraction: f32,
        lod: usize,
        mesh_colour: [f32; 4],
        flips: [TextureFlip; 2],
        animation: Option<&InitialAnimationFrame>,
        seed: &mut u32,
    ) -> Result<Self, String> {
        let rgb = definition
            .colour
            .sample_lod(fraction, lod, &definition.colour_lods, seed)?;
        let alpha = definition
            .alpha
            .sample_lod(fraction, lod, &definition.alpha_lods, seed)?;
        let mut colour = [
            multiply(rgb[0], definition.base_colour[0]),
            multiply(rgb[1], definition.base_colour[1]),
            multiply(rgb[2], definition.base_colour[2]),
            multiply(alpha, definition.base_colour[3]),
        ];
        if definition.use_spawn_mesh_colour {
            colour = std::array::from_fn(|i| multiply(colour[i], mesh_colour[i]));
        }
        let colour = pack_rgba8(colour);
        let rotation = definition.rotation.sample(fraction, seed)?;
        let size = definition
            .size
            .sample_lod(fraction, lod, &definition.size_lods, seed)?;
        let pivot = definition.pivot.sample(fraction, seed)?;
        let animation_frame = animation.map(|animation| animation.sample(seed));
        let u = flips[0].enabled(crate::random::lcg_step(seed) & (1 << 19) != 0);
        let v = flips[1].enabled(crate::random::lcg_step(seed) & (1 << 18) != 0);
        Ok(Self {
            colour,
            flip_bits: u8::from(u) | (u8::from(v) << 1),
            rotation,
            size,
            pivot,
            animation_frame,
        })
    }
    pub fn advance(
        &mut self,
        appearance: &MeshAppearance,
        animation: &SpriteFlipbook,
        age: f32,
        delta: f32,
        seed: &mut u32,
    ) -> Result<(), String> {
        let rate = appearance.rotation_rate_over_life.sample(age, seed)?;
        self.rotation = std::array::from_fn(|i| add(multiply(delta, rate[i]), self.rotation[i]));
        if let Some(frame) = &mut self.animation_frame {
            *frame = add(multiply(animation.rate.sample(age, seed)?, delta), *frame);
        }
        Ok(())
    }
    pub fn colour_at(
        &self,
        appearance: &MeshAppearance,
        age: f32,
        owner: [f32; 4],
        seed: &mut u32,
    ) -> Result<u32, String> {
        let initial = unpack_rgba8(self.colour);
        let base: [f32; 4] = std::array::from_fn(|i| multiply(initial[i], owner[i]));
        let rgb = appearance.colour_over_life.sample(age, seed)?;
        let alpha = appearance.alpha_over_life.sample(age, seed)?;
        Ok(pack_rgba8([
            multiply(rgb[0], base[0]),
            multiply(rgb[1], base[1]),
            multiply(rgb[2], base[2]),
            multiply(alpha, base[3]),
        ]))
    }
}
