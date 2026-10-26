//! 2010d0's simulation and rendering planes. The physics owner supplies its
//! selected water-body AABB height or the prepared ocean height.
use crate::{
    assets::fx::particle_declaration::ParticleClipping,
    float::{add, multiply, subtract},
    gameplay::reference_frame::ReferenceFrame,
};

#[derive(Clone, Copy)]
pub enum ParticleWaterSurface {
    Volume(f32),
    Ocean(f32),
}

pub struct ParticleClipPlanes {
    pub world: [f32; 4],
    pub local: [f32; 4],
    pub rendering: [f32; 4],
    pub fade_distance: f32,
}
impl ParticleClipPlanes {
    pub fn new(
        settings: &ParticleClipping,
        owner: ReferenceFrame,
        surface: ParticleWaterSurface,
    ) -> Option<Self> {
        if !settings.use_ocean_plane {
            return None;
        }
        let (height, offset) = match surface {
            ParticleWaterSurface::Volume(height) => (height, settings.water_volume_offset),
            ParticleWaterSurface::Ocean(height) => (height, settings.ocean_plane_offset),
        };
        let normal = [0_f32, 0., 1.].map(|value| {
            if settings.invert_ocean_plane {
                f32::from_bits(value.to_bits() ^ 0x8000_0000)
            } else {
                value
            }
        });
        let xy = add(multiply(normal[1], 0.), multiply(normal[0], 0.));
        let negative = |value: f32| f32::from_bits(value.to_bits() ^ 0x8000_0000);
        let world = [
            normal[0],
            normal[1],
            normal[2],
            negative(add(multiply(normal[2], add(height, offset)), xy)),
        ];
        let local_normal: [f32; 3] = owner.basis.map(|axis| {
            add(
                add(multiply(axis[0], normal[0]), multiply(axis[1], normal[1])),
                multiply(axis[2], normal[2]),
            )
        });
        let point = [
            subtract(0., owner.position[0]),
            subtract(0., owner.position[1]),
            subtract(add(height, offset), owner.position[2]),
        ];
        let point: [f32; 3] = owner.basis.map(|axis| {
            add(
                add(multiply(axis[0], point[0]), multiply(axis[1], point[1])),
                multiply(axis[2], point[2]),
            )
        });
        let distance = negative(add(
            add(
                multiply(point[1], local_normal[1]),
                multiply(point[0], local_normal[0]),
            ),
            multiply(point[2], local_normal[2]),
        ));
        Some(Self {
            world,
            local: [local_normal[0], local_normal[1], local_normal[2], distance],
            rendering: [
                normal[0],
                normal[1],
                normal[2],
                negative(add(
                    multiply(add(height, settings.rendering_offset), normal[2]),
                    xy,
                )),
            ],
            fade_distance: settings.rendering_fade_distance,
        })
    }
}
