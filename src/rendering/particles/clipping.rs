//! The system's clipping planes, rebuilt every update by 1401fe2b0: the
//! water plane of 2010d0 first, then each authored clipping_t::plane_t. The
//! physics owner supplies the selected water-body AABB height or the
//! prepared ocean height.
use crate::{
    assets::fx::particle_declaration::ParticleClipping,
    float::{add, divide, multiply, subtract},
    gameplay::reference_frame::ReferenceFrame,
};

#[derive(Clone, Copy)]
pub enum ParticleWaterSurface {
    Volume(f32),
    Ocean(f32),
}

pub struct ParticleClipPlanes {
    /// System +0x1dd0: world-space simulation planes.
    pub world: Vec<[f32; 4]>,
    /// System +0x1de8: the same planes in the system's own space.
    pub local: Vec<[f32; 4]>,
    /// fx/ClipPlane, when the water plane or an authored plane sets it.
    pub rendering: Option<[f32; 4]>,
    pub fade_distance: f32,
}
impl ParticleClipPlanes {
    pub fn new(
        settings: &ParticleClipping,
        owner: ReferenceFrame,
        surface: Option<ParticleWaterSurface>,
    ) -> Option<Self> {
        let negative = |value: f32| f32::from_bits(value.to_bits() ^ 0x8000_0000);
        let mut planes = Self {
            world: Vec::new(),
            local: Vec::new(),
            rendering: None,
            fade_distance: settings.rendering_fade_distance,
        };
        if let Some(surface) = surface.filter(|_| settings.use_ocean_plane) {
            let (height, offset) = match surface {
                ParticleWaterSurface::Volume(height) => (height, settings.water_volume_offset),
                ParticleWaterSurface::Ocean(height) => (height, settings.ocean_plane_offset),
            };
            let normal = [0_f32, 0., 1.].map(|value| {
                if settings.invert_ocean_plane { negative(value) } else { value }
            });
            let xy = add(multiply(normal[1], 0.), multiply(normal[0], 0.));
            planes.world.push([
                normal[0],
                normal[1],
                normal[2],
                negative(add(multiply(normal[2], add(height, offset)), xy)),
            ]);
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
            planes.local.push([local_normal[0], local_normal[1], local_normal[2], distance]);
            planes.rendering = Some([
                normal[0],
                normal[1],
                normal[2],
                negative(add(
                    multiply(add(height, settings.rendering_offset), normal[2]),
                    xy,
                )),
            ]);
        }
        let m = owner.basis;
        let p = owner.position;
        for plane in &settings.planes {
            let [x, y, z] = plane.normal;
            // The squared length is clamped to FLT_MIN (DAT_141ae3c60).
            let squared = add(add(multiply(y, y), multiply(x, x)), multiply(z, z))
                .max(f32::MIN_POSITIVE);
            let inverse = divide(1., squared.sqrt());
            let (x, y, z) = (multiply(x, inverse), multiply(y, inverse), multiply(z, inverse));
            let d = plane.distance;
            // -(((n.y*d + p.y)*n.y + (n.x*d + p.x)*n.x) + (n.z*d + p.z)*n.z)
            let through = |n: [f32; 3]| negative(add(
                add(
                    multiply(add(multiply(n[1], d), p[1]), n[1]),
                    multiply(add(multiply(n[0], d), p[0]), n[0]),
                ),
                multiply(add(multiply(n[2], d), p[2]), n[2]),
            ));
            if plane.worldspace {
                planes.world.push([x, y, z, through([x, y, z])]);
                let n = [
                    add(add(multiply(y, m[0][1]), multiply(x, m[0][0])), multiply(z, m[0][2])),
                    add(add(multiply(x, m[1][0]), multiply(y, m[1][1])), multiply(z, m[1][2])),
                    add(add(multiply(x, m[2][0]), multiply(y, m[2][1])), multiply(z, m[2][2])),
                ];
                planes.local.push([n[0], n[1], n[2], negative(add(
                    add(multiply(multiply(n[1], n[1]), d), multiply(multiply(n[0], n[0]), d)),
                    multiply(multiply(n[2], n[2]), d),
                ))]);
            } else {
                let n = [
                    add(add(multiply(y, m[1][0]), multiply(x, m[0][0])), multiply(z, m[2][0])),
                    add(add(multiply(y, m[1][1]), multiply(x, m[0][1])), multiply(z, m[2][1])),
                    add(add(multiply(y, m[1][2]), multiply(x, m[0][2])), multiply(z, m[2][2])),
                ];
                planes.world.push([n[0], n[1], n[2], through(n)]);
                planes.local.push([x, y, z, negative(add(
                    add(multiply(multiply(y, d), y), multiply(multiply(x, d), x)),
                    multiply(multiply(z, d), z),
                ))]);
            }
        }
        // Without a water plane, fx/ClipPlane takes the first world plane's
        // normal through the system origin plus m_renderingOffset; the
        // plane's own distance is not used.
        if planes.rendering.is_none() && !settings.planes.is_empty() {
            let [x, y, z, _] = planes.world[0];
            planes.rendering = Some([x, y, z, negative(add(
                add(add(multiply(x, p[0]), multiply(p[1], y)), multiply(p[2], z)),
                settings.rendering_offset,
            ))]);
        }
        (!planes.world.is_empty()).then_some(planes)
    }
}
