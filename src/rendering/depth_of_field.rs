//! 3160f0's installed material selection. Rendering these variants still needs
//! the DOF task's real alpha/depth/autofocus inputs and combine state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DofPass {
    Alpha,
    Near,
    Far,
}

/// Validated filter metadata; authored fixed points remain installed assets.
#[derive(Clone, Debug)]
pub struct BlurKernel {
    kind: u8,
    samples: u32,
    radius_multiplier: f32,
    /// Empty for the procedural disk and circle filters.
    offsets: Vec<[f32; 2]>,
}
impl BlurKernel {
    pub fn kind(&self) -> u8 {
        self.kind
    }
    pub fn samples(&self) -> u32 {
        self.samples
    }
    pub fn radius_multiplier(&self) -> f32 {
        self.radius_multiplier
    }
    pub fn offsets(&self) -> &[[f32; 2]] {
        &self.offsets
    }
    /// Folded constants used by the compiled colour shaders, before their
    /// dynamic depth/strength samples. Keeping the inverse avoids new divides.
    pub fn shader_coefficients(&self) -> [f32; 4] {
        [
            self.radius_multiplier,
            self.radius_multiplier * 3.0,
            (1.0 / 255.0) / self.radius_multiplier,
            1.0 / self.samples as f32,
        ]
    }
    pub fn from_offsets(kind: u8, samples: u32, offsets: Vec<[f32; 2]>) -> Result<Self, String> {
        let (count, multiplier) = match kind {
            0 => (21, 1.),
            1 => (20, 2.25),
            2 => (21, 0.7),
            3 => (8, 2.5),
            4 => (60, 2.3),
            5 => (16, 0.7),
            _ => return Err(format!("unknown camera blur filter {kind}")),
        };
        let expected = if matches!(kind, 2 | 5) {
            0
        } else {
            count as usize
        };
        if samples != count
            || offsets.len() != expected
            || offsets.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(format!(
                "camera blur filter {kind} has inconsistent sample data"
            ));
        }
        Ok(Self {
            kind,
            samples,
            radius_multiplier: multiplier,
            offsets,
        })
    }
}

/// 3185e0's two separable near-mask draws. This is the size of the actual
/// near-mask target, not the full gameplay viewport.
#[derive(Clone, Debug)]
pub struct NearMaskBlur {
    pub offsets: [[f32; 2]; 8],
    pub weights: [f32; 8],
}
impl NearMaskBlur {
    pub fn separable(extent: [u32; 2], gain: f32) -> Result<[Self; 2], String> {
        if extent.contains(&0) {
            return Err("near-mask blur requires a nonempty target".into());
        }
        let mut weights = [
            0.015625, 0.09375, 0.234375, 0.3125, 0.234375, 0.09375, 0.015625, 0.0,
        ]
        .map(|weight| weight * gain);
        // The eighth uniform is written as literal zero, independently of gain.
        weights[7] = 0.0;
        Ok(std::array::from_fn(|axis| {
            let reciprocal = 1.0 / extent[axis] as f32;
            Self {
                offsets: [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 0.0].map(|value| {
                    let mut offset = [0.0; 2];
                    offset[axis] = reciprocal * value;
                    offset
                }),
                weights,
            }
        }))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DofMaterialVariant {
    pub pass: DofPass,
    /// None selects the unspecialized material, not filter zero.
    pub filter: Option<u8>,
    pub fringing: u8,
    pub cat_eye: u8,
}
impl DofMaterialVariant {
    pub fn select(filter: i32, cat_eye: u8, fringing: u8, pass_mask: u32) -> Option<Self> {
        let pass = match pass_mask {
            8 => DofPass::Alpha,
            16 => DofPass::Near,
            32 => DofPass::Far,
            _ => return None,
        };
        let variant = u32::from(cat_eye) + 2 * u32::from(fringing);
        let filter = if variant > 3 {
            None
        } else {
            Some(match filter {
                1 => 1,
                2 => 2,
                3 | 4 => 3,
                5 => 5,
                _ => 0,
            })
        };
        Some(Self {
            pass,
            filter,
            fringing: (variant / 2) as u8,
            cat_eye: (variant % 2) as u8,
        })
    }
    /// Resolve this once while preparing the immutable material bank.
    pub fn material_name(&self) -> String {
        let base = match self.pass {
            DofPass::Alpha => "dofAlpha",
            DofPass::Near => "dofNear",
            DofPass::Far => "dofFar",
        };
        match self.filter {
            Some(filter) => format!(
                "PostFX/DOF/{base}_{filter}_{}_{}.material",
                self.fringing, self.cat_eye
            ),
            None => format!("PostFX/DOF/{base}.material"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn near_blur_setup_matches_captured_constants() {
        for line in include_str!("../../tests/fixtures/camera_mask_blur_parameters.txt").lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            let extent = [words[1].parse().unwrap(), words[2].parse().unwrap()];
            let axis = words[3].parse::<usize>().unwrap();
            let gain = words[4].parse().unwrap();
            let p = &NearMaskBlur::separable(extent, gain).unwrap()[axis];
            let bytes: Vec<_> = p
                .offsets
                .iter()
                .flatten()
                .copied()
                .chain(p.weights)
                .flat_map(f32::to_le_bytes)
                .collect();
            let actual = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
            assert_eq!(actual, words[5]);
        }
    }
    #[test]
    fn material_selection_matches_executable() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/dof_material_selection.txt").lines() {
            let columns: Vec<_> = row.split_whitespace().collect();
            let actual = DofMaterialVariant::select(
                columns[0].parse().unwrap(),
                columns[1].parse().unwrap(),
                columns[2].parse().unwrap(),
                columns[3].parse().unwrap(),
            );
            assert_eq!(
                actual
                    .map(|v| v.material_name())
                    .as_deref()
                    .unwrap_or("none"),
                columns[4],
                "{row}"
            );
            count += 1;
        }
        assert_eq!(count, 315);
    }
}
