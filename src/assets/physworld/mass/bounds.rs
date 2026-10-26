use super::PrincipalMassProperties;

#[derive(Clone, Copy, Debug)]
pub struct BoxMassProperties {
    pub volume: f32,
    pub inertia: [f32; 4],
}

/// c29650: no output for a nonpositive/unordered mass; negative half extents
/// retain the source's signed volume rather than being silently clamped.
pub fn box_mass_properties(half: [f32; 4], mass: f32) -> Option<BoxMassProperties> {
    if mass.is_nan() || mass <= 0. {
        return None;
    }
    let squared = half.map(|v| v * v);
    let factor = f32::from_bits(0x3eaa_aaab) * mass;
    Some(BoxMassProperties {
        volume: ((half[1] * half[0]) * half[2]) * 8.,
        inertia: [
            (squared[2] + squared[1]) * factor,
            (squared[2] + squared[0]) * factor,
            (squared[1] + squared[0]) * factor,
            1.,
        ],
    })
}

/// ce0820's mass from the actual shape AABB. A negative/unordered requested
/// mass takes the source density branch. Caller owns shape dispatch/config.
pub fn bounds_mass_properties(config: [u32; 3], bounds: [[f32; 4]; 2]) -> PrincipalMassProperties {
    let half = std::array::from_fn(|i| {
        let value = (bounds[1][i] - bounds[0][i]) * 0.5;
        value
            + if value == 0. {
                f32::from_bits(0x3500_0000)
            } else {
                0.
            }
    });
    let unit = box_mass_properties(half, 1.).unwrap();
    let requested = f32::from_bits(config[2]);
    let mass = if requested.is_nan() || requested < 0. {
        f32::from_bits((requested * unit.volume).to_bits() ^ 0x8000_0000)
    } else {
        requested
    };
    let scale = mass * f32::from_bits(config[1]);
    PrincipalMassProperties {
        volume: unit.volume,
        mass,
        center: std::array::from_fn(|i| (bounds[0][i] - bounds[1][i]) * 0.5 + bounds[1][i]),
        inertia: unit.inertia.map(|v| scale * v),
        orientation: [0., 0., 0., 1.],
    }
}
