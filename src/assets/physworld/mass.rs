//! Shape mass properties used by rigid-body and constraint construction.
use super::{
    float, le32, normalize_body_quaternion, parse_embedded_havok, quaternion_basis, slice,
};
mod bounds;
pub use bounds::{bounds_mass_properties, box_mass_properties, BoxMassProperties};
#[path = "mass/tensor.rs"]
mod tensor;
pub use tensor::{add_parallel_axis, rotate_inertia, subtract_parallel_axis};
#[path = "mass/combine.rs"]
mod combine;
pub use combine::{combine_mass, MassElement};
#[path = "mass/principal_axes.rs"]
mod principal_axes;
pub use principal_axes::{diagonalize_inertia, principal_inertia_axes};

#[derive(Clone, Copy, Debug)]
pub struct MassProperties {
    pub volume: f32,
    pub mass: f32,
    pub center: [f32; 4],
    /// Three column vectors, including the Havok padding lanes.
    pub inertia: [[f32; 4]; 3],
}

/// 140c291c0's mass-space representation, before conversion to a full tensor.
#[derive(Clone, Copy, Debug)]
pub struct PrincipalMassProperties {
    pub volume: f32,
    pub mass: f32,
    pub center: [f32; 4],
    pub inertia: [f32; 4],
    pub orientation: [f32; 4],
}

impl MassProperties {
    /// c28d50: diagonalize the tensor and retain the source's volume in the
    /// fourth principal-inertia lane before converting the axes to a quaternion.
    pub fn to_principal(self) -> PrincipalMassProperties {
        let mut tensor = self.inertia;
        let axes = principal_inertia_axes(&mut tensor);
        PrincipalMassProperties {
            volume: self.volume,
            mass: self.mass,
            center: self.center,
            inertia: [tensor[0][0], tensor[1][1], tensor[2][2], self.volume],
            orientation: super::rotation_quaternion(axes),
        }
    }
}

impl PrincipalMassProperties {
    /// c28e20 rotates principal inertia into the full tensor. This is the
    /// column product with a transposed basis used by ae1ba0, including W.
    pub fn to_tensor(self) -> MassProperties {
        let basis = quaternion_basis(self.orientation);
        let scaled: [[f32; 4]; 3] = std::array::from_fn(|i| basis[i].map(|v| self.inertia[i] * v));
        let inertia = std::array::from_fn(|column| {
            std::array::from_fn(|row| {
                (basis[1][column] * scaled[1][row] + basis[0][column] * scaled[0][row])
                    + basis[2][column] * scaled[2][row]
            })
        });
        MassProperties {
            volume: self.volume,
            mass: self.mass,
            center: self.center,
            inertia,
        }
    }

    /// 140caf5c0's explicit nonnegative mass override. A negative value leaves
    /// the shape's mass unchanged, as does an unordered override; zero preserves
    /// the existing inertia tensor. COMISS/JC supplies the unordered branch.
    pub fn set_mass(&mut self, mass: f32) {
        if mass.is_nan() || mass < 0. {
            return;
        }
        if mass != 0. {
            let scale = mass / self.mass;
            self.inertia = self.inertia.map(|v| scale * v);
        }
        self.mass = mass;
    }
}

#[cfg(test)]
#[path = "mass/tensor_tests.rs"]
mod tensor_tests;

/// caf5c0 selects cached/shape-calculated mass or an authored distribution,
/// then applies the explicit override. The caller must supply the real shape
/// calculation for an uncached shape; absence never fabricates unit inertia.
/// The three callback words preserve the source's mass-calculation config.
pub fn creation_mass_properties(
    cached: Option<PrincipalMassProperties>,
    distribution: Option<super::body::MassDistribution>,
    mass_override: f32,
    calculate: impl FnOnce([u32; 3]) -> Result<PrincipalMassProperties, String>,
) -> Result<PrincipalMassProperties, String> {
    if let Some(distribution) = distribution {
        let mut properties = PrincipalMassProperties {
            volume: distribution.center_of_mass_and_volume[3],
            mass: 1.,
            center: [
                distribution.center_of_mass_and_volume[0],
                distribution.center_of_mass_and_volume[1],
                distribution.center_of_mass_and_volume[2],
                0.,
            ],
            inertia: [
                distribution.inertia_tensor[0],
                distribution.inertia_tensor[1],
                distribution.inertia_tensor[2],
                0.,
            ],
            orientation: distribution.major_axis_space,
        };
        let requested = if mass_override.is_nan() || mass_override < 0. {
            cached.map_or(1., |properties| properties.mass)
        } else {
            mass_override
        };
        properties.set_mass(requested);
        Ok(properties)
    } else {
        let mut properties = match cached {
            Some(properties) => properties,
            None => calculate([0, 0x3fc0_0000, 0xc47a_0000])?,
        };
        properties.set_mass(mass_override);
        Ok(properties)
    }
}

pub fn decode_principal(bytes: &[u8]) -> Result<PrincipalMassProperties, String> {
    let bytes = slice(bytes, 0, 32)?;
    let words: [u16; 12] =
        std::array::from_fn(|i| u16::from_le_bytes(bytes[i * 2..i * 2 + 2].try_into().unwrap()));
    let vector = |at: usize| {
        let scale = f32::from_bits(u32::from(words[at + 3]) << 16);
        std::array::from_fn(|i| scale * ((u32::from(words[at + i]) << 16) as i32 as f32))
    };
    let orientation = normalize_body_quaternion(std::array::from_fn(|i| {
        ((u32::from(words[8 + i]) << 16).wrapping_add(0x8000_0000) as i32 as f32)
            * f32::from_bits(0x300b_cf65)
    }));
    Ok(PrincipalMassProperties {
        volume: float(bytes, 28)?,
        mass: float(bytes, 24)?,
        center: vector(0),
        inertia: vector(4),
        orientation,
    })
}

/// FUN_140c29080. Packed vector exponents are float bit patterns; their
/// mantissas are signed integers shifted sixteen bits, not half floats.
pub fn decode_compressed(bytes: &[u8]) -> Result<MassProperties, String> {
    let bytes = slice(bytes, 0, 32)?;
    let words: [u16; 12] =
        std::array::from_fn(|i| u16::from_le_bytes(bytes[i * 2..i * 2 + 2].try_into().unwrap()));
    let packed_vector = |at: usize| {
        let scale = f32::from_bits(u32::from(words[at + 3]) << 16);
        std::array::from_fn::<_, 4, _>(|i| scale * ((u32::from(words[at + i]) << 16) as i32 as f32))
    };
    let principal = packed_vector(4);
    let coefficient = f32::from_bits(0x300b_cf65);
    let orientation = normalize_body_quaternion(std::array::from_fn(|i| {
        ((u32::from(words[8 + i]) << 16).wrapping_add(0x8000_0000) as i32 as f32) * coefficient
    }));
    let basis = quaternion_basis(orientation);
    let scaled: [[f32; 4]; 3] = std::array::from_fn(|i| basis[i].map(|v| v * principal[i]));
    let inertia = std::array::from_fn(|column| {
        std::array::from_fn(|row| {
            (basis[1][column] * scaled[1][row] + basis[0][column] * scaled[0][row])
                + basis[2][column] * scaled[2][row]
        })
    });
    Ok(MassProperties {
        volume: float(bytes, 28)?,
        mass: float(bytes, 24)?,
        center: packed_vector(0),
        inertia,
    })
}

/// Read the installed `.massProps` TAG0 resource through its own type metadata.
pub fn parse(bytes: &[u8]) -> Result<MassProperties, String> {
    decode_compressed(compressed_payload(bytes)?)
}

pub fn parse_principal(bytes: &[u8]) -> Result<PrincipalMassProperties, String> {
    decode_principal(compressed_payload(bytes)?)
}

fn compressed_payload(bytes: &[u8]) -> Result<&[u8], String> {
    if le32(bytes, 0)? != 0x4d50_0010 {
        return Err("unsupported shape mass-properties header".into());
    }
    let tagged = parse_embedded_havok(bytes)?;
    let item = tagged.items.get(1).ok_or("mass-properties root missing")?;
    if tagged.types[item.type_index].name != "hknpShapeMassProperties" {
        return Err("unexpected shape mass-properties root type".into());
    }
    let offset = tagged.field_offset(item.type_index, "compressedMassProperties")?;
    let root = tagged.item_at(bytes, 1, 0)?;
    slice(root, offset, 32)
}

#[cfg(test)]
#[path = "mass/bounds_tests.rs"]
mod bounds_tests;

#[cfg(test)]
#[path = "mass/creation_tests.rs"]
mod creation_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_mass_frames_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/principal_mass_cases.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let bytes: Vec<u8> = bits[..8].iter().flat_map(|v| v.to_le_bytes()).collect();
            let frame = decode_principal(&bytes).unwrap();
            let actual: Vec<u32> = [frame.volume, frame.mass]
                .into_iter()
                .chain(frame.center)
                .chain(frame.inertia)
                .chain(frame.orientation)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, bits[8..], "principal frame case {count}");
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn compressed_mass_properties_match_executable() {
        let mut count = 0;
        for row in include_str!("../../../tests/fixtures/compressed_mass_cases.txt")
            .lines()
            .filter(|row| !row.starts_with('#') && !row.is_empty())
        {
            let v: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let bytes: Vec<_> = v[..8].iter().flat_map(|v| v.to_le_bytes()).collect();
            let decoded = decode_compressed(&bytes).unwrap();
            let actual: Vec<_> = [decoded.volume, decoded.mass]
                .into_iter()
                .chain(decoded.center)
                .chain(decoded.inertia.into_iter().flatten())
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, v[8..], "mass decoder case {count}");
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_shape_mass_properties_decode() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(&base).unwrap();
        let mut count = 0;
        for path in catalog.paths_with_suffix(".massprops") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let properties = parse(&bytes).unwrap_or_else(|error| panic!("{path}: {error}"));
            assert!(
                properties.mass.is_finite() && properties.mass >= 0.,
                "{path}"
            );
            assert!(
                properties.volume.is_finite() && properties.volume >= 0.,
                "{path}"
            );
            assert!(
                properties
                    .center
                    .iter()
                    .chain(properties.inertia.iter().flatten())
                    .all(|v| v.is_finite()),
                "{path}"
            );
            count += 1;
        }
        assert!(count > 1000);
        println!("Decoded {count} installed shape mass-property resources");
    }
}
