//! The seven orientation modes in 1f78f0, including camera-relative and
//! velocity/custom-axis frames. The caller supplies the actual view axes.
use super::parameters::VectorParameter;
use crate::float::{add, divide, multiply, subtract};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum OrientationMode {
    Emitter = 0,
    FaceCamera = 1,
    CameraAxes = 2,
    VelocityZ = 3,
    VelocityX = 4,
    CustomZ = 5,
    CustomX = 6,
}

pub struct ParticleOrientation {
    pub mode: OrientationMode,
    pub primary: VectorParameter,
    pub primary_worldspace: bool,
    pub secondary: VectorParameter,
    pub secondary_worldspace: bool,
    pub secondary_uses_view_direction: bool,
}
impl Default for ParticleOrientation {
    /// 1f8a70 initializes facing mode1 and two constant [1,0,0] axes.
    fn default() -> Self {
        Self {
            mode: OrientationMode::FaceCamera,
            primary: VectorParameter::constant([1., 0., 0.]),
            primary_worldspace: false,
            secondary: VectorParameter::constant([1., 0., 0.]),
            secondary_worldspace: false,
            secondary_uses_view_direction: true,
        }
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        subtract(multiply(a[1], b[2]), multiply(a[2], b[1])),
        subtract(multiply(a[2], b[0]), multiply(a[0], b[2])),
        subtract(multiply(a[0], b[1]), multiply(a[1], b[0])),
    ]
}
fn negative(v: [f32; 3]) -> [f32; 3] {
    v.map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000))
}

/// 0b8670's scalar normalization also returns its original length. It retains
/// zero vectors, clamps only squared length, and does not use Havok's rsqrt.
pub fn normalize(value: &mut [f32; 3]) -> f32 {
    let squared = add(
        add(multiply(value[0], value[0]), multiply(value[1], value[1])),
        multiply(value[2], value[2]),
    );
    let mut clamped = squared;
    if clamped <= f32::MIN_POSITIVE {
        clamped = f32::MIN_POSITIVE;
    }
    let reciprocal = divide(1., clamped.sqrt());
    *value = value.map(|v| multiply(v, reciprocal));
    multiply(reciprocal, squared)
}

/// 00acd0's compact basis multiplication, with the source's Y/X sum on X.
fn transform(value: [f32; 3], basis: [[f32; 3]; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        let pair = if i == 0 {
            add(
                multiply(value[1], basis[1][i]),
                multiply(value[0], basis[0][i]),
            )
        } else {
            add(
                multiply(value[0], basis[0][i]),
                multiply(value[1], basis[1][i]),
            )
        };
        add(pair, multiply(value[2], basis[2][i]))
    })
}

impl ParticleOrientation {
    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &self,
        age: f32,
        seed: &mut u32,
        custom_basis: [[f32; 3]; 3],
        emitter_basis: [[f32; 3]; 3],
        to_camera: [f32; 3],
        velocity: [f32; 3],
        camera_axes: [[f32; 3]; 3],
    ) -> Result<[[f32; 3]; 3], String> {
        match self.mode {
            OrientationMode::Emitter => return Ok(emitter_basis),
            OrientationMode::FaceCamera => {
                let camera_second = negative(camera_axes[1]);
                let side = [
                    subtract(
                        multiply(camera_second[2], to_camera[1]),
                        multiply(camera_second[1], to_camera[2]),
                    ),
                    subtract(
                        multiply(camera_second[0], to_camera[2]),
                        multiply(camera_second[2], to_camera[0]),
                    ),
                    subtract(
                        multiply(camera_second[1], to_camera[0]),
                        multiply(camera_second[0], to_camera[1]),
                    ),
                ];
                let second = negative([
                    subtract(
                        multiply(side[2], to_camera[1]),
                        multiply(side[1], to_camera[2]),
                    ),
                    subtract(
                        multiply(side[0], to_camera[2]),
                        multiply(side[2], to_camera[0]),
                    ),
                    subtract(
                        multiply(side[1], to_camera[0]),
                        multiply(side[0], to_camera[1]),
                    ),
                ]);
                return Ok([
                    to_camera,
                    second,
                    [
                        subtract(
                            multiply(second[2], to_camera[1]),
                            multiply(second[1], to_camera[2]),
                        ),
                        subtract(
                            multiply(second[0], to_camera[2]),
                            multiply(second[2], to_camera[0]),
                        ),
                        subtract(
                            multiply(second[1], to_camera[0]),
                            multiply(second[0], to_camera[1]),
                        ),
                    ],
                ]);
            }
            OrientationMode::CameraAxes => {
                return Ok([
                    negative(camera_axes[0]),
                    negative(camera_axes[1]),
                    camera_axes[2],
                ])
            }
            _ => {}
        }
        let primary = match self.mode {
            OrientationMode::VelocityZ | OrientationMode::VelocityX => velocity,
            OrientationMode::CustomZ | OrientationMode::CustomX => {
                let mut value = self.primary.sample(age, seed)?;
                if !self.primary_worldspace {
                    value = transform(value, custom_basis);
                }
                normalize(&mut value);
                value
            }
            _ => unreachable!(),
        };
        let secondary = if self.secondary_uses_view_direction {
            to_camera
        } else {
            let mut value = self.secondary.sample(age, seed)?;
            if !self.secondary_worldspace {
                value = transform(value, custom_basis);
            }
            normalize(&mut value);
            value
        };
        Ok(match self.mode {
            OrientationMode::VelocityZ | OrientationMode::CustomZ => {
                let mut second = cross(primary, secondary);
                normalize(&mut second);
                [cross(second, primary), second, primary]
            }
            OrientationMode::VelocityX | OrientationMode::CustomX => {
                let mut second = cross(secondary, primary);
                normalize(&mut second);
                [primary, second, cross(primary, second)]
            }
            _ => unreachable!(),
        })
    }
}
