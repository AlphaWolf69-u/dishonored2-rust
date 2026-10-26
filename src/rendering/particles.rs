//! Arithmetic from the installed particles render program. Scene depth and
//! particle-instance producers remain separate from these shader helpers.
use bevy_math::{Vec2, Vec4};
pub mod bounds;
pub mod clipping;
pub mod definition;
pub mod geometry;
pub mod material;
pub mod publication;
pub mod transparency;
pub mod view;
pub mod world_sources;

/// 1ee880's twelve-byte instance record, consumed by particles_instancing.
/// The upper half of size is unused by GetParticleRadius and starts at zero.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct PackedParticleParameters {
    pub colour: u32,
    pub flipbook_seed_uv: u32,
    pub size: u32,
}
impl PackedParticleParameters {
    /// An unanimated sprite passes its normalized particle age as phase; an
    /// animated sprite supplies its retained animation phase instead.
    pub fn new(colour: u32, particle_seed: u32, flip_bits: u8, phase: f32, radius: f32) -> Self {
        Self {
            colour,
            flipbook_seed_uv: u32::from(crate::float::pack_half_truncated(phase))
                | (((particle_seed >> 1) & 0x3fff) << 16)
                | (u32::from(flip_bits & 3) << 30),
            size: u32::from(crate::float::pack_half_truncated(radius)),
        }
    }
}

/// View inputs for a single sprite after 1f2c50 has transformed its particle
/// into world space. Custom orientation and emitter axes have distinct owners.
#[derive(Clone, Copy)]
pub struct SpriteView {
    pub camera_position: [f32; 3],
    pub camera_axes: [[f32; 3]; 3],
    pub custom_basis: [[f32; 3]; 3],
    pub emitter_basis: [[f32; 3]; 3],
    pub owner_colour: [f32; 4],
    pub owner_scale: f32,
}

impl SpriteView {
    /// 1ee9b0 applies query visibility to owner alpha and sprite scale before
    /// 1edb20 publishes individual particles. Query evaluation consumes no RNG.
    pub fn with_query_visibility(
        &self,
        descriptor: &crate::assets::fx::particle_sprite::ParticleSpriteAsset,
        query: f32,
        alpha_scale: f32,
    ) -> Result<Self, String> {
        use crate::float::multiply;
        let mut colour = self.owner_colour;
        colour[3] = multiply(
            multiply(descriptor.query_alpha.sample_visibility(query)?, colour[3]),
            alpha_scale,
        );
        Ok(Self {
            owner_colour: colour,
            owner_scale: multiply(
                descriptor.query_size.sample_visibility(query)?,
                self.owner_scale,
            ),
            ..*self
        })
    }
}

pub struct SpriteInstance {
    pub position: [f32; 3],
    /// The normal remains unscaled; the two billboard axes include size.
    pub axes: [[f32; 3]; 3],
    pub parameters: PackedParticleParameters,
}

impl SpriteInstance {
    /// 1edb20's publication order: velocity offset, lifetime and velocity size,
    /// rotation, orientation, pivot, angular fade, colour and packed UV state.
    /// All samples use one fresh copy of the particle's stored seed per frame.
    pub fn new(
        descriptor: &crate::assets::fx::particle_sprite::ParticleSpriteAsset,
        initial: &crate::gameplay::particles::sprite::SpriteInitialState,
        particle: &crate::gameplay::particles::ParticleState,
        particle_seed: u32,
        view: &SpriteView,
    ) -> Result<Self, String> {
        use crate::{
            float::{add, multiply, subtract},
            gameplay::{particles::orientation::normalize, reference_frame::axis_angle_basis},
        };
        let appearance = &descriptor.appearance;
        let age = particle.age;
        let mut seed = particle_seed;
        let mut velocity = particle.velocity;
        // The source computes Y*Y + X*X + Z*Z before normalization.
        let squared = add(
            add(
                multiply(velocity[1], velocity[1]),
                multiply(velocity[0], velocity[0]),
            ),
            multiply(velocity[2], velocity[2]),
        );
        let reciprocal = crate::float::divide(1., squared.max(f32::MIN_POSITIVE).sqrt());
        velocity = velocity.map(|value| multiply(value, reciprocal));
        let speed = multiply(reciprocal, squared);
        let offset = appearance
            .velocity_offset_over_life
            .sample(age, &mut seed)?;
        let mut position =
            std::array::from_fn(|i| add(particle.position[i], multiply(velocity[i], offset)));
        let life_size = appearance.size_over_life.sample(age, &mut seed)?;
        let velocity_size = appearance.size_over_velocity.sample(speed, &mut seed)?;
        let size: [f32; 3] = std::array::from_fn(|i| {
            multiply(
                multiply(multiply(initial.size[i], life_size[i]), view.owner_scale),
                velocity_size[i],
            )
        });
        let angle = add(
            appearance.rotation_over_life.sample(age, &mut seed)?,
            initial.rotation,
        );
        let mut to_camera = std::array::from_fn(|i| subtract(view.camera_position[i], position[i]));
        normalize(&mut to_camera);
        let mut axes = descriptor.orientation.sample(
            age,
            &mut seed,
            view.custom_basis,
            view.emitter_basis,
            to_camera,
            velocity,
            view.camera_axes,
        )?;
        if axes[0].iter().any(|v| v.to_bits() & 0x7fff_ffff != 0) {
            let angle = f32::from_bits(
                multiply(f32::from_bits(0x3c8e_fa35), angle).to_bits() ^ 0x8000_0000,
            );
            let rotation = axis_angle_basis(axes[0], angle);
            for axis in &mut axes[1..] {
                *axis = std::array::from_fn(|i| {
                    add(
                        add(
                            multiply(axis[1], rotation[1][i]),
                            multiply(axis[0], rotation[0][i]),
                        ),
                        multiply(axis[2], rotation[2][i]),
                    )
                });
            }
        }
        axes[1] = axes[1].map(|v| multiply(v, size[0]));
        axes[2] = axes[2].map(|v| multiply(v, size[1]));
        let pivot = appearance.pivot_offset_over_life.sample(age, &mut seed)?;
        let pivot = [
            add(pivot[0], initial.pivot[0]),
            add(pivot[1], initial.pivot[1]),
        ];
        position = std::array::from_fn(|i| {
            add(
                add(
                    multiply(axes[1][i], pivot[0]),
                    multiply(axes[2][i], pivot[1]),
                ),
                position[i],
            )
        });
        let fade = descriptor.angular_fade.sample(axes[0], to_camera);
        let colour = initial.colour_at(appearance, age, view.owner_colour, fade, &mut seed)?;
        let radius = add(
            add(multiply(size[1], size[1]), multiply(size[0], size[0])),
            multiply(size[2], size[2]),
        )
        .sqrt();
        Ok(Self {
            position,
            axes,
            parameters: PackedParticleParameters::new(
                colour,
                particle_seed,
                initial.flip_bits,
                initial.animation_frame.unwrap_or(age),
                radius,
            ),
        })
    }
}

fn saturate(value: f32) -> f32 {
    if value > 0.0 {
        value.min(1.0)
    } else {
        0.0
    }
}

/// ViewDependentFade consumes DirectX pixel SV_Position.w (clip W).
/// Installed pixel stage 177 uses this input directly.
pub fn view_dependent_fade(clip_w: f32, start: f32, end: f32) -> f32 {
    let t = saturate((clip_w - start) * (1.0 / (end - start)));
    t * t * (3.0 - 2.0 * t)
}

/// ComputeSoftParticleScale after the caller has sampled its depth texture.
pub fn soft_particle_scale(scene_depth: f32, fragment_depth: f32, softness: f32) -> f32 {
    let scene = 1.0 / (1.0 - scene_depth);
    let fragment = 1.0 / (1.0 - fragment_depth);
    saturate((scene - fragment) * softness)
}

/// Opacity_Cube's alpha path, given the original pixel context's depth values.
pub fn cubic_opacity(scene_depth: f32, fragment_depth: f32, radius: f32, threshold: f32) -> f32 {
    saturate(((scene_depth - fragment_depth) / radius) / threshold)
}

/// InvertSoftContact applies the complement of the selected contact mode.
pub fn invert_soft_contact(opacity: f32) -> f32 {
    1.0 - opacity
}

/// ComputeDistortionMV retains both blurred and raw distortion coordinates.
pub fn distortion_motion_vector(distortion: Vec2, blur: f32) -> Vec4 {
    (distortion * blur)
        .extend(distortion.x)
        .extend(distortion.y)
}

/// ComputeAnimCrossFadeColor after sampling both animation frames. The frame
/// fraction is not clamped; keep the shader's multiply/add order.
pub fn animation_crossfade(first: Vec4, second: Vec4, fraction: f32) -> Vec4 {
    first * (1.0 - fraction) + second * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_helpers_match_installed_hlsl() {
        let expected = include_str!("../../tests/fixtures/particle_math_cases.txt")
            .split_whitespace()
            .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(expected.len(), 64 * 16);
        for id in 0..64 {
            let d = (id % 17) as f32 / 8.0 - 0.25;
            let a = (id % 5) as f32 / 8.0;
            let b = (id % 7) as f32 / 8.0;
            let cubic = cubic_opacity(
                1.0 + (id % 13) as f32,
                (id % 7) as f32 * 0.5,
                0.25 * (1.0 + (id % 3) as f32),
                (id % 5) as f32 / 4.0,
            );
            let values = [
                Vec4::new(
                    view_dependent_fade(d, a, b),
                    view_dependent_fade(d, b, a),
                    view_dependent_fade(a, a, a),
                    view_dependent_fade(a + 0.25, a, a),
                ),
                Vec4::new(
                    soft_particle_scale(
                        (id % 9) as f32 / 8.0,
                        (id % 11) as f32 / 8.0,
                        (id % 5) as f32 * 0.2,
                    ),
                    cubic,
                    invert_soft_contact(cubic),
                    0.0,
                ),
                distortion_motion_vector(
                    Vec2::new((id % 7) as f32 - 3.0, (id % 11) as f32 - 5.0) * 0.25,
                    (id % 5) as f32 * 0.3,
                ),
                animation_crossfade(
                    Vec4::new(0.2, 0.4, 0.8, 0.6),
                    Vec4::new(0.7, 0.1, 0.9, 0.3),
                    (id % 9) as f32 / 4.0 - 0.5,
                ),
            ];
            for (field, actual) in values.into_iter().flat_map(|v| v.to_array()).enumerate() {
                let reference = expected[id * 16 + field];
                assert!(
                    (actual - reference).abs() <= 0.000002 * reference.abs().max(1.0),
                    "particle case {id} field {field}: {actual} != {reference}"
                );
            }
        }
    }
}
