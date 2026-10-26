//! Sprite spawn state and lifetime colour/rotation (1ee5f0, 1eeea0, 1ec140).
//! Billboard geometry and material submission consume these stable slot values.
use super::parameters::{ScalarParameter, VectorParameter};
use crate::{
    colour::{pack_rgba8, unpack_rgba8},
    float::{add, multiply, subtract},
    gameplay::effects::curve::{EffectFloatCurve, EffectFloatCurvePair},
};

pub struct SpriteSpawnDefinition {
    pub base_colour: [f32; 4],
    pub use_spawn_mesh_colour: bool,
    pub colour: VectorParameter,
    pub colour_lods: Vec<[[f32; 3]; 2]>,
    pub alpha: ScalarParameter,
    pub alpha_lods: Vec<[f32; 2]>,
    pub size: VectorParameter,
    pub size_lods: Vec<[[f32; 3]; 2]>,
    pub pivot: VectorParameter<2>,
    pub rotation: ScalarParameter,
}
impl Default for SpriteSpawnDefinition {
    /// 1ec240. Initial size is a Vec3; an authored XY edit retains Z=.25.
    fn default() -> Self {
        Self {
            base_colour: [1.; 4],
            use_spawn_mesh_colour: false,
            colour: VectorParameter::constant([1.; 3]),
            colour_lods: Vec::new(),
            alpha: ScalarParameter::constant(1.),
            alpha_lods: Vec::new(),
            size: VectorParameter::constant([0.25; 3]),
            size_lods: Vec::new(),
            pivot: VectorParameter::constant([0.; 2]),
            rotation: ScalarParameter::constant(0.),
        }
    }
}

pub struct SpriteAppearance {
    pub colour_over_life: VectorParameter,
    pub alpha_over_life: ScalarParameter,
    pub size_over_life: VectorParameter,
    pub size_over_velocity: VectorParameter,
    pub velocity_offset_over_life: ScalarParameter,
    pub pivot_offset_over_life: VectorParameter<2>,
    pub rotation_over_life: ScalarParameter,
    pub rotation_rate_over_life: ScalarParameter,
}
impl Default for SpriteAppearance {
    fn default() -> Self {
        let mut alpha = ScalarParameter::constant(1.);
        alpha.curve = EffectFloatCurvePair {
            maximum: EffectFloatCurve {
                times: vec![0., 0.1, 0.9, 1.],
                values: vec![0., 0., 0., 0., 1., 0., 0., 1., 0., 0., 0., 0.],
            },
            minimum: EffectFloatCurve::default(),
        };
        let mut rotation = ScalarParameter::constant(0.);
        rotation.curve = EffectFloatCurvePair {
            maximum: EffectFloatCurve {
                times: vec![0., 1.],
                values: vec![0., 0., 0., 0., 1., 0.],
            },
            minimum: EffectFloatCurve::default(),
        };
        Self {
            colour_over_life: VectorParameter::constant([1.; 3]),
            alpha_over_life: alpha,
            size_over_life: VectorParameter::constant([1.; 3]),
            size_over_velocity: VectorParameter::constant([1.; 3]),
            velocity_offset_over_life: ScalarParameter::constant(0.),
            pivot_offset_over_life: VectorParameter::constant([0.; 2]),
            rotation_over_life: rotation,
            rotation_rate_over_life: ScalarParameter::constant(0.),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(u32)]
pub enum TextureFlip {
    #[default]
    None = 0,
    Random = 1,
    Always = 2,
}
impl TextureFlip {
    /// 1f9280: enum1 follows the supplied random bit; enum2 always flips.
    pub fn enabled(self, random: bool) -> bool {
        matches!(self, Self::Always) || (matches!(self, Self::Random) && random)
    }
}

pub struct InitialAnimationFrame {
    pub range: [f32; 2],
    pub divisions: i32,
}

/// The descriptor's flipbook controls, independent of the material's sheet.
/// 1f8900 zeroes its first 24 bytes and initializes the rate to one.
pub struct SpriteFlipbook {
    pub flips: [TextureFlip; 2],
    pub cycle_rate: bool,
    pub initial: InitialAnimationFrame,
    pub rate: ScalarParameter,
}
impl Default for SpriteFlipbook {
    fn default() -> Self {
        Self {
            flips: [TextureFlip::None; 2],
            cycle_rate: false,
            initial: InitialAnimationFrame {
                range: [0.; 2],
                divisions: 0,
            },
            rate: ScalarParameter::constant(1.),
        }
    }
}

/// 1ecc60 converts the two editable degree angles into 1ec0e0's dot-space
/// offset and slope. Retain the source's cosine and angle wrapping order.
pub struct AngularFade {
    pub angles: [f32; 2],
    pub slope: f32,
    pub minimum: f32,
}
impl AngularFade {
    pub fn new(angles: [f32; 2]) -> Self {
        let cosine = |degrees| {
            let mut angle = subtract(
                f32::from_bits(0x3fc9_0fdb),
                multiply(f32::from_bits(0x3c8e_fa35), degrees),
            );
            let period = f32::from_bits(0x40c9_0fdb);
            if angle < 0. || period <= angle {
                let turns = multiply(angle, f32::from_bits(0x3e22_f983)).floor();
                angle = subtract(angle, multiply(turns, period));
            }
            (angle as f64).cos() as f32
        };
        let first = cosine(angles[0]);
        let minimum = cosine(angles[1]);
        Self {
            angles,
            slope: crate::float::divide(1., subtract(first, minimum)),
            minimum,
        }
    }
    pub fn sample(&self, normal: [f32; 3], view: [f32; 3]) -> f32 {
        angular_fade(normal, view, self.slope, self.minimum)
    }
}
impl InitialAnimationFrame {
    /// 1f93f0 keeps the fractional part, optionally after discrete-frame
    /// quantization. Its single random draw belongs after all sprite samples.
    pub fn sample(&self, seed: &mut u32) -> f32 {
        let fraction = crate::random::lcg_unit(seed);
        let first = crate::gameplay::effects::normalize_small(self.range[0]);
        let second = crate::gameplay::effects::normalize_small(self.range[1]);
        let mut value = add(
            multiply(subtract(1., fraction), second),
            multiply(first, fraction),
        );
        if self.divisions != 0 {
            value = crate::float::divide(add(add(value, 0.5).floor(), 0.5), self.divisions as f32);
        }
        subtract(value, value.floor())
    }
}

pub struct SpriteInitialState {
    pub colour: u32,
    pub flip_bits: u8,
    pub rotation: f32,
    pub size: [f32; 3],
    pub pivot: [f32; 2],
    pub animation_frame: Option<f32>,
}
impl SpriteInitialState {
    /// 1eeea0 advances an allocated CYCLE_RATE phase before rotation. Phase
    /// remains unwrapped; the material's volume sampler owns its address mode.
    pub fn advance(
        &mut self,
        appearance: &SpriteAppearance,
        animation: &SpriteFlipbook,
        age: f32,
        delta: f32,
        seed: &mut u32,
    ) -> Result<(), String> {
        if let Some(frame) = &mut self.animation_frame {
            *frame = add(multiply(animation.rate.sample(age, seed)?, delta), *frame);
        }
        self.advance_rotation(appearance, age, delta, seed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        definition: &SpriteSpawnDefinition,
        cycle_fraction: f32,
        lod: usize,
        mesh_colour: [f32; 4],
        flips: [TextureFlip; 2],
        animation: Option<&InitialAnimationFrame>,
        seed: &mut u32,
    ) -> Result<Self, String> {
        let colour =
            definition
                .colour
                .sample_lod(cycle_fraction, lod, &definition.colour_lods, seed)?;
        let alpha =
            definition
                .alpha
                .sample_lod(cycle_fraction, lod, &definition.alpha_lods, seed)?;
        let mut colour = [
            multiply(colour[0], definition.base_colour[0]),
            multiply(colour[1], definition.base_colour[1]),
            multiply(colour[2], definition.base_colour[2]),
            multiply(alpha, definition.base_colour[3]),
        ];
        if definition.use_spawn_mesh_colour {
            colour = std::array::from_fn(|i| multiply(colour[i], mesh_colour[i]));
        }
        let colour = pack_rgba8(colour);
        // These two steps occur even when the authored flip mode is None.
        let flip_u = flips[0].enabled(crate::random::lcg_step(seed) & (1 << 19) != 0);
        let flip_v = flips[1].enabled(crate::random::lcg_step(seed) & (1 << 18) != 0);
        let rotation = definition.rotation.sample(cycle_fraction, seed)?;
        let size = definition
            .size
            .sample_lod(cycle_fraction, lod, &definition.size_lods, seed)?;
        let pivot = definition.pivot.sample(cycle_fraction, seed)?;
        let animation_frame = animation.map(|definition| definition.sample(seed));
        Ok(Self {
            colour,
            flip_bits: u8::from(flip_u) | (u8::from(flip_v) << 1),
            rotation,
            size,
            pivot,
            animation_frame,
        })
    }

    /// 1eeea0 integrates rotation RATE. RotationOverLife is an independent
    /// publication-time angle and must not be integrated again each frame.
    pub fn advance_rotation(
        &mut self,
        appearance: &SpriteAppearance,
        age: f32,
        delta: f32,
        seed: &mut u32,
    ) -> Result<(), String> {
        self.rotation = add(
            multiply(appearance.rotation_rate_over_life.sample(age, seed)?, delta),
            self.rotation,
        );
        Ok(())
    }

    /// The angular/view factor scales RGBA before 1ec140 samples lifetime
    /// colour and opacity. Initial colour retains its spawn-time byte packing.
    pub fn colour_at(
        &self,
        appearance: &SpriteAppearance,
        age: f32,
        owner: [f32; 4],
        angular_fade: f32,
        seed: &mut u32,
    ) -> Result<u32, String> {
        let initial = unpack_rgba8(self.colour);
        let base: [f32; 4] =
            std::array::from_fn(|i| multiply(multiply(initial[i], owner[i]), angular_fade));
        let colour = appearance.colour_over_life.sample(age, seed)?;
        let alpha = appearance.alpha_over_life.sample(age, seed)?;
        Ok(pack_rgba8([
            multiply(colour[0], base[0]),
            multiply(colour[1], base[1]),
            multiply(colour[2], base[2]),
            multiply(alpha, base[3]),
        ]))
    }
}

/// 1ec0e0 consumes prepared dot-space fade parameters, not the authored degree
/// angles. The descriptor's preparation owns conversion to this slope/offset.
pub fn angular_fade(normal: [f32; 3], view: [f32; 3], slope: f32, minimum: f32) -> f32 {
    let dot = add(
        add(multiply(normal[1], view[1]), multiply(normal[0], view[0])),
        multiply(normal[2], view[2]),
    );
    let absolute = f32::from_bits(dot.to_bits() & 0x7fff_ffff);
    let fade = multiply(subtract(absolute, minimum), slope);
    if fade >= 0. {
        if fade <= 1. {
            fade
        } else {
            1.
        }
    } else {
        0.
    }
}
