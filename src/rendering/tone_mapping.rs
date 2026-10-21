//! Authored filmic curve from 325420 and PostFX/ToneMapping/common.
//! Final colour-cube blending and display-pass ownership are separate stages.
use crate::assets::environment::ParameterSource;

#[derive(Clone, Copy, Debug)]
pub struct ToneSettings {
    pub overbright: f32,
    pub black: f32,
    pub white: f32,
    pub junction: f32,
    pub toe: f32,
    pub shoulder: f32,
}
impl ToneSettings {
    pub fn resolve(read: &impl ParameterSource) -> Result<Self, String> {
        Ok(Self {
            overbright: read.scalar("env/tonemapping/overbright_level")?,
            black: read.scalar("env/tonemapping/black_level")?,
            white: read.scalar("env/tonemapping/white_level")?,
            junction: read.scalar("env/tonemapping/junction_point")?,
            toe: read.scalar("env/tonemapping/toe_strength")?,
            shoulder: read.scalar("env/tonemapping/shoulder_strength")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct DisplaySettings {
    pub tone: ToneSettings,
    pub gamma: f32,
    pub brightness: f32,
    pub colour_cube: String,
}
impl DisplaySettings {
    pub fn resolve(read: &impl ParameterSource) -> Result<Self, String> {
        let gamma_brightness = read.vector("env/tonemapping/gamma_brightness")?;
        Ok(Self {
            tone: ToneSettings::resolve(read)?,
            gamma: gamma_brightness[0],
            brightness: gamma_brightness[1],
            colour_cube: read.image_source("env/tonemapping/colorcube")?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToneCurve {
    pub junctions: [f32; 4],
    pub toe: [f32; 4],
    pub shoulder: [f32; 4],
    pub inverse_toe: [f32; 4],
    pub inverse_shoulder: [f32; 4],
}
impl ToneCurve {
    pub fn new(settings: ToneSettings) -> Self {
        let epsilon = f32::from_bits(0x3a83126f);
        let cap = f32::from_bits(0x3f7fbe77);
        let bright = if epsilon <= settings.overbright {
            settings.overbright
        } else {
            epsilon
        };
        let toe = settings.toe.powf(2.);
        let toe = if toe <= cap { toe } else { cap };
        let shoulder = settings.shoulder.powf(f32::from_bits(0x3e4ccccd));
        let shoulder = if shoulder <= cap { shoulder } else { cap };
        let lower = settings.black + epsilon;
        let junction = if lower <= settings.junction {
            settings.junction
        } else {
            lower
        };
        let upper = settings.white - epsilon;
        let junction = if junction <= upper { junction } else { upper };
        let toe_scale = 1. - toe;
        let shoulder_span = (1. - shoulder) * (settings.white - junction);
        let toe_span = (junction - settings.black) * toe_scale;
        let k = toe_span / (shoulder_span + toe_span);
        let toe_denominator = junction - toe_scale * settings.black;
        let shoulder_denominator = shoulder_span - shoulder * junction;
        let toe_numerator = toe_scale * k * bright;
        let toe_offset = -(k * settings.black * toe_scale) * bright;
        let shoulder_numerator = ((shoulder - 1.) * k + 1.) * bright;
        let shoulder_offset = (((1. - shoulder) * k * (settings.white - junction)
            - k * shoulder * junction)
            - (1. - k) * junction)
            * bright;
        Self {
            junctions: [junction, k * bright, 0., 0.],
            toe: [toe_numerator, -toe, toe_offset, toe_denominator],
            shoulder: [
                shoulder_numerator,
                shoulder,
                shoulder_offset,
                shoulder_denominator,
            ],
            inverse_toe: [-toe_denominator, -toe, toe_offset, -toe_numerator],
            inverse_shoulder: [
                -shoulder_denominator,
                shoulder,
                shoulder_offset,
                -shoulder_numerator,
            ],
        }
    }
    pub fn map(&self, value: f32) -> f32 {
        let c = if value < self.junctions[0] {
            self.toe
        } else {
            self.shoulder
        };
        (c[0] * value + c[2]) / (c[1] * value + c[3])
    }
    /// Divide by the exposure factor separately to recover world radiance.
    pub fn inverse(&self, value: f32) -> f32 {
        let c = if value < self.junctions[1] {
            self.inverse_toe
        } else {
            self.inverse_shoulder
        };
        (c[0] * value + c[2]) / (c[1] * value + c[3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coefficients_match_executable_including_inverse_parameters() {
        let mut cases = 0;
        for line in include_str!("../../tests/fixtures/tone_curve.txt").lines() {
            let v: Vec<f32> = line
                .split_whitespace()
                .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
                .collect();
            assert_eq!(v.len(), 26);
            let curve = ToneCurve::new(ToneSettings {
                overbright: v[0],
                black: v[1],
                white: v[2],
                junction: v[3],
                toe: v[4],
                shoulder: v[5],
            });
            for (i, (actual, expected)) in [
                curve.junctions,
                curve.toe,
                curve.shoulder,
                curve.inverse_toe,
                curve.inverse_shoulder,
            ]
            .into_iter()
            .flatten()
            .zip(v[6..].iter().copied())
            .enumerate()
            {
                assert!(
                    (actual - expected).abs() <= 3e-5 * expected.abs().max(1.),
                    "case{cases} word{i}: {actual} != {expected}"
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 96);
    }

    #[test]
    #[ignore = "requires installed levels and registered display declarations"]
    fn installed_display_settings_and_area_colour_cubes_resolve() {
        use crate::assets::{
            archive::Catalog, environment::ParameterReader, level::LevelResources,
        };
        use crate::entities;
        let c = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let empty = entities::EnvironmentRenderParameters { layers: Vec::new() };
        let defaults = DisplaySettings::resolve(&ParameterReader {
            catalog: &c,
            parameters: &empty,
        })
        .unwrap();
        assert_eq!(defaults.colour_cube, "_defaultColorCube");
        assert_eq!((defaults.gamma, defaults.brightness), (1., 1.));
        let validate_cube = |name: &str| {
            if name.eq_ignore_ascii_case("_defaultColorCube") {
                return;
            }
            let path = crate::material::compiled_image_stem(&c, name)
                .unwrap_or_else(|| panic!("unresolved display colour cube {name}"));
            assert!(
                c.read(&path).unwrap().is_some(),
                "missing colour cube {path}"
            );
        };
        let mut levels = 0;
        let mut area_cubes = 0;
        for path in c.paths_with_suffix(".bwm") {
            let level = LevelResources::resolve(&c, &path).unwrap();
            let Some(bytes) = level.read_entities(&c).unwrap() else {
                continue;
            };
            let doc = entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
            let parameters = if let Some(name) = entities::world_environment(&doc) {
                let data = c
                    .read(&format!("generated/decls/env/{name}.decl"))
                    .unwrap()
                    .unwrap();
                entities::environment_render_parameters_with_inheritance(
                    &name,
                    &String::from_utf8_lossy(&data),
                    |parent| {
                        c.read(&format!("generated/decls/env/{parent}.decl"))
                            .map(|v| v.map(|b| String::from_utf8_lossy(&b).into_owned()))
                            .map_err(|e| e.to_string())
                    },
                )
                .unwrap()
            } else {
                empty.clone()
            };
            let settings = DisplaySettings::resolve(&ParameterReader {
                catalog: &c,
                parameters: &parameters,
            })
            .unwrap();
            validate_cube(&settings.colour_cube);
            let curve = ToneCurve::new(settings.tone);
            assert!(
                [curve.junctions, curve.toe, curve.shoulder]
                    .into_iter()
                    .flatten()
                    .all(f32::is_finite),
                "{path}"
            );
            for area in crate::rendering::environment::load_area_environments(&doc, &c).unwrap() {
                if area
                    .parameters
                    .parameter("env/tonemapping/colorcube")
                    .is_some()
                {
                    let read = ParameterReader {
                        catalog: &c,
                        parameters: &area.parameters,
                    };
                    validate_cube(&read.image_source("env/tonemapping/colorcube").unwrap());
                    area_cubes += 1;
                }
            }
            levels += 1;
        }
        assert!(levels >= 36, "only{levels} installed levels audited");
        eprintln!("{levels} installed world display settings, {area_cubes} area colour-cube overrides resolved");
    }
}
