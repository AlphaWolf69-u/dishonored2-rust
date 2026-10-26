//! Runtime packed motion initialized from actual motion creation data.
use super::{motion_info::MotionInfo, rigid_body::BodyPose, space_cells::SpaceCells};
#[path = "motion/velocity_frame.rs"]
mod velocity_frame;
#[path = "motion/body_pose.rs"]
mod body_pose;
#[derive(Clone, Copy, Debug, Default)]
pub struct Motion {
    pub pose: BodyPose,
    pub inverse_mass: [u16; 4],
    pub first_body_id: u32,
    pub solver_id: u32,
    /// Three bfloat16 lanes; the fourth source half-word is integration_factor.
    pub linear_velocity_cage: [u16; 3],
    pub integration_factor: u16,
    pub properties_id: u16,
    /// Packed linear acceleration distance and rotation limits per step.
    pub limits: [u16; 2],
    pub cell: u8,
    pub space_splitter_weight: u8,
    pub linear_velocity: [f32; 4],
    pub angular_velocity: [f32; 4],
    pub previous_step_linear_velocity: [f32; 4],
    pub previous_step_angular_velocity: [f32; 4],
}
impl Motion {
    /// c92be0 chooses the dynamic attachment path for a nonzero solver ID.
    /// Zero inverse mass alone does not make a keyframed motion static.
    pub fn requires_dynamic_body(&self) -> bool {
        self.solver_id != 0
    }

    /// ca6e30. Cell assignment comes from the world's actual space cells.
    pub fn initialize(info: &MotionInfo, cells: &SpaceCells) -> Result<Self, String> {
        let inverse = [
            info.inverse_inertia[0],
            info.inverse_inertia[1],
            info.inverse_inertia[2],
            info.inverse_mass,
        ];
        let inverse_mass = inverse.map(|value| {
            let shifted = (value * f32::from_bits(0x3f80_8000)).to_bits() as i32 >> 16;
            shifted.clamp(i16::MIN as i32, i16::MAX as i32) as u16
        });
        let limits = info
            .limits
            .map(|value| ((value * f32::from_bits(0x3f80_8000)).to_bits() as i32 >> 16) as u16);
        let mut linear_velocity = info.linear_velocity;
        linear_velocity[3] = 0.;
        let mut angular_velocity = info.pose.inverse_rotate_vector(info.angular_velocity);
        angular_velocity[3] = 0.;
        Ok(Self {
            pose: info.pose,
            inverse_mass,
            first_body_id: 0x00ff_ffff,
            solver_id: 0x7fff_ffff,
            linear_velocity_cage: [0; 3],
            integration_factor: 0,
            properties_id: if info.properties_id == u16::MAX {
                1
            } else {
                info.properties_id
            },
            limits,
            cell: cells.select(info.pose.center)?,
            space_splitter_weight: 1,
            linear_velocity,
            angular_velocity,
            previous_step_linear_velocity: [0.; 4],
            previous_step_angular_velocity: [0.; 4],
        })
    }
}
#[cfg(test)]
#[path = "motion/tests.rs"]
mod tests;
