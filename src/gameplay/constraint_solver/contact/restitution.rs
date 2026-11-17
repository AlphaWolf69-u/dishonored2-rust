//! hknpRestitutionModifier's actual persistent-point callbacks.
//! Vtable141ec6a18; 140cf2140, 140cf2180/2330, 140cf2500 and140cf2750.
use super::{ContactCache, ContactRows};
use crate::gameplay::{constraint_solver::step::SolverParameters, motion::Motion};

#[derive(Clone, Copy, Debug, Default)]
pub struct RestitutionCache {
    pub velocities: [f32; 4],
    pub point_ids: [u32; 4],
    pub count: u8,
}

impl RestitutionCache {
    /// cf2750 keeps the same actual point's captured velocity when native
    /// contact IDs reorder; a genuinely new point starts at zero.
    pub fn remap(&mut self, ids: &[u32]) -> Result<(), &'static str> {
        if ids.len() > 4 {
            return Err("restitution cache exceeds the native contact manifold");
        }
        let previous = *self;
        for (index, &id) in ids.iter().enumerate() {
            self.velocities[index] =
                if index < usize::from(previous.count) && previous.point_ids[index] == id {
                    previous.velocities[index]
                } else {
                    previous.point_ids[..usize::from(previous.count)]
                        .iter()
                        .position(|&old| old == id)
                        .map_or(0.0, |slot| previous.velocities[slot])
                };
            self.point_ids[index] = id;
        }
        self.velocities[ids.len()..].fill(0.0);
        self.count = ids.len() as u8;
        Ok(())
    }

    /// Invoke after the real Jacobian build and before stream solve, as the
    /// registered modifier callback does. It changes the existing rows only.
    pub fn apply(
        &mut self,
        rows: &mut ContactRows,
        contact_cache: &mut ContactCache,
        motions: [&Motion; 2],
        step: &SolverParameters,
        restitution: f32,
    ) -> Result<bool, &'static str> {
        if usize::from(self.count) != usize::from(rows.count) {
            return Err("restitution cache has not been matched to the actual manifold");
        }
        if restitution.to_bits() & 0x7fff_ffff == 0 {
            return Ok(false);
        }
        let [a, b] = motions;
        let inverse_mass = f32::from_bits(u32::from(b.inverse_mass[3]) << 16)
            + f32::from_bits(u32::from(a.inverse_mass[3]) << 16);
        let mut bounced = false;
        let mut relative = [0.0; 4];
        for (index, row) in rows.rows[..usize::from(rows.count)].iter().enumerate() {
            let velocity: [f32; 3] = std::array::from_fn(|i| {
                row.angular_b[i] * b.angular_velocity[i]
                    + a.angular_velocity[i] * row.angular_a[i]
                    + (a.linear_velocity[i] - b.linear_velocity[i]) * rows.normal[i]
            });
            relative[index] = (velocity[1] + velocity[0]) + velocity[2];
        }
        for (index, row) in rows.rows[..usize::from(rows.count)].iter_mut().enumerate() {
            let captured = self.velocities[index];
            // 141ec6a00 /1423ccd40 are supplied by the audited modifier
            // constants below; they are not configurable replacement tuning.
            if captured < -(RESTITUTION_THRESHOLD * step.gravity_length) * step.seconds
                && captured * CAPTURE_THRESHOLD < relative[index]
            {
                let inverse_damping = super::reciprocal_estimate(step.damping);
                row.rhs = ((((-inverse_damping * restitution) * captured) * row.effective_mass)
                    * inverse_mass)
                    * inverse_damping;
                self.velocities[index] = 0.0;
                bounced = true;
            }
        }
        for (index, value) in self.velocities[..usize::from(rows.count)]
            .iter_mut()
            .enumerate()
        {
            *value = super::min(CAPTURE_RECOVERY * step.seconds + *value, relative[index]);
        }
        if bounced {
            // cf2500 clears native normal retraction through c4e5a0.
            // Disassembly cf2719..2724 supplies XMM1=.25/XMM2=0 to
            // c4dfe0: zero RHS, quarter torsion mass; tangent inverse matrix
            // was already built and is not changed by that callback.
            rows.retract_normal_impulses = false;
            if let Some(friction) = rows.friction.as_mut() {
                friction.rhs = [0.0; 2];
                friction.spin_effective_mass *= f32::from_bits(0x3e80_0000);
                friction.spin_rhs = 0.0;
            }
            contact_cache.friction_factor = 0.0;
            self.velocities.fill(0.0);
        }
        Ok(bounced)
    }
}

// Constants are filled from the audited installed executable, not guessed
// thresholds. See the source-address comments at the producer above.
const RESTITUTION_THRESHOLD: f32 = f32::from_bits(0x3f800000);
const CAPTURE_THRESHOLD: f32 = f32::from_bits(0x3e800000);
const CAPTURE_RECOVERY: f32 = f32::from_bits(0x3f800000);
