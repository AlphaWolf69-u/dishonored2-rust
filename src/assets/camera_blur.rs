//! Authored camera-blur sample banks, read directly from the installed include.
use super::archive::Catalog;
use crate::rendering::depth_of_field::{BlurKernel, DofMaterialVariant, DofPass};
use std::collections::HashMap;

/// Immutable inputs prepared during level loading and shared with render views.
#[derive(Clone, Debug)]
pub struct BlurResources {
    pub kernels: KernelBank,
    pub materials: MaterialBank,
}
impl BlurResources {
    pub fn load(catalog: &Catalog) -> Result<Self, String> {
        Ok(Self {
            kernels: KernelBank::load(catalog)?,
            materials: MaterialBank::load(catalog)?,
        })
    }
}

/// A selected material's actual options, including the unspecialized fallback.
#[derive(Clone, Debug)]
pub struct BlurMaterial {
    pub pass: DofPass,
    pub filter: u8,
    pub fringing: bool,
    pub cat_eye: bool,
    pub state: crate::render_state::RenderState,
}

#[derive(Clone, Debug)]
pub struct MaterialBank {
    materials: HashMap<DofMaterialVariant, BlurMaterial>,
}
impl MaterialBank {
    pub fn load(catalog: &Catalog) -> Result<Self, String> {
        use crate::rendering::material;
        let read = |path: &str| {
            catalog
                .read(path)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("camera material resource not indexed: {path}"))
        };
        let program = read("generated/decls/renderprog/postfx_dof.decl")?;
        let mut base = [0; 9];
        crate::render_state::reset_render_state(&mut base);
        let base =
            crate::render_state::material_render_state(&String::from_utf8_lossy(&program), base)?;
        let mut materials = HashMap::new();
        for pass in [DofPass::Alpha, DofPass::Near, DofPass::Far] {
            for filter in [Some(0), Some(1), Some(2), Some(3), Some(5), None] {
                let count = if filter.is_some() { 4 } else { 1 };
                for flags in 0..count {
                    let variant = DofMaterialVariant {
                        pass,
                        filter,
                        fringing: flags / 2,
                        cat_eye: flags % 2,
                    };
                    let name = variant.material_name();
                    let path = format!(
                        "generated/decls/material/{}.decl",
                        name.to_ascii_lowercase()
                    );
                    let bytes = read(&path)?;
                    let selected = material::authored_program_override(&bytes, "mainProgram")
                        .ok_or_else(|| format!("camera material has no main program: {name}"))?;
                    if !selected.eq_ignore_ascii_case("PostFX_DOF") {
                        return Err(format!(
                            "unsupported camera material program {selected}: {name}"
                        ));
                    }
                    const OPTIONS: [&str; 4] = [
                        "PostFX/DOF/DOF_TYPE",
                        "PostFX/DOF/FILTER",
                        "PostFX/DOF/USE_FRINGING",
                        "PostFX/DOF/USE_CATEYE",
                    ];
                    let values = material::material_option_parameters(&bytes, |n| {
                        OPTIONS.iter().any(|o| n.eq_ignore_ascii_case(o))
                    })?;
                    // Every installed member explicitly authors these four options.
                    // Missing overrides require resolving the compiled RPD defaults;
                    // they must not silently become filter zero or false switches.
                    let option = |name: &str| {
                        values
                            .iter()
                            .rev()
                            .find(|(n, _)| n.eq_ignore_ascii_case(name))
                            .map(|(_, v)| *v)
                            .ok_or_else(|| format!("unresolved camera option {name} in {path}"))
                    };
                    let mode = option(OPTIONS[0])?;
                    if mode
                        != match pass {
                            DofPass::Alpha => 0,
                            DofPass::Near => 1,
                            DofPass::Far => 2,
                        }
                    {
                        return Err(format!(
                            "camera material pass does not match its bank: {path}"
                        ));
                    }
                    let kind = option(OPTIONS[1])?;
                    if !(0..=5).contains(&kind) {
                        return Err(format!("unknown camera filter {kind}: {path}"));
                    }
                    materials.insert(
                        variant,
                        BlurMaterial {
                            pass,
                            filter: kind as u8,
                            fringing: option(OPTIONS[2])? != 0,
                            cat_eye: option(OPTIONS[3])? != 0,
                            state: crate::render_state::material_render_state(
                                &String::from_utf8_lossy(&bytes),
                                base,
                            )?,
                        },
                    );
                }
            }
        }
        Ok(Self { materials })
    }
    pub fn resolve(&self, mut variant: DofMaterialVariant) -> Option<&BlurMaterial> {
        // Every unspecialized selection uses the same material name regardless
        // of the out-of-range selector bytes that led to the fallback.
        if variant.filter.is_none() {
            variant.fringing = 0;
            variant.cat_eye = 0;
        }
        self.materials.get(&variant)
    }
}

#[derive(Clone, Debug)]
pub struct KernelBank {
    kernels: [BlurKernel; 6],
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed game archives"]
    fn camera_kernels_match_shader_constants() {
        let catalog = Catalog::open(&super::super::installation::game_base().unwrap()).unwrap();
        let bank = KernelBank::load(&catalog).unwrap();
        let mut cases = 0;
        for line in include_str!("../../tests/fixtures/camera_blur_coefficients.txt").lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            let kind = words[0].parse().unwrap();
            let actual = bank
                .get(kind)
                .unwrap()
                .shader_coefficients()
                .map(f32::to_bits);
            let expected: Vec<_> = words[1..]
                .iter()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            assert_eq!(
                actual.as_slice(),
                expected,
                "installed camera kernel {kind}"
            );
            cases += 1;
        }
        assert_eq!(cases, 6);
    }

    #[test]
    #[ignore = "requires installed game archives"]
    fn all_selected_camera_materials_resolve_from_archives() {
        let catalog = Catalog::open(&super::super::installation::game_base().unwrap()).unwrap();
        let bank = MaterialBank::load(&catalog).unwrap();
        assert_eq!(bank.materials.len(), 63);
        for pass in [DofPass::Alpha, DofPass::Near, DofPass::Far] {
            let fallback = bank
                .resolve(DofMaterialVariant {
                    pass,
                    filter: None,
                    fringing: 255,
                    cat_eye: 255,
                })
                .unwrap();
            assert_eq!(fallback.pass, pass);
            assert_eq!(fallback.filter, 0);
            assert!(!fallback.fringing && !fallback.cat_eye);
        }
    }
}
impl KernelBank {
    pub fn load(catalog: &Catalog) -> Result<Self, String> {
        let path = "generated/decls/renderprog/dof.decl";
        let bytes = catalog
            .read(path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("camera blur include not indexed: {path}"))?;
        Self::parse(&String::from_utf8_lossy(&bytes))
    }
    pub fn get(&self, kind: u8) -> Option<&BlurKernel> {
        self.kernels.get(usize::from(kind))
    }
    fn parse(source: &str) -> Result<Self, String> {
        let names = [
            "E_LINEAR",
            "E_POISSON",
            "E_DISK",
            "E_DOT",
            "E_HEXAGON",
            "E_CIRCLE",
        ];
        let lines: Vec<_> = source.lines().collect();
        let mut kernels = Vec::with_capacity(6);
        for (kind, name) in names.into_iter().enumerate() {
            let start = lines
                .iter()
                .position(|line| {
                    let words: Vec<_> = line.split_whitespace().collect();
                    matches!(words.first(), Some(&"#if") | Some(&"#elif"))
                        && words.last() == Some(&name)
                })
                .ok_or_else(|| format!("camera filter branch missing: {name}"))?;
            let tail = &lines[start..];
            let section = tail
                .iter()
                .copied()
                .take(1)
                .chain(tail.iter().copied().skip(1).take_while(|line| {
                    let first = line.split_whitespace().next();
                    !matches!(first, Some("#elif") | Some("#else") | Some("#endif"))
                }))
                .collect::<Vec<_>>()
                .join("\n");
            let samples = section
                .lines()
                .find_map(|line| {
                    let words: Vec<_> = line.split_whitespace().collect();
                    (words.first() == Some(&"#define") && words.get(1) == Some(&"NB_SAMPLES"))
                        .then(|| words.get(2)?.parse::<u32>().ok())
                        .flatten()
                })
                .ok_or_else(|| format!("camera filter sample count missing: {name}"))?;
            let mut offsets = Vec::new();
            if !matches!(kind, 2 | 5) {
                let clean = crate::entities::declaration_lexemes(&section)?.join(" ");
                let array = clean
                    .split_once("Kernel[NB_SAMPLES]")
                    .and_then(|(_, s)| s.split_once('{'))
                    .and_then(|(_, s)| s.split_once('}'))
                    .map(|(s, _)| s)
                    .ok_or_else(|| format!("camera kernel array missing: {name}"))?;
                for expression in array.split("float2").skip(1) {
                    let arguments = expression
                        .trim_start()
                        .strip_prefix('(')
                        .and_then(|s| s.split_once(')'))
                        .map(|(s, _)| s)
                        .ok_or_else(|| format!("invalid camera kernel vector: {name}"))?;
                    let (x, y) = arguments
                        .split_once(',')
                        .ok_or_else(|| format!("invalid camera kernel pair: {name}"))?;
                    offsets.push([
                        x.trim()
                            .parse()
                            .map_err(|_| format!("invalid camera kernel X: {name}"))?,
                        y.trim()
                            .parse()
                            .map_err(|_| format!("invalid camera kernel Y: {name}"))?,
                    ]);
                }
            }
            kernels.push(BlurKernel::from_offsets(kind as u8, samples, offsets)?);
        }
        Ok(Self {
            kernels: kernels
                .try_into()
                .map_err(|_| "incomplete camera kernel bank")?,
        })
    }
}
