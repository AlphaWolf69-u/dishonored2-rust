//! Authored arkDefault material inputs. Formulas and defaults come from the
//! installed arkDefaultHelpers and renderParm declarations, not a PBR preset.
use std::collections::HashMap;

pub const TEXTURE_PARAMETERS: [&str; 35] = [
    "diffusemap",
    "layer1_diffusemap",
    "layer2_diffusemap",
    "bumpmap",
    "layer1_bumpmap",
    "layer2_bumpmap",
    "glossmap",
    "layer1_glossmap",
    "layer2_glossmap",
    "metallicmap",
    "layer1_metallicmap",
    "layer2_metallicmap",
    "layer0_maskmap",
    "layer1_maskmap",
    "layer2_maskmap",
    "occlusionmap",
    "specularmap",
    "layer1_specularmap",
    "layer2_specularmap",
    "detailbumpmap",
    "emissivemap",
    "emissivemaskmap",
    "lightmap",
    "fx/diffuse",
    "fx/disto1",
    "fx/disto2",
    "fx/lookup_fire",
    "particlefresnelnormalmap",
    "particlefresnellut",
    "opacitymap",
    "densitymap",
    "backdensitymap",
    "texturemap",
    "skydepth",
    "clouddepth",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Program {
    Unsupported = 0,
    Default = 1,
    Vegetation = 2,
    Water = 3,
    Vista = 4,
    InteriorVolume = 5,
    Filling = 6,
    Sky = 7,
    Clouds = 8,
    SimpleSprite = 9,
    BoatVolume = 10,
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub diffuse: Option<String>,
    pub bump: Option<String>,
    pub gloss: Option<String>,
    pub metallic: Option<String>,
    pub specular: Option<String>,
    pub specular_reuse: u32,
    pub mask: Option<String>,
    pub color: [f32; 4],
    pub use_color: bool,
    pub mask_mode: u32,
    pub mask_channel: usize,
    pub rescale: [f32; 2],
    pub invert_mask: bool,
    pub uv_scale_bias: [f32; 4],
    pub mask_scale_bias: [f32; 4],
    pub uv_set: u32,
    pub mask_uv_set: u32,
    pub reuse: [usize; 5],
}

#[derive(Debug, Clone)]
pub struct Surface {
    pub main_program: Option<String>,
    pub lightmap: String,
    pub filling_images: [String; 6],
    pub cloud_images: [String; 3],
    pub sky_depth: String,
    pub layers: [Layer; 3],
    pub extra_layers: usize,
    pub has_bump: bool,
    pub has_gloss: bool,
    pub has_metallic: bool,
    pub has_specular: bool,
    pub specular_dielectric: [f32; 3],
    pub diffuse_metal: [f32; 3],
    pub has_occlusion: bool,
    pub occlusion: Option<String>,
    pub gloss_in_alpha: bool,
    pub vertex_color_multiply: bool,
    pub ward: [f32; 4],
    pub sheen: [f32; 2],
    pub metallic_min_max: [f32; 2],
    pub alpha_test: bool,
    pub alpha_cutoff: f32,
    pub detail_bump: String,
    pub has_detail_bump: bool,
    pub detail_tiling: f32,
    pub detail_intensity: f32,
    pub emissive_map: String,
    pub emissive_mask: String,
    pub has_emissive_map: bool,
    pub has_emissive_mask: bool,
    pub animated_emissive: bool,
    pub emissive_color: [f32; 3],
    pub emissive_intensity: f32,
    pub emissive_clip: [f32; 2],
}

fn fields(bytes: &[u8]) -> HashMap<String, String> {
    crate::entities::declaration_text(bytes)
        .lines()
        .filter_map(|line| {
            let line = line.split("//").next()?.trim();
            let at = line.find(char::is_whitespace)?;
            Some((
                line[..at].to_ascii_lowercase(),
                line[at..].trim().to_owned(),
            ))
        })
        .collect()
}

fn number(values: &HashMap<String, String>, key: &str, default: u32) -> u32 {
    values
        .get(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn vector<const N: usize>(
    values: &HashMap<String, String>,
    key: &str,
    default: [f32; N],
) -> [f32; N] {
    let Some(value) = values.get(key) else {
        return default;
    };
    let Ok(parsed) = value
        .trim_matches(['{', '}'])
        .split(',')
        .map(|v| v.trim().parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
    else {
        return default;
    };
    // Serialized renderParm values can use all four storage lanes even when
    // the shader declaration only consumes float/float2 (e.g. blood ranges).
    if (parsed.len() != N && parsed.len() != 4) || !parsed.iter().all(|v| v.is_finite()) {
        return default;
    }
    std::array::from_fn(|i| parsed[i])
}

/// Keep authored image names and sampler suffixes intact here. Resource
/// resolution and format/colour-space selection are separate loader jobs.
fn image(values: &HashMap<String, String>, key: &str) -> Option<String> {
    values.get(key).map(|v| v.trim_matches('"').to_owned())
}

/// global.hlsl UnpackBC5NormalMap consumes signed BC5 XY. The original
/// intentionally reconstructs Z as 1.001-saturate(dot(XY,XY)), not sqrt.
pub fn unpack_bc5_normal_map(xy: [f32; 2]) -> [f32; 3] {
    let z = 1.001 - (xy[0] * xy[0] + xy[1] * xy[1]).clamp(0.0, 1.0);
    let length = (xy[0] * xy[0] + xy[1] * xy[1] + z * z).sqrt();
    [xy[0] / length, xy[1] / length, z / length]
}

impl Surface {
    pub fn program(&self) -> Program {
        match self
            .main_program
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("arkdefault" | "arkdecal") => Program::Default,
            Some("arkvegetation") => Program::Vegetation,
            Some("arkwater") => Program::Water,
            Some("arkvista") => Program::Vista,
            Some("particle/filing_container") => Program::Filling,
            Some("arksky") => Program::Sky,
            Some("arkclouds") => Program::Clouds,
            Some("arksimplesprite") => Program::SimpleSprite,
            Some("arkboatvolume") => Program::BoatVolume,
            Some("atm/worlddistancefog/arkinteriorvolume") => Program::InteriorVolume,
            _ => Program::Unsupported,
        }
    }

    /// arkVegetation tests the raw diffuse alpha against 0.5, before gloss
    /// consumes it. Its vertex RGB channels control wind, not albedo.
    pub fn forward_alpha_cutoff(&self) -> f32 {
        if matches!(self.program(), Program::Vegetation | Program::Vista) {
            0.5
        } else {
            self.alpha_cutoff
        }
    }

    pub fn parse(bytes: &[u8]) -> Self {
        let v = fields(bytes);
        let layers = std::array::from_fn(|index| {
            let prefix = format!("layer{index}_");
            let key = |suffix: &str| format!("{prefix}{suffix}");
            let texture = |suffix: &str| {
                let name = if index == 0 && suffix != "maskmap" {
                    suffix.to_string()
                } else {
                    key(suffix)
                };
                image(&v, &name).or_else(|| {
                    // Installed renderParm defaults, including intrinsic images.
                    Some(
                        match suffix {
                            "bumpmap" => "ipr_constantColor(0.5,0.5,0,0)",
                            "specularmap" => "ipr_constantColor(0,0,0,0)",
                            "glossmap" | "metallicmap" if index > 0 => "ipr_constantColor(0,0,0,0)",
                            _ => "_default",
                        }
                        .to_owned(),
                    )
                })
            };
            Layer {
                diffuse: if index == 0
                    && v.get("mainprogram").is_some_and(|n| {
                        n.trim_matches('"').eq_ignore_ascii_case("arkSimpleSprite")
                    }) {
                    Some(image(&v, "texturemap").unwrap_or_else(|| "_default".into()))
                } else {
                    texture("diffusemap")
                },
                bump: texture("bumpmap"),
                gloss: texture("glossmap"),
                metallic: texture("metallicmap"),
                specular: texture("specularmap"),
                specular_reuse: number(&v, &key("specularreuselayer"), 0),
                mask: texture("maskmap"),
                color: vector(&v, &key("colorconstant"), [1.0; 4]),
                use_color: number(&v, &format!("use_{prefix}colorconstant"), 0) != 0,
                mask_mode: number(&v, &key("maskmode"), 0),
                mask_channel: number(&v, &key("vtxcolormaskchannel"), 3) as usize,
                rescale: vector(&v, &key("rescalevalues"), [0.4, 0.6]),
                invert_mask: number(&v, &key("invertmask"), 0) != 0,
                uv_scale_bias: vector(&v, &key("scalebias"), [1.0, 1.0, 0.0, 0.0]),
                mask_scale_bias: vector(&v, &key("maskscalebias"), [1.0, 1.0, 0.0, 0.0]),
                uv_set: number(&v, &key("uvset"), 0),
                mask_uv_set: number(&v, &key("mask_uvset"), 0),
                reuse: ["diffuse", "bump", "gloss", "metallic", "mask"]
                    .map(|semantic| number(&v, &key(&format!("{semantic}reuselayer")), 0) as usize),
            }
        });
        Self {
            // 1402d61ef supplies the arkDefault interned name when absent.
            main_program: Some(
                v.get("mainprogram")
                    .map_or("arkDefault", |name| name.trim_matches('"'))
                    .to_owned(),
            ),
            lightmap: image(&v, "lightmap").unwrap_or_else(|| "_default".into()),
            filling_images: std::array::from_fn(|i| {
                image(&v, crate::rendering::filling::IMAGE_NAMES[i])
                    .unwrap_or_else(|| crate::rendering::filling::IMAGE_DEFAULTS[i].into())
            }),
            cloud_images: ["opacitymap", "densitymap", "backdensitymap"]
                .map(|n| image(&v, n).unwrap_or_else(|| "_black".into())),
            sky_depth: image(
                &v,
                if v.get("mainprogram")
                    .is_some_and(|name| name.trim_matches('"').eq_ignore_ascii_case("arkClouds"))
                {
                    "clouddepth"
                } else {
                    "skydepth"
                },
            )
            .unwrap_or_else(|| "_black".into()),
            layers,
            extra_layers: number(&v, "extralayer", 0) as usize,
            has_bump: number(&v, "hasbumpmap", 0) != 0,
            has_gloss: number(&v, "hasglossmap", 0) != 0,
            has_metallic: number(&v, "hasmetallicmap", 0) != 0,
            has_specular: number(&v, "hasspecularmap", 0) != 0,
            specular_dielectric: vector(&v, "speculartintdielectric", [0.04; 3]),
            diffuse_metal: vector(&v, "diffusealbedometallic", [0.0; 3]),
            has_occlusion: number(&v, "hasocclusionmap", 0) != 0,
            occlusion: Some(image(&v, "occlusionmap").unwrap_or_else(|| "_default".into())),
            gloss_in_alpha: number(&v, "glossindiffusealpha", 0) != 0,
            vertex_color_multiply: number(&v, "vtxcolormultiply", 0) != 0,
            ward: vector(&v, "wardroughness", [0.0, 0.5, 0.0, 0.0]),
            // Installed sheen / sheenImportance renderParm initializers.
            sheen: [vector(&v,"sheen",[0.0])[0], vector(&v,"sheenimportance",[1.0])[0]],
            metallic_min_max: vector(&v, "metallicminmax", [0.0, 0.5]),
            alpha_test: number(&v, "alphatest", 0) != 0,
            alpha_cutoff: vector(&v, "defaultalphatest", [0.5])[0],
            detail_bump: image(&v, "detailbumpmap").unwrap_or_else(|| "_default".into()),
            has_detail_bump: number(&v, "hasdetailbumpmap", 0) != 0,
            detail_tiling: vector(&v, "detailbumpmaptiling", [2.0])[0],
            detail_intensity: vector(&v, "detailbumpmapintensity", [1.0])[0],
            emissive_map: image(&v, "emissivemap").unwrap_or_else(|| "_default".into()),
            emissive_mask: image(&v, "emissivemaskmap").unwrap_or_else(|| "_default".into()),
            has_emissive_map: number(&v, "hasemissivemap", 0) != 0,
            has_emissive_mask: number(&v, "hasemissivemaskmap", 0) != 0,
            animated_emissive: number(&v, "hasanimatedemissive", 0) != 0,
            emissive_color: vector(&v, "emissivecolorconstant", [0.; 3]),
            emissive_intensity: vector(&v, "emissiveintensity", [0.])[0],
            emissive_clip: vector(&v, "emissiveclipparms", [0., 0.001]),
        }
    }

    /// Image roles consumed by the implemented original material branches.
    /// Disabled and reused maps do not cause independent texture requests.
    pub fn texture_bindings(&self, is_decal: bool) -> Vec<(usize, &str)> {
        if self.program() == Program::SimpleSprite {
            return vec![(0, self.layers[0].diffuse.as_deref().unwrap())];
        }
        if matches!(self.program(), Program::Sky | Program::Clouds) {
            let mut bindings = vec![
                (0, self.layers[0].diffuse.as_deref().unwrap()),
                (33, self.sky_depth.as_str()),
            ];
            if self.program() == Program::Clouds {
                for (i, name) in self.cloud_images.iter().enumerate() {
                    bindings.push((29 + i, name.as_str()));
                }
            }
            return bindings;
        }
        if matches!(self.program(), Program::InteriorVolume | Program::BoatVolume) {
            return Vec::new();
        }
        if self.program() == Program::Vista {
            let mut bindings = vec![
                (0, self.layers[0].diffuse.as_deref().unwrap()),
                (22, self.lightmap.as_str()),
            ];
            if self.has_bump {
                bindings.push((3, self.layers[0].bump.as_deref().unwrap()));
            }
            if self.has_gloss {
                bindings.push((6, self.layers[0].gloss.as_deref().unwrap()));
            }
            if self.has_emissive_map {
                bindings.push((20, self.emissive_map.as_str()));
            }
            return bindings;
        }
        let mut bindings = Vec::new();
        if self.program() == Program::Filling {
            for (i, name) in self.filling_images.iter().enumerate() {
                bindings.push((23 + i, name.as_str()));
            }
        }

        let last = if is_decal {
            0
        } else {
            self.extra_layers.min(2)
        };
        for index in 0..=last {
            let l = &self.layers[index];
            for (role, name, enabled) in [
                (index, &l.diffuse, index == 0 || l.reuse[0] == 0),
                (
                    3 + index,
                    &l.bump,
                    self.has_bump && (index == 0 || l.reuse[1] == 0),
                ),
                (
                    6 + index,
                    &l.gloss,
                    self.has_gloss && !self.gloss_in_alpha && (index == 0 || l.reuse[2] == 0),
                ),
                (
                    9 + index,
                    &l.metallic,
                    self.has_metallic && (index == 0 || l.reuse[3] == 0),
                ),
                (
                    12 + index,
                    &l.mask,
                    (index != 0 || l.use_color) && l.mask_mode != 0 && l.reuse[4] == 0,
                ),
                (
                    16 + index,
                    &l.specular,
                    self.has_specular && (index == 0 || l.specular_reuse == 0),
                ),
            ] {
                if enabled {
                    if let Some(name) = name {
                        bindings.push((role, name.as_str()));
                    }
                }
            }
        }
        if self.has_occlusion && !is_decal {
            if let Some(name) = &self.occlusion {
                bindings.push((15, name));
            }
        }
        if self.has_detail_bump && self.has_bump && !is_decal {
            bindings.push((19, &self.detail_bump));
        }
        if !self.animated_emissive && self.emissive_intensity != 0. {
            if !is_decal && self.has_emissive_map {
                bindings.push((20, &self.emissive_map));
            }
            if !is_decal && self.has_emissive_mask {
                bindings.push((21, &self.emissive_mask));
            }
        }
        bindings
    }

    /// HQ/AQ_FetchAO's ordinary branch; LQ_FetchAO always returns one.
    pub fn occlusion_factor(&self, sample_red: f32, low_quality: bool) -> f32 {
        if self.has_occlusion && !low_quality {
            sample_red
        } else {
            1.0
        }
    }

    /// HQ_FetchMasks, ordinary non-cloth branch. Samples are already evaluated
    /// at the shader's mask UVs. Reuse follows the original *raw* sample, not
    /// another layer's rescaled or inverted mask.
    pub fn masks(&self, vertex: [f32; 4], samples: [f32; 3]) -> Result<[f32; 3], String> {
        if self.extra_layers > 2 {
            return Err("invalid authored extraLayer".into());
        }
        let mut raw = [0.0; 3];
        let mut masks = [0.0; 3];
        for index in 0..=self.extra_layers {
            let layer = &self.layers[index];
            if index != 0 || layer.use_color {
                let channel = *vertex
                    .get(layer.mask_channel)
                    .ok_or("invalid vertex mask channel")?;
                let reuse = layer.reuse[4];
                if reuse > index {
                    return Err("invalid mask reuse layer".into());
                }
                raw[index] = if reuse == 0 {
                    samples[index]
                } else {
                    raw[reuse - 1]
                };
                masks[index] = match layer.mask_mode {
                    0 => channel,
                    1 => raw[index],
                    _ => ((raw[index] * channel - layer.rescale[0])
                        / (layer.rescale[1] - layer.rescale[0]))
                        .clamp(0.0, 1.0),
                };
                // Mode 0 does not fetch a texture; a subsequent reuse must see
                // the original function's initialized raw value of zero.
                if layer.mask_mode == 0 {
                    raw[index] = 0.0;
                }
            }
            if layer.invert_mask {
                masks[index] = 1.0 - masks[index];
            }
        }
        Ok(masks)
    }

    /// HQ_FetchDiffuseAlbedo's ordinary static, non-debug/non-effect branch.
    pub fn albedo(
        &self,
        vertex: [f32; 4],
        masks: [f32; 3],
        samples: [[f32; 4]; 3],
    ) -> Result<[f32; 4], String> {
        if self.extra_layers > 2 {
            return Err("invalid authored extraLayer".into());
        }
        if self.program() == Program::Vegetation {
            return Ok(samples[0]);
        }
        let mut textures = samples;
        let mut result = textures[0];
        if self.layers[0].use_color {
            for lane in 0..4 {
                result[lane] *= 1.0 + (self.layers[0].color[lane] - 1.0) * masks[0];
            }
        }
        for index in 1..=self.extra_layers {
            let layer = &self.layers[index];
            let reuse = layer.reuse[0];
            if reuse > index {
                return Err("invalid diffuse reuse layer".into());
            }
            if reuse != 0 {
                textures[index] = textures[reuse - 1];
            }
            let mut value = textures[index];
            if layer.use_color {
                for lane in 0..4 {
                    value[lane] *= layer.color[lane];
                }
            }
            for lane in 0..4 {
                result[lane] += (value[lane] - result[lane]) * masks[index];
            }
        }
        if self.vertex_color_multiply {
            for lane in 0..4 {
                result[lane] *= vertex[lane];
            }
        }
        Ok(result.map(|v| v.max(0.00001)))
    }

    /// HQ_FetchMetallic, ordinary non-debug branch.
    pub fn metallic(&self, masks: [f32; 3], samples: [f32; 3]) -> Result<f32, String> {
        if !self.has_metallic {
            return Ok(self.metallic_min_max[1]);
        }
        if self.extra_layers > 2 {
            return Err("invalid authored extraLayer".into());
        }
        let mut raw = samples;
        let mut value = raw[0];
        for index in 1..=self.extra_layers {
            let reuse = self.layers[index].reuse[3];
            if reuse > index {
                return Err("invalid metallic reuse layer".into());
            }
            if reuse != 0 {
                raw[index] = raw[reuse - 1];
            }
            if index != 1 || reuse == 0 {
                value += (raw[index] - value) * masks[index];
            }
        }
        Ok(
            self.metallic_min_max[0]
                + (self.metallic_min_max[1] - self.metallic_min_max[0]) * value,
        )
    }

    /// HQ_FetchGloss followed by HQ_ComputeLightingParams. Gloss in diffuse
    /// alpha consumes that channel: it must not make a surface transparent.
    pub fn roughness(
        &self,
        alpha: &mut f32,
        masks: [f32; 3],
        samples: [f32; 3],
    ) -> Result<f32, String> {
        let gloss = if self.gloss_in_alpha {
            let gloss = self.ward[0] + (self.ward[1] - self.ward[0]) * *alpha;
            *alpha = 1.0;
            gloss
        } else if self.has_gloss {
            if self.extra_layers > 2 {
                return Err("invalid authored extraLayer".into());
            }
            let mut raw = samples;
            let mut value = raw[0];
            for index in 1..=self.extra_layers {
                let reuse = self.layers[index].reuse[2];
                if reuse > index {
                    return Err("invalid gloss reuse layer".into());
                }
                if reuse != 0 {
                    raw[index] = raw[reuse - 1];
                }
                // HQ explicitly skips layer1 interpolation when it reuses
                // layer0, including extrapolating vertex masks.
                if index != 1 || reuse == 0 {
                    value += (raw[index] - value) * masks[index];
                }
            }
            self.ward[0] + (self.ward[1] - self.ward[0]) * value
        } else {
            self.ward[1]
        };
        Ok(1.0 - gloss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_program_selects_default_and_authored_maps() {
        let s = Surface::parse(b"hasGlossMap 1\nhasMetallicMap 1\nmetallicMinMax {0,0.8,0,0}\nwardRoughness {0.3,1,0,0}\n");
        assert_eq!(s.main_program.as_deref(), Some("arkDefault"));
        assert_eq!(s.program(), Program::Default);
        assert_eq!(s.metallic([0.0; 3], [0.0; 3]).unwrap(), 0.0);
        assert_eq!(s.roughness(&mut 1.0, [0.0; 3], [0.0; 3]).unwrap(), 0.7);
        assert_eq!(
            Surface::parse(b"mainProgram arkCloth\n").program(),
            Program::Unsupported
        );
        assert_eq!(
            Surface::parse(b"mainProgram arkVegetation\n").program(),
            Program::Vegetation
        );
    }

    #[test]
    fn signed_bc5_reconstructs_non_sqrt_z() {
        assert_eq!(unpack_bc5_normal_map([0.0, 0.0]), [0.0, 0.0, 1.0]);
        let normal = unpack_bc5_normal_map([0.5, -0.25]);
        let z = 1.001_f32 - 0.3125;
        let length = (0.3125 + z * z).sqrt();
        assert_eq!(normal, [0.5 / length, -0.25 / length, z / length]);
        let extreme = unpack_bc5_normal_map([-1.0, 1.0]);
        assert!(extreme[0] < 0.0 && extreme[1] > 0.0);
        assert!(extreme[2] > 0.0 && extreme[2] < 0.001);
    }

    #[test]
    fn ordinary_ao_uses_red_but_low_quality_does_not_sample() {
        let s = Surface::parse(b"hasOcclusionMap 1\nocclusionMap models/stone_ao\n");
        assert_eq!(s.occlusion.as_deref(), Some("models/stone_ao"));
        assert_eq!(s.occlusion_factor(0.25, false), 0.25);
        assert_eq!(s.occlusion_factor(0.25, true), 1.0);
        assert_eq!(Surface::parse(b"").occlusion_factor(0.25, false), 1.0);
    }

    #[test]
    fn vegetation_uses_diffuse_not_wind_vertex_colors_or_layer_tints() {
        let s = Surface::parse(b"mainProgram arkVegetation\nalphaTest 1\nDefaultAlphaTest {0.9}\nvtxColorMultiply 1\nuse_Layer0_ColorConstant 1\nLayer0_ColorConstant {1,0,0,1}\n");
        assert_eq!(s.program(), Program::Vegetation);
        assert_eq!(s.forward_alpha_cutoff(), 0.5);
        let diffuse = [0.0, 0.4, 0.2, 0.49];
        assert_eq!(
            s.albedo(
                [1.0, 0.0, 0.0, 0.0],
                [1.0; 3],
                [diffuse, [1.0; 4], [1.0; 4]]
            )
            .unwrap(),
            diffuse
        );
        assert_eq!(
            Surface::parse(b"mainProgram unknown\n").program(),
            Program::Unsupported
        );
        assert_eq!(
            Surface::parse(b"mainProgram arkDecal\n").program(),
            Program::Default
        );
        assert_eq!(
            Surface::parse(b"mainProgram arkwater\n").program(),
            Program::Water
        );
    }

    #[test]
    fn layer_and_mask_uv_sets_are_independent() {
        let surface = Surface::parse(b"Layer1_UVSet 1\nLayer1_Mask_UVSet 0\nLayer2_Mask_UVSet 1\n");
        assert_eq!(surface.layers[0].uv_set, 0);
        assert_eq!(surface.layers[1].uv_set, 1);
        assert_eq!(surface.layers[1].mask_uv_set, 0);
        assert_eq!(surface.layers[2].mask_uv_set, 1);
    }

    #[test]
    fn defaults_and_no_map_roughness() {
        let s = Surface::parse(b"{\nmainprogram arkDefault\n}\n");
        assert_eq!(s.ward, [0.0, 0.5, 0.0, 0.0]);
        assert_eq!(s.layers[0].mask_channel, 3);
        assert_eq!(s.layers[2].uv_scale_bias, [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(s.roughness(&mut 0.25, [0.0; 3], [0.0; 3]).unwrap(), 0.5);
    }

    #[test]
    fn omitted_layer_maps_use_black_defaults_and_preserve_overrides() {
        let s = Surface::parse(b"extraLayer 2\nhasMetallicMap 1\nhasGlossMap 1\nhasSpecularMap 1\nmetallicMap base_mt\nLayer2_MetallicMap coat_mt\nLayer1_GlossMap coat_g\n");
        let black = Some("ipr_constantColor(0,0,0,0)");
        assert_eq!(s.layers[0].metallic.as_deref(), Some("base_mt"));
        assert_eq!(s.layers[1].metallic.as_deref(), black);
        assert_eq!(s.layers[2].metallic.as_deref(), Some("coat_mt"));
        assert_eq!(s.layers[1].gloss.as_deref(), Some("coat_g"));
        assert_eq!(s.layers[2].gloss.as_deref(), black);
        assert!(s.layers.iter().all(|l| l.specular.as_deref() == black));
        // A fully selected coat with no metallic map is dielectric even when
        // the underlying layer is metal (arkDefault HQ_FetchMetallic).
        assert_eq!(s.metallic([0., 1., 0.], [1., 0., 1.]).unwrap(), 0.0);
    }

    #[test]
    fn all_standard_texture_roles_resolve_defaults() {
        let s=Surface::parse(b"extraLayer 2\nhasBumpMap 1\nhasGlossMap 1\nhasMetallicMap 1\nhasSpecularMap 1\nhasOcclusionMap 1\nhasDetailBumpMap 1\nhasEmissiveMap 1\nhasEmissiveMaskMap 1\nemissiveIntensity 1\nuse_Layer0_ColorConstant 1\nLayer0_MaskMode 1\nLayer1_MaskMode 1\nLayer2_MaskMode 1\n");
        let bindings = s.texture_bindings(false);
        assert_eq!(bindings.len(), 22);
        for (role, name) in bindings {
            assert_eq!(
                name,
                match role {
                    3..=5 => "ipr_constantColor(0.5,0.5,0,0)",
                    7 | 8 | 10 | 11 | 16..=18 => "ipr_constantColor(0,0,0,0)",
                    _ => "_default",
                },
                "role{role}"
            );
        }
        let decal = s.texture_bindings(true);
        assert!(!decal.iter().any(|(r, _)| matches!(r, 15 | 19 | 20 | 21)));
        assert_eq!(s.detail_tiling, 2.0);
        assert_eq!(s.detail_intensity, 1.0);
        assert_eq!(s.emissive_clip, [0., 0.001]);
    }

    #[test]
    fn float2_parameters_accept_four_serialized_storage_lanes() {
        // Installed blend_dirt_blood_puddle_01 and arkDefault float2 declarations.
        let s = Surface::parse(b"extraLayer 1\nmetallicminmax {0,0,0,0}\nLayer1_MaskMode 2\nLayer1_RescaleValues {0,0.22,0,0}\nLayer1_VtxColorMaskChannel 0\n");
        assert_eq!(s.metallic_min_max, [0.0, 0.0]);
        assert_eq!(s.metallic([0.0; 3], [1.0; 3]).unwrap(), 0.0);
        assert_eq!(s.layers[1].rescale, [0.0, 0.22]);
        let masks = s.masks([0.5, 0.0, 0.0, 1.0], [0.0, 0.22, 0.0]).unwrap();
        assert!((masks[1] - 0.5).abs() < 1e-6);
        assert_eq!(
            vector(&fields(b"value {1,2,3,4,5}"), "value", [0.4, 0.6]),
            [0.4, 0.6]
        );
    }

    #[test]
    fn mask_is_layer_blend_not_blanket_opacity() {
        let s = Surface::parse(b"extraLayer 1\nuse_Layer0_ColorConstant 1\nLayer0_ColorConstant {0.2,0.4,0.6,1}\nLayer0_MaskMode 2\nLayer0_RescaleValues {0.2,0.8}\nLayer1_MaskMode 0\nLayer1_VtxColorMaskChannel 0\n");
        let masks = s.masks([0.25, 0.0, 0.0, 0.5], [1.0, 0.0, 0.0]).unwrap();
        assert!((masks[0] - 0.5).abs() < 1e-6);
        let c = s
            .albedo([1.0; 4], masks, [[1.0; 4], [0.0, 0.0, 0.0, 1.0], [1.0; 4]])
            .unwrap();
        assert!((c[0] - 0.45).abs() < 1e-6);
        assert_eq!(c[3], 1.0);
    }

    #[test]
    fn gloss_in_alpha_restores_opaque_alpha() {
        let s = Surface::parse(b"glossInDiffuseAlpha 1\nwardRoughness {0.1,0.9,0,0}\n");
        let mut alpha = 0.25;
        assert!((s.roughness(&mut alpha, [0.0; 3], [0.0; 3]).unwrap() - 0.7).abs() < 1e-6);
        assert_eq!(alpha, 1.0);
    }

    #[test]
    fn metallic_uses_authored_range_and_raw_reuse() {
        let s = Surface::parse(b"hasMetallicMap 1\nextraLayer 2\nmetallicMinMax {0.2,0.8}\nLayer1_metallicReuseLayer 1\nLayer2_metallicReuseLayer 2\n");
        assert!((s.metallic([0.0, 3.0, 0.5], [0.25, 1.0, 1.0]).unwrap() - 0.35).abs() < 1e-6);
    }

    #[test]
    fn reuse_does_not_reuse_tinted_diffuse() {
        let s = Surface::parse(b"extraLayer 2\nLayer1_diffuseReuseLayer 1\nLayer2_diffuseReuseLayer 2\nuse_Layer1_ColorConstant 1\nLayer1_ColorConstant {0.1,0.1,0.1,1}\n");
        let c = s
            .albedo(
                [1.0; 4],
                [0.0, 1.0, 1.0],
                [[0.8, 0.6, 0.4, 1.0], [0.0; 4], [0.0; 4]],
            )
            .unwrap();
        assert_eq!(c, [0.8, 0.6, 0.4, 1.0]);
    }
}

#[cfg(test)]
mod vista_tests {
    use super::*;
    #[test]
    fn vista_uses_uv1_lightmap_and_only_its_own_enabled_maps() {
        let s=Surface::parse(b"mainProgram arkVista\nlightMap distant_lighting.tga\nhasGlossMap 1\nhasMetallicMap 1\nhasDetailBumpMap 1\n");
        assert_eq!(s.program(), Program::Vista);
        let roles = s.texture_bindings(false);
        assert_eq!(
            roles.iter().map(|(role, _)| *role).collect::<Vec<_>>(),
            vec![0, 22, 6]
        );
        assert_eq!(roles[1].1, "distant_lighting.tga");
        assert_eq!(s.forward_alpha_cutoff(), 0.5);
    }
    #[test]
    fn interior_depth_volume_is_not_a_colour_texture_material() {
        let s = Surface::parse(b"mainProgram atm/worldDistanceFog/arkInteriorVolume\n");
        assert_eq!(s.program(), Program::InteriorVolume);
        assert!(s.texture_bindings(false).is_empty());
    }
}
