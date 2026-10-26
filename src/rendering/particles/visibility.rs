//! Particle view scheduling from 1ff460 and the per-view threshold in 200ba0.
//! The scene supplies its current root-admission result; no query is invented.
use crate::{
    assets::fx::particle_declaration::ParticleVisibility,
    float::{add, divide, multiply, subtract},
};

pub struct ParticleSchedule {
    pub draw: bool,
    pub simulate: bool,
    pub update_cycles: bool,
    pub flush: bool,
    pub delta: f32,
}

pub struct ParticleAdmission {
    pub bounds: [[f32; 3]; 2],
    pub camera: [f32; 3],
    pub emitter_distance: f32,
    pub minimum_angular_size: f32,
    pub root_visible: bool,
    pub distance_alpha: f32,
    pub distance_size: f32,
}

pub struct ParticleVisibilityState {
    culled: bool,
    simulated: bool,
}
impl Default for ParticleVisibilityState {
    /// 1fcaa0 starts culled with no completed simulation (flags81 after the
    /// FX owner selects its accumulated-delta clock with1fda00).
    fn default() -> Self {
        Self {
            culled: true,
            simulated: false,
        }
    }
}

/// 374e40 copies vertical FOV radians from view+3a70 to+a0. 200ba0 passes
/// FOV*5*.5*.01 to the tangent library, not to a configurable distance limit.
pub fn minimum_angular_size(vertical_fov: f32) -> f32 {
    multiply(
        multiply(multiply(vertical_fov, 5.), 0.5),
        f32::from_bits(0x3c23_d70a),
    )
    .tan()
}

impl ParticleVisibilityState {
    pub fn prepare(
        &mut self,
        settings: &ParticleVisibility,
        input: ParticleAdmission,
        delta: f32,
        first_simulation: bool,
        force_simulate: bool,
    ) -> ParticleSchedule {
        let epsilon = f32::from_bits(0x3400_0000);
        let center: [f32; 3] =
            std::array::from_fn(|i| multiply(add(input.bounds[0][i], input.bounds[1][i]), 0.5));
        let length = |v: [f32; 3]| {
            add(
                add(multiply(v[1], v[1]), multiply(v[0], v[0])),
                multiply(v[2], v[2]),
            )
            .sqrt()
        };
        let mut small = false;
        if input.bounds[0][0] <= input.bounds[1][0] {
            // 0fbcc0 takes the farther of BOTH corners on every axis,
            // accumulating X/Y/Z squares even when center rounding differs.
            let mut squared = 0.;
            for i in 0..3 {
                let minimum = subtract(center[i], input.bounds[0][i]).abs();
                let maximum = subtract(input.bounds[1][i], center[i]).abs();
                let extent = if minimum <= maximum { maximum } else { minimum };
                squared = add(squared, multiply(extent, extent));
            }
            let radius = squared.sqrt();
            if epsilon < radius {
                let distance = length(std::array::from_fn(|i| {
                    subtract(center[i], input.camera[i])
                }));
                small = divide(radius, add(distance, f32::from_bits(0x38d1_b717)))
                    < input.minimum_angular_size;
            }
        }
        let culled = 100_000_000. < input.emitter_distance
            || small
            || input.distance_size < epsilon
            || (settings.alpha_culling && !(epsilon <= input.distance_alpha))
            || !input.root_visible;
        let flush = culled && !self.culled && settings.flush_on_occlusion;
        let simulate = !culled
            || first_simulation
            || !self.simulated
            || force_simulate
            || settings.always_simulate;
        let update_cycles = simulate || settings.always_update_cycles;
        self.culled = culled;
        ParticleSchedule {
            draw: !culled,
            simulate,
            update_cycles,
            flush,
            delta: if !update_cycles {
                0.
            } else if 0.125 < delta {
                0.125
            } else {
                delta
            },
        }
    }

    pub fn simulated(&mut self) {
        self.simulated = true;
    }
}
