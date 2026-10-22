//! 3160f0's installed material selection. Rendering these variants still needs
//! the DOF task's real alpha/depth/autofocus inputs and combine state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DofPass {
    Alpha,
    Near,
    Far,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
