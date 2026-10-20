//! World-step selection, solver coefficients and substep phase ownership.
use super::inverse_length;

#[derive(Clone, Copy, Debug)]
pub struct SolverParameters {
    pub seconds: f32,
    pub inverse_seconds: f32,
    pub substep_seconds: f32,
    pub inverse_substep_seconds: f32,
    pub substeps: u32,
    pub iterations: u32,
    pub inverse_substeps: f32,
    pub inverse_iterations: f32,
    pub tau: f32,
    pub damping: f32,
    pub velocity_scale: f32,
    pub inverse_velocity_scale: f32,
    pub inverse_step_tau: f32,
    pub minimum_correction: f32,
    pub gravity: [f32; 4],
    pub gravity_substep: [f32; 4],
    pub gravity_step: [f32; 4],
    pub gravity_length: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolverPhase {
    pub substep: u32,
    pub velocity_fraction: f32,
    pub flags: u32,
}

impl SolverParameters {
    /// Numeric coefficient producers 140c9f5b0 and 140c9f6c0.
    pub fn new(
        seconds: f32,
        substeps: u32,
        iterations: u32,
        tau: f32,
        damping: f32,
        gravity: [f32; 4],
    ) -> Self {
        let inverse_seconds = 1. / seconds;
        let inverse_substeps = 1. / substeps as f32;
        let inverse_iterations = 1. / iterations as f32;
        let substep_seconds = inverse_substeps * seconds;
        let inverse_substep_seconds = substeps as f32 * inverse_seconds;
        let gravity_substep = gravity.map(|v| v * substep_seconds);
        let mut gravity_step = gravity.map(|v| v * seconds);
        let squared = (gravity_step[1] * gravity_step[1] + gravity_step[0] * gravity_step[0])
            + gravity_step[2] * gravity_step[2];
        gravity_step[3] = inverse_length(squared) * squared;
        let squared = (gravity[1] * gravity[1] + gravity[0] * gravity[0]) + gravity[2] * gravity[2];
        let length = inverse_length(squared) * squared;
        Self {
            seconds,
            inverse_seconds,
            substep_seconds,
            inverse_substep_seconds,
            substeps,
            iterations,
            inverse_substeps,
            inverse_iterations,
            tau,
            damping,
            velocity_scale: damping / tau,
            inverse_velocity_scale: (1. / damping) * tau,
            inverse_step_tau: inverse_substep_seconds * ((1. / damping) * tau),
            minimum_correction: -(inverse_substep_seconds * ((1. / damping) * (tau * 0.5))),
            gravity,
            gravity_substep,
            gravity_step,
            gravity_length: if length == 0. {
                f32::from_bits(0x411cf5c3)
            } else {
                length
            },
        }
    }

    /// 140097bc0 chooses the largest advancing time space and rounds its
    /// 120-Hz substep count. Per-entity rate ratios remain the world's inputs.
    pub fn from_game_deltas(deltas: &[f32], gravity: [f32; 4]) -> Option<Self> {
        let mut seconds = 0.;
        for &delta in deltas {
            if delta > seconds {
                seconds = delta;
            }
        }
        if seconds <= 0. {
            return None;
        }
        let substeps = (seconds * 120. + 0.5) as u32;
        (substeps > 0).then(|| {
            Self::new(
                seconds,
                substeps,
                1,
                f32::from_bits(0x3f19999a),
                1.,
                gravity,
            )
        })
    }

    /// Complete 140cba140, excluding untouched padding in its output record.
    pub fn phase(&self, substep: u32, iteration: u32) -> SolverPhase {
        let first = u32::from(substep == 0 && iteration == 0);
        let last = u32::from(substep == self.substeps - 1 && iteration == self.iterations - 1) * 2;
        SolverPhase {
            substep,
            velocity_fraction: self.inverse_velocity_scale * substep as f32 + 1.,
            flags: first | last,
        }
    }
}

/// World construction tuning, separate from the game's later overrides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolverTuning {
    pub tau: f32,
    pub damping: f32,
    pub iterations: u32,
}
impl SolverTuning {
    /// Complete 140ca5260. Unknown quality IDs leave the existing values.
    pub fn set_quality(&mut self, quality: u32) {
        if !(1..=9).contains(&quality) {
            return;
        }
        self.iterations = match quality {
            1..=3 => 2,
            4..=6 => 4,
            _ => 8,
        };
        (self.tau, self.damping) = match (quality - 1) % 3 {
            0 => (0.3, 0.9),
            1 => (0.6, 1.),
            _ => (0.9, 1.1),
        };
    }
}

/// Stabilized constraint query coefficients, sampled at 140712bd0 across
/// 30 live machinery callbacks. The velocity solver has separate .6/1 tuning.
#[derive(Clone, Copy, Debug)]
pub struct ConstraintParameters {
    pub impulse_seconds: f32,
    pub inverse_impulse_seconds: f32,
    pub inverse_step_tau: f32,
    pub damping: f32,
    pub stabilized_tau: f32,
    pub stabilized_damping: f32,
}
impl ConstraintParameters {
    pub fn from_step(step: &SolverParameters) -> Self {
        let stabilized_tau = 0.7;
        let stabilized_damping = 1.1;
        Self {
            impulse_seconds: step.substep_seconds,
            inverse_impulse_seconds: step.inverse_substep_seconds,
            inverse_step_tau: step.inverse_substep_seconds * (stabilized_tau / stabilized_damping),
            damping: 1.,
            stabilized_tau,
            stabilized_damping,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solver_quality_presets_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/solver_quality.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let mut tuning = SolverTuning {
                tau: f32::from_bits(words[1]),
                damping: f32::from_bits(words[2]),
                iterations: words[3],
            };
            tuning.set_quality(words[0]);
            assert_eq!(
                [
                    tuning.tau.to_bits(),
                    tuning.damping.to_bits(),
                    tuning.iterations
                ],
                words[4..],
                "quality {count}"
            );
            count += 1;
        }
        assert_eq!(count, 16);
    }
    #[test]
    fn constraint_coefficients_match_live_machinery() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/constraint_step_coefficients.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(words[i]);
            let step = SolverParameters::from_game_deltas(&[v(3)], [0.; 4]).unwrap();
            let c = ConstraintParameters::from_step(&step);
            let actual = [
                step.substep_seconds,
                c.impulse_seconds,
                c.inverse_impulse_seconds,
                step.seconds,
                step.inverse_seconds,
                c.inverse_step_tau,
                c.damping,
                c.stabilized_tau,
                c.stabilized_damping,
            ];
            assert_eq!(
                actual.map(f32::to_bits).as_slice(),
                words.as_slice(),
                "constraint timing case{count}"
            );
            count += 1;
        }
        assert_eq!(count, 30);
    }
    #[test]
    fn solver_steps_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/solver_steps.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let s = SolverParameters::new(
                v(0),
                v(1) as u32,
                v(2) as u32,
                v(3),
                v(4),
                std::array::from_fn(|i| v(5 + i)),
            );
            let phase = s.phase(v(9) as u32, v(10) as u32);
            let actual = [
                s.seconds,
                s.inverse_seconds,
                s.substep_seconds,
                s.inverse_substep_seconds,
                s.inverse_substeps,
                s.inverse_iterations,
                s.tau,
                s.damping,
                s.velocity_scale,
                s.inverse_velocity_scale,
                s.inverse_step_tau,
                s.minimum_correction,
            ];
            let mut actual: Vec<u32> = actual
                .into_iter()
                .chain(s.gravity_substep)
                .chain(s.gravity_step)
                .chain([s.gravity_length, phase.velocity_fraction])
                .map(f32::to_bits)
                .collect();
            actual.push(phase.flags);
            assert_eq!(actual, bits[11..], "solver step case {count}");
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn world_uses_largest_advancing_time_space() {
        assert!(SolverParameters::from_game_deltas(&[0.; 5], [0.; 4]).is_none());
        let s = SolverParameters::from_game_deltas(
            &[0., 1. / 120., 1. / 60., 0., 0.],
            [0., 0., -9.81, 0.],
        )
        .unwrap();
        assert_eq!(s.seconds, 1. / 60.);
        assert_eq!(s.substeps, 2);
        assert_eq!(s.phase(0, 0).flags, 1);
        assert_eq!(s.phase(1, 0).flags, 2);
    }
}
