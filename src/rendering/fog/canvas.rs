//! Camera-coherent turbulence coordinates: 2ff2f0 initialization and 2ff520.
use bevy_math::{Quat, Vec3, Vec4};

pub struct CanvasView {
    pub position_delta: Vec3,
    pub forward: Vec3,
    /// Game camera axis 1 (+Y for a +X-facing camera), pointing left.
    /// 2ff520 negates this axis when computing lateral movement.
    pub left: Vec3,
    pub up: Vec3,
    pub fov_radians: f32,
    pub inverse_frequency: f32,
    pub focus_distance: f32,
}
pub struct CanvasParameters {
    pub s: Vec4,
    pub t: Vec4,
    pub warp: Vec3,
    pub octaves: [Vec3; 10],
}
pub struct DynamicCanvas {
    offset: [f32; 2],
    previous_fov: f32,
    pitch: f32,
    azimuth: f32,
    bank: f32,
    spin: f32,
    zoom: f32,
    cursor: usize,
    offsets: [[f32; 2]; 128],
}
impl DynamicCanvas {
    pub fn new(counter: u64) -> Self {
        let mut random = crate::random::MwcRandom::new(counter);
        Self::with_offsets(std::array::from_fn(|_| {
            [random.between(-50., 50.), random.between(-50., 50.)]
        }))
    }
    fn with_offsets(offsets: [[f32; 2]; 128]) -> Self {
        Self {
            offset: [0.; 2],
            previous_fov: 1.,
            pitch: 0.,
            azimuth: 0.,
            bank: 0.,
            spin: 0.,
            zoom: 1.,
            cursor: 0,
            offsets,
        }
    }
    pub fn update(&mut self, view: CanvasView) -> CanvasParameters {
        let focus = view.focus_distance.max(0.01);
        let tangent = (view.fov_radians * 0.5).tan();
        let roll = if view.forward.truncate().length() > f32::from_bits(0x34000000) {
            view.left.z.atan2(view.up.z)
        } else {
            0.
        };
        let up = Quat::from_axis_angle(view.forward, -roll) * view.up;
        let pitch = up.truncate().length().atan2(up.z) * if view.forward.z > 0. { 1. } else { -1. };
        let azimuth = if up.z.abs() > view.forward.z.abs() {
            view.forward.y.atan2(-view.forward.x)
        } else if view.forward.z < 0. {
            up.y.atan2(-up.x)
        } else {
            (-up.y).atan2(up.x)
        };
        let difference = |previous: f32, current: f32| {
            if previous * current >= f32::from_bits(0xbf9e0610) {
                current - previous
            } else if previous <= 0. {
                (current - std::f32::consts::TAU) - previous
            } else {
                (std::f32::consts::TAU + current) - previous
            }
        };
        let pitch_delta = difference(self.pitch, pitch);
        let azimuth_delta = difference(self.azimuth, azimuth);
        let projected_bank = -view.left.dot(up);
        let bank = projected_bank.clamp(-1., 1.).asin();
        let bank_delta = difference(self.bank, bank);
        let correction = self.pitch.sin() * azimuth_delta;
        self.spin = (correction + bank_delta) + self.spin;
        self.pitch = pitch;
        self.azimuth = azimuth;
        self.bank = bank;
        let forward_motion = view.forward.dot(view.position_delta);
        self.zoom = ((focus - (forward_motion % focus)) / focus)
            * self.zoom
            * (view.fov_radians / self.previous_fov);
        self.previous_fov = view.fov_radians;
        while self.zoom >= 2. {
            self.zoom *= 0.5;
            self.offset = self.offset.map(|v| v * 0.5);
            self.cursor = (self.cursor + 127) % 128;
        }
        while self.zoom < 1. {
            self.zoom *= 2.;
            self.offset = self.offset.map(|v| v * 2.);
            self.cursor = (self.cursor + 1) % 128;
        }
        let scale = self.zoom / view.inverse_frequency;
        let angular_scale = scale / tangent;
        let (pan_x, pan_y) = if f64::from(pitch.abs()) >= 0.2 {
            let arc = angular_scale / pitch.tan();
            (
                correction.sin() * arc,
                (1. - correction.cos()) * arc + angular_scale * pitch_delta,
            )
        } else {
            (azimuth_delta * angular_scale, pitch_delta * angular_scale)
        };
        let (sin, cos) = self.spin.sin_cos();
        let lateral = -view.left.dot(view.position_delta);
        let vertical = view.up.dot(view.position_delta);
        self.offset[0] = (((cos * lateral - sin * vertical) * angular_scale) / focus
            + ((cos * pan_x - sin * pan_y) + self.offset[0]))
            % view.inverse_frequency;
        self.offset[1] = (((cos * vertical + sin * lateral) * angular_scale) / focus
            + ((cos * pan_y + sin * pan_x) + self.offset[1]))
            % view.inverse_frequency;
        let mut weight = 1024. / 1023.;
        let octaves = std::array::from_fn(|i| {
            let fade = if i == 0 {
                self.zoom - 1.
            } else if i == 9 {
                2. - self.zoom
            } else {
                1.
            };
            let offset = self.offsets[(self.cursor + i) % 128];
            let value = Vec3::new(offset[0], offset[1], (fade * weight) / self.zoom);
            weight *= 0.5;
            value
        });
        let remaining = std::f32::consts::FRAC_PI_2 - pitch.abs();
        let warp_blend = if f64::from(view.fov_radians) * 0.5 <= f64::from(remaining)
            && remaining < view.fov_radians
        {
            (view.fov_radians - remaining) / (view.fov_radians * 0.5)
        } else if f64::from(view.fov_radians) * 0.5 > f64::from(remaining) {
            1.
        } else {
            0.
        };
        CanvasParameters {
            s: Vec4::new(scale * cos, -scale * sin, 0., self.offset[0]),
            t: Vec4::new(scale * sin, scale * cos, 0., self.offset[1]),
            warp: Vec3::new(1. / tangent, pitch, warp_blend),
            octaves,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_motion_and_octaves_match_executable() {
        let mut canvas = DynamicCanvas::with_offsets(std::array::from_fn(|i| {
            [i as f32 * 0.137 - 8., i as f32 * (-0.193) + 12.]
        }));
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fog_canvas.txt"
        ));
        let mut cases = 0;
        for (id, line) in fixture.lines().enumerate() {
            let row = line
                .split_whitespace()
                .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(row.len(), 63);
            let vec = |at| Vec3::from_slice(&row[at..at + 3]);
            let actual = canvas.update(CanvasView {
                fov_radians: row[0],
                inverse_frequency: row[1],
                focus_distance: row[2],
                position_delta: vec(3) - vec(6) - vec(18) * row[21],
                forward: vec(9),
                left: vec(12),
                up: vec(15),
            });
            let values = actual
                .s
                .to_array()
                .into_iter()
                .chain(actual.t.to_array())
                .chain(actual.warp.to_array())
                .chain(actual.octaves.into_iter().flat_map(|v| v.to_array()));
            for (field, (value, reference)) in values.zip(row[22..].iter()).enumerate() {
                assert!(
                    (value - reference).abs() < 0.00003,
                    "canvas{id} field{field}: {value} != {reference}"
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 96);
    }
}
