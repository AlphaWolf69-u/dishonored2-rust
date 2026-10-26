//! Motion creation data, before reservation and the runtime's packed storage.
use super::{constraint_solver::reciprocal, rigid_body::BodyPose};
use crate::assets::physworld::{
    body::{BodyInfo, MassDistribution},
    mass::{creation_mass_properties, PrincipalMassProperties},
};

#[derive(Clone, Copy, Debug)]
pub struct MotionInfo {
    pub properties_id: u16,
    pub enable_deactivation: bool,
    pub inverse_mass: f32,
    /// Maximum linear acceleration distance per step (+8), then maximum
    /// rotation per step (+c), identified by hknpMotionCinfo's reflection fields.
    pub limits: [f32; 2],
    pub inverse_inertia: [f32; 4],
    pub pose: BodyPose,
    pub linear_velocity: [f32; 4],
    pub angular_velocity: [f32; 4],
}

impl Default for MotionInfo {
    /// cc2b10. Padding is not modeled; all meaningful creation fields are.
    fn default() -> Self {
        Self {
            properties_id: u16::MAX,
            enable_deactivation: true,
            inverse_mass: 1.,
            limits: [f32::from_bits(0x5f7f_fff0); 2],
            inverse_inertia: [1.; 4],
            pose: BodyPose {
                center: [0.; 4],
                orientation: [0., 0., 0., 1.],
            },
            linear_velocity: [0.; 4],
            angular_velocity: [0.; 4],
        }
    }
}

impl MotionInfo {
    /// The compound shape's c97760 mode-zero branch delegates through c96ab0
    /// to ce0820. Its identity calcAabb still changes rounding and the W lane;
    /// use the verified shape transform rather than passing raw stored bounds.
    pub fn from_compound_body(
        body: &BodyInfo,
        cached: Option<PrincipalMassProperties>,
        distribution: Option<MassDistribution>,
        bounds: [[f32; 4]; 2],
    ) -> Result<Self, String> {
        Self::from_body(body, cached, distribution, |config| {
            calculate_compound_mass(config, bounds)
        })
    }

    /// cc2b10 -> cb9680 -> caf5c0. An uncached shape must provide its actual
    /// mass calculation; the mass-frame choice is never inferred from flags.
    pub fn from_body(
        body: &BodyInfo,
        cached: Option<PrincipalMassProperties>,
        distribution: Option<MassDistribution>,
        calculate: impl FnOnce([u32; 3]) -> Result<PrincipalMassProperties, String>,
    ) -> Result<Self, String> {
        if (body.mass_distribution != 0) != distribution.is_some() {
            return Err("motion mass distribution reference is unresolved".into());
        }
        let frame = creation_mass_properties(cached, distribution, body.mass, calculate)?;
        let mut info = Self::default();
        info.apply_body(body, frame);
        Ok(info)
    }

    /// cb9680 after selecting the actual mass frame. Keep the constructor's
    /// limit/deactivation fields, including on a reused creation record.
    pub fn apply_body(&mut self, body: &BodyInfo, frame: PrincipalMassProperties) {
        self.pose = BodyPose::from_mass_frame(frame, body.orientation, body.position);
        self.inverse_mass = if frame.mass == 0. {
            0.
        } else {
            1. / frame.mass
        };
        self.inverse_inertia = [
            if frame.inertia[0] == 0. {
                0.
            } else {
                reciprocal(frame.inertia[0])
            },
            if frame.inertia[1] == 0. {
                0.
            } else {
                reciprocal(frame.inertia[1])
            },
            if frame.inertia[2] == 0. {
                0.
            } else {
                reciprocal(frame.inertia[2])
            },
            1.,
        ];
        if body.motion_type == 1 {
            self.inverse_mass = 0.;
            self.inverse_inertia = [0.; 4];
        }
        self.properties_id = body.motion_properties_id;
        self.linear_velocity = body.linear_velocity;
        self.angular_velocity = body.angular_velocity;
    }
}

pub(crate) fn calculate_compound_mass(
    config: [u32; 3],
    bounds: [[f32; 4]; 2],
) -> Result<PrincipalMassProperties, String> {
    if config[0] != 0 {
        return Err(
            "compound child-mass aggregation requires its separate calculation mode".into(),
        );
    }
    let identity = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    let bounds = super::shape_transform::transform_bounds(identity, bounds);
    Ok(crate::assets::physworld::mass::bounds_mass_properties(
        config, bounds,
    ))
}

#[cfg(test)]
#[path = "motion_info/tests.rs"]
mod tests;
